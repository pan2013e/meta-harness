use std::path::Path;

use mh_core::PermissionPolicy;

pub(crate) fn acp_provider(
    configured: &str,
    command: &str,
    args: &[String],
) -> Option<&'static str> {
    let executable = Path::new(command)
        .file_stem()
        .and_then(|name| name.to_str());
    if configured == "codex_local"
        || executable == Some("codex-acp")
        || args
            .iter()
            .any(|arg| arg == "@agentclientprotocol/codex-acp")
    {
        Some("codex_local")
    } else if configured == "claude_local"
        || executable == Some("claude-agent-acp")
        || args
            .iter()
            .any(|arg| arg == "@agentclientprotocol/claude-agent-acp")
    {
        Some("claude_local")
    } else {
        None
    }
}

pub(crate) fn acp_mode_candidates(
    provider: &str,
    policy: PermissionPolicy,
) -> &'static [&'static str] {
    use PermissionPolicy::{ApproveForMe, FullAccess, RequireApproval};

    match (provider, policy) {
        ("codex_local", ApproveForMe) => &["agent", "auto"],
        ("codex_local", FullAccess) => &["agent-full-access", "full-access"],
        ("codex_local", RequireApproval) => &["workspace-write", "read-only"],
        ("claude_local", ApproveForMe) => &["auto", "acceptEdits", "default"],
        ("claude_local", FullAccess) => &["bypassPermissions"],
        ("claude_local", RequireApproval) => &["acceptEdits", "default", "manual"],
        _ => &[],
    }
}

pub(crate) fn apply_launch_mode(command: &str, args: &mut Vec<String>, policy: PermissionPolicy) {
    if crate::profile::is_agy(command) && args.first().is_some_and(|arg| arg == "acp") {
        remove_flags(
            args,
            &["--sandbox", "--dangerously-skip-permissions"],
            false,
        );
        args.push(if policy == PermissionPolicy::FullAccess {
            "--dangerously-skip-permissions".into()
        } else {
            "--sandbox".into()
        });
    } else if Path::new(command)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("gemini"))
        && args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--acp" | "--experimental-acp"))
    {
        remove_flags(args, &["--approval-mode"], true);
        remove_flags(args, &["--yolo", "-y"], false);
        args.push(format!(
            "--approval-mode={}",
            match policy {
                PermissionPolicy::RequireApproval => "default",
                PermissionPolicy::ApproveForMe => "auto_edit",
                PermissionPolicy::FullAccess => "yolo",
            }
        ));
        if policy == PermissionPolicy::FullAccess {
            remove_flags(args, &["--sandbox", "--no-sandbox", "-s"], false);
            args.push("--sandbox=false".into());
        }
    }
}

fn remove_flags(args: &mut Vec<String>, flags: &[&str], takes_value: bool) {
    let mut input = std::mem::take(args).into_iter().peekable();
    while let Some(arg) = input.next() {
        let name = arg.split('=').next().unwrap_or_default();
        if flags.contains(&name) {
            if name == arg
                && input.peek().is_some_and(|value| {
                    !value.starts_with('-')
                        && (takes_value || matches!(value.as_str(), "true" | "false"))
                })
            {
                input.next();
            }
        } else {
            args.push(arg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_mode_replaces_google_cli_permission_flags() {
        let mut args = vec!["acp".into(), "--dangerously-skip-permissions=true".into()];
        apply_launch_mode("agy.exe", &mut args, PermissionPolicy::RequireApproval);
        assert_eq!(args, ["acp", "--sandbox"]);
        apply_launch_mode("agy.exe", &mut args, PermissionPolicy::FullAccess);
        assert_eq!(args, ["acp", "--dangerously-skip-permissions"]);

        let mut args = vec!["--acp".into(), "--approval-mode".into(), "yolo".into()];
        apply_launch_mode("gemini.cmd", &mut args, PermissionPolicy::ApproveForMe);
        assert_eq!(args, ["--acp", "--approval-mode=auto_edit"]);
    }
}
