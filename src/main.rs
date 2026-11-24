#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod diagnostic_log;
mod hunt;
mod language;
#[cfg(target_os = "macos")]
mod macos_chrome;
mod process;
mod screen_stats;
mod settings;
mod stat_fields;
mod stats_ui;
mod update;

fn main() -> eframe::Result {
    let mut arguments = std::env::args_os().skip(1);
    let argument = arguments.next();
    if argument
        .as_deref()
        .is_some_and(|value| value == "--smoke-test")
    {
        app::icon().expect("embedded trainer icon should decode");
        let core = env!("CARGO_PKG_VERSION").split('-').next().unwrap();
        update::version(core).expect("trainer package version should be valid");
        #[cfg(windows)]
        if let Some(ready) = arguments.next() {
            update::confirm_ready(std::path::Path::new(&ready)).expect("smoke startup marker");
        }
        return Ok(());
    }
    #[cfg(windows)]
    if argument
        .as_deref()
        .is_some_and(|value| value == "--smoke-test-release")
    {
        let expected = arguments.next().and_then(|tag| tag.into_string().ok());
        let recognized = update::check("0.0.0").is_ok_and(|status| {
            expected.as_deref() == Some(&status.tag) && status.available.is_some()
        });
        return if recognized {
            Ok(())
        } else {
            std::process::exit(1);
        };
    }
    let _log = settings::state_directory()
        .and_then(|directory| diagnostic_log::Log::start(directory.join("reborn-rust.log")))
        .ok();
    #[cfg(windows)]
    if argument
        .as_deref()
        .is_some_and(|value| value == "--install-update" || value == "--smoke-test-update")
    {
        let result = if argument
            .as_deref()
            .is_some_and(|value| value == "--smoke-test-update")
        {
            update::smoke_install()
        } else {
            (|| {
                let invalid = || std::io::Error::other("Invalid installer arguments.");
                let target = arguments.next().ok_or_else(invalid)?;
                let digest = arguments
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .ok_or_else(invalid)?;
                let pid = arguments
                    .next()
                    .and_then(|value| value.to_str()?.parse().ok())
                    .ok_or_else(invalid)?;
                let created = arguments
                    .next()
                    .and_then(|value| value.to_str()?.parse().ok())
                    .ok_or_else(invalid)?;
                if arguments.next().is_some() {
                    return Err(invalid());
                }
                update::install(std::path::Path::new(&target), &digest, pid, created)
            })()
        };
        if let Err(error) = result {
            diagnostic_log::event("update.helper_failed", format_args!("{error}"));
            drop(_log);
            std::process::exit(1);
        }
        return Ok(());
    }
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(8));
        update::cleanup();
    });
    diagnostic_log::event(
        "app.start",
        format_args!("version={}", env!("CARGO_PKG_VERSION")),
    );
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |details| {
        diagnostic_log::event("app.panic", format_args!("{details}"));
        default_panic(details);
    }));
    let update_ready = argument
        .filter(|value| value == "--update-ready")
        .and_then(|_| arguments.next())
        .map(std::path::PathBuf::from);
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([980.0, 810.0])
        .with_resizable(false);
    if let Ok(icon) = app::icon() {
        viewport = viewport.with_icon(icon);
    }
    let result = eframe::run_native(
        "Azeroth Survivor Reborn Trainer",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |context| Ok(Box::new(app::TrainerApp::new(context, update_ready)))),
    );
    match &result {
        Ok(()) => diagnostic_log::event("app.exit", format_args!("ok")),
        Err(error) => diagnostic_log::event("app.exit", format_args!("error={error}")),
    }
    result
}
