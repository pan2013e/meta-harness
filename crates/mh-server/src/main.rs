use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use mh_acp::{AcpSupervisor, QuotaTracker, open_url_in_browser};
use mh_core::{AppConfig, AppPaths};
use mh_server::auth::{AuthConfig, generate_random_hex_token};
use mh_server::mcp_bridge::run_mcp_bridge;
use mh_server::{AppState, build_router};
use mh_store::Store;
use mh_switch::SwitchEngine;

#[derive(Debug, Parser)]
#[command(
    name = "meta-harness",
    version,
    about = "Meta-Harness ACP Group Chat & Network Switch"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long)]
    bind: Option<String>,
    #[arg(long)]
    no_open: bool,
    #[arg(long)]
    auth_token: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Serve {
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        no_open: bool,
        #[arg(long)]
        auth_token: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    McpBridge {
        #[arg(long)]
        url: String,
        #[arg(long)]
        session: String,
        #[arg(long)]
        caller: String,
    },
    #[command(hide = true)]
    InternalMockAgent,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::InternalMockAgent) = cli.command {
        mh_acp::mock_agent::run_mock_acp_agent_stdio().await;
        return Ok(());
    }

    if let Some(Commands::McpBridge {
        url,
        session,
        caller,
    }) = cli.command
    {
        return run_mcp_bridge(url, session, caller).await;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let (bind_opt, no_open, auth_token, config_path, data_dir_opt) = match cli.command {
        Some(Commands::Serve {
            bind,
            no_open,
            auth_token,
            config,
            data_dir,
        }) => (bind, no_open, auth_token, config, data_dir),
        _ => (
            cli.bind,
            cli.no_open,
            cli.auth_token,
            cli.config,
            cli.data_dir,
        ),
    };

    let mut paths = AppPaths::resolve();
    if let Some(dd) = data_dir_opt {
        paths.data_dir = dd;
    }

    let cfg = AppConfig::load_or_default(config_path.as_deref(), &paths)?;
    let bind = bind_opt.unwrap_or_else(|| cfg.server.bind.clone());
    let open_browser = !no_open && cfg.server.open_browser_on_start;

    let store = Store::open(&paths.db_path())?;
    let quota = Arc::new(QuotaTracker::new());
    for (id, acfg) in &cfg.adapters {
        quota
            .configure_adapter(id, None, acfg.exact_quota_probe_opt_in, None, None)
            .await;
    }
    for pref in store.list_adapter_quota_prefs()? {
        quota
            .configure_adapter(
                &pref.adapter_id,
                pref.plan_label,
                pref.exact_probe_opt_in,
                pref.manual_5h_pct,
                pref.manual_weekly_pct,
            )
            .await;
    }

    let supervisor = AcpSupervisor::new(cfg.switch.idle_process_ttl_secs);
    let self_exe = std::env::current_exe()?;
    let bridge_token = generate_random_hex_token();

    let switch = SwitchEngine::new_with_paths(
        cfg,
        store,
        Arc::clone(&supervisor),
        quota,
        self_exe,
        String::new(),
        bridge_token,
        paths,
    );

    let auth = AuthConfig::from_bind_and_flag(&bind, auth_token);
    let state = AppState {
        switch: Arc::clone(&switch),
        auth: auth.clone(),
    };

    let listener = TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    let internal_mcp_url = format!("http://127.0.0.1:{}/internal/mcp", local_addr.port());
    switch.set_internal_mcp_url(internal_mcp_url).await;

    let display_url = if auth.required {
        let host = local_addr.ip();
        format!("http://{host}:{}/?token={}", local_addr.port(), auth.token)
    } else {
        format!("http://127.0.0.1:{}/", local_addr.port())
    };

    info!("Meta-Harness listening on http://{}", local_addr);
    if auth.required {
        println!("\n  Meta-Harness Remote Pairing URL:\n    {display_url}\n");
    } else {
        println!("\n  Meta-Harness UI:\n    {display_url}\n");
    }

    if open_browser
        && !auth.required
        && let Err(e) = open_url_in_browser(&display_url)
    {
        warn!("could not open browser automatically: {e}");
    }

    let app = build_router(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("shutting down ACP child processes...");
    supervisor.shutdown_all().await;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
