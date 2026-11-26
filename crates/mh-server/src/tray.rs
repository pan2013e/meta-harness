use tao::{
    event::Event,
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
};
use tokio::{runtime::Runtime, sync::oneshot};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};

use super::Cli;

enum TrayEvent {
    Ready(String),
    Menu(MenuEvent),
    Exit,
}

pub(super) fn run(cli: Cli, runtime: Runtime) -> anyhow::Result<()> {
    let mut builder = EventLoopBuilder::<TrayEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.set_activation_policy(ActivationPolicy::Accessory);
    }
    let mut event_loop = builder.build();
    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(std::sync::Arc::new(move |event| {
        let _ = menu_proxy.send_event(TrayEvent::Menu(event));
    })));

    let menu = Menu::new();
    let open = MenuItem::new("Open Meta-Harness", false, None);
    let quit = MenuItem::new("Quit Meta-Harness", true, None);
    menu.append_items(&[&open, &quit])?;
    let icon = Icon::from_rgba(include_bytes!("../../../assets/tray.rgba").to_vec(), 32, 32)?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let server_proxy = proxy.clone();
    let server = runtime.spawn(async move {
        let result = super::run(cli, Some(shutdown_rx), Some(ready_tx)).await;
        let _ = server_proxy.send_event(TrayEvent::Exit);
        result
    });
    runtime.spawn(async move {
        if let Ok(url) = ready_rx.await {
            let _ = proxy.send_event(TrayEvent::Ready(url));
        }
    });

    let mut url = None;
    let mut tray = None;
    let mut setup_error = None;
    event_loop.run_return(|event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(tao::event::StartCause::Init) => {
                let result = TrayIconBuilder::new()
                    .with_icon(icon.clone())
                    .with_tooltip("Meta-Harness")
                    .with_menu(Box::new(menu.clone()))
                    .with_menu_on_left_click(true)
                    .build();
                match result {
                    Ok(icon) => tray = Some(icon),
                    Err(error) => {
                        setup_error = Some(error);
                        *flow = ControlFlow::Exit;
                    }
                }
            }
            Event::UserEvent(TrayEvent::Ready(ready)) => {
                url = Some(ready);
                open.set_enabled(true);
            }
            Event::UserEvent(TrayEvent::Menu(event)) if event.id == *quit.id() => {
                *flow = ControlFlow::Exit;
            }
            Event::UserEvent(TrayEvent::Menu(event)) if event.id == *open.id() => {
                if let Some(url) = url.clone() {
                    runtime.spawn_blocking(move || {
                        let _ = mh_acp::open_url_in_browser(&url);
                    });
                }
            }
            Event::UserEvent(TrayEvent::Exit) => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
    drop(tray);
    let _ = shutdown_tx.send(());
    let result = runtime
        .block_on(async { tokio::time::timeout(std::time::Duration::from_secs(5), server).await });
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    if let Some(error) = setup_error {
        return Err(error.into());
    }
    result??
}
