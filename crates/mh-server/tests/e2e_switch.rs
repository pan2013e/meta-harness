use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::TcpListener;

use mh_acp::{AcpSupervisor, QuotaTracker, init_git_repo};
use mh_core::{AdapterConfig, AppConfig, DispatchTaskSpec, MessageKind, PromoteMode};
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

    // 7. Promote session integration branch to main checkout via Squash
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
