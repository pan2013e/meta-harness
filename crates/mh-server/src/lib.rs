pub mod auth;
pub mod harness_jobs;
pub mod mcp_bridge;
pub mod pty;
pub mod routes;
mod settings;
pub mod ws;

use std::sync::Arc;

use axum::{
    Router, middleware,
    routing::{delete, get, patch, post},
};
use rust_embed::RustEmbed;

use mh_switch::SwitchEngine;

use auth::{AuthConfig, auth_status, create_auth_session, require_user_auth};

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
pub struct WebAssets;

#[derive(Clone)]
pub struct AppState {
    pub switch: Arc<SwitchEngine>,
    pub auth: AuthConfig,
    pub harness_jobs: harness_jobs::HarnessJobs,
}

pub fn build_router(state: AppState) -> Router {
    let public_and_authed_api = Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/session", post(create_auth_session))
        .route("/api/fs/list", get(routes::list_fs))
        .route("/api/fs/pick-folder", post(routes::pick_fs_folder))
        .route("/api/adapters", get(routes::list_adapters))
        .route(
            "/api/context/estimate",
            post(routes::estimate_draft_context),
        )
        .route("/api/adapters/probe", get(routes::probe_adapters))
        .route(
            "/api/settings",
            get(settings::export_settings).post(settings::import_settings),
        )
        .route("/api/personas", post(settings::save_persona))
        .route("/api/personas/{handle}", delete(settings::delete_persona))
        .route(
            "/api/sessions/{id}/personas/{handle}",
            post(settings::add_persona_to_session),
        )
        .route("/api/adapters/{id}/version", get(harness_jobs::version))
        .route(
            "/api/adapters/{id}/profile",
            patch(harness_jobs::select_profile),
        )
        .route(
            "/api/adapters/{id}/operation",
            get(harness_jobs::status)
                .post(harness_jobs::start)
                .delete(harness_jobs::cancel),
        )
        .route(
            "/api/adapters/{id}/operation/input",
            post(harness_jobs::input),
        )
        .route(
            "/api/adapters/{id}/defaults",
            patch(routes::update_adapter_defaults),
        )
        .route("/api/quotas", get(routes::list_quotas))
        .route(
            "/api/adapters/{id}/shared-usage",
            post(routes::shared_antigravity_usage),
        )
        .route("/api/quotas/refresh", post(routes::refresh_quotas))
        .route(
            "/api/adapters/{id}/quota",
            patch(routes::update_adapter_quota),
        )
        .route(
            "/api/projects",
            get(routes::list_projects).post(routes::create_project),
        )
        .route("/api/projects/{id}", delete(routes::delete_project))
        .route(
            "/api/projects/{id}/git-init",
            post(routes::git_init_project),
        )
        .route(
            "/api/projects/{id}/sessions",
            get(routes::list_sessions).post(routes::create_session),
        )
        .route("/api/projects/{id}/memory", get(routes::get_project_memory))
        .route(
            "/api/projects/{id}/memory/facts",
            post(routes::create_project_fact),
        )
        .route(
            "/api/projects/{id}/memory/facts/{fact_id}",
            delete(routes::delete_project_fact),
        )
        .route(
            "/api/memory/jobs/{job_id}",
            patch(routes::patch_memory_job).delete(routes::delete_memory_job),
        )
        .route(
            "/api/sessions",
            get(routes::list_standalone_sessions).post(routes::create_standalone_session),
        )
        .route(
            "/api/sessions/{id}",
            get(routes::get_session)
                .patch(routes::patch_session)
                .delete(routes::delete_session),
        )
        .route(
            "/api/sessions/{id}/compact",
            post(routes::compact_session_handler),
        )
        .route("/api/sessions/{id}/pty", get(pty::session_pty_ws_handler))
        .route("/api/storage", get(routes::get_storage_handler))
        .route(
            "/api/storage/cleanup",
            post(routes::cleanup_storage_handler),
        )
        .route(
            "/api/sessions/{id}/members",
            post(routes::upsert_session_member),
        )
        .route(
            "/api/sessions/{id}/members/{handle}",
            delete(routes::delete_session_member),
        )
        .route(
            "/api/sessions/{id}/members/{handle}/config",
            patch(routes::patch_member_config),
        )
        .route(
            "/api/sessions/{id}/context",
            get(routes::get_session_context_handler),
        )
        .route(
            "/api/sessions/{id}/messages",
            get(routes::list_messages).post(routes::post_message),
        )
        .route("/api/sessions/{id}/cancel", post(routes::cancel_session))
        .route(
            "/api/sessions/{id}/chains/{chain_id}/continue",
            post(routes::continue_chain),
        )
        .route(
            "/api/sessions/{id}/dispatches/{dispatch_id}/confirm",
            post(routes::confirm_dispatch),
        )
        .route(
            "/api/sessions/{id}/dispatches/{dispatch_id}/cancel",
            post(routes::cancel_dispatch),
        )
        .route(
            "/api/sessions/{id}/permissions/{req_id}",
            post(routes::resolve_permission),
        )
        .route(
            "/api/sessions/{id}/permission-rules/{rule_id}",
            delete(routes::delete_permission_rule),
        )
        .route(
            "/api/sessions/{id}/diff",
            get(routes::get_session_diff_handler),
        )
        .route(
            "/api/sessions/{id}/files/{*path}",
            get(routes::serve_session_worktree_file),
        )
        .route(
            "/api/sessions/{id}/run-command",
            post(routes::run_session_preview_command),
        )
        .route(
            "/api/sessions/{id}/open-terminal",
            post(routes::open_session_terminal),
        )
        .route(
            "/api/sessions/{id}/promote",
            post(routes::promote_session_handler),
        )
        .route("/api/trajectories", get(routes::list_trajectories))
        .route("/api/trajectories/{turn_id}", get(routes::get_trajectory))
        .route("/api/ws", get(ws::ws_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_user_auth,
        ));

    Router::new()
        .merge(public_and_authed_api)
        .route("/internal/mcp", post(routes::internal_mcp_handler))
        .fallback(routes::serve_embedded_spa)
        .with_state(state)
}
