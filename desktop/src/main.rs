pub mod controller;
pub mod pairing;
pub mod protocol;
pub mod security;
pub mod session;
pub mod virtual_gamepad;

mod discovery;

use crate::pairing::PairingApprovalBroker;
use std::time::Duration;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = MainWindow::new()?;
    let weak = app.as_weak();

    let status_weak = weak.clone();
    let approval_ui_weak = weak.clone();
    let approval_broker = PairingApprovalBroker::default();

    let approve_broker = approval_broker.clone();
    app.on_approve_pairing(move || {
        approve_broker.decide(true);
    });
    let reject_broker = approval_broker.clone();
    app.on_reject_pairing(move || {
        reject_broker.decide(false);
    });

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
            let peer_name = request.peer_display_name.clone();
            let comparison = request.comparison_string.clone();
            let weak = approval_ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.set_pairing_peer(peer_name.into());
                    app.set_pairing_code(comparison.into());
                    app.set_pairing_pending(true);
                    app.set_discovery_status(
                        "First connection · compare the code and approve".into(),
                    );
                }
            });

            let approved = worker_approval_broker.request(request, Duration::from_secs(120));
            let weak = approval_ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.set_pairing_pending(false);
                    app.set_pairing_peer("".into());
                    app.set_pairing_code("".into());
                }
            });
            approved
        },
    );

    if let Err(error) = &_discovery {
        app.set_discovery_status(format!("Discovery unavailable · {error}").into());
    }

    // Requests are bounded and are also rejected immediately when the app exits.
    let app_result = app.run();
    approval_broker.cancel_pending();
    drop(_discovery);
    app_result?;
    Ok(())
}
