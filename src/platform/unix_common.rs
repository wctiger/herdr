pub(crate) fn classify_child_exit(status: &portable_pty::ExitStatus) -> super::ChildExitReason {
    if status.signal().is_some() {
        super::ChildExitReason::Interrupted
    } else {
        super::ChildExitReason::Exited
    }
}

pub(crate) fn read_fd(fd: std::os::fd::RawFd, data: &mut [u8]) -> std::io::Result<usize> {
    let result = unsafe { libc::read(fd, data.as_mut_ptr().cast(), data.len()) };
    if result < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(result as usize)
    }
}

/// The tty's erase character (VERASE) on stdin, when one is set.
pub(super) fn terminal_erase_byte() -> Option<u8> {
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut termios) } != 0 {
        return None;
    }
    let erase = termios.c_cc[libc::VERASE];
    // 0 (Linux) and 0xff (BSD) disable the special character.
    (erase != 0 && erase != 0xff).then_some(erase)
}

pub(crate) fn poll_fd_readable(fd: std::os::fd::RawFd, timeout_ms: i32) -> std::io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
    if result < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(result > 0)
    }
}

pub(crate) fn shutdown_client_stream(stream: &crate::ipc::LocalStream) -> std::io::Result<()> {
    let crate::ipc::LocalStream::UdSocket(stream) = stream;
    stream.inner().shutdown(std::net::Shutdown::Both)
}

pub(crate) struct ClientStreamReader<'a>(pub(crate) &'a mut crate::ipc::LocalStream);

impl std::io::Read for ClientStreamReader<'_> {
    fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
        use std::os::fd::AsRawFd as _;

        loop {
            match self.0.read(data) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    let crate::ipc::LocalStream::UdSocket(stream) = &*self.0;
                    let mut descriptor = libc::pollfd {
                        fd: stream.inner().as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    // Sleep until input or shutdown, without polling quiet observers.
                    if unsafe { libc::poll(&mut descriptor, 1, -1) } < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.kind() != std::io::ErrorKind::Interrupted {
                            return Err(error);
                        }
                    }
                }
                result => return result,
            }
        }
    }
}

pub(crate) fn write_client_stream(
    stream: &crate::ipc::LocalStream,
    mut data: &[u8],
) -> std::io::Result<()> {
    use std::io::{self, Write as _};
    use std::os::fd::AsRawFd as _;
    use std::time::Instant;

    let crate::ipc::LocalStream::UdSocket(socket) = stream;
    let mut socket = socket.inner();
    let Some(timeout) = socket.write_timeout()? else {
        return socket.write_all(data);
    };
    let timed_out = || {
        // Dropping the writer clone alone would leave the reader blocked.
        let _ = shutdown_client_stream(stream);
        io::Error::new(
            io::ErrorKind::TimedOut,
            "terminal observer stopped receiving output",
        )
    };
    let mut progress = Instant::now();
    while !data.is_empty() {
        match socket.write(data) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                data = &data[written..];
                progress = Instant::now();
                continue;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        let remaining = timeout
            .checked_sub(progress.elapsed())
            .ok_or_else(timed_out)?;
        let mut descriptor = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        let wait_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut descriptor, 1, wait_ms) };
        if ready == 0 {
            return Err(timed_out());
        }
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    Ok(())
}

pub(crate) fn wait_client_stream_readable(
    stream: &crate::ipc::LocalStream,
    timeout: std::time::Duration,
) -> std::io::Result<()> {
    use std::os::fd::{AsFd as _, AsRawFd as _};
    let crate::ipc::LocalStream::UdSocket(stream) = stream;
    let mut descriptor = libc::pollfd {
        fd: stream.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // Bound cancellation latency without polling idle connections hundreds of times per second.
    let wait_ms = timeout.as_millis().clamp(1, 100) as i32;
    let result = unsafe { libc::poll(&mut descriptor, 1, wait_ms) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

pub(super) fn read_terminal_grid_size() -> std::io::Result<(u16, u16)> {
    crossterm::terminal::window_size().map(|size| (size.columns, size.rows))
}

fn set_sigpipe_disposition(handler: libc::sighandler_t) {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = handler;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        // Rust starts with SIGPIPE ignored. If this best-effort transition
        // fails, stdout retains the existing Rust behavior.
        libc::sigaction(libc::SIGPIPE, &action, std::ptr::null_mut());
    }
}

static LAST_SERVER_HANGUP_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn monotonic_millis() -> u64 {
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } != 0 {
        return 0;
    }
    (now.tv_sec as u64)
        .saturating_mul(1000)
        .saturating_add(now.tv_nsec as u64 / 1_000_000)
        .saturating_add(1)
}

fn record_server_hangup() {
    LAST_SERVER_HANGUP_MS.store(monotonic_millis(), std::sync::atomic::Ordering::Relaxed);
}

fn within_server_hangup_window(hangup_ms: u64, now_ms: u64) -> bool {
    hangup_ms != 0
        && now_ms
            .checked_sub(hangup_ms)
            .is_some_and(|elapsed| elapsed <= 5_000)
}

pub(crate) fn server_hangup_is_recent() -> bool {
    within_server_hangup_window(
        LAST_SERVER_HANGUP_MS.load(std::sync::atomic::Ordering::Relaxed),
        monotonic_millis(),
    )
}

/// The server must outlive the terminal or shell that launched it, like tmux.
/// Installed at process start so SIGHUP cannot stop the server before its event
/// loop runs. A handler, unlike SIG_IGN, resets to the default on exec, so
/// child processes keep normal hangup behavior.
pub(crate) fn ignore_server_hangup() {
    // Record before async pane-exit handling, even if Tokio has not polled SIGHUP yet.
    if let Err(err) =
        unsafe { signal_hook::low_level::register(libc::SIGHUP, record_server_hangup) }
    {
        tracing::warn!(%err, "failed to install SIGHUP handler");
    }
}

/// Routes SIGINT and SIGTERM to `on_quit`, and logs SIGHUP without stopping.
pub(crate) fn spawn_server_signal_monitor(
    on_quit: impl Fn(super::ServerQuitSignal) + Send + Sync + 'static,
) {
    use tokio::signal::unix::{signal, SignalKind};
    use tracing::{info, warn};

    let on_quit = std::sync::Arc::new(on_quit);
    for (kind, quit_signal) in [
        (SignalKind::interrupt(), super::ServerQuitSignal::Interrupt),
        (SignalKind::terminate(), super::ServerQuitSignal::Terminate),
    ] {
        match signal(kind) {
            Ok(mut stream) => {
                let on_quit = on_quit.clone();
                tokio::spawn(async move {
                    while stream.recv().await.is_some() {
                        on_quit(quit_signal);
                    }
                });
            }
            Err(err) => warn!(%err, signal = %quit_signal, "failed to install server stop handler"),
        }
    }
    match signal(SignalKind::hangup()) {
        Ok(mut stream) => {
            tokio::spawn(async move {
                while stream.recv().await.is_some() {
                    info!("ignoring SIGHUP; server keeps running");
                }
            });
        }
        Err(err) => warn!(%err, "failed to install SIGHUP handler"),
    }
}

/// Describes the process on the other end of a local socket, for logs.
pub(crate) fn local_stream_peer_description(stream: &crate::ipc::LocalStream) -> Option<String> {
    use std::os::fd::{AsFd as _, AsRawFd as _};

    let crate::ipc::LocalStream::UdSocket(socket) = stream;
    let pid = super::socket_peer_pid(socket.as_fd().as_raw_fd())?;
    let Some((name, parent)) = super::process_name_and_parent(pid) else {
        return Some(format!("pid {pid}"));
    };
    let mut description = format!("pid {pid} ({name})");
    if parent > 1 {
        description.push_str(&format!(", parent pid {parent}"));
        if let Some((parent_name, _)) = super::process_name_and_parent(parent) {
            description.push_str(&format!(" ({parent_name})"));
        }
    }
    Some(description)
}

pub(crate) fn begin_cli_output() {
    set_sigpipe_disposition(libc::SIG_DFL);
}

/// The machine's node name, as shown by tmux's `#h`.
pub(crate) fn hostname() -> Option<String> {
    let mut buffer = [0_u8; 256];
    let result =
        unsafe { libc::gethostname(buffer.as_mut_ptr().cast::<libc::c_char>(), buffer.len()) };
    if result != 0 {
        return None;
    }
    let end = buffer
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(buffer.len());
    let name = String::from_utf8_lossy(&buffer[..end]).into_owned();
    (!name.is_empty()).then_some(name)
}

pub(crate) fn local_datetime() -> Option<time::PrimitiveDateTime> {
    let mut timestamp: libc::time_t = 0;
    if unsafe { libc::time(&mut timestamp) } == -1 {
        return None;
    }
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&timestamp, &mut local) }.is_null() {
        return None;
    }
    datetime_from_tm(&local)
}

pub(crate) fn status_commands_supported() -> bool {
    true
}

pub(crate) fn configure_status_command(process: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    process.process_group(0);
}

pub(crate) struct StatusCommandGuard {
    process_group_id: Option<i32>,
}

impl StatusCommandGuard {
    pub(crate) fn new(child: &tokio::process::Child) -> std::io::Result<Self> {
        let process_id = child
            .id()
            .ok_or_else(|| std::io::Error::other("status command has no process id"))?;
        let process_group_id = i32::try_from(process_id)
            .map_err(|_| std::io::Error::other("status command process id exceeds i32"))?;
        Ok(Self {
            process_group_id: Some(process_group_id),
        })
    }
}

impl StatusCommandGuard {
    pub(crate) fn terminate(&mut self) {
        if let Some(process_group_id) = self.process_group_id.take() {
            // The command was spawned as this process group's leader. Killing the
            // group also cleans up background descendants on completion/cancellation.
            unsafe {
                libc::kill(-process_group_id, libc::SIGKILL);
            }
        }
    }
}

impl Drop for StatusCommandGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn datetime_from_tm(value: &libc::tm) -> Option<time::PrimitiveDateTime> {
    let month = time::Month::try_from(u8::try_from(value.tm_mon + 1).ok()?).ok()?;
    let date = time::Date::from_calendar_date(
        value.tm_year + 1900,
        month,
        u8::try_from(value.tm_mday).ok()?,
    )
    .ok()?;
    let time = time::Time::from_hms(
        u8::try_from(value.tm_hour).ok()?,
        u8::try_from(value.tm_min).ok()?,
        u8::try_from(value.tm_sec).ok()?,
    )
    .ok()?;
    Some(time::PrimitiveDateTime::new(date, time))
}

pub(crate) fn set_default_plugin_pane_pwd(env: &mut Vec<(String, String)>, cwd: &std::path::Path) {
    if !env.iter().any(|(key, _)| key == "PWD") {
        env.push(("PWD".to_string(), cwd.display().to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_hangup_evidence_expires() {
        assert!(!within_server_hangup_window(0, 100));
        assert!(within_server_hangup_window(100, 100));
        assert!(within_server_hangup_window(100, 5_100));
        assert!(!within_server_hangup_window(100, 5_101));
        assert!(!within_server_hangup_window(100, 99));
    }

    #[test]
    fn server_hangup_is_recorded_before_async_handling() {
        ignore_server_hangup();
        assert_eq!(unsafe { libc::raise(libc::SIGHUP) }, 0);
        assert!(server_hangup_is_recent());
    }

    #[test]
    fn server_signal_monitor_survives_hangup_and_reports_terminate() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            ignore_server_hangup();
            spawn_server_signal_monitor(move |signal| {
                let _ = tx.send(signal);
            });

            // An unhandled SIGHUP would end the test process here.
            // raise() runs the handler on this thread before returning, unlike kill().
            assert_eq!(unsafe { libc::raise(libc::SIGHUP) }, 0);
            assert!(server_hangup_is_recent());
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            assert!(rx.try_recv().is_err());

            assert_eq!(unsafe { libc::kill(libc::getpid(), libc::SIGTERM) }, 0);
            let signal = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .unwrap();
            assert_eq!(signal, Some(super::super::ServerQuitSignal::Terminate));
        });
    }

    #[test]
    fn plugin_pane_pwd_defaults_to_cwd_without_overriding_explicit_env() {
        let cwd = std::path::Path::new("/plugin-cwd");
        let mut derived = vec![("OTHER".to_string(), "value".to_string())];
        set_default_plugin_pane_pwd(&mut derived, cwd);
        assert!(derived.contains(&("PWD".to_string(), "/plugin-cwd".to_string())));

        let mut explicit = vec![("PWD".to_string(), "/caller-pwd".to_string())];
        set_default_plugin_pane_pwd(&mut explicit, cwd);
        assert_eq!(explicit, [("PWD".to_string(), "/caller-pwd".to_string())]);
    }
}
