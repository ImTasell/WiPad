pub mod protocol;

mod discovery;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = MainWindow::new()?;
    let weak = app.as_weak();

    let _discovery = discovery::Discovery::start(move |status| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_discovery_status(status.into());
            }
        });
    });

    if let Err(error) = &_discovery {
        app.set_discovery_status(format!("Discovery unavailable · {error}").into());
    }

    app.run()?;
    Ok(())
}
