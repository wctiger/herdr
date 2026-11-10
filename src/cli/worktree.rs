#[path = "worktree_bindings.rs"]
pub(crate) mod bindings;

pub(crate) use bindings::command;

pub(super) fn run_worktree_command(args: &[String]) -> std::io::Result<i32> {
    super::run_json_request(bindings::request(args))
}

fn normalize_path_arg(value: &str) -> std::io::Result<String> {
    let path = crate::worktree::expand_tilde_path(value);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(absolute.display().to_string())
}

#[cfg(test)]
mod machine_tests {
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn simple_families_worktree_rejects_invalid_grammar_before_transport() {
        for values in [
            vec!["list", "--workspace"],
            vec!["list", "--workspace", "2", "--cwd", "."],
            vec!["create", "--branch"],
            vec!["create", "--base"],
            vec!["create", "--path"],
            vec!["create", "--label"],
            vec!["create", "--focus=true"],
            vec!["create", "--workspace", "2", "--cwd", "."],
            vec!["open"],
            vec!["open", "--json"],
            vec!["open", "--path", ".", "--branch", "topic"],
            vec![
                "open",
                "--workspace",
                "2",
                "--cwd",
                ".",
                "--branch",
                "topic",
            ],
            vec!["remove"],
            vec!["remove", "--json", "--force", "--trust-repository"],
            vec!["remove", "--workspace"],
        ] {
            assert!(
                super::bindings::request(&args(&values)).is_err(),
                "argv: {values:?}"
            );
        }
    }

    #[test]
    fn simple_families_worktree_dto_literal_defaults() {
        use crate::api::schema::{
            WorktreeCreateParams, WorktreeListParams, WorktreeOpenParams, WorktreeRemoveParams,
        };
        assert_eq!(
            serde_json::to_value(WorktreeListParams::default()).unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            serde_json::to_value(WorktreeCreateParams::default()).unwrap(),
            serde_json::json!({"focus": false})
        );
        assert_eq!(
            serde_json::to_value(WorktreeOpenParams::default()).unwrap(),
            serde_json::json!({"focus": false})
        );
        let remove: WorktreeRemoveParams =
            serde_json::from_value(serde_json::json!({"workspace_id": "ws-2"})).unwrap();
        assert!(!remove.force);
        assert!(!remove.trust_repository);
        assert_eq!(
            serde_json::to_value(remove).unwrap(),
            serde_json::json!({"workspace_id": "ws-2", "force": false})
        );
    }

}
