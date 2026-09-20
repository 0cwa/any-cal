use any_cal_app::{parse_cli, AppConfig};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut config = args
        .windows(2)
        .find(|pair| pair[0] == "--config")
        .map(|pair| AppConfig::from_file(std::path::Path::new(&pair[1])))
        .transpose()
        .unwrap_or_else(|error| {
            eprintln!("configuration error: {error:?}");
            std::process::exit(2)
        })
        .unwrap_or_else(AppConfig::defaults);
    if let Err(error) = config.apply_env(std::env::vars()) {
        eprintln!("configuration error: {error:?}");
        std::process::exit(2);
    }
    let mut filtered = Vec::with_capacity(args.len());
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
    match parse_cli(filtered, config).and_then(|(command, c)| {
        if c.transport_mode == "http" {
            let mut app =
                any_cal_app::AppGeneric::<any_cal_anytype_adapter::HttpAnytypeTransport>::http(c)?;
            if command == "check" {
                println!("{}", app.check());
                Ok(())
            } else {
                app.serve()
                    .map_err(|e| any_cal_app::ConfigError::Invalid(format!("listen failed: {e}")))
            }
        } else if c.transport_mode == "fake" {
            // Fake mode is intentionally explicit: it is useful for fixtures
            // and local development but must never be the production fallback.
            let mut app = any_cal_app::App::fake(c)?;
            if command == "check" {
                println!("{}", app.check());
                Ok(())
            } else {
                app.serve()
                    .map_err(|e| any_cal_app::ConfigError::Invalid(format!("listen failed: {e}")))
            }
        } else {
            Err(any_cal_app::ConfigError::Invalid(
                "transport_mode must be http or explicit fake".into(),
            ))
        }
    }) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("configuration error: {error:?}");
            std::process::exit(2)
        }
    }
}
