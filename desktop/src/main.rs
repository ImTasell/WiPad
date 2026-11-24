pub mod controller;
pub mod pairing;
pub mod protocol;
pub mod security;
pub mod virtual_gamepad;

mod discovery;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = MainWindow::new()?;
    let weak = app.as_weak();

    let status_weak = weak.clone();
    let approval_broker = PairingApprovalBroker::default();
    let worker_approval_broker = approval_broker.clone();
    let _discovery = discovery::Discovery::start(
        move |status| {
            let weak = status_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.set_discovery_status(status.into());
                }
            });
        },
        move |request| {
            let status = format!(
                "Pairing approval pending · {} · {}",
                request.peer_display_name, request.comparison_string
            );
            let weak = status_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.set_discovery_status(status.into());
                }
            });
            worker_approval_broker.request(request, Duration::from_secs(120))
        },
    );

    if let Err(error) = &_discovery {
        app.set_discovery_status(format!("Discovery unavailable · {error}").into());
    }

    // Kept alive for the future approval controls; requests time out and fail closed.
    let _approval_broker = approval_broker;
    app.run()?
    Ok(())
}
