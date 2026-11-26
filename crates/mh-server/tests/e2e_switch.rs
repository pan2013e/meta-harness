use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::TcpListener;

use mh_acp::{AcpSupervisor, QuotaTracker, ensure_member_worktree, init_git_repo};
use mh_core::{AdapterConfig, AppConfig, AppPaths, DispatchTaskSpec, MessageKind, PromoteMode};
use mh_server::auth::AuthConfig;
use mh_server::{AppState, build_router};
use mh_store::Store;
use mh_switch::{SwitchEngine, SwitchError, execute_mcp_tool};

fn mock_app_config() -> AppConfig {
    let exe = env!("CARGO_BIN_EXE_meta-harness").to_string();
    let mut cfg = AppConfig::default();
    let mut adapters = BTreeMap::new();
    for (id, name) in [
        ("codex", "Codex"),
        ("claude", "Claude Code"),
        ("antigravity", "Antigravity"),
    ] {
        adapters.insert(
            id.to_string(),
            AdapterConfig {
                display_name: name.into(),
                accent_color: "#6366f1".into(),
                enabled: true,
                command: exe.clone(),
                args: vec!["internal-mock-agent".into()],
                fallback_commands: Vec::new(),
                quota_provider: "manual".into(),
                exact_quota_probe_opt_in: false,
                profile_dir: None,
                characteristics: Default::default(),
            },
        );
    }
    cfg.adapters = adapters;
    cfg.memory.curation_mode = "orchestrator_llm".into();
    cfg
}

async fn start_test_server(
    auth_required: bool,
) -> (
    String,
    Arc<SwitchEngine>,
    AuthConfig,
    tokio::task::JoinHandle<()>,
) {
    let cfg = mock_app_config();
    let store = Store::open_in_memory().unwrap();
    let supervisor = AcpSupervisor::new(300);
    let quota = Arc::new(QuotaTracker::new());
    let exe = std::path::PathBuf::from(env!("CARGO_BIN_EXE_meta-harness"));
    let bridge_token = "test_bridge_secret_123".to_string();

    let switch = SwitchEngine::new(
        cfg,
        store,
        supervisor,
        quota,
        exe,
        String::new(),
        bridge_token,
    );

    let auth = if auth_required {
        AuthConfig::from_bind_and_flag("0.0.0.0:4410", Some("lan_secret_xyz".into()))
    } else {
        AuthConfig::from_bind_and_flag("127.0.0.1:4410", None)
    };

    let state = AppState {
        switch: Arc::clone(&switch),
        auth: auth.clone(),
        harness_jobs: Default::default(),
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");
    switch
        .set_internal_mcp_url(format!("{base_url}/internal/mcp"))
        .await;

    let app = build_router(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (base_url, switch, auth, handle)
}

#[tokio::test]
async fn lan_auth_guard_enforces_bearer_token() {
    let (base_url, switch, auth, srv) = start_test_server(true).await;
    let http = reqwest::Client::new();

    let unauth = http
        .get(format!("{base_url}/api/projects"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let authed = http
        .get(format!("{base_url}/api/projects"))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(authed.status(), reqwest::StatusCode::OK);

    switch.supervisor.shutdown_all().await;
    srv.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_parallel_worktrees_consult_cycle_trajectory_orchestrator_and_promote() {
    let (base_url, switch, _auth, srv) = start_test_server(false).await;
    let http = reqwest::Client::new();

    let tmp_repo = tempfile::tempdir().unwrap();
    let repo_root = tmp_repo.path();
    std::fs::write(repo_root.join("README.md"), "# Test Repo\n").unwrap();
    init_git_repo(repo_root).await.unwrap();

    // 1. Create project & session via REST API
    let proj_resp: Value = http
        .post(format!("{base_url}/api/projects"))
        .json(&json!({ "root_path": repo_root.to_string_lossy() }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let project_id = proj_resp["project"]["id"].as_str().unwrap().to_string();

    let sess_resp: Value = http
        .post(format!("{base_url}/api/projects/{project_id}/sessions"))
        .json(&json!({
            "title": "E2E Switch Session",
            "orchestrator_member": "antigravity"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = sess_resp["session"]["id"].as_str().unwrap().to_string();
    let int_wt = sess_resp["session"]["integration_wt_path"]
        .as_str()
        .unwrap()
        .to_string();

    let original_claude = {
        let mut config = switch.config.write().await;
        let adapter = config.adapters.get_mut("claude").unwrap();
        std::mem::replace(
            &mut adapter.command,
            repo_root.join("missing-claude").display().to_string(),
        )
    };
    let missing = switch
        .submit_orchestrator_dispatch(
            &session_id,
            "antigravity",
            "Use Claude".into(),
            vec![DispatchTaskSpec {
                target: "@claude".into(),
                model: None,
                effort: None,
                task_brief: "Test".into(),
                mode: "parallel".into(),
            }],
        )
        .await
        .unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("Unavailable harnesses: @claude")
    );
    assert!(missing.to_string().contains("@codex"));
    assert!(
        switch
            .handle_user_message(&session_id, "@claude test".into())
            .await
            .is_err()
    );
    assert!(
        switch
            .store
            .list_messages(&session_id, None, 10)
            .unwrap()
            .is_empty()
    );
    switch
        .config
        .write()
        .await
        .adapters
        .get_mut("claude")
        .unwrap()
        .command = original_claude;

    // 2. Parallel @codex + @claude disjoint edits -> clean zero-token auto-merge
    let req_a = http
        .post(format!("{base_url}/api/sessions/{session_id}/messages"))
        .json(&json!({
            "content": "@codex [MOCK:WRITE path=\"codex.txt\" content=\"hello from codex\\n\"]"
        }))
        .send();
    let req_b = http
        .post(format!("{base_url}/api/sessions/{session_id}/messages"))
        .json(&json!({
            "content": "@claude [MOCK:WRITE path=\"claude.txt\" content=\"hello from claude\\n\"]"
        }))
        .send();
    let (ra, rb) = tokio::join!(req_a, req_b);
    ra.unwrap();
    rb.unwrap();

    // Wait until both files appear in integration worktree and deterministic/curated memory lands
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let a = std::path::Path::new(&int_wt).join("codex.txt").exists();
        let b = std::path::Path::new(&int_wt).join("claude.txt").exists();
        if a && b {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for parallel worktree auto-merge"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Main checkout must still be clean before Promote
    assert!(!repo_root.join("codex.txt").exists());

    // 3. Wait for members to go idle before testing peer_consult & trajectory_read
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Peer consult over real `meta-harness mcp-bridge` subprocess!
    http.post(format!("{base_url}/api/sessions/{session_id}/messages"))
        .json(&json!({
            "content": "@codex [MOCK:MCP tool=\"peer_consult\" args={\"target\":\"claude\",\"question\":\"Verify API shape\"}]"
        }))
        .send()
        .await
        .unwrap();

    let deadline2 = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        let msgs = switch.store.list_messages(&session_id, None, 100).unwrap();
        let has_consult_answer = msgs.iter().any(|m| {
            m.kind == MessageKind::ConsultThread
                && m.parent_msg_id.is_some()
                && m.sender_id == "claude"
        });
        if has_consult_answer {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline2,
            "timed out waiting for peer_consult thread"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 4. Verify WaitForGraph cycle detection prevents deadlocks
    {
        let rt = switch.runtime(&session_id).await;
        rt.wait_for_graph
            .lock()
            .await
            .try_acquire_edge("claude", "codex")
            .unwrap();
        let cycle_err = execute_mcp_tool(
            &switch,
            &session_id,
            "codex",
            "peer_consult",
            json!({ "target": "claude", "question": "Loop back?" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(cycle_err, SwitchError::CycleDetected(_)));
        rt.wait_for_graph.lock().await.clear();
    }

    // 5. Verify trajectory_read produces an inline ActivityPill
    let trajs = switch
        .store
        .list_trajectories(&session_id, Some("codex"), 10)
        .unwrap();
    let codex_turn = trajs.first().unwrap().turn_id.clone();
    let _ = execute_mcp_tool(
        &switch,
        &session_id,
        "claude",
        "trajectory_read",
        json!({ "turn_id": codex_turn, "detail": "diffs" }),
    )
    .await
    .unwrap();
    let msgs_after_read = switch.store.list_messages(&session_id, None, 100).unwrap();
    assert!(
        msgs_after_read.iter().any(
            |m| m.kind == MessageKind::ActivityPill && m.content.contains("claude read @codex")
        )
    );

    // 6. Test Orchestrator quota-aware dispatch + 429 cooldown avoidance + curated memory
    switch.quota.mark_rate_limited("codex", 600).await;
    switch
        .quota
        .configure_adapter("claude", Some("pro".into()), false, Some(95.0), Some(95.0))
        .await;
    switch
        .quota
        .configure_adapter(
            "antigravity",
            Some("pro".into()),
            false,
            Some(30.0),
            Some(60.0),
        )
        .await;

    http.post(format!("{base_url}/api/sessions/{session_id}/messages"))
        .json(&json!({
            "content": "[MOCK:ORCH_AUTO] Build distributed cache architecture"
        }))
        .send()
        .await
        .unwrap();

    let deadline3 = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let msgs = switch.store.list_messages(&session_id, None, 150).unwrap();
        let dispatched_claude = msgs.iter().any(|m| {
            m.kind == MessageKind::DispatchCard && m.recipients.contains(&"claude".to_string())
        });
        let claude_completed = msgs
            .iter()
            .filter(|m| {
                m.sender_id == "claude"
                    && m.kind == MessageKind::Text
                    && m.metadata.get("streaming") == Some(&json!(false))
            })
            .count();
        let jobs = switch
            .store
            .recall_jobs(&project_id, Some("claude"), 10)
            .unwrap();
        if dispatched_claude && claude_completed >= 2 && jobs.len() >= 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline3,
            "timed out waiting for Orchestrator dispatch + curated memory"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 7. Serve preview file from worktree & block traversal, then promote via Squash & verify diff resets
    let preview_ok = http
        .get(format!(
            "{base_url}/api/sessions/{session_id}/files/codex.txt"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_ok.status(), reqwest::StatusCode::OK);
    assert!(
        preview_ok
            .text()
            .await
            .unwrap()
            .contains("hello from codex")
    );

    let preview_escape = http
        .get(format!(
            "{base_url}/api/sessions/{session_id}/files/..%2FREADME.md"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_escape.status(), reqwest::StatusCode::NOT_FOUND);

    let prom_resp = http
        .post(format!("{base_url}/api/sessions/{session_id}/promote"))
        .json(&json!({ "mode": PromoteMode::Squash }))
        .send()
        .await
        .unwrap();
    let prom_status = prom_resp.status();
    if prom_status != reqwest::StatusCode::OK
        || !repo_root.join("codex.txt").exists()
        || !repo_root.join("claude.txt").exists()
    {
        switch.supervisor.shutdown_all().await;
        srv.abort();
        panic!(
            "promote check failed (status={prom_status}, codex={}, claude={})",
            repo_root.join("codex.txt").exists(),
            repo_root.join("claude.txt").exists()
        );
    }

    let diff_after: Value = http
        .get(format!("{base_url}/api/sessions/{session_id}/diff"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(diff_after["diff"]["ahead_commits"], 0);
    assert_eq!(diff_after["diff"]["diff_stat"]["files_changed"], 0);

    // 8. Delete session -> worktrees pruned cleanly
    let del_resp = http
        .delete(format!("{base_url}/api/sessions/{session_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);
    assert_eq!(switch.supervisor.active_process_count().await, 0);
    assert!(
        !repo_root
            .join(".meta-harness/worktrees")
            .join(&session_id)
            .exists()
    );

    switch.supervisor.shutdown_all().await;
    srv.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_new_session_title_auto_renames_context_defaults_and_web_shell() {
    let (base_url, switch, _auth, srv) = start_test_server(false).await;
    let http = reqwest::Client::new();

    let tmp_repo = tempfile::tempdir().unwrap();
    let repo_root = tmp_repo.path();
    std::fs::create_dir_all(repo_root.join("sub")).unwrap();
    std::fs::write(repo_root.join("sub/keep.txt"), "ok\n").unwrap();
    std::fs::write(repo_root.join("README.md"), "# Test Repo\n").unwrap();
    init_git_repo(repo_root).await.unwrap();

    // OS native picker & terminal disabled
    let pick: Value = http
        .post(format!("{base_url}/api/fs/pick-folder"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pick["supported"], false);

    // Save adapter model/effort defaults
    let def_resp = http
        .patch(format!("{base_url}/api/adapters/codex/defaults"))
        .json(&json!({ "default_model": "gpt-5.4-mini", "default_effort": "high" }))
        .send()
        .await
        .unwrap();
    assert_eq!(def_resp.status(), reqwest::StatusCode::OK);

    let proj_resp: Value = http
        .post(format!("{base_url}/api/projects"))
        .json(&json!({ "root_path": repo_root.to_string_lossy() }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let project_id = proj_resp["project"]["id"].as_str().unwrap().to_string();

    // Creating a session with omitted title uses "New session" and seeds adapter defaults
    let sess_resp: Value = http
        .post(format!("{base_url}/api/projects/{project_id}/sessions"))
        .json(&json!({ "orchestrator_member": "codex" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sess_resp["session"]["title"], "New session");
    let session_id = sess_resp["session"]["id"].as_str().unwrap().to_string();
    let codex_member = sess_resp["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["handle"] == "codex")
        .unwrap();
    assert_eq!(codex_member["active_model"], "gpt-5.4-mini");
    assert_eq!(codex_member["active_effort"], "high");

    // OS terminal endpoint returns 400
    let os_term = http
        .post(format!(
            "{base_url}/api/sessions/{session_id}/open-terminal"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(os_term.status(), reqwest::StatusCode::BAD_REQUEST);

    // Web shell supports cd inside workspace and rejects escaping
    let cd_ok: Value = http
        .post(format!("{base_url}/api/sessions/{session_id}/run-command"))
        .json(&json!({ "command": "cd sub" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cd_ok["exit_code"], 0);
    assert!(cd_ok["cwd"].as_str().unwrap().ends_with("sub"));
    let cd_main: Value = http
        .post(format!("{base_url}/api/sessions/{session_id}/run-command"))
        .json(&json!({ "command": format!("cd \"{}\"", repo_root.display()) }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cd_main["exit_code"], 1);

    let file = http
        .get(format!(
            "{base_url}/api/sessions/{session_id}/files/README.md"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(file.status(), reqwest::StatusCode::OK);
    assert!(
        file.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .starts_with("sandbox")
    );

    // Per-member model config update works
    let cfg_upd: Value = http
        .patch(format!(
            "{base_url}/api/sessions/{session_id}/members/codex/config"
        ))
        .json(&json!({ "active_model": "gpt-5.4" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg_upd["member"]["active_model"], "gpt-5.4");

    // Context usage endpoint returns breakdown
    let ctx_resp: Value = http
        .get(format!("{base_url}/api/sessions/{session_id}/context"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !ctx_resp["context"]["members"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // First user message triggers background LLM session auto-title replacement
    http.post(format!("{base_url}/api/sessions/{session_id}/messages"))
        .json(&json!({ "content": "Build distributed pelican tracker" }))
        .send()
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        let s = switch.store.get_session(&session_id).unwrap();
        if s.title != "New session" {
            assert!(!s.title.trim().is_empty());
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for auto session title"
        );
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    switch.supervisor.shutdown_all().await;
    srv.abort();
}

async fn start_test_server_with_paths() -> (
    String,
    Arc<SwitchEngine>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
    AppPaths,
) {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let paths = AppPaths {
        config_dir: base.join("config"),
        data_dir: base.join("data"),
        workspaces_dir: base.join("workspaces"),
    };
    let cfg = mock_app_config();
    let store = Store::open(&paths.db_path()).unwrap();
    let supervisor = AcpSupervisor::new(300);
    let quota = Arc::new(QuotaTracker::new());
    let exe = std::path::PathBuf::from(env!("CARGO_BIN_EXE_meta-harness"));
    let bridge_token = "test_bridge_secret_123".to_string();

    let switch = SwitchEngine::new_with_paths(
        cfg,
        store,
        supervisor,
        quota,
        exe,
        String::new(),
        bridge_token,
        paths.clone(),
    );

    let auth = AuthConfig::from_bind_and_flag("127.0.0.1:4410", None);
    let state = AppState {
        switch: Arc::clone(&switch),
        auth,
        harness_jobs: Default::default(),
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");
    switch
        .set_internal_mcp_url(format!("{base_url}/internal/mcp"))
        .await;

    let app = build_router(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (base_url, switch, handle, tmp, paths)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn standalone_archive_compact_and_storage_cleanup_lifecycle() {
    let (base_url, switch, srv, _tmp, paths) = start_test_server_with_paths().await;
    let http = reqwest::Client::new();

    // 1. Standalone session creation with project_id == null & isolated workspace
    let sess_resp: Value = http
        .post(format!("{base_url}/api/sessions"))
        .json(&json!({ "orchestrator_member": "codex" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sess_resp["session"]["project_id"].is_null());
    let sid = sess_resp["session"]["id"].as_str().unwrap().to_string();
    let ws_root = paths.standalone_session_root(&sid);
    let int_wt = std::path::PathBuf::from(
        sess_resp["session"]["integration_wt_path"]
            .as_str()
            .unwrap(),
    );
    assert!(ws_root.join(".git").exists());
    assert!(int_wt.exists());

    let listed: Value = http
        .get(format!("{base_url}/api/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(listed["sessions"][0]["id"], sid);

    // 2. Reject Promote on standalone session
    let prom = http
        .post(format!("{base_url}/api/sessions/{sid}/promote"))
        .json(&json!({ "mode": PromoteMode::Squash }))
        .send()
        .await
        .unwrap();
    assert_eq!(prom.status(), reqwest::StatusCode::BAD_REQUEST);

    let int_branch = sess_resp["session"]["integration_branch"].as_str().unwrap();
    let shell_worktree = ensure_member_worktree(&ws_root, &sid, int_branch, "codex")
        .await
        .unwrap();
    std::fs::write(
        shell_worktree.path.join("shell-notes.txt"),
        "keep this work\n",
    )
    .unwrap();

    // 3. Archive prunes checkouts, hides from default list, unarchive restores checkout
    let arch_resp: Value = http
        .patch(format!("{base_url}/api/sessions/{sid}"))
        .json(&json!({ "archived": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(arch_resp["session"]["archived_at"].is_number());
    assert!(!int_wt.exists());

    let blocked = http
        .post(format!("{base_url}/api/sessions/{sid}/messages"))
        .json(&json!({ "content": "This session is archived" }))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), reqwest::StatusCode::BAD_REQUEST);
    let shell = http
        .post(format!("{base_url}/api/sessions/{sid}/run-command"))
        .json(&json!({ "command": "cd" }))
        .send()
        .await
        .unwrap();
    assert_eq!(shell.status(), reqwest::StatusCode::CONFLICT);

    let branch_out = std::process::Command::new("git")
        .args(["branch", "--list", &format!("mh/{sid}/integration")])
        .current_dir(&ws_root)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&branch_out.stdout).contains(&format!("mh/{sid}/integration")));

    let list_active: Value = http
        .get(format!("{base_url}/api/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_active["sessions"].as_array().unwrap().is_empty());

    let list_all: Value = http
        .get(format!("{base_url}/api/sessions?include_archived=true"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list_all["sessions"].as_array().unwrap().len(), 1);

    let unarch_resp: Value = http
        .patch(format!("{base_url}/api/sessions/{sid}"))
        .json(&json!({ "archived": false }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(unarch_resp["session"]["archived_at"].is_null());
    assert!(int_wt.exists());

    let restored = ensure_member_worktree(&ws_root, &sid, int_branch, "codex")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(restored.path.join("shell-notes.txt"))
            .unwrap()
            .trim_end(),
        "keep this work"
    );

    // 4. Compact updates working_summary and posts compaction message
    http.post(format!("{base_url}/api/sessions/{sid}/messages"))
        .json(&json!({ "content": "@codex [MOCK:WRITE path=\"note.txt\" content=\"hi\\n\"]" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    while !int_wt.join("note.txt").is_file() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "restored session did not run an ACP turn and integrate its edit"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let cmp_resp: Value = http
        .post(format!("{base_url}/api/sessions/{sid}/compact"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!cmp_resp["message"]["id"].as_str().unwrap().is_empty());
    let updated_sess = switch.store.get_session(&sid).unwrap();
    assert!(!updated_sess.working_summary.trim().is_empty());

    // 5. Storage stats + clear_caches (orphans) + delete_archived cleanup
    let orphan_dir = paths.workspaces_dir.join("s_orphan_dir");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    std::fs::write(orphan_dir.join("junk.bin"), vec![42u8; 4096]).unwrap();

    let st_before: Value = http
        .get(format!("{base_url}/api/storage"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(st_before["storage"]["standalone_count"], 1);
    assert_eq!(st_before["storage"]["orphan_dirs_count"], 1);

    let clear_resp: Value = http
        .post(format!("{base_url}/api/storage/cleanup"))
        .json(&json!({ "action": "clear_caches" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(clear_resp["result"]["removed_dirs"], 1);
    assert!(!orphan_dir.exists());

    // Archive again then delete archived sessions via cleanup API
    http.patch(format!("{base_url}/api/sessions/{sid}"))
        .json(&json!({ "archived": true }))
        .send()
        .await
        .unwrap();
    let del_arch: Value = http
        .post(format!("{base_url}/api/storage/cleanup"))
        .json(&json!({ "action": "delete_archived" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(del_arch["result"]["removed_sessions"], 1);
    assert!(!ws_root.exists());
    assert_eq!(del_arch["storage"]["archived_sessions_count"], 0);

    switch.supervisor.shutdown_all().await;
    srv.abort();
}

#[tokio::test]
async fn personas_share_an_adapter_with_independent_preferences_and_can_be_removed() {
    let (base_url, switch, srv, _tmp, _paths) = start_test_server_with_paths().await;
    let http = reqwest::Client::new();
    let created: Value = http
        .post(format!("{base_url}/api/sessions"))
        .json(&json!({"title":"Persona test", "orchestrator_member":"codex"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = created["session"]["id"].as_str().unwrap();
    let endpoint = format!("{base_url}/api/sessions/{sid}");
    for (handle, model, effort) in [
        ("reviewer", "gpt-5.4", "high"),
        ("writer", "gpt-5.4-mini", "low"),
    ] {
        let member: Value = http
            .post(format!("{endpoint}/members"))
            .json(&json!({"handle":handle, "adapter_id":"codex",
                "active_model":model, "active_effort":effort}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(member["member"]["adapter_id"], "codex");
        assert_eq!(member["member"]["active_model"], model);
        assert_eq!(member["member"]["active_effort"], effort);
    }
    http.patch(&endpoint)
        .json(&json!({"orchestrator_member":"reviewer"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    http.delete(format!("{endpoint}/members/reviewer"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(switch.store.get_member(sid, "reviewer").is_err());
    assert!(
        switch
            .store
            .get_session(sid)
            .unwrap()
            .orchestrator_member
            .is_none()
    );
    let writer = switch.store.get_member(sid, "writer").unwrap();
    assert_eq!(writer.active_model.as_deref(), Some("gpt-5.4-mini"));
    assert_eq!(writer.active_effort.as_deref(), Some("low"));
    switch.supervisor.shutdown_all().await;
    srv.abort();
}

#[tokio::test]
async fn settings_backup_and_global_personas_preserve_session_choices() {
    let (base_url, switch, srv, _tmp, _paths) = start_test_server_with_paths().await;
    let http = reqwest::Client::new();
    let settings_url = format!("{base_url}/api/settings");
    let sessions_url = format!("{base_url}/api/sessions");
    let create_session = |title: &'static str| {
        http.post(&sessions_url)
            .json(&json!({"title": title, "orchestrator_member": "codex"}))
    };

    let existing: Value = create_session("Existing session")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let existing_id = existing["session"]["id"].as_str().unwrap();

    let backup = json!({
        "version": 1,
        "theme": "daylight",
        "preferences": {
            "global_personas": [{
                "handle": "@Reviewer", "adapter_id": "claude", "display_name": "Reviewer",
                "persona_prompt": "Review diffs.",
                "active_model": "mock-sonnet", "active_effort": "low"
            }],
            "adapter_defaults": [{
                "adapter_id": "codex", "default_model": "mock-fast", "default_effort": "medium"
            }],
            "quota_preferences": [{
                "adapter_id": "codex", "plan_label": "Fixture", "exact_probe_opt_in": false,
                "manual_5h_pct": 25.0, "manual_weekly_pct": null
            }]
        }
    });
    let mut invalid = backup.clone();
    invalid["preferences"]["quota_preferences"][0]["manual_5h_pct"] = json!(150.0);
    assert_eq!(
        http.post(&settings_url)
            .json(&invalid)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert!(switch.store.list_global_personas().unwrap().is_empty());

    let imported: Value = http
        .post(&settings_url)
        .json(&backup)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut expected = backup;
    expected["preferences"]["global_personas"][0]["handle"] = json!("reviewer");
    assert_eq!(imported, expected);

    let exported: Value = http
        .get(&settings_url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(exported["preferences"], imported["preferences"]);
    assert!(switch.store.get_member(existing_id, "reviewer").is_err());

    let add = || http.post(format!("{sessions_url}/{existing_id}/personas/reviewer"));
    add().send().await.unwrap().error_for_status().unwrap();
    assert_eq!(
        add().send().await.unwrap().status(),
        reqwest::StatusCode::CONFLICT
    );

    let created: Value = create_session("New session from backup")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = created["session"]["id"].as_str().unwrap();
    let reviewer = switch.store.get_member(sid, "reviewer").unwrap();
    assert_eq!(reviewer.active_model.as_deref(), Some("mock-sonnet"));
    assert_eq!(reviewer.persona_prompt, "Review diffs.");
    assert_eq!(
        switch
            .store
            .get_member(sid, "codex")
            .unwrap()
            .active_model
            .as_deref(),
        Some("mock-fast")
    );

    http.post(format!("{sessions_url}/{sid}/messages"))
        .json(&json!({
            "content":"@codex and @reviewer[mentioned-model] summarize this.",
            "recipient_overrides":{
                "codex":{"model":"mock-strong","effort":"high"},
                "reviewer":{"model":"from-menu","effort":"medium"},
                "antigravity":{"model":"unaddressed"}
            }
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let find_turn = |handle: &str| {
        switch
            .store
            .list_trajectories(sid, Some(handle), 1)
            .unwrap()
            .pop()
    };
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let (Some(codex), Some(reviewer)) = (find_turn("codex"), find_turn("reviewer")) {
                assert_eq!(codex.model.as_deref(), Some("mock-strong"));
                assert_eq!(codex.effort.as_deref(), Some("high"));
                assert_eq!(reviewer.model.as_deref(), Some("mentioned-model"));
                assert_eq!(reviewer.effort.as_deref(), Some("medium"));
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .unwrap();
    assert!(find_turn("antigravity").is_none());
    assert_eq!(
        switch
            .store
            .get_member(sid, "reviewer")
            .unwrap()
            .active_effort
            .as_deref(),
        Some("low")
    );

    http.delete(format!("{base_url}/api/personas/reviewer"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(switch.store.list_global_personas().unwrap().is_empty());
    assert!(switch.store.get_member(sid, "reviewer").is_ok());
    switch.supervisor.shutdown_all().await;
    srv.abort();
}
