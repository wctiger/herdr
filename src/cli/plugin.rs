use std::path::Path;

use super::projection::{self, ParseError};
use crate::api::schema::{
    InstalledPluginInfo, Method, PluginLinkParams, PluginListParams, PluginSourceKind,
    PluginUnlinkParams, Request, ResponseResult, SuccessResponse,
};

pub(crate) mod bindings;

pub(crate) fn command() -> clap::Command {
    bindings::command()
}

const PLUGIN_INSTALL_USAGE: &str =
    "usage: sy-term plugin install [--ref REF] [--yes|-y] <owner>/<repo>[/subdir...]";

pub(super) fn run_plugin_command(args: &[String]) -> std::io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("uninstall") => return plugin_uninstall(&args[1..]),
        Some("config-dir") => return plugin_config_dir_command(&args[1..]),
        _ => {}
    }
    let (method, json) = match projection::parse(
        command().bin_name("sy-term plugin"),
        args,
        bindings::from_matches,
    ) {
        Ok(parsed) => parsed,
        Err(ParseError::Io(error)) => return Err(error),
        Err(ParseError::Usage(message)) => {
            eprintln!("{message}");
            return Ok(2);
        }
        Err(ParseError::Clap(error)) => return super::print_clap_error(&error),
    };
    match method {
        Method::PluginLink(params) => plugin_link(params),
        Method::PluginList(params) => plugin_list(params, json),
        method => print_plugin_response(method),
    }
}

fn plugin_link(params: PluginLinkParams) -> std::io::Result<i32> {
    let response = match super::send_request(&Request {
        id: "cli:plugin".into(),
        method: Method::PluginLink(params.clone()),
    }) {
        Ok(response) => response,
        Err(err) if is_connection_error(&err) => offline_plugin_link_response(&params)?,
        Err(err) => return Err(err),
    };
    super::print_response(&response)
}

fn plugin_config_dir_command(args: &[String]) -> std::io::Result<i32> {
    let Some(plugin_id) = args.first() else {
        eprintln!("usage: sy-term plugin config-dir <plugin_id>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: sy-term plugin config-dir <plugin_id>");
        return Ok(2);
    }
    let path = crate::plugin_paths::plugin_config_dir(plugin_id);
    crate::plugin_paths::ensure_plugin_user_dirs(plugin_id)?;
    println!("{}", path.display());
    Ok(0)
}

fn plugin_list(params: PluginListParams, json: bool) -> std::io::Result<i32> {
    let response = match super::send_request(&Request {
        id: "cli:plugin".into(),
        method: Method::PluginList(params.clone()),
    }) {
        Ok(response) => response,
        Err(err) if is_connection_error(&err) => offline_plugin_list_response(&params)?,
        Err(err) => return Err(err),
    };
    if json {
        return super::print_response(&response);
    }
    print_plugin_list_human(&response)
}

fn plugin_uninstall(args: &[String]) -> std::io::Result<i32> {
    let Some(target) = args.first() else {
        eprintln!("usage: sy-term plugin uninstall <plugin_id|owner/repo[/subdir...]>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: sy-term plugin uninstall <plugin_id|owner/repo[/subdir...]>");
        return Ok(2);
    }

    let (plugin_id, existing) = match GithubPluginSource::parse(target) {
        Ok(source) => {
            let existing = installed_plugin_by_github_source(&source)?;
            let Some(existing) = existing else {
                eprintln!("plugin not installed: {}", source.display());
                return Ok(1);
            };
            (existing.plugin_id.clone(), Some(existing))
        }
        Err(_) => {
            let existing = match live_installed_plugin_info(target) {
                Ok(plugin) => plugin,
                Err(err) if is_connection_error(&err) => registry_plugin_info(target),
                Err(err) => return Err(err),
            };
            (target.clone(), existing)
        }
    };
    let _checkout_lock = existing
        .as_ref()
        .filter(|plugin| plugin.source.kind == PluginSourceKind::Github)
        .map(|_| lock_managed_checkout(&plugin_id))
        .transpose()?;

    match super::send_request(&Request {
        id: "cli:plugin".into(),
        method: Method::PluginUnlink(PluginUnlinkParams {
            plugin_id: plugin_id.clone(),
        }),
    }) {
        Ok(response) => {
            if response.get("error").is_some() {
                return super::print_response(&response);
            }
            if response["result"]["removed"].as_bool() == Some(false) {
                eprintln!("plugin not installed: {target}");
                return Ok(1);
            }
        }
        Err(err) if is_connection_error(&err) => {
            let (removed, _) = crate::persist::plugin_registry::update(|plugins| {
                let before = plugins.len();
                plugins.retain(|plugin| plugin.plugin_id != plugin_id);
                before != plugins.len()
            })?;
            if !removed {
                eprintln!("plugin not installed: {target}");
                return Ok(1);
            }
        }
        Err(err) => return Err(err),
    }

    println!("Uninstalled {plugin_id}.");
    Ok(0)
}

fn normalize_plugin_path_arg(value: &str) -> std::io::Result<String> {
    let path = crate::worktree::expand_tilde_path(value);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(absolute.display().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubPluginSource {
    owner: String,
    repo: String,
    subdir: Option<String>,
}

impl GithubPluginSource {
    fn parse(value: &str) -> Result<Self, String> {
        if value.starts_with("http://")
            || value.starts_with("https://")
            || value.starts_with("git@")
            || value.contains(':')
        {
            return Err("plugin install v1 accepts only owner/repo[/subdir] shorthand".into());
        }
        let parts = value.split('/').collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err(PLUGIN_INSTALL_USAGE.into());
        }
        let owner = parts[0];
        let repo = parts[1];
        validate_github_segment("owner", owner)?;
        validate_github_segment("repo", repo)?;
        let subdir_parts = &parts[2..];
        for part in subdir_parts {
            validate_subdir_segment(part)?;
        }
        let subdir = if subdir_parts.is_empty() {
            None
        } else {
            Some(subdir_parts.join("/"))
        };
        Ok(Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            subdir,
        })
    }

    fn display(&self) -> String {
        match &self.subdir {
            Some(subdir) => format!("{}/{}/{}", self.owner, self.repo, subdir),
            None => format!("{}/{}", self.owner, self.repo),
        }
    }
}

fn validate_github_segment(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("GitHub {label} must not be empty"));
    }
    if value == "." || value == ".." {
        return Err(format!("GitHub {label} is invalid: {value}"));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!(
            "GitHub {label} contains invalid characters: {value}"
        ));
    }
    Ok(())
}

fn validate_subdir_segment(value: &str) -> Result<(), String> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(format!("invalid plugin subdir segment: {value}"));
    }
    if value.contains('\\') || value.contains('\0') {
        return Err(format!("invalid plugin subdir segment: {value}"));
    }
    Ok(())
}

fn load_cli_plugin_manifest(path: &Path, enabled: bool) -> std::io::Result<InstalledPluginInfo> {
    crate::app::load_plugin_manifest(&path.display().to_string(), enabled)
        .map_err(|(_, message)| std::io::Error::other(message))
}

fn persist_plugin_offline(
    plugin: &InstalledPluginInfo,
    preserve_enabled: bool,
) -> std::io::Result<()> {
    crate::plugin_paths::ensure_plugin_user_dirs(&plugin.plugin_id)?;
    crate::persist::plugin_registry::update(|plugins| {
        let mut plugin = plugin.clone();
        if preserve_enabled {
            if let Some(existing) = plugins
                .iter()
                .find(|entry| entry.plugin_id == plugin.plugin_id)
            {
                plugin.enabled = existing.enabled;
            }
        }
        plugins.retain(|entry| entry.plugin_id != plugin.plugin_id);
        plugins.push(plugin);
    })?;
    Ok(())
}

fn live_installed_plugin_info(plugin_id: &str) -> std::io::Result<Option<InstalledPluginInfo>> {
    let response = super::send_request(&Request {
        id: "cli:plugin".into(),
        method: Method::PluginList(PluginListParams {
            plugin_id: Some(plugin_id.to_string()),
        }),
    })?;
    if response.get("error").is_some() {
        return Err(std::io::Error::other(
            serde_json::to_string(&response).unwrap(),
        ));
    }
    plugin_info_from_list_response(response)
}

fn installed_plugin_by_github_source(
    source: &GithubPluginSource,
) -> std::io::Result<Option<InstalledPluginInfo>> {
    let plugins = match live_installed_plugins() {
        Ok(plugins) => plugins,
        Err(err) if is_connection_error(&err) => registry_plugins(),
        Err(err) => return Err(err),
    };
    Ok(plugin_by_github_source(plugins, source))
}

fn live_installed_plugins() -> std::io::Result<Vec<InstalledPluginInfo>> {
    let response = super::send_request(&Request {
        id: "cli:plugin".into(),
        method: Method::PluginList(PluginListParams { plugin_id: None }),
    })?;
    if response.get("error").is_some() {
        return Err(std::io::Error::other(
            serde_json::to_string(&response).unwrap(),
        ));
    }
    plugin_list_from_response(response)
}

fn registry_plugin_info(plugin_id: &str) -> Option<InstalledPluginInfo> {
    registry_plugins()
        .into_iter()
        .find(|plugin| plugin.plugin_id == plugin_id)
}

fn registry_plugins() -> Vec<InstalledPluginInfo> {
    crate::persist::plugin_registry::load()
}

fn plugin_info_from_list_response(
    response: serde_json::Value,
) -> std::io::Result<Option<InstalledPluginInfo>> {
    let mut plugins = plugin_list_from_response(response)?;
    Ok(plugins.pop())
}

fn plugin_list_from_response(
    response: serde_json::Value,
) -> std::io::Result<Vec<InstalledPluginInfo>> {
    let parsed: SuccessResponse =
        serde_json::from_value(response).map_err(std::io::Error::other)?;
    let ResponseResult::PluginList { mut plugins } = parsed.result else {
        return Err(std::io::Error::other("expected plugin_list response"));
    };
    plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
    Ok(plugins)
}

fn plugin_by_github_source(
    plugins: impl IntoIterator<Item = InstalledPluginInfo>,
    source: &GithubPluginSource,
) -> Option<InstalledPluginInfo> {
    plugins
        .into_iter()
        .find(|plugin| plugin_matches_github_source(plugin, source))
}

fn plugin_matches_github_source(plugin: &InstalledPluginInfo, source: &GithubPluginSource) -> bool {
    plugin.source.kind == PluginSourceKind::Github
        && plugin
            .source
            .owner
            .as_deref()
            .is_some_and(|owner| owner.eq_ignore_ascii_case(&source.owner))
        && plugin
            .source
            .repo
            .as_deref()
            .is_some_and(|repo| repo.eq_ignore_ascii_case(&source.repo))
        && plugin.source.subdir.as_deref() == source.subdir.as_deref()
}

fn offline_plugin_link_response(params: &PluginLinkParams) -> std::io::Result<serde_json::Value> {
    let plugin = load_cli_plugin_manifest(Path::new(&params.path), params.enabled)?;
    persist_plugin_offline(&plugin, false)?;
    serde_json::to_value(SuccessResponse {
        id: "cli:plugin".into(),
        result: ResponseResult::PluginLinked { plugin },
    })
    .map_err(std::io::Error::other)
}

fn offline_plugin_list_response(params: &PluginListParams) -> std::io::Result<serde_json::Value> {
    let entries = crate::persist::plugin_registry::load();
    let mut plugins =
        crate::persist::plugin_registry::reload_manifests(entries, |path, enabled| {
            crate::app::load_plugin_manifest(path, enabled).map_err(|(_, msg)| msg)
        })
        .into_iter()
        .filter(|plugin| {
            params
                .plugin_id
                .as_deref()
                .is_none_or(|plugin_id| plugin.plugin_id == plugin_id)
        })
        .collect::<Vec<_>>();
    plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
    serde_json::to_value(SuccessResponse {
        id: "cli:plugin".into(),
        result: ResponseResult::PluginList { plugins },
    })
    .map_err(std::io::Error::other)
}

fn print_plugin_list_human(response: &serde_json::Value) -> std::io::Result<i32> {
    if response.get("error").is_some() {
        return super::print_response(response);
    }
    let parsed: SuccessResponse =
        serde_json::from_value(response.clone()).map_err(std::io::Error::other)?;
    let ResponseResult::PluginList { plugins } = parsed.result else {
        return super::print_response(response);
    };
    if plugins.is_empty() {
        println!("No plugins installed.");
        return Ok(0);
    }
    println!(
        "{} plugin{} installed:",
        plugins.len(),
        if plugins.len() == 1 { "" } else { "s" }
    );
    for plugin in plugins {
        let enabled = if plugin.enabled {
            "enabled"
        } else {
            "disabled"
        };
        let warning = if plugin.warnings.is_empty() {
            String::new()
        } else {
            format!("; {} warning(s)", plugin.warnings.len())
        };
        println!(
            "- {} ({}) {} [{}{}]",
            plugin.plugin_id,
            plugin.name,
            enabled,
            source_display(&plugin),
            warning
        );
        println!(
            "  config: {}",
            crate::plugin_paths::plugin_config_dir(&plugin.plugin_id).display()
        );
        for warning in plugin.warnings {
            println!("  warning: {warning}");
        }
    }
    Ok(0)
}

fn source_display(plugin: &InstalledPluginInfo) -> String {
    match plugin.source.kind {
        PluginSourceKind::Github => {
            let owner = plugin.source.owner.as_deref().unwrap_or("unknown");
            let repo = plugin.source.repo.as_deref().unwrap_or("unknown");
            let subdir = plugin
                .source
                .subdir
                .as_deref()
                .map(|subdir| format!("/{subdir}"))
                .unwrap_or_default();
            let reference = plugin
                .source
                .requested_ref
                .as_deref()
                .or(plugin.source.resolved_commit.as_deref())
                .unwrap_or("unknown");
            format!("github:{owner}/{repo}{subdir}@{reference}")
        }
        PluginSourceKind::Local => format!("local:{}", plugin.plugin_root),
    }
}

fn lock_managed_checkout(plugin_id: &str) -> std::io::Result<std::fs::File> {
    let lock_path = crate::plugin_paths::managed_checkout_lock_path(plugin_id);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock()?;
    Ok(lock)
}

fn is_connection_error(err: &std::io::Error) -> bool {
    // A `server_not_running` marker is a connect failure for recovery purposes:
    // treating it as a connection error lets plugin commands fall back to the
    // offline registry. The marker carries (but does not print) a friendly
    // response, so recovering here prints nothing.
    super::server_not_running_was_reported(err)
        || matches!(
            err.kind(),
            std::io::ErrorKind::NotFound
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
        )
}

fn print_plugin_response(method: Method) -> std::io::Result<i32> {
    super::print_response(&super::send_request(&Request {
        id: "cli:plugin".into(),
        method,
    })?)
}

#[cfg(test)]
fn unique_plugin_id(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("test.{label}.{}.{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_plugin_source_parses_subdir() {
        let source =
            GithubPluginSource::parse("ogulcancelik/herdr-plugin-examples/worktree-bootstrap")
                .unwrap();
        assert_eq!(source.owner, "ogulcancelik");
        assert_eq!(source.repo, "herdr-plugin-examples");
        assert_eq!(source.subdir.as_deref(), Some("worktree-bootstrap"));
    }

    #[test]
    fn github_plugin_source_rejects_non_shorthand_sources() {
        for source in [
            "https://github.com/ogulcancelik/herdr-plugin-examples",
            "git@github.com:ogulcancelik/herdr-plugin-examples.git",
            "ogulcancelik",
            "ogulcancelik/herdr-plugin-examples/../bad",
            "ogulcancelik/herdr-plugin-examples//bad",
        ] {
            assert!(
                GithubPluginSource::parse(source).is_err(),
                "{source} should be rejected"
            );
        }
    }

    #[test]
    fn cli_user_dir_creation_seeds_legacy_config_before_printing_config_dir() {
        let plugin_id = unique_plugin_id("legacy-config");
        let config_dir = crate::plugin_paths::plugin_config_dir(&plugin_id);
        let state_dir = crate::plugin_paths::plugin_state_dir(&plugin_id);
        let legacy_dir = crate::config::config_dir().join("plugins").join(&plugin_id);
        let _ = std::fs::remove_dir_all(&config_dir);
        let _ = std::fs::remove_dir_all(&state_dir);
        let _ = std::fs::remove_dir_all(&legacy_dir);
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join(".env"), "TOKEN=legacy\n").unwrap();

        assert_eq!(
            plugin_config_dir_command(std::slice::from_ref(&plugin_id)).unwrap(),
            0
        );

        assert_eq!(
            std::fs::read_to_string(config_dir.join(".env")).unwrap(),
            "TOKEN=legacy\n"
        );

        let _ = std::fs::remove_dir_all(config_dir);
        let _ = std::fs::remove_dir_all(state_dir);
        let _ = std::fs::remove_dir_all(legacy_dir);
    }
}
