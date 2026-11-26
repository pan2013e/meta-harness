use std::path::{Path, PathBuf};

use mh_core::AdapterConfig;

use crate::AcpError;

const PROFILE_SESSION_PREFIX: &str = "mh-profile:";

pub fn store_profile_session(profile: Option<&Path>, id: &str) -> String {
    match profile {
        Some(profile) => format!(
            "{PROFILE_SESSION_PREFIX}{}",
            serde_json::json!([profile, id])
        ),
        None => id.to_string(),
    }
}

pub fn resume_profile_session(profile: Option<&Path>, stored: &str) -> Option<String> {
    match (profile, stored.strip_prefix(PROFILE_SESSION_PREFIX)) {
        (None, None) => Some(stored.to_string()),
        (Some(profile), Some(reference)) => {
            let (previous, id): (PathBuf, String) = serde_json::from_str(reference).ok()?;
            (previous == profile).then_some(id)
        }
        _ => None,
    }
}

pub(crate) fn profile_home(
    profile: Option<&Path>,
    variable: &str,
    default_dir: &str,
) -> Option<PathBuf> {
    profile
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os(variable)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| dirs::home_dir().map(|home| home.join(default_dir)))
}

pub(crate) fn is_agy(command: &str) -> bool {
    Path::new(command)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name.to_ascii_lowercase().as_str(), "agy" | "antigravity"))
}

#[cfg(windows)]
pub(crate) fn has_agy_keyring_entry() -> bool {
    use windows_sys::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CredFree, CredReadW};

    let target = "gemini:antigravity\0".encode_utf16().collect::<Vec<_>>();
    let mut credential = std::ptr::null_mut();
    // Check for the exact credential record used by agy. Never inspect its contents.
    unsafe {
        if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) == 0 {
            return false;
        }
        CredFree(credential.cast());
        true
    }
}

#[cfg(not(windows))]
pub(crate) fn has_agy_keyring_entry() -> bool {
    false
}

pub(crate) async fn apply_profile(
    adapter: &AdapterConfig,
    command: &str,
    args: &mut Vec<String>,
    env: &mut Vec<(String, String)>,
) -> Result<(), AcpError> {
    let Some(path) = &adapter.profile_dir else {
        return Ok(());
    };
    let variable = match adapter.quota_provider.as_str() {
        "codex_local" => "CODEX_HOME",
        "claude_local" => "CLAUDE_CONFIG_DIR",
        "google_local" if is_agy(command) && args.first().is_some_and(|arg| arg == "acp") => {
            if args.iter().any(|arg| {
                [
                    "--gemini-dir",
                    "--gemini_dir",
                    "--app-data-dir",
                    "--app_data_dir",
                ]
                .contains(&arg.split('=').next().unwrap_or_default())
            }) {
                return Err(AcpError::InvalidProfile(
                    "set profile_dir or the Antigravity directory flags without combining them"
                        .into(),
                ));
            }
            "GEMINI_HOME"
        }
        "google_local"
            if Path::new(command)
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("gemini")) =>
        {
            "GEMINI_CLI_HOME"
        }
        _ => {
            return Err(AcpError::InvalidProfile(format!(
                "{} has no supported profile launcher",
                adapter.display_name
            )));
        }
    };
    tokio::fs::create_dir_all(path).await?;
    let path = dunce::canonicalize(path)?.to_string_lossy().into_owned();
    if variable == "GEMINI_HOME" {
        args.extend([
            "--gemini-dir".into(),
            path.clone(),
            "--app-data-dir".into(),
            "antigravity-cli".into(),
        ]);
    }
    env.push((variable.into(), path));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_resume_only_inside_the_profile_that_created_them() {
        let personal = "previous-native-session";
        let isolated = Path::new("profiles/isolated");
        let reference = store_profile_session(Some(isolated), "isolated-session");
        assert_eq!(
            resume_profile_session(Some(isolated), &reference).as_deref(),
            Some("isolated-session")
        );
        assert_eq!(resume_profile_session(Some(isolated), personal), None);
        assert_eq!(
            resume_profile_session(Some(Path::new("profiles/other")), &reference),
            None
        );
        assert_eq!(resume_profile_session(None, &reference), None);
        assert_eq!(
            resume_profile_session(None, personal).as_deref(),
            Some(personal)
        );
    }

    #[tokio::test]
    async fn antigravity_uses_a_separate_acp_data_root() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Antigravity Profile");
        let adapter = AdapterConfig {
            quota_provider: "google_local".into(),
            profile_dir: Some(dir.clone()),
            ..Default::default()
        };
        let mut args = vec!["acp".into()];
        let mut env = Vec::new();
        apply_profile(&adapter, "agy", &mut args, &mut env)
            .await
            .unwrap();
        let expected = dunce::canonicalize(dir)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            args,
            [
                "acp",
                "--gemini-dir",
                &expected,
                "--app-data-dir",
                "antigravity-cli"
            ]
        );
        assert_eq!(env, [("GEMINI_HOME".into(), expected)]);
    }
}
