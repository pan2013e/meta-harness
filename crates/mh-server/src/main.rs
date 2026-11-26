#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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
use tokio::sync::oneshot;

#[cfg(any(windows, target_os = "macos"))]
mod tray;

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
    /// Run the server without a system tray icon.
    #[arg(long, global = true)]
    no_tray: bool,
    #[arg(long, global = true, hide = true)]
    desktop_child: bool,
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
    /// Check for and install the latest published standalone release.
    Update {
        /// Check the release feed without replacing the executable.
        #[arg(long)]
        check: bool,
    },
    /// Install this release binary in a user-owned bin directory.
    Install {
        #[arg(long)]
        bin_dir: Option<PathBuf>,
    },
    /// Remove the installed binary while keeping sessions and settings.
    Uninstall {
        #[arg(long)]
        bin_dir: Option<PathBuf>,
    },
    #[command(hide = true)]
    StageDesktopUpdate {
        #[arg(long)]
        version: String,
    },
    #[command(hide = true)]
    InternalMockAgent,
}

fn main() -> anyhow::Result<()> {
    #[cfg(all(windows, not(debug_assertions)))]
    if std::env::args_os().any(|arg| {
        matches!(
            arg.to_str(),
            Some(
                "--help"
                    | "-h"
                    | "--version"
                    | "-V"
                    | "--no-tray"
                    | "update"
                    | "install"
                    | "uninstall"
            )
        )
    }) && !std::env::args_os().any(|arg| arg == "--desktop-child")
    {
        // The published binary uses the GUI subsystem to stay silent when launched from Explorer.
        // Attach only to an existing parent console when invoked as an interactive CLI.
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    let mut cli = Cli::parse();
    if cli.desktop_child {
        anyhow::ensure!(cli.command.is_none(), "Invalid desktop launcher arguments");
        cli.auth_token = Some(std::env::var("MH_DESKTOP_TOKEN")?);
        // SAFETY: No async runtime or background threads have started. Keep the desktop
        // pairing token out of the environment inherited by agents and shells.
        unsafe {
            std::env::remove_var("MH_DESKTOP_TOKEN");
        }
    }
    let runtime = tokio::runtime::Runtime::new()?;
    #[cfg(any(windows, target_os = "macos"))]
    if !cfg!(debug_assertions)
        && !cli.no_tray
        && !cli.desktop_child
        && matches!(cli.command, None | Some(Commands::Serve { .. }))
    {
        return tray::run(cli, runtime);
    }
    runtime.block_on(run(cli, None, None))
}

async fn run(
    cli: Cli,
    tray_shutdown: Option<oneshot::Receiver<()>>,
    tray_ready: Option<oneshot::Sender<String>>,
) -> anyhow::Result<()> {
    match &cli.command {
        Some(Commands::Update { check }) => return mh_server::release::update_cli(*check).await,
        Some(Commands::Install { bin_dir }) => {
            return mh_server::release::install_cli(bin_dir.clone());
        }
        Some(Commands::Uninstall { bin_dir }) => {
            return mh_server::release::uninstall_cli(bin_dir.clone());
        }
        Some(Commands::StageDesktopUpdate { version }) => {
            let path = mh_server::release::stage_desktop_update(version).await?;
            println!("{}", serde_json::to_string(&path)?);
            return Ok(());
        }
        _ => {}
    }
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

    let mut cfg = AppConfig::load_or_default(config_path.as_deref(), &paths)?;
    mh_server::harness_jobs::restore_profiles(&mut cfg, &paths.data_dir);
    let bind = bind_opt.unwrap_or_else(|| cfg.server.bind.clone());
    let open_browser = !no_open && !cli.desktop_child && cfg.server.open_browser_on_start;

    let store = Store::open(&paths.db_path())?;
    let quota = Arc::new(QuotaTracker::new());
    for (id, acfg) in &cfg.adapters {
        quota.configure_launch_profile(id, acfg).await;
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
        harness_jobs: Default::default(),
    };

    let listener = TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    let (bridge_addr, bridge_task) = if local_addr.ip().is_loopback() {
        (local_addr, None)
    } else {
        // A listener bound to a particular LAN address cannot receive loopback connections.
        let bridge_listener = TcpListener::bind("127.0.0.1:0").await?;
        let bridge_addr = bridge_listener.local_addr()?;
        let bridge_router = axum::Router::new()
            .route(
                "/internal/mcp",
                axum::routing::post(mh_server::routes::internal_mcp_handler),
            )
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(bridge_listener, bridge_router).await {
                warn!(%error, "local MCP listener stopped");
            }
        });
        (bridge_addr, Some(task))
    };
    let internal_mcp_url = format!("http://{bridge_addr}/internal/mcp");
    switch.set_internal_mcp_url(internal_mcp_url).await;

    let display_url = if auth.required {
        format!("http://{local_addr}/?token={}", auth.token)
    } else {
        format!("http://{local_addr}/")
    };

    info!("Meta-Harness listening on http://{}", local_addr);
    if cli.desktop_child {
        use std::io::Write;
        println!("MH_READY {display_url}");
        std::io::stdout().flush()?;
    } else if let Some(ready) = tray_ready {
        let _ = ready.send(display_url.clone());
    } else if auth.required {
        println!("\n  Meta-Harness Remote Pairing URL:\n    {display_url}\n");
    } else {
        println!("\n  Meta-Harness UI:\n    {display_url}\n");
    }

    if open_browser
        && local_addr.ip().is_loopback()
        && let Err(e) = open_url_in_browser(&display_url)
    {
        warn!("could not open browser automatically: {e}");
    }

    let app = build_router(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(tray_shutdown, cli.desktop_child))
        .await?;

    info!("shutting down ACP child processes...");
    supervisor.shutdown_all().await;
    if let Some(task) = bridge_task {
        task.abort();
    }
    Ok(())
}

async fn shutdown_signal(tray: Option<oneshot::Receiver<()>>, desktop_child: bool) {
    use tokio::io::AsyncReadExt;

    tokio::select! {
        _ = async {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
        } => {}
        _ = async {
            if let Some(tray) = tray {
                let _ = tray.await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {}
        _ = async {
            if desktop_child {
                let _ = tokio::io::stdin().read(&mut [0]).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {}
    }
}
