use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use mh_core::AdapterConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;

use crate::AcpError;
use crate::platform::{
    ProcessTreeGuard, find_cli_binary, prepare_isolated_command, resolve_command,
};
use crate::profile::{apply_profile, is_agy};
use crate::supervisor::{codex_model_command, resolve_adapter_command};

const CODEX_ACP: &str = "@agentclientprotocol/codex-acp";

#[derive(Clone, Copy, PartialEq, Eq)]
enum CliKind {
    Codex,
    Claude,
    Agy,
    Gemini,
}

fn kind(adapter: &AdapterConfig) -> Option<CliKind> {
    if !adapter.enabled {
        return None;
    }
    match adapter.quota_provider.as_str() {
        "codex_local" => Some(CliKind::Codex),
        "claude_local" if find_cli_binary("claude").is_some() => Some(CliKind::Claude),
        "google_local" => {
            let (program, _) = resolve_adapter_command(adapter).ok()?;
            if is_agy(&program) {
                Some(CliKind::Agy)
            } else if Path::new(&program)
                .file_stem()
                .is_some_and(|name| name.eq_ignore_ascii_case("gemini"))
            {
                Some(CliKind::Gemini)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn adapter_binary(adapter: &AdapterConfig, kind: CliKind) -> Option<Command> {
    let binary = match kind {
        CliKind::Codex => return codex_model_command(adapter),
        CliKind::Claude => find_cli_binary("claude")?,
        CliKind::Agy | CliKind::Gemini => resolve_adapter_command(adapter).ok()?.0.into(),
    };
    let (program, args) = resolve_command(&binary.to_string_lossy(), &[]).ok()?;
    let mut command = Command::new(program);
    command.args(args);
    Some(command)
}

fn npm_codex_launcher(adapter: &AdapterConfig) -> bool {
    adapter.command == "npx"
        && adapter.args == ["-y", CODEX_ACP]
        && std::env::var_os("CODEX_PATH").is_none_or(|value| value.is_empty())
}

fn npm_command(args: &[&str], binary: &str) -> Option<Command> {
    let args: Vec<_> = args.iter().map(|arg| (*arg).to_string()).collect();
    let (program, args) = resolve_command(binary, &args).ok()?;
    let mut command = Command::new(program);
    command.args(args);
    Some(command)
}

pub fn cli_upgrade_command(adapter: &AdapterConfig) -> Option<Command> {
    let kind = kind(adapter)?;
    if kind == CliKind::Codex && npm_codex_launcher(adapter) {
        let (launcher, _) = resolve_adapter_command(adapter).ok()?;
        let cached = Path::new(&launcher)
            .components()
            .any(|part| part.as_os_str() == "_npx")
            || Path::new(&launcher)
                .file_stem()
                .is_some_and(|name| name.eq_ignore_ascii_case("npx"));
        if cached {
            return npm_command(&["--prefer-online", "-y", CODEX_ACP, "--version"], "npx");
        }
        if Path::new(&launcher).parent().is_some_and(|parent| {
            parent
                .join("node_modules")
                .join(CODEX_ACP)
                .join("package.json")
                .is_file()
                || parent
                    .join("../lib/node_modules")
                    .join(CODEX_ACP)
                    .join("package.json")
                    .is_file()
        }) {
            return npm_command(
                &["install", "-g", "@agentclientprotocol/codex-acp@latest"],
                "npm",
            );
        }
        return None;
    }
    let mut command = adapter_binary(adapter, kind)?;
    if kind == CliKind::Gemini || (kind == CliKind::Codex && command.as_std().get_args().len() > 0)
    {
        return None;
    }
    command.arg("update");
    Some(command)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexLoginMode {
    #[default]
    Browser,
    DeviceCode,
}

fn codex_login_args(mode: CodexLoginMode) -> Vec<String> {
    let mut args = vec!["login".into()];
    if mode == CodexLoginMode::DeviceCode {
        args.push("--device-auth".into());
    }
    args
}

pub async fn cli_login_command(
    adapter: &AdapterConfig,
    mode: CodexLoginMode,
) -> Result<Command, AcpError> {
    let unavailable =
        || AcpError::InvalidProfile("This installed launcher has no web sign-in workflow.".into());
    let kind = kind(adapter).ok_or_else(unavailable)?;
    if kind == CliKind::Gemini {
        return Err(unavailable());
    }
    if mode == CodexLoginMode::DeviceCode && kind != CliKind::Codex {
        return Err(AcpError::InvalidProfile(
            "Device-code sign-in is available for Codex only.".into(),
        ));
    }
    let mut process = adapter_binary(adapter, kind).ok_or_else(unavailable)?;
    let mut args = match kind {
        CliKind::Codex => codex_login_args(mode),
        CliKind::Claude => vec!["auth".into(), "login".into()],
        CliKind::Agy => vec!["acp".into(), "--login".into()],
        CliKind::Gemini => return Err(unavailable()),
    };
    let mut env = Vec::new();
    apply_profile(
        adapter,
        &process.as_std().get_program().to_string_lossy(),
        &mut args,
        &mut env,
    )
    .await?;
    process
        .args(args)
        .envs(env)
        .stdin(if kind == CliKind::Codex {
            Stdio::null()
        } else {
            Stdio::piped()
        });
    if kind == CliKind::Agy {
        process.env("AGY_CLI_DISABLE_AUTO_UPDATE", "true");
    }
    Ok(process)
}

#[derive(Serialize)]
pub struct CliVersion {
    pub installed: Option<String>,
    pub published: Option<String>,
    pub can_upgrade: bool,
    pub can_sign_in: bool,
}

fn parse_version(text: &str) -> Option<String> {
    text.split(|ch: char| !ch.is_ascii_digit() && ch != '.')
        .find_map(|part| {
            let parts = part.split('.').collect::<Vec<_>>();
            (parts.len() == 3 && parts.iter().all(|part| part.parse::<u32>().is_ok()))
                .then(|| part.to_string())
        })
}

async fn command_output(mut command: Command) -> Option<String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    prepare_isolated_command(&mut command);
    let child = command.spawn().ok()?;
    let mut guard = ProcessTreeGuard::attach(&child);
    let out = tokio::time::timeout(Duration::from_secs(12), child.wait_with_output()).await;
    guard.kill_tree();
    let out = out.ok()?.ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn published_agy_version() -> Option<String> {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "darwin",
        "linux" => "linux",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return None,
    };
    let libc = if cfg!(all(target_os = "linux", target_env = "musl")) {
        "_musl"
    } else {
        ""
    };
    // This is the same release manifest used by Google's official CLI installer.
    let manifest = format!(
        "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/{os}_{arch}{libc}.json"
    );
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let value: Value = http
        .get(manifest)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    parse_version(value.get("version")?.as_str()?)
}

pub async fn cli_version(adapter: &AdapterConfig) -> CliVersion {
    let kind = kind(adapter);
    let installed = if let Some(mut command) = kind.and_then(|kind| adapter_binary(adapter, kind)) {
        command
            .arg("--version")
            .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true");
        command_output(command)
            .await
            .as_deref()
            .and_then(parse_version)
    } else {
        None
    };
    let published = match kind {
        Some(CliKind::Agy) => published_agy_version().await,
        Some(kind) => {
            let package = match kind {
                CliKind::Codex if npm_codex_launcher(adapter) => "@agentclientprotocol%2fcodex-acp",
                CliKind::Codex => "@openai%2fcodex",
                CliKind::Claude => "@anthropic-ai%2fclaude-code",
                CliKind::Gemini => "@google%2fgemini-cli",
                CliKind::Agy => "",
            };
            let result = async {
                let http = reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .ok()?;
                let version: Value = http
                    .get(format!("https://registry.npmjs.org/{package}/latest"))
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                if kind == CliKind::Codex && npm_codex_launcher(adapter) {
                    parse_version(
                        version
                            .get("dependencies")?
                            .get("@openai/codex")?
                            .as_str()?,
                    )
                } else {
                    parse_version(version.get("version")?.as_str()?)
                }
            };
            result.await
        }
        None => None,
    };
    CliVersion {
        installed,
        published,
        can_upgrade: cli_upgrade_command(adapter).is_some(),
        can_sign_in: kind.is_some_and(|kind| kind != CliKind::Gemini),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_login_uses_browser_by_default_and_device_code_on_request() {
        assert_eq!(codex_login_args(CodexLoginMode::default()), ["login"]);
        assert_eq!(
            codex_login_args(CodexLoginMode::DeviceCode),
            ["login", "--device-auth"]
        );
    }
}
