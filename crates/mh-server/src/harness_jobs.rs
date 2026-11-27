use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
};
use mh_acp::{
    ProcessTreeGuard,
    maintenance::{CodexLoginMode, cli_login_command, cli_upgrade_command, cli_version},
    platform::prepare_isolated_command,
};
use mh_core::AppConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{ChildStdin, Command},
    sync::{Mutex, OwnedRwLockWriteGuard, watch},
};

use crate::{AppState, routes::ApiResult};

const PROFILE_MARKER: &str = ".meta-harness-profile";
const DEFAULT_PROFILE_MARKER: &str = ".use-default-profile";
const MAX_OUTPUT_CHARS: usize = 8_000;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Operation {
    SignIn,
    Upgrade,
}

#[derive(Clone, Serialize)]
struct Snapshot {
    kind: Operation,
    status: &'static str,
    output: String,
    accepts_input: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization_url: Option<String>,
}

struct Job {
    snapshot: Snapshot,
    input: Option<Arc<Mutex<ChildStdin>>>,
    cancel: watch::Sender<bool>,
    agy_code_flow: bool,
}

#[derive(Clone, Default)]
pub struct HarnessJobs(Arc<Mutex<HashMap<String, Job>>>);

fn error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.into() })))
}

fn managed_profile(data_dir: &Path, id: &str) -> Option<PathBuf> {
    (!id.is_empty()
        && id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"-_".contains(&ch)))
    .then(|| data_dir.join("profiles").join(id))
}

pub fn restore_profiles(config: &mut AppConfig, data_dir: &Path) {
    for (id, adapter) in &mut config.adapters {
        if adapter.profile_dir.is_none()
            && let Some(path) = managed_profile(data_dir, id)
            && path.join(PROFILE_MARKER).is_file()
            && !path.join(DEFAULT_PROFILE_MARKER).is_file()
        {
            adapter.profile_dir = Some(path);
        }
    }
}

pub(crate) fn has_saved_profile(data_dir: &Path, id: &str, current: Option<&Path>) -> bool {
    managed_profile(data_dir, id).is_some_and(|path| {
        current.is_none_or(|current| current == path) && path.join(PROFILE_MARKER).is_file()
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSelection {
    Default,
    Separate,
}

#[derive(Deserialize)]
pub struct SelectProfile {
    profile: ProfileSelection,
}

pub async fn select_profile(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<SelectProfile>,
) -> ApiResult<Value> {
    let _activity = state
        .switch
        .supervisor
        .adapter_activity(&id)
        .await
        .try_write_owned()
        .map_err(|_| {
            error(
                StatusCode::CONFLICT,
                "Wait for this harness's active turns or maintenance to finish.",
            )
        })?;
    let current = state
        .switch
        .config
        .read()
        .await
        .adapters
        .get(&id)
        .cloned()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Unknown harness"))?;
    let data_dir = &state.switch.app_paths.data_dir;
    let path = managed_profile(data_dir, &id)
        .filter(|_| has_saved_profile(data_dir, &id, current.profile_dir.as_deref()))
        .ok_or_else(|| {
            error(
                StatusCode::CONFLICT,
                "No web-managed separate profile is saved for this harness.",
            )
        })?;
    let profile = match body.profile {
        ProfileSelection::Default => {
            tokio::fs::write(path.join(DEFAULT_PROFILE_MARKER), b"")
                .await
                .map_err(profile_io_error)?;
            None
        }
        ProfileSelection::Separate => {
            remove_default_marker(&path)
                .await
                .map_err(profile_io_error)?;
            Some(path)
        }
    };
    activate_profile(&state, &id, profile.clone()).await;
    state.harness_jobs.0.lock().await.remove(&id);
    Ok(Json(json!({ "profile_dir": profile })))
}

async fn remove_default_marker(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path.join(DEFAULT_PROFILE_MARKER)).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn profile_io_error(cause: std::io::Error) -> (StatusCode, Json<Value>) {
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Could not save profile selection: {cause}"),
    )
}

async fn activate_profile(state: &AppState, id: &str, profile: Option<PathBuf>) {
    state.switch.supervisor.retire_adapter(id).await;
    let mut config = state.switch.config.write().await;
    if let Some(adapter) = config.adapters.get_mut(id) {
        adapter.profile_dir = profile;
        state
            .switch
            .quota
            .configure_launch_profile(id, adapter)
            .await;
        state.switch.quota.clear_rate_limit(id).await;
    }
    state.switch.supervisor.invalidate_catalog(id).await;
    state.switch.quota.invalidate(id).await;
}

#[derive(Deserialize)]
pub struct StartOperation {
    kind: Operation,
    #[serde(default)]
    login_mode: CodexLoginMode,
}

#[derive(Deserialize)]
pub struct SignInInput {
    value: String,
}

pub async fn version(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Value> {
    let adapter = state
        .switch
        .config
        .read()
        .await
        .adapters
        .get(&id)
        .cloned()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Unknown harness"))?;
    let _activity = state
        .switch
        .supervisor
        .adapter_activity(&id)
        .await
        .try_read_owned()
        .map_err(|_| {
            error(
                StatusCode::CONFLICT,
                "This harness is undergoing maintenance.",
            )
        })?;
    Ok(Json(json!({ "version": cli_version(&adapter).await })))
}

pub async fn status(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Json<Value> {
    let jobs = state.harness_jobs.0.lock().await;
    Json(json!({ "operation": jobs.get(&id).map(|job| &job.snapshot) }))
}

pub async fn start(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<StartOperation>,
) -> ApiResult<Value> {
    let activity = state
        .switch
        .supervisor
        .adapter_activity(&id)
        .await
        .try_write_owned()
        .map_err(|_| {
            error(
                StatusCode::CONFLICT,
                "Wait for this harness's active turns or maintenance to finish.",
            )
        })?;
    let mut adapter = state
        .switch
        .config
        .read()
        .await
        .adapters
        .get(&id)
        .cloned()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Unknown harness"))?;
    let mut activated = None;
    let command = match request.kind {
        Operation::SignIn => {
            if adapter.profile_dir.is_none() {
                let profile =
                    managed_profile(&state.switch.app_paths.data_dir, &id).ok_or_else(|| {
                        error(StatusCode::BAD_REQUEST, "Invalid harness profile name")
                    })?;
                adapter.profile_dir = Some(profile.clone());
                activated = Some(profile);
            }
            cli_login_command(&adapter, request.login_mode)
                .await
                .map_err(|cause| error(StatusCode::BAD_REQUEST, cause.to_string()))?
        }
        Operation::Upgrade => cli_upgrade_command(&adapter).ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "This launcher does not provide a supported upgrade command.",
            )
        })?,
    };
    let agy_code_flow = request.kind == Operation::SignIn
        && command.as_std().get_args().any(|arg| arg == "--login");
    let (cancel, signal) = watch::channel(false);
    let snapshot = Snapshot {
        kind: request.kind,
        status: "running",
        output: String::new(),
        accepts_input: false,
        authorization_url: None,
    };
    state.harness_jobs.0.lock().await.insert(
        id.clone(),
        Job {
            snapshot: snapshot.clone(),
            input: None,
            cancel,
            agy_code_flow,
        },
    );
    tokio::spawn(run_operation(
        state,
        id,
        command,
        request.kind,
        activated,
        signal,
        activity,
    ));
    Ok(Json(json!({ "operation": snapshot })))
}

pub async fn input(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<SignInInput>,
) -> ApiResult<Value> {
    let value = payload.value.trim();
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Enter the one-line authorization code or callback URL.",
        ));
    }
    let unavailable = || {
        error(
            StatusCode::CONFLICT,
            "The sign-in process is not accepting input.",
        )
    };
    let (writer, value) = {
        let mut jobs = state.harness_jobs.0.lock().await;
        let job = jobs
            .get_mut(&id)
            .filter(|job| job.snapshot.accepts_input)
            .ok_or_else(unavailable)?;
        if job.agy_code_flow {
            let authorization = job
                .snapshot
                .authorization_url
                .as_deref()
                .ok_or_else(unavailable)?;
            let code = agy_authorization_code(value, authorization)
                .map_err(|message| error(StatusCode::BAD_REQUEST, message))?;
            let writer = job.input.take().ok_or_else(unavailable)?;
            job.snapshot.accepts_input = false;
            (writer, code)
        } else {
            (job.input.clone().ok_or_else(unavailable)?, value.to_owned())
        }
    };
    let mut writer = writer.lock().await;
    writer
        .write_all(format!("{value}\n").as_bytes())
        .await
        .map_err(|_| error(StatusCode::CONFLICT, "The sign-in process has finished."))?;
    writer
        .flush()
        .await
        .map_err(|_| error(StatusCode::CONFLICT, "The sign-in process has finished."))?;
    Ok(Json(json!({ "accepted": true })))
}

pub async fn cancel(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Value> {
    if let Some(job) = state.harness_jobs.0.lock().await.get(&id) {
        let _ = job.cancel.send(true);
    }
    Ok(Json(json!({ "cancelled": true })))
}

async fn collect_output(mut reader: impl AsyncRead + Unpin, jobs: HarnessJobs, id: String) {
    let mut bytes = [0; 2048];
    while let Ok(count) = reader.read(&mut bytes).await {
        if count == 0 {
            break;
        }
        let mut entries = jobs.0.lock().await;
        let Some(job) = entries.get_mut(&id) else {
            break;
        };
        job.snapshot.output.extend(
            String::from_utf8_lossy(&bytes[..count])
                .chars()
                .filter(|ch| !ch.is_control() || matches!(*ch, '\n' | '\r' | '\t' | '\x1b')),
        );
        if job.snapshot.output.len() > MAX_OUTPUT_CHARS {
            let start = job
                .snapshot
                .output
                .ceil_char_boundary(job.snapshot.output.len() - MAX_OUTPUT_CHARS);
            job.snapshot.output.drain(..start);
        }
        if job.agy_code_flow && job.snapshot.authorization_url.is_none() {
            job.snapshot.authorization_url = agy_authorization_url(&job.snapshot.output);
            job.snapshot.accepts_input =
                job.input.is_some() && job.snapshot.authorization_url.is_some();
        }
    }
}

fn agy_authorization_url(output: &str) -> Option<String> {
    // The CLI writes the URL on its own line. A polling client may see an incomplete chunk.
    let (complete, _) = output.rsplit_once('\n')?;
    complete.lines().find_map(|line| {
        let start = line.find("https://")?;
        let candidate = line[start..]
            .split(|ch: char| ch.is_whitespace() || ch.is_control())
            .next()?;
        let url = reqwest::Url::parse(candidate).ok()?;
        (url.scheme() == "https"
            && url.host_str() == Some("accounts.google.com")
            && url.path().starts_with("/o/oauth")
            && url
                .query_pairs()
                .any(|(key, value)| key == "response_type" && value == "code")
            && url.query_pairs().any(|(key, value)| {
                key == "redirect_uri" && value == "https://antigravity.google/oauth-callback"
            }))
        .then(|| url.to_string())
    })
}

fn agy_authorization_code(value: &str, authorization: &str) -> Result<String, &'static str> {
    if !value.contains("://") && !value.chars().any(char::is_whitespace) {
        return Ok(value.to_owned());
    }
    let callback = reqwest::Url::parse(value)
        .map_err(|_| "Paste the authorization code or the full Antigravity callback URL.")?;
    if callback.scheme() != "https"
        || callback.host_str() != Some("antigravity.google")
        || callback.path() != "/oauth-callback"
    {
        return Err("Paste the URL of the Antigravity page Google redirected you to.");
    }
    let state = callback
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned());
    let requested_state = reqwest::Url::parse(authorization).ok().and_then(|url| {
        url.query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
    });
    if state.is_some() && requested_state.is_some() && state != requested_state {
        return Err(
            "This callback belongs to a different sign-in attempt. Open the link for the current attempt.",
        );
    }
    callback.query_pairs().find(|(key, value)| key == "code" && !value.is_empty() && !value.chars().any(|ch| ch.is_whitespace() || ch.is_control()))
        .map(|(_, value)| value.into_owned()).ok_or("Google did not return an authorization code. Open the current sign-in link and try again.")
}

async fn run_operation(
    state: AppState,
    id: String,
    mut command: Command,
    kind: Operation,
    profile: Option<PathBuf>,
    mut cancel: watch::Receiver<bool>,
    _activity: OwnedRwLockWriteGuard<()>,
) {
    let result: Result<(), String> = async {
        if kind == Operation::Upgrade {
            state.switch.supervisor.retire_adapter(&id).await;
        }
        if kind == Operation::Upgrade {
            command.stdin(Stdio::null());
        }
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command.current_dir(&state.switch.app_paths.data_dir);
        prepare_isolated_command(&mut command);
        let mut child = command
            .spawn()
            .map_err(|cause| format!("Could not start the CLI: {cause}"))?;
        let mut process_tree = ProcessTreeGuard::attach(&child);
        if let Some(entry) = state.harness_jobs.0.lock().await.get_mut(&id) {
            entry.input = child.stdin.take().map(|stdin| Arc::new(Mutex::new(stdin)));
            entry.snapshot.accepts_input = entry.input.is_some() && !entry.agy_code_flow;
        }
        let stdout = child
            .stdout
            .take()
            .map(|out| tokio::spawn(collect_output(out, state.harness_jobs.clone(), id.clone())));
        let stderr = child
            .stderr
            .take()
            .map(|out| tokio::spawn(collect_output(out, state.harness_jobs.clone(), id.clone())));
        let outcome = tokio::select! {
            _ = cancel.wait_for(|cancelled| *cancelled) => Err("Cancelled.".to_string()),
            result = tokio::time::timeout(Duration::from_secs(600), child.wait()) => match result {
                Ok(Ok(status)) if status.success() => Ok(()),
                Ok(Ok(status)) => Err(format!("The CLI exited with {status}.")),
                Ok(Err(cause)) => Err(cause.to_string()),
                Err(_) => Err("The CLI operation timed out.".into()),
            }
        };
        process_tree.kill_tree();
        let _ = child.wait().await;
        for reader in [stdout, stderr].into_iter().flatten() {
            let _ = reader.await;
        }
        outcome?;
        if let Some(path) = profile {
            tokio::fs::write(path.join(PROFILE_MARKER), b"enabled\n")
                .await
                .map_err(|cause| cause.to_string())?;
            remove_default_marker(&path)
                .await
                .map_err(|cause| cause.to_string())?;
            activate_profile(&state, &id, Some(path)).await;
        } else if kind == Operation::SignIn {
            let current = state
                .switch
                .config
                .read()
                .await
                .adapters
                .get(&id)
                .and_then(|adapter| adapter.profile_dir.clone());
            activate_profile(&state, &id, current).await;
        } else {
            state.switch.supervisor.invalidate_catalog(&id).await;
            state.switch.quota.invalidate(&id).await;
        }
        Ok(())
    }
    .await;
    if let Some(job) = state.harness_jobs.0.lock().await.get_mut(&id) {
        job.input = None;
        job.snapshot.accepts_input = false;
        job.snapshot.authorization_url = None;
        let summary = match result {
            Ok(()) => {
                job.snapshot.status = "completed";
                if kind == Operation::SignIn {
                    "Signed in to the separate profile.".into()
                } else {
                    "Upgrade command completed.".into()
                }
            }
            Err(message) => {
                job.snapshot.status = "failed";
                if kind == Operation::SignIn {
                    sign_in_failure(message, &job.snapshot.output)
                } else {
                    message
                }
            }
        };
        if kind == Operation::SignIn {
            job.snapshot.output = summary;
        } else {
            job.snapshot.output.push_str(&format!("\n{summary}"));
        }
    }
}

// Only surface recognized diagnostics after sign-in. The live transcript can include one-time codes.
fn sign_in_failure(summary: String, transcript: &str) -> String {
    let text = transcript.to_ascii_lowercase();
    let diagnostic = if (text.contains("device code") || text.contains("device auth"))
        && ["403", "forbidden", "not enabled", "not supported"]
            .iter()
            .any(|part| text.contains(part))
    {
        "The provider refused device-code sign-in. Try browser sign-in, or enable device-code authorization in ChatGPT security settings."
    } else if [
        "address already in use",
        "already bound",
        "port already in use",
        "os error 10048",
    ]
    .iter()
    .any(|part| text.contains(part))
        || (text.contains("port") && text.contains("already in use"))
    {
        "The CLI could not open its local browser callback port. Close the other sign-in attempt and retry."
    } else if [
        "error sending request",
        "failed to connect",
        "certificate verify",
        "certificate has expired",
        "unknownissuer",
        "dns error",
    ]
    .iter()
    .any(|part| text.contains(part))
    {
        "The CLI could not reach the provider's sign-in service. Check its network, proxy, or certificate settings."
    } else if text.contains("invalid_grant") || text.contains("token exchange failed") {
        "The provider rejected the sign-in exchange. Start a fresh attempt and use its newest authorization code or callback URL once."
    } else {
        return summary;
    };
    format!("{summary}\n{diagnostic}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_sign_in_waits_for_a_complete_link_and_extracts_the_current_code() {
        let link = "https://accounts.google.com/o/oauth2/auth?response_type=code&redirect_uri=https%3A%2F%2Fantigravity.google%2Foauth-callback&state=current";
        assert!(agy_authorization_url(link).is_none());
        let transcript = format!("Sign in with Google:\n  {link}\nPaste your authorization code:");
        let authorization = agy_authorization_url(&transcript).unwrap();
        let callback = "https://antigravity.google/oauth-callback?state=current&code=4%2Ffixture";
        assert_eq!(
            agy_authorization_code(callback, &authorization),
            Ok("4/fixture".into())
        );
        assert!(
            agy_authorization_code(
                "https://antigravity.google/oauth-callback?state=old&code=old",
                &authorization
            )
            .is_err()
        );
        assert!(agy_authorization_code("https://example.com/?code=other", &authorization).is_err());
    }

    #[test]
    fn fixture_cli_subprocess() {
        if std::env::var_os("MH_JOB_FIXTURE").is_none() {
            return;
        }
        println!("Ready for fixture input");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "fixture-value");
    }

    #[tokio::test]
    async fn sign_in_job_accepts_input_and_activates_profile_after_cli_success() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().to_path_buf();
        let profile = managed_profile(&data, "antigravity").unwrap();
        std::fs::create_dir_all(&profile).unwrap();
        let program = std::env::current_exe().unwrap();
        let switch = mh_switch::SwitchEngine::new_with_paths(
            AppConfig::default(),
            mh_store::Store::open_in_memory().unwrap(),
            mh_acp::AcpSupervisor::new(60),
            Arc::new(mh_acp::QuotaTracker::new()),
            program.clone(),
            String::new(),
            "fixture".into(),
            mh_core::AppPaths {
                config_dir: data.clone(),
                data_dir: data.clone(),
                workspaces_dir: data,
            },
        );
        let state = AppState {
            switch,
            auth: crate::auth::AuthConfig::for_listener(
                "127.0.0.1:4410".parse().unwrap(),
                false,
                None,
            )
            .unwrap(),
            harness_jobs: Default::default(),
        };
        let id = "antigravity".to_string();
        let (cancel, signal) = watch::channel(false);
        state.harness_jobs.0.lock().await.insert(
            id.clone(),
            Job {
                snapshot: Snapshot {
                    kind: Operation::SignIn,
                    status: "running",
                    output: String::new(),
                    accepts_input: false,
                    authorization_url: None,
                },
                input: None,
                cancel,
                agy_code_flow: false,
            },
        );
        let permit = state
            .switch
            .supervisor
            .adapter_activity(&id)
            .await
            .write_owned()
            .await;
        let mut command = Command::new(program);
        command
            .stdin(Stdio::piped())
            .args([
                "--exact",
                "harness_jobs::tests::fixture_cli_subprocess",
                "--nocapture",
            ])
            .env("MH_JOB_FIXTURE", "1");
        let task = tokio::spawn(run_operation(
            state.clone(),
            id.clone(),
            command,
            Operation::SignIn,
            Some(profile.clone()),
            signal,
            permit,
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if state.harness_jobs.0.lock().await[&id]
                    .snapshot
                    .output
                    .contains("Ready for fixture input")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(!profile.join(PROFILE_MARKER).is_file());
        let response = input(
            State(state.clone()),
            AxumPath(id.clone()),
            Json(SignInInput {
                value: "fixture-value".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.0["accepted"], true);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        let jobs = state.harness_jobs.0.lock().await;
        assert_eq!(jobs[&id].snapshot.status, "completed");
        assert!(!jobs[&id].snapshot.output.contains("fixture input"));
        assert!(profile.join(PROFILE_MARKER).is_file());
        assert_eq!(
            state.switch.config.read().await.adapters[&id]
                .profile_dir
                .as_ref(),
            Some(&profile)
        );
        drop(jobs);

        let credentials = profile.join("fixture-auth");
        std::fs::write(&credentials, b"fixture-credentials").unwrap();
        let response = select_profile(
            State(state.clone()),
            AxumPath(id.clone()),
            Json(SelectProfile {
                profile: ProfileSelection::Default,
            }),
        )
        .await
        .unwrap();
        assert!(response.0["profile_dir"].is_null());
        assert!(
            state.switch.config.read().await.adapters[&id]
                .profile_dir
                .is_none()
        );
        let mut restarted = AppConfig::default();
        restore_profiles(&mut restarted, &state.switch.app_paths.data_dir);
        assert!(restarted.adapters[&id].profile_dir.is_none());
        assert_eq!(std::fs::read(&credentials).unwrap(), b"fixture-credentials");

        let response = select_profile(
            State(state.clone()),
            AxumPath(id.clone()),
            Json(SelectProfile {
                profile: ProfileSelection::Separate,
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.0["profile_dir"], json!(profile));
        restore_profiles(&mut restarted, &state.switch.app_paths.data_dir);
        assert_eq!(
            restarted.adapters[&id].profile_dir.as_deref(),
            Some(profile.as_path())
        );
    }

    #[test]
    fn failed_sign_in_keeps_the_diagnosis_and_discards_codes_and_urls() {
        let result = sign_in_failure(
            "CLI exited with code 1.".into(),
            "Error logging in with device code: device code request failed with status 403 Forbidden\nhttps://auth.openai.com/test?code=private-fixture",
        );
        assert!(result.contains("Try browser sign-in"));
        assert!(!result.contains("private-fixture"));
        assert!(!result.contains("https://"));
    }

    #[test]
    fn startup_restores_only_activated_managed_profiles_and_respects_config() {
        let root = tempfile::tempdir().unwrap();
        let codex = managed_profile(root.path(), "codex").unwrap();
        let unfinished = managed_profile(root.path(), "antigravity").unwrap();
        let claude = managed_profile(root.path(), "claude").unwrap();
        for dir in [&codex, &unfinished, &claude] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(codex.join(PROFILE_MARKER), "enabled\n").unwrap();
        std::fs::write(claude.join(PROFILE_MARKER), "enabled\n").unwrap();
        let mut cfg = AppConfig::default();
        let explicit = root.path().join("custom-claude");
        cfg.adapters.get_mut("claude").unwrap().profile_dir = Some(explicit.clone());
        restore_profiles(&mut cfg, root.path());
        assert_eq!(cfg.adapters["codex"].profile_dir.as_ref(), Some(&codex));
        assert_eq!(cfg.adapters["claude"].profile_dir.as_ref(), Some(&explicit));
        assert_eq!(cfg.adapters["antigravity"].profile_dir, None);
        assert!(managed_profile(root.path(), "../outside").is_none());
    }
}
