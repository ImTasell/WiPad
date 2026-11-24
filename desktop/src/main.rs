pub mod controller;
pub mod pairing;
pub mod protocol;
pub mod security;

mod discovery;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = MainWindow::new()?;
    let weak = app.as_weak();

    let status_weak = weak.clone();
    let approval_weak = weak.clone();
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
                "Pairing requires approval · {} · {}",
                request.peer_display_name, request.comparison_string
            );
            let weak = approval_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.set_discovery_status(status.into());
                }
            });
            // No approval UI exists yet; never auto-trust a first-contact peer.
            false
        },
    );

    if let Err(error) = &_discovery {
        app.set_discovery_status(format!("Discovery unavailable · {error}").into());
    }

    app.run()?;
    Ok(())
}
