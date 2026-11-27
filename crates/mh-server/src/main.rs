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
use mh_server::{AppState, build_bridge_router, build_router};
use mh_store::Store;
use mh_switch::SwitchEngine;
use tokio::sync::oneshot;

mod privilege;
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
    /// Server-side outbound proxy URL for Meta-Harness and its harnesses.
    #[arg(long, global = true)]
    proxy: Option<String>,
    /// PEM certificate chain for HTTPS. Required when listening on a LAN address.
    #[arg(long, global = true, requires = "tls_key")]
    tls_cert: Option<PathBuf>,
    /// PEM private key for the HTTPS certificate.
    #[arg(long, global = true, requires = "tls_cert")]
    tls_key: Option<PathBuf>,
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
    let refusal = match privilege::elevated() {
        Ok(false) => None,
        Ok(true) => Some(privilege::REFUSAL_MESSAGE.to_string()),
        Err(error) => Some(format!("Could not verify the process privileges: {error}")),
    };
    if let Some(message) = refusal {
        #[cfg(windows)]
        if !cli.desktop_child {
            privilege::show_if_no_stderr(&message);
        }
        eprintln!("{message}");
        std::process::exit(privilege::REFUSAL_EXIT_CODE);
    }
    let mut desktop_network_settings = None;
    if cli.desktop_child {
        anyhow::ensure!(cli.command.is_none(), "Invalid desktop launcher arguments");
        cli.auth_token = Some(std::env::var("MH_DESKTOP_TOKEN")?);
        desktop_network_settings =
            std::env::var_os("MH_DESKTOP_NETWORK_SETTINGS").map(PathBuf::from);
        // SAFETY: No async runtime or background threads have started. Keep the desktop
        // pairing token out of the environment inherited by agents and shells.
        unsafe {
            std::env::remove_var("MH_DESKTOP_TOKEN");
            std::env::remove_var("MH_DESKTOP_NETWORK_SETTINGS");
        }
    }
    if !matches!(cli.command, Some(Commands::McpBridge { .. })) {
        mh_server::proxy_health::initialize(cli.proxy.as_deref(), desktop_network_settings)
            .map_err(anyhow::Error::msg)?;
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
    if !matches!(
        cli.command,
        Some(Commands::McpBridge { .. } | Commands::Install { .. } | Commands::Uninstall { .. })
    ) {
        mh_server::proxy_health::check_at_startup(cli.desktop_child).await;
    }
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
    let listener = TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    let auth = AuthConfig::for_listener(local_addr, cli.tls_cert.is_some(), auth_token)
        .map_err(anyhow::Error::msg)?;
    let tls = match (cli.tls_cert, cli.tls_key) {
        (Some(cert), Some(key)) => {
            Some(axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?)
        }
        _ => None,
    };

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

    let proxy_health_task = tokio::spawn(mh_server::proxy_health::run_periodically());

    let state = AppState {
        switch: Arc::clone(&switch),
        auth: auth.clone(),
        harness_jobs: Default::default(),
    };

    // The internal capability endpoint stays on a separate loopback listener.
    let bridge_listener = TcpListener::bind("127.0.0.1:0").await?;
    let bridge_addr = bridge_listener.local_addr()?;
    let bridge_router = build_bridge_router(state.clone());
    let bridge_task = tokio::spawn(async move {
        if let Err(error) = axum::serve(bridge_listener, bridge_router).await {
            warn!(%error, "local MCP listener stopped");
        }
    });
    let internal_mcp_url = format!("http://{bridge_addr}/internal/mcp");
    switch.set_internal_mcp_url(internal_mcp_url).await;

    let scheme = if tls.is_some() { "https" } else { "http" };
    let authority = bind.rsplit_once(':').map_or_else(
        || local_addr.to_string(),
        |(host, _)| format!("{host}:{}", local_addr.port()),
    );
    let mut display_url = reqwest::Url::parse(&format!("{scheme}://{authority}/"))?;
    if auth.required {
        display_url
            .query_pairs_mut()
            .append_pair("token", &auth.token);
        let fragment = display_url.query().map(str::to_string);
        display_url.set_query(None);
        display_url.set_fragment(fragment.as_deref());
    }
    let display_url = display_url.as_str().to_string();

    info!("Meta-Harness listening on {scheme}://{local_addr}");
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
    let shutdown = shutdown_signal(tray_shutdown, cli.desktop_child);
    let result = if let Some(tls) = tls {
        let handle = axum_server::Handle::new();
        let server =
            axum_server::from_tcp_rustls(listener.into_std()?, tls)?.handle(handle.clone());
        let signal = tokio::spawn(async move {
            shutdown.await;
            handle.graceful_shutdown(Some(std::time::Duration::from_secs(5)));
        });
        let result = server.serve(app.into_make_service()).await;
        signal.abort();
        result
    } else {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
    };

    info!("shutting down ACP child processes...");
    supervisor.shutdown_all().await;
    bridge_task.abort();
    proxy_health_task.abort();
    Ok(result?)
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

#[cfg(test)]
mod tests {
    use mh_server::proxy_health::proxy_reaches_internet;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    #[tokio::test]
    async fn a_reachable_proxy_without_internet_fails_health_check() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut connection = loop {
                match listener.accept() {
                    Ok((connection, _)) => break connection,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("proxy did not receive the connectivity check: {error}"),
                }
            };
            connection
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(&mut connection);
            reader.read_line(&mut request).unwrap();
            assert!(request.starts_with("CONNECT www.gstatic.com:443 "));
            connection
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        assert!(!proxy_reaches_internet(&format!("http://{address}")).await);
        server.join().unwrap();
    }
}
