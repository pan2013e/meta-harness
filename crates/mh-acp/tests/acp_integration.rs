use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};

use mh_acp::{
    AcpError, AcpSupervisor, BrokerCallbacks, MemberSpawnSpec, PermissionResolution,
    TurnStreamListener,
};
use mh_core::{
    AdapterConfig, DiscoveredConfigOption, PendingPermissionView, PermissionPolicy,
    TrajectoryEventKind,
};

struct NoopCallbacks {
    events: Mutex<Vec<(TrajectoryEventKind, Value)>>,
    touched: Mutex<Vec<String>>,
}

#[async_trait]
impl BrokerCallbacks for NoopCallbacks {
    async fn request_user_permission(
        &self,
        _prompt: PendingPermissionView,
    ) -> Result<PermissionResolution, AcpError> {
        Ok(PermissionResolution::Selected {
            option_id: "allow_once".into(),
            allow_pattern: None,
        })
    }

    async fn record_trajectory_event(
        &self,
        _turn_id: &str,
        kind: TrajectoryEventKind,
        payload: Value,
    ) {
        self.events.lock().await.push((kind, payload));
    }

    async fn record_touched_file(&self, _turn_id: &str, rel_path: String) {
        self.touched.lock().await.push(rel_path);
    }
}

struct CollectingListener {
    text: Mutex<String>,
    thoughts: Mutex<String>,
}

#[async_trait]
impl TurnStreamListener for CollectingListener {
    async fn on_text_delta(&self, _turn_id: &str, delta: String, replace: bool) {
        let mut g = self.text.lock().await;
        if replace {
            *g = delta;
        } else {
            g.push_str(&delta);
        }
    }

    async fn on_thought_delta(&self, _turn_id: &str, delta: String) {
        self.thoughts.lock().await.push_str(&delta);
    }

    async fn on_trajectory_event(
        &self,
        _turn_id: &str,
        _kind: TrajectoryEventKind,
        _payload: Value,
    ) {
    }

    async fn on_config_options_updated(&self, _options: Vec<DiscoveredConfigOption>) {}
}

fn mock_adapter_config() -> AdapterConfig {
    AdapterConfig {
        display_name: "Mock Agent".into(),
        command: env!("CARGO_BIN_EXE_mock-acp-agent").into(),
        args: Vec::new(),
        ..Default::default()
    }
}

#[tokio::test]
async fn acp_config_options_diff_capture_and_cancel() {
    let tmp = tempfile::tempdir().unwrap();
    let wt = tmp.path().join("wt");
    std::fs::create_dir_all(&wt).unwrap();

    let sup = AcpSupervisor::new(300);
    let cb = Arc::new(NoopCallbacks {
        events: Mutex::new(Vec::new()),
        touched: Mutex::new(Vec::new()),
    });

    let managed = sup
        .get_or_spawn(
            MemberSpawnSpec {
                session_id: "s_acp".into(),
                member_handle: "mock".into(),
                adapter_id: "mock".into(),
                adapter: mock_adapter_config(),
                project_root: tmp.path().to_path_buf(),
                member_worktree: wt.clone(),
                permission_policy: PermissionPolicy::WorktreeSmart,
                existing_acp_session_id: None,
                extra_env: Vec::new(),
                mcp_bridge_spec: None,
                rules: Arc::new(RwLock::new(Vec::new())),
            },
            cb.clone(),
        )
        .await
        .unwrap();

    let opts = sup.get_discovered_options("s_acp", "mock").await;
    assert!(opts.iter().any(|o| o.id == "model"));
    assert!(opts.iter().any(|o| o.id == "reasoning_effort"));

    *managed.active_turn_id.write().await = Some("turn_1".into());
    let (m, e) = managed
        .client
        .apply_model_and_effort(Some("mock-pro"), Some("high"), None)
        .await;
    assert_eq!(m.as_deref(), Some("mock-pro"));
    assert_eq!(e.as_deref(), Some("high"));

    let listener = Arc::new(CollectingListener {
        text: Mutex::new(String::new()),
        thoughts: Mutex::new(String::new()),
    });

    let (_, output) = managed
        .client
        .prompt(
            "turn_1",
            "[MOCK:WRITE path=\"src/main.rs\" content=\"fn main() {}\\n\"]",
            listener.clone(),
        )
        .await
        .unwrap();
    *managed.active_turn_id.write().await = None;

    assert!(output.final_text.contains("mock-pro:high"));
    assert!(wt.join("src/main.rs").exists());
    assert!(
        cb.events
            .lock()
            .await
            .iter()
            .any(|(k, _)| *k == TrajectoryEventKind::Diff)
    );

    // Verify fast cancellation (< 500ms)
    let sup_clone = Arc::clone(&sup);
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        sup_clone.cancel_session("s_acp").await;
    });

    let t0 = Instant::now();
    let res = managed
        .client
        .prompt("turn_2", "[MOCK:SLEEP ms=5000]", listener)
        .await;
    let elapsed = t0.elapsed();
    let _ = cancel_task.await;

    assert!(matches!(res, Err(AcpError::Cancelled)));
    assert!(
        elapsed < Duration::from_millis(600),
        "cancel took {elapsed:?}"
    );

    sup.shutdown_all().await;
}
