use any_cal_app::{parse_cli, AppConfig};
use any_cal_gui::{ConfigViewModel, ConfigWindow, LocalHealthClient};
use slint::{ComponentHandle, SharedString};
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut config = args
        .windows(2)
        .find(|pair| pair[0] == "--config")
        .map(|pair| AppConfig::from_file(std::path::Path::new(&pair[1])))
        .transpose()
        .map_err(|e| std::io::Error::other(format!("configuration error: {e:?}")))?
        .unwrap_or_else(AppConfig::defaults);
    config
        .apply_env(std::env::vars())
        .map_err(|e| std::io::Error::other(format!("configuration error: {e:?}")))?;
    let mut filtered = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if arg == "--config" {
            skip = true;
            continue;
        }
        filtered.push(arg);
    }
    let (_, config) = parse_cli(filtered, config)
        .map_err(|e| std::io::Error::other(format!("configuration error: {e:?}")))?;
    let window = ConfigWindow::new()?;
    let model = Rc::new(RefCell::new(ConfigViewModel::new(config)));
    {
        let c = model.borrow().config.clone();
        window.set_endpoint(c.endpoint.into());
        window.set_api_version(c.api_version.into());
        window.set_space_id(c.space_id.into());
        window.set_contacts_collection(c.contacts_collection.into());
        window.set_tasks_collection(c.tasks_collection.into());
        window.set_listen_address(c.listen_address.into());
    }
    let callback_model = model.clone();
    let callback_window = window.as_weak();
    window.on_health_clicked(move || {
        if let Some(window) = callback_window.upgrade() {
            let mut model = callback_model.borrow_mut();
            model.set_field("endpoint", window.get_endpoint().to_string());
            model.set_field("api_version", window.get_api_version().to_string());
            model.set_field("space_id", window.get_space_id().to_string());
            model.set_field(
                "contacts_collection",
                window.get_contacts_collection().to_string(),
            );
            model.set_field(
                "tasks_collection",
                window.get_tasks_collection().to_string(),
            );
            model.set_field("listen_address", window.get_listen_address().to_string());
            model.set_field("token", window.get_token().to_string());
            let local_auth = window.get_local_auth().to_string();
            if !local_auth.is_empty() {
                model.set_field("local_auth", local_auth);
            }
            let status = if model.validate() {
                model.health(&LocalHealthClient)
            } else {
                model.status.clone()
            };
            window.set_status_text(SharedString::from(status));
        }
    });
    Ok(window.run()?)
}
