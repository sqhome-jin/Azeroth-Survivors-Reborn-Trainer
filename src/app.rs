use std::{
    io::{self, Cursor},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Button, Color32, RichText};
use egui_extras::{Column, TableBuilder};

use crate::{
    diagnostic_log::event,
    hunt::{self, Snapshot},
    language::Language,
    process::{self, Games, Session},
    settings::Settings,
    stats_ui::{STATS, Stat},
    update,
};

const MACOS: bool = cfg!(target_os = "macos");
const GOLD: Color32 = Color32::from_rgb(227, 190, 105);
const MINT: Color32 = if MACOS {
    Color32::from_rgb(117, 224, 184)
} else {
    Color32::from_rgb(114, 224, 190)
};
const FRAME: Color32 = if MACOS {
    Color32::from_rgb(24, 26, 31)
} else {
    Color32::from_rgb(20, 32, 44)
};
const SURFACE: Color32 = if MACOS {
    Color32::from_rgb(39, 43, 52)
} else {
    Color32::from_rgb(36, 54, 75)
};
const MUTED: Color32 = if MACOS {
    Color32::from_rgb(163, 171, 187)
} else {
    Color32::from_rgb(169, 184, 200)
};
const TEXT: Color32 = if MACOS {
    Color32::from_rgb(235, 239, 248)
} else {
    Color32::from_rgb(229, 233, 239)
};
const ACCENT: Color32 = if MACOS {
    Color32::from_rgb(110, 165, 255)
} else {
    Color32::from_rgb(86, 214, 176)
};
const ACCENT_TEXT: Color32 = if MACOS {
    Color32::from_rgb(14, 28, 53)
} else {
    Color32::from_rgb(16, 37, 31)
};
const EDITOR_WIDTH: f32 = 242.0;
const VERSION: &str = env!("CARGO_PKG_VERSION");
const LOCATE_LABEL: &str = if cfg!(windows) {
    "Locate exe file"
} else {
    "Locate game file"
};

pub fn icon() -> io::Result<egui::IconData> {
    let image = include_bytes!("../assets/trainer-logo.png");
    let mut reader = png::Decoder::new(Cursor::new(image))
        .read_info()
        .map_err(io::Error::other)?;
    let len = reader
        .output_buffer_size()
        .ok_or_else(|| io::Error::other("Trainer icon is too large."))?;
    let mut rgba = vec![0; len];
    let frame = reader.next_frame(&mut rgba).map_err(io::Error::other)?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err(io::Error::other("Trainer icon must be RGBA8."));
    }
    rgba.truncate(frame.buffer_size());
    Ok(egui::IconData {
        rgba,
        width: frame.width,
        height: frame.height,
    })
}

struct Search {
    apply_targets: bool,
    stop: Arc<AtomicBool>,
    // True while OCR is scanning for the Stats panel, so the window can ask for the Character panel.
    waiting_for_panel: Arc<AtomicBool>,
    started: Instant,
    session: Session,
    result: Receiver<io::Result<FindOutcome>>,
}

struct FindOutcome {
    snapshot: Option<Snapshot>,
    attempts: usize,
    screen_stats: Option<String>,
    ocr_error: Option<String>,
    last_error: Option<String>,
}

const MEMORY_RETRIES: usize = 5;
// Pause between OCR scans while the Stats panel is not on screen: 20 steps of 100 ms (2 seconds).
const OCR_RETRY_DELAY_STEPS: usize = 20;

fn check_stop(stop: &AtomicBool) -> io::Result<()> {
    if stop.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Find cancelled.",
        ))
    } else {
        Ok(())
    }
}

// Waits in 100 ms steps so that Stop is honoured quickly.
fn pause(stop: &AtomicBool, steps: usize) -> io::Result<()> {
    for _ in 0..steps {
        check_stop(stop)?;
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

// Sets `waiting` for the whole scan, including the error paths.
fn wait_for_stats_panel(
    session: &Session,
    stop: &AtomicBool,
    waiting: &AtomicBool,
    outcome: &mut FindOutcome,
) -> io::Result<()> {
    waiting.store(true, Ordering::Release);
    let result = scan_for_panel(session, stop, outcome);
    waiting.store(false, Ordering::Release);
    result
}

// OCR only locates the Stats panel, so it keeps scanning until the panel is on screen.
fn scan_for_panel(
    session: &Session,
    stop: &AtomicBool,
    outcome: &mut FindOutcome,
) -> io::Result<()> {
    let mut scan = 0;
    loop {
        check_stop(stop)?;
        scan += 1;
        match crate::screen_stats::read(session.pid) {
            Ok(reading) => {
                event(
                    "hunt.ocr_completed",
                    format_args!("game_pid={} scan={scan}", session.pid),
                );
                outcome.screen_stats = Some(reading);
                outcome.ocr_error = None;
                return Ok(());
            }
            Err(error) => {
                event(
                    "hunt.ocr_failed",
                    format_args!("game_pid={} scan={scan} error={error}", session.pid),
                );
                outcome.ocr_error = Some(error.to_string());
                pause(stop, OCR_RETRY_DELAY_STEPS)?;
            }
        }
    }
}

// Returns true when a verified stat mapping was found.
fn memory_discovery(
    session: &Session,
    stop: &AtomicBool,
    attempts: usize,
    outcome: &mut FindOutcome,
) -> io::Result<bool> {
    for attempt in 1..=attempts {
        check_stop(stop)?;
        outcome.attempts += 1;
        event(
            "hunt.attempt",
            format_args!("game_pid={} attempt={attempt} mode=Memory", session.pid),
        );
        match hunt::find(session, stop) {
            Ok(snapshot) if snapshot.stats.is_some() => {
                outcome.snapshot = Some(snapshot);
                return Ok(true);
            }
            Ok(snapshot) => {
                outcome.last_error = Some(format!(
                    "Build {} did not produce a verified stat mapping.",
                    snapshot.build
                ));
                outcome.snapshot = Some(snapshot);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => outcome.last_error = Some(error.to_string()),
        }
        if attempt < attempts {
            pause(stop, 5)?;
        }
    }
    Ok(false)
}

// OCR finds the Stats panel on screen. Memory then reads the character and sets the values.
fn find_with_fallback(
    session: &Session,
    stop: &AtomicBool,
    waiting: &AtomicBool,
) -> io::Result<FindOutcome> {
    let mut outcome = FindOutcome {
        snapshot: None,
        attempts: 0,
        screen_stats: None,
        ocr_error: None,
        last_error: None,
    };
    // OCR confirms the Stats panel is on screen where supported; memory discovery follows.
    if crate::screen_stats::SUPPORTED {
        wait_for_stats_panel(session, stop, waiting, &mut outcome)?;
    }
    memory_discovery(session, stop, MEMORY_RETRIES + 1, &mut outcome)?;
    Ok(outcome)
}

impl Drop for Search {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct Apply {
    stop: Arc<AtomicBool>,
    result: Receiver<io::Result<hunt::ApplyOutcome>>,
}

impl Drop for Apply {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

pub struct TrainerApp {
    language: Language,
    #[cfg(target_os = "macos")]
    native_chrome_installed: bool,
    #[cfg(target_os = "macos")]
    chinese_font_loaded: bool,
    settings: Settings,
    session: Option<Session>,
    process_result: Option<Receiver<io::Result<Games>>>,
    launch_result: Option<Receiver<io::Result<Session>>>,
    stop_result: Option<Receiver<io::Result<()>>>,
    confirm_stop: Option<Session>,
    next_process_check: Instant,
    running: bool,
    scan: Option<Search>,
    apply: Option<Apply>,
    enabled: [bool; 17],
    targets: [u32; 17],
    target_entry: String,
    last_apply_error: Option<String>,
    applied_stats: std::collections::BTreeSet<&'static str>,
    field_issues: std::collections::BTreeMap<&'static str, String>,
    snapshot: Option<Snapshot>,
    status: String,
    error: Option<String>,
    show_details: bool,
    screen_stats: Option<String>,
    screen_stats_result: Option<Receiver<io::Result<String>>>,
    log_tail: String,
    next_log_read: Instant,
    selected: usize,
    logo: Option<egui::TextureHandle>,
    update_result: Option<Receiver<io::Result<update::ReleaseStatus>>>,
    update_status: String,
    update_asset: Option<update::Asset>,
    update_error: Option<String>,
    manual_update_check: bool,
    using_latest_until: Option<Instant>,
    install_result: Option<Receiver<io::Result<()>>>,
    update_ready: Option<PathBuf>,
}

fn status_color(ui: &egui::Ui) -> Color32 {
    if MACOS && !ui.visuals().dark_mode {
        Color32::from_rgb(21, 103, 77)
    } else {
        MINT
    }
}

fn muted_color(ui: &egui::Ui) -> Color32 {
    if MACOS && !ui.visuals().dark_mode {
        Color32::from_rgb(83, 95, 115)
    } else {
        MUTED
    }
}

fn right_aligned_row(ui: &mut egui::Ui, height: f32, content: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        content,
    );
}

fn update_button_row(ui: &mut egui::Ui, buttons: impl FnOnce(&mut egui::Ui)) {
    let height = ui.spacing().interact_size.y + ui.spacing().button_padding.y * 2.0;
    right_aligned_row(ui, height, buttons);
}

fn checked_targets(enabled: &[bool; 17], targets: &[u32; 17]) -> hunt::Targets {
    STATS
        .iter()
        .zip(enabled)
        .zip(targets)
        .filter_map(|((stat, enabled), target)| enabled.then_some((stat.key, *target)))
        .collect()
}

fn task<T: Send + 'static>(
    context: &egui::Context,
    call: impl FnOnce() -> T + Send + 'static,
) -> Receiver<T> {
    let (sender, receiver) = mpsc::channel();
    let context = context.clone();
    thread::spawn(move || {
        let _ = sender.send(call());
        context.request_repaint();
    });
    receiver
}

fn finished<T>(receiver: &mut Option<Receiver<io::Result<T>>>) -> Option<io::Result<T>> {
    let result = match receiver.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => {
            Err(io::Error::other("Background task stopped unexpectedly."))
        }
    };
    *receiver = None;
    Some(result)
}

impl TrainerApp {
    pub fn new(context: &eframe::CreationContext<'_>, update_ready: Option<PathBuf>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = FRAME;
        visuals.window_fill = FRAME;
        visuals.override_text_color = Some(TEXT);
        visuals.widgets.inactive.weak_bg_fill = SURFACE;
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(53, 81, 108);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(53, 81, 108);
        visuals.selection.bg_fill = Color32::from_rgb(35, 68, 63);
        if MACOS {
            let mut light = egui::Visuals::light();
            light.panel_fill = Color32::from_rgb(242, 244, 249);
            light.window_fill = Color32::WHITE;
            light.faint_bg_color = Color32::from_black_alpha(5);
            light.selection.bg_fill = Color32::from_rgb(211, 228, 255);
            light.selection.stroke.color = Color32::from_rgb(21, 39, 68);
            light.override_text_color = Some(Color32::from_rgb(32, 39, 53));
            visuals.extreme_bg_color = Color32::from_rgb(22, 24, 29);
            visuals.faint_bg_color = Color32::from_white_alpha(7);
            visuals.selection.bg_fill = Color32::from_rgb(43, 67, 105);
            visuals.selection.stroke.color = TEXT;
            for (palette, backgrounds, border) in [
                (
                    &mut visuals,
                    [
                        SURFACE,
                        Color32::from_rgb(54, 62, 77),
                        Color32::from_rgb(62, 76, 98),
                    ],
                    Color32::from_white_alpha(27),
                ),
                (
                    &mut light,
                    [
                        Color32::WHITE,
                        Color32::from_rgb(232, 238, 248),
                        Color32::from_rgb(218, 230, 249),
                    ],
                    Color32::from_black_alpha(32),
                ),
            ] {
                palette.window_corner_radius = egui::CornerRadius::same(14);
                for (widget, background) in [
                    (&mut palette.widgets.inactive, backgrounds[0]),
                    (&mut palette.widgets.hovered, backgrounds[1]),
                    (&mut palette.widgets.active, backgrounds[2]),
                    (&mut palette.widgets.open, backgrounds[0]),
                ] {
                    widget.bg_fill = background;
                    widget.weak_bg_fill = background;
                    widget.bg_stroke = egui::Stroke::new(1.0, border);
                    widget.corner_radius = egui::CornerRadius::same(8);
                }
            }
            context.egui_ctx.set_visuals_of(egui::Theme::Dark, visuals);
            context.egui_ctx.set_visuals_of(egui::Theme::Light, light);
        } else {
            context.egui_ctx.set_visuals(visuals);
        }
        context.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 7.0);
            style.spacing.button_padding = egui::vec2(12.0, 7.0);
            style.spacing.interact_size.y = 27.0;
            for (style_name, size) in [
                (egui::TextStyle::Body, 15.0),
                (egui::TextStyle::Button, 14.0),
            ] {
                style
                    .text_styles
                    .insert(style_name, egui::FontId::proportional(size));
            }
        });
        let logo = icon().ok().map(|icon| {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            context
                .egui_ctx
                .load_texture("trainer-logo", image, egui::TextureOptions::LINEAR)
        });
        let (settings, error) = match Settings::load() {
            Ok(settings) => {
                event(
                    "settings.loaded",
                    format_args!("saved_game_path={}", settings.game_path().is_some()),
                );
                let notice = settings.recovery_notice.clone();
                if let Some(notice) = &notice {
                    event("settings.recovered", format_args!("{notice}"));
                }
                (settings, notice)
            }
            Err(error) => {
                event("settings.load_failed", format_args!("{error}"));
                (Settings::default(), Some(format!("Settings: {error}")))
            }
        };
        let prefs = settings.stat_preferences();
        let enabled = std::array::from_fn(|index| STATS[index].checked(prefs));
        let targets = std::array::from_fn(|index| STATS[index].saved_target(prefs));
        Self {
            language: Language::default(),
            #[cfg(target_os = "macos")]
            native_chrome_installed: false,
            #[cfg(target_os = "macos")]
            chinese_font_loaded: false,
            settings,
            error,
            session: None,
            process_result: None,
            launch_result: None,
            stop_result: None,
            confirm_stop: None,
            next_process_check: Instant::now(),
            running: false,
            scan: None,
            apply: None,
            enabled,
            targets,
            target_entry: targets[0].to_string(),
            last_apply_error: None,
            applied_stats: std::collections::BTreeSet::new(),
            field_issues: Default::default(),
            snapshot: None,
            status: "Start Reborn to find a character.".into(),
            show_details: false,
            screen_stats: None,
            screen_stats_result: None,
            log_tail: String::new(),
            next_log_read: Instant::now(),
            selected: 0,
            logo,
            update_result: Some(task(&context.egui_ctx, || update::check(VERSION))),
            update_status: "Checking…".into(),
            update_asset: None,
            update_error: None,
            manual_update_check: false,
            using_latest_until: None,
            install_result: None,
            update_ready,
        }
    }

    fn set_session(&mut self, session: Option<Session>) {
        if self.session != session {
            if let Some(old) = &self.session {
                event("process.detached", format_args!("game_pid={}", old.pid));
            }
            if let Some(new) = &session {
                event("process.attached", format_args!("game_pid={}", new.pid));
            }
            if self.scan.is_some() {
                event("hunt.cancelled", format_args!("reason=connection_changed"));
            }
            self.session = session;
            self.screen_stats = None;
            self.screen_stats_result = None;
            self.last_apply_error = None;
            self.scan = None;
            self.reset_apply();
            self.snapshot = None;
            self.applied_stats.clear();
            self.status = if self.session.is_some() {
                "Start a run, then click Find.".into()
            } else if self.running {
                "Choose the correct game installation.".into()
            } else {
                "Start Reborn to find a character.".into()
            };
        }
    }

    fn poll(&mut self, context: &egui::Context) {
        if let Some(result) = finished(&mut self.screen_stats_result) {
            self.screen_stats = Some(result.unwrap_or_else(|error| error.to_string()));
        }
        if let Some(result) = finished(&mut self.process_result) {
            match result {
                Ok(games) => {
                    if self.running != games.running() {
                        event(
                            "process.presence",
                            format_args!("game_running={}", games.running()),
                        );
                    }
                    self.running = games.running();
                    self.set_session(
                        games
                            .select(self.game_path().as_deref(), self.session.as_ref())
                            .cloned(),
                    );
                }
                Err(error) => {
                    self.running = true;
                    self.set_session(None);
                    let message = format!("Game detection: {error}");
                    if self.error.as_deref() != Some(message.as_str()) {
                        event("process.discovery_failed", format_args!("{error}"));
                    }
                    self.error = Some(message);
                }
            }
        }
        if let Some(result) = finished(&mut self.launch_result) {
            match result {
                Ok(session) => {
                    event(
                        "process.launch_finished",
                        format_args!("game_pid={}", session.pid),
                    );
                    self.running = true;
                    self.set_session(Some(session));
                }
                Err(error) => {
                    event("process.launch_failed", format_args!("{error}"));
                    self.error = Some(error.to_string());
                }
            }
        }
        if let Some(result) = finished(&mut self.stop_result) {
            if let Err(error) = result {
                event("process.stop_failed", format_args!("{error}"));
                self.error = Some(format!("Could not stop Reborn: {error}"));
            } else {
                event(
                    "process.stop_sent",
                    format_args!("waiting_for_process_exit"),
                );
            }
            self.next_process_check = Instant::now();
        }
        if self.process_result.is_none()
            && self.launch_result.is_none()
            && self.stop_result.is_none()
            && Instant::now() >= self.next_process_check
        {
            self.process_result = Some(task(context, process::discover));
            self.next_process_check = Instant::now() + Duration::from_secs(1);
        }
        let outcome = self
            .scan
            .as_ref()
            .and_then(|search| match search.result.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err(io::Error::other("Find stopped unexpectedly.")))
                }
            });
        if let Some(outcome) = outcome {
            let search = self.scan.take().unwrap();
            if self.session.as_ref() == Some(&search.session)
                && !search.stop.load(Ordering::Relaxed)
            {
                match outcome {
                    Ok(outcome) => {
                        if let Some(stats) = outcome.screen_stats {
                            self.screen_stats = Some(stats);
                        }
                        let Some(snapshot) = outcome.snapshot else {
                            self.status = outcome.ocr_error.unwrap_or_else(|| {
                                outcome
                                    .last_error
                                    .unwrap_or_else(|| "No character or stats were found.".into())
                            });
                            return;
                        };
                        event(
                            "hunt.finished",
                            format_args!(
                                "game_pid={} build={} character_found={} elapsed_ms={}",
                                search.session.pid,
                                snapshot.build,
                                snapshot.character.is_some(),
                                search.started.elapsed().as_millis()
                            ),
                        );
                        self.status = if snapshot.stats.is_some() {
                            "Character and stats found.".into()
                        } else if snapshot.character.is_some() {
                            let reason = match snapshot.stat_layout {
                                hunt::MatchStatus::DifferentLayout => "player stat layout changed",
                                hunt::MatchStatus::MissingInstructionProof => {
                                    "stat instructions did not match"
                                }
                                hunt::MatchStatus::Verified => "no live stat mapping",
                            };
                            let ocr_note = if self.screen_stats.is_some() {
                                " OCR captured the visible panel; it cannot verify writable memory fields."
                            } else {
                                " Open the Character panel with C for OCR fallback."
                            };
                            format!(
                                "Build {}: character found, {reason}.{ocr_note}",
                                snapshot.build
                            )
                        } else {
                            let detail = outcome
                                .ocr_error
                                .or(outcome.last_error)
                                .unwrap_or_else(|| "No live character verified.".into());
                            format!(
                                "{detail} ({} memory attempts). Open the Character panel with C for OCR fallback.",
                                outcome.attempts
                            )
                        };
                        let verified = snapshot.stats.is_some();
                        self.snapshot = Some(snapshot);
                        if verified && search.apply_targets {
                            let targets = checked_targets(&self.enabled, &self.targets);
                            if !targets.is_empty() {
                                self.start_apply(context, targets);
                            }
                        }
                    }
                    Err(error) => {
                        event(
                            "hunt.failed",
                            format_args!(
                                "game_pid={} elapsed_ms={} error={error}",
                                search.session.pid,
                                search.started.elapsed().as_millis()
                            ),
                        );
                        self.status = error.to_string();
                    }
                }
            }
        }
        if let Some(outcome) = self
            .apply
            .as_ref()
            .and_then(|apply| match apply.result.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err(io::Error::other("Apply worker stopped unexpectedly.")))
                }
                Err(TryRecvError::Empty) => None,
            })
        {
            self.apply = None;
            match outcome {
                Ok(result) => {
                    if let Some(snapshot) = &mut self.snapshot {
                        snapshot.values = result.values;
                    }
                    self.field_issues = result.skipped;
                    let count = result.applied.len();
                    self.applied_stats.extend(result.applied);
                    let warning = self.field_issues.len();
                    let outcome = if MACOS {
                        "confirmed in game memory"
                    } else {
                        "applied"
                    };
                    self.status = if self.language.chinese {
                        if warning > 0 {
                            format!("已确认 {count} 项，跳过 {warning} 项。")
                        } else {
                            format!("已在游戏内存中确认 {count} 项。")
                        }
                    } else {
                        format!(
                            "Apply · {count} {outcome}{}",
                            if warning > 0 {
                                format!(" · {warning} skipped")
                            } else {
                                String::new()
                            }
                        )
                    };
                    event(
                        "apply.batch_finished",
                        format_args!("applied={count} skipped={warning}"),
                    );
                }
                Err(error) => {
                    event("apply.failed", format_args!("{error}"));
                    self.last_apply_error = Some(error.to_string());
                    self.status = if error.kind() == io::ErrorKind::PermissionDenied {
                        format!("Apply blocked. {error}")
                    } else {
                        format!("Apply stopped. {error}")
                    };
                }
            }
        }
        if let Some(result) = finished(&mut self.update_result) {
            let manual = std::mem::take(&mut self.manual_update_check);
            match result {
                Ok(status) => {
                    self.update_asset = status.available;
                    self.update_status = if status.newer {
                        "Update Available"
                    } else {
                        "Latest"
                    }
                    .into();
                    if manual && !status.newer {
                        self.using_latest_until = Some(Instant::now() + Duration::from_secs(3));
                    }
                }
                Err(error) => {
                    event("update.check_failed", format_args!("{error}"));
                    self.update_status = "Update check failed".into();
                    self.update_error = Some(error.to_string());
                }
            }
        }
        if let Some(result) = finished(&mut self.install_result) {
            match result {
                Ok(()) => {
                    self.reset_apply();
                    self.scan = None;
                    self.update_status = "Restarting…".into();
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(error) => {
                    event("update.download_failed", format_args!("{error}"));
                    self.update_asset = None;
                    self.update_status = "Update failed".into();
                    self.update_error = Some(error.to_string());
                }
            }
        }
        context.request_repaint_after(Duration::from_millis(250));
    }

    fn start_find(&mut self, context: &egui::Context, apply_targets: bool) {
        if let Some(session) = self.session.clone() {
            if apply_targets && !self.commit_target() {
                return;
            }
            self.reset_apply();
            self.snapshot = None;
            self.applied_stats.clear();
            self.error = None;
            self.status = "Finding character and stats…".into();
            event(
                "hunt.requested",
                format_args!("game_pid={} apply_targets={apply_targets}", session.pid),
            );
            let stop = Arc::new(AtomicBool::new(false));
            let worker_session = session.clone();
            let worker_stop = stop.clone();
            let waiting = Arc::new(AtomicBool::new(false));
            let worker_waiting = waiting.clone();
            let result = task(context, move || {
                find_with_fallback(&worker_session, &worker_stop, &worker_waiting)
            });
            self.scan = Some(Search {
                apply_targets,
                stop,
                waiting_for_panel: waiting,
                started: Instant::now(),
                session,
                result,
            });
        }
    }

    fn reset_apply(&mut self) {
        self.apply = None;
        self.field_issues.clear();
    }

    /// The configured game binary, or the official install location when none was chosen.
    fn game_path(&self) -> Option<PathBuf> {
        self.settings
            .game_path()
            .map(PathBuf::from)
            .or_else(process::default_game_path)
    }

    fn save_preferences(&mut self) {
        let enabled: Vec<_> = STATS
            .iter()
            .enumerate()
            .filter_map(|(index, stat)| self.enabled[index].then_some(stat.key))
            .collect();
        let targets: std::collections::BTreeMap<_, _> = STATS
            .iter()
            .enumerate()
            .map(|(index, stat)| (stat.key, self.targets[index]))
            .collect();
        if let Err(error) = self
            .settings
            .set_stat_preferences(serde_json::json!({"enabled": enabled, "targets": targets}))
        {
            self.error = Some(format!("Could not save stat preferences: {error}"));
            event("settings.save_failed", format_args!("{error}"));
        }
    }

    fn commit_target(&mut self) -> bool {
        let stat = &STATS[self.selected];
        if stat.key == "health" {
            return true;
        }
        let value = self
            .target_entry
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| stat.valid_target(*value));
        let Some(value) = value else {
            self.status = format!(
                "{} target must be between {} and {}.",
                stat.label, stat.min, stat.max
            );
            return false;
        };
        if value != self.targets[self.selected] {
            self.targets[self.selected] = value;
            self.save_preferences();
        }
        true
    }

    fn start_apply(&mut self, context: &egui::Context, targets: hunt::Targets) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let Some(proof) = snapshot.stats.clone() else {
            return;
        };
        if !targets
            .keys()
            .any(|key| snapshot.mapped_stats.contains(key))
        {
            let supported = STATS
                .iter()
                .filter(|stat| snapshot.mapped_stats.contains(stat.key))
                .map(|stat| stat.label)
                .collect::<Vec<_>>()
                .join(", ");
            self.status = format!("No requested stats are available. Try {supported}.");
            event(
                "apply.no_mapped_targets",
                format_args!("build={} available={supported}", snapshot.build),
            );
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        self.status = "Applying stats…".into();
        self.field_issues.clear();
        for key in targets.keys() {
            self.applied_stats.remove(key);
        }
        let result = task(context, move || {
            hunt::apply_stats(&proof, &targets, &worker_stop)
        });
        self.apply = Some(Apply { stop, result });
    }

    fn locate(&mut self) -> Option<PathBuf> {
        event("settings.locate_game", format_args!("dialog_opened"));
        let mut dialog = rfd::FileDialog::new();
        if cfg!(windows) {
            dialog = dialog.add_filter("Windows executable", &["exe"]);
        } else if let Some(directory) = process::default_game_path()
            .as_deref()
            .and_then(std::path::Path::parent)
        {
            dialog = dialog.set_directory(directory);
        }
        let Some(path) = dialog.pick_file() else {
            event("settings.locate_game_cancelled", format_args!(""));
            return None;
        };
        if !path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(process::GAME_EXE))
        {
            event(
                "settings.game_path_rejected",
                format_args!("unexpected_executable_name"),
            );
            self.error = Some(format!("Choose {}.", process::GAME_EXE));
            return None;
        }
        if let Err(error) = self.settings.set_game_path(&path) {
            event("settings.save_failed", format_args!("{error}"));
            self.error = Some(format!("Could not save game path: {error}"));
        } else {
            event("settings.saved", format_args!("changed=game_executable"));
        }
        Some(path)
    }

    fn footer(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        let language = self.language;
        ui.columns(2, |columns| {
            columns[0].horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.running
                            && self.launch_result.is_none()
                            && self.process_result.is_none(),
                        Button::new(if MACOS {
                            language.text("Run", "启动")
                        } else {
                            "Run Game"
                        }),
                    )
                    .clicked()
                    && let Some(path) = self.game_path().or_else(|| self.locate())
                {
                    event("process.launch_requested", format_args!(""));
                    self.launch_result = Some(task(context, move || process::launch(&path)));
                }
                if ui
                    .add_enabled(
                        self.session.is_some()
                            && self.stop_result.is_none()
                            && self.apply.is_none(),
                        Button::new(if MACOS {
                            language.text("Stop", "停止")
                        } else {
                            language.text("Stop Game", "停止游戏")
                        }),
                    )
                    .clicked()
                {
                    self.confirm_stop = self.session.clone();
                }
                if ui
                    .button(if MACOS {
                        language.text("Locate…", "定位游戏…")
                    } else {
                        LOCATE_LABEL
                    })
                    .clicked()
                {
                    self.locate();
                }
                if ui
                    .button(if MACOS {
                        language.text("Diagnostics", "诊断")
                    } else {
                        "Advanced"
                    })
                    .clicked()
                {
                    event("diagnostics.opened", format_args!(""));
                    self.show_details = true;
                    self.next_log_read = Instant::now();
                }
            });
            let ui = &mut columns[1];
            update_button_row(ui, |ui| {
                let mut status = RichText::new(format!(
                    "{VERSION} · {}",
                    language.status(&self.update_status)
                ))
                .size(16.0)
                .strong();
                if self.update_status == "Update Available" {
                    let seconds = context.input(|input| input.time);
                    let glow = ((seconds * std::f64::consts::TAU / 5.0).cos() + 1.0) * 0.5;
                    status = status
                        .color(Color32::from_rgb(
                            255,
                            200 + (45.0 * glow) as u8,
                            35 + (105.0 * glow) as u8,
                        ))
                        .background_color(Color32::from_rgba_unmultiplied(
                            255,
                            215,
                            65,
                            (28.0 * glow) as u8,
                        ));
                    context.request_repaint_after(Duration::from_millis(50));
                } else {
                    status = status.color(status_color(ui));
                }
                let response = ui.label(status);
                if let Some(error) = &self.update_error {
                    response.on_hover_text(error);
                }

                let latest = self
                    .using_latest_until
                    .is_some_and(|until| Instant::now() < until);
                let idle = self.update_result.is_none() && self.install_result.is_none() && !latest;
                let label = if self.install_result.is_some() {
                    language.text("Downloading…", "正在下载…")
                } else if self.update_result.is_some() {
                    language.text("Checking…", "正在检查…")
                } else if latest {
                    language.text("Using Latest", "已是最新版")
                } else if self.update_asset.is_some() {
                    language.text("Download & Install", "下载并安装")
                } else {
                    language.text("Check updates", "检查更新")
                };
                if ui.add_enabled(idle, Button::new(label)).clicked() {
                    self.update_error = None;
                    if let Some(asset) = self.update_asset.clone() {
                        self.update_status = "Downloading update…".into();
                        self.install_result =
                            Some(task(context, move || update::stage_and_launch(&asset)));
                    } else {
                        self.update_status = "Checking…".into();
                        self.manual_update_check = true;
                        self.update_result = Some(task(context, || update::check(VERSION)));
                    }
                }
                if ui
                    .button(if MACOS { "GitHub" } else { "Github Page" })
                    .clicked()
                {
                    context.open_url(egui::OpenUrl::new_tab(update::PROJECT));
                }
            });
            if self.update_result.is_some() || self.install_result.is_some() {
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                    ui.allocate_ui(egui::vec2(180.0, 8.0), Self::work_progress);
                });
            }
        });
    }

    fn stop_confirmation(&mut self, context: &egui::Context) {
        let language = self.language;
        let Some(session) = self.confirm_stop.clone() else {
            return;
        };
        let mut cancel = false;
        let mut stop = false;
        egui::Modal::new(egui::Id::new("stop-reborn")).show(context, |ui| {
            ui.heading(language.text("Stop Reborn?", "停止游戏？"));
            ui.label(format!("PID {}", session.pid));
            ui.label(session.path.display().to_string());
            ui.label(language.text(
                "Stopping Reborn can lose unsaved run progress.",
                "停止游戏可能丢失本局未保存的进度。",
            ));
            ui.horizontal(|ui| {
                cancel = ui.button(language.text("Cancel", "取消")).clicked();
                stop = ui.button(language.text("Stop Game", "停止游戏")).clicked();
            });
        });
        if cancel || context.input(|input| input.key_pressed(egui::Key::Escape)) {
            event(
                "process.stop_cancelled",
                format_args!("game_pid={}", session.pid),
            );
            self.confirm_stop = None;
        } else if stop {
            event(
                "process.stop_requested",
                format_args!("game_pid={}", session.pid),
            );
            self.confirm_stop = None;
            self.scan = None;
            self.reset_apply();
            self.snapshot = None;
            self.applied_stats.clear();
            self.stop_result = Some(task(context, move || process::stop(&session)));
        }
    }

    fn stat_display(&self, stat: &Stat) -> Option<String> {
        let snapshot = self.snapshot.as_ref()?;
        if let Some(value) = snapshot.values.get(stat.key) {
            return Some(stat.display(*value));
        }
        let character = snapshot.character.as_ref()?;
        let index = match stat.key {
            "health" => 0,
            "max_health" => 1,
            _ => return None,
        };
        Some(format!(
            "≈ {}",
            stat.display(character.health_candidate[index] as u64)
        ))
    }

    fn stats_table(&mut self, context: &egui::Context, ui: &mut egui::Ui, width: f32) {
        let language = self.language;
        let titles = if language.chinese {
            ["启用", "属性", "当前值", "目标值", "状态"]
        } else {
            ["Enabled", "Stat", "Current", "Target", "Status"]
        };
        let spacing = ui.spacing().item_spacing.x * (titles.len() - 1) as f32;
        let widths = [
            76.0,
            (width - spacing - 383.0).max(100.0),
            88.0,
            102.0,
            117.0,
        ];
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut selected = self.selected;
        let mut preferences_changed = false;
        let mut auto_apply = hunt::Targets::new();
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .sense(egui::Sense::click())
            .auto_shrink([false, false]);
        for width in widths {
            table = table.column(Column::exact(width));
        }
        table
            .header(31.0, |mut header| {
                for title in titles {
                    header.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
                    });
                }
            })
            .body(|mut body| {
                for (index, stat) in STATS.iter().enumerate() {
                    let target = if language.chinese && stat.key == "health" {
                        "回满".into()
                    } else {
                        stat.target(self.targets[index])
                    };
                    let mut checked = self.enabled[index];
                    let current = self.stat_display(stat);
                    let verified = self
                        .snapshot
                        .as_ref()
                        .is_some_and(|s| s.values.contains_key(stat.key));
                    let (status, status_color) = if self.applied_stats.contains(stat.key) {
                        (if MACOS { language.text("Confirmed", "已确认") } else { "Applied" }, status_color(ui))
                    } else if verified || current.is_some() {
                        (language.text("Found", "已读取"), ui.visuals().text_color())
                    } else {
                        ("-", muted_color(ui))
                    };
                    body.row(29.0, |mut row| {
                        row.set_selected(index == self.selected);
                        row.col(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 22.0),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    let fill = if MACOS && checked {
                                        Color32::from_rgb(44, 99, 176)
                                    } else if MACOS {
                                        ui.visuals().widgets.inactive.bg_fill
                                    } else if checked {
                                        Color32::from_rgb(31, 107, 87)
                                    } else {
                                        Color32::from_rgb(44, 58, 72)
                                    };
                                    let visuals = ui.visuals_mut();
                                    visuals.widgets.inactive.bg_fill = fill;
                                    visuals.widgets.hovered.bg_fill = fill.gamma_multiply(1.25);
                                    visuals.widgets.active.bg_fill = fill.gamma_multiply(1.4);
                                    let response = ui.add_enabled(
                                        self.apply.is_none(),
                                        egui::Checkbox::without_text(&mut checked),
                                    );
                                    let response = if MACOS {
                                        response.on_hover_text(language.text("Enabling a mapped stat writes its target immediately after Find.", "查找完成后，勾选已验证的属性会立即写入目标值。"))
                                    } else {
                                        response
                                    };
                                    if response.changed() {
                                        self.enabled[index] = checked;
                                        preferences_changed = true;
                                        if checked
                                            && let Some(snapshot) = &self.snapshot
                                            && snapshot.stats.is_some()
                                            && snapshot.mapped_stats.contains(stat.key)
                                        {
                                            auto_apply.insert(stat.key, self.targets[index]);
                                        }
                                    }
                                },
                            );
                        });
                        row.col(|ui| {
                            ui.label(language.text(stat.label, stat.chinese_label));
                        });
                        row.col(|ui| {
                            ui.label(current.as_deref().unwrap_or("—"));
                        });
                        row.col(|ui| {
                            ui.label(target);
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(status).color(status_color));
                        });
                        if row.response().clicked() {
                            if selected != index {
                                event("stats.selected", format_args!("stat={}", stat.key));
                            }
                            selected = index;
                        }
                    });
                }
            });
        if self.selected != selected && self.commit_target() {
            self.selected = selected;
            self.target_entry = self.targets[selected].to_string();
        }
        if preferences_changed {
            self.save_preferences();
        }
        if !auto_apply.is_empty() {
            self.start_apply(context, auto_apply);
        }
    }

    fn work_progress(ui: &mut egui::Ui) {
        let time = ui.input(|input| input.time) as f32;
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 4, ui.visuals().widgets.inactive.bg_fill);
        let width = rect.width() * 0.32;
        let phase = (time * 0.45).fract() * 2.0;
        let progress = if phase < 1.0 { phase } else { 2.0 - phase };
        let left = rect.left() + progress * (rect.width() - width);
        let fill = egui::Rect::from_min_size(
            egui::pos2(left, rect.top()),
            egui::vec2(width, rect.height()),
        );
        ui.painter().rect_filled(fill, 4, ACCENT);
        ui.ctx().request_repaint_after(Duration::from_millis(35));
    }

    fn stat_editor(&mut self, context: &egui::Context, ui: &mut egui::Ui) {
        let language = self.language;
        let width = ui.available_width();
        let finding = self.scan.is_some();
        let applying = self.apply.is_some();
        let title = if finding {
            language.text("Stop search", "停止查找")
        } else if applying {
            language.text("Cancel Apply", "取消应用")
        } else {
            language.text("Find", "查找")
        };
        let find = Button::new(
            RichText::new(title)
                .size(if MACOS { 15.0 } else { 24.0 })
                .strong()
                .color(ACCENT_TEXT),
        )
        .fill(ACCENT)
        .min_size(egui::vec2(width, if MACOS { 34.0 } else { 53.0 }));
        if ui
            .add_enabled(applying || finding || self.session.is_some(), find)
            .clicked()
        {
            if finding {
                event("hunt.cancelled", format_args!("reason=user"));
                self.scan = None;
                self.status = "Search stopped.".into();
            } else if applying {
                event("apply.cancelled", format_args!("reason=user"));
                self.reset_apply();
                self.status = "Apply cancelled.".into();
            } else {
                self.start_find(context, true);
            }
        }
        if MACOS
            && ui
                .add_enabled(
                    !applying && !finding && self.session.is_some(),
                    Button::new(language.text("Read current values", "读取当前值"))
                        .min_size(egui::vec2(width, 29.0)),
                )
                .clicked()
        {
            self.start_find(context, false);
        }
        let verified_layout = self.snapshot.as_ref().is_some_and(|s| s.stats.is_some());
        if self.scan.is_some() || self.apply.is_some() {
            Self::work_progress(ui);
        }
        if let Some(snapshot) = &self.snapshot
            && snapshot.stats.is_some()
            && !snapshot.values.is_empty()
        {
            ui.colored_label(
                status_color(ui),
                format!(
                    "{} {}",
                    language.text("Stats found:", "已读取属性："),
                    snapshot.values.len()
                ),
            );
        }
        ui.colored_label(status_color(ui), language.status(&self.status));
        if let Some(scan) = &self.scan {
            let elapsed = scan.started.elapsed().as_secs();
            ui.label(
                RichText::new(format!(
                    "{}:{:02} {}",
                    elapsed / 60,
                    elapsed % 60,
                    language.text("elapsed", "已用时")
                ))
                .color(muted_color(ui)),
            );
        }
        ui.separator();
        let stat = &STATS[self.selected];
        let current = self.stat_display(stat);
        ui.heading(
            RichText::new(language.text(stat.label, stat.chinese_label))
                .size(19.0)
                .strong(),
        );
        ui.label(
            RichText::new(current.as_deref().unwrap_or("—"))
                .size(26.0)
                .color(status_color(ui)),
        );
        ui.label(if stat.percent {
            language.text("Target (%)", "目标值（%）")
        } else {
            language.text("Target", "目标值")
        });
        let editable = self.apply.is_none() && stat.key != "health";
        let reply = ui.add_enabled(
            editable,
            egui::TextEdit::singleline(&mut self.target_entry).desired_width(width - 10.0),
        );
        if reply.lost_focus() {
            self.commit_target();
        }
        let selected_mapped = self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.mapped_stats.contains(stat.key));
        let can_apply = selected_mapped
            && self.apply.is_none()
            && self.stop_result.is_none()
            && self.session.is_some()
            && self.scan.is_none();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    can_apply,
                    Button::new(RichText::new(language.text("Apply", "应用")).color(ACCENT_TEXT))
                        .fill(ACCENT),
                )
                .clicked()
                && self.commit_target()
            {
                self.start_apply(context, [(stat.key, self.targets[self.selected])].into());
            }
            if ui
                .add_enabled(
                    editable,
                    Button::new(language.text("Use maximum", "使用最大值")),
                )
                .clicked()
            {
                self.target_entry = stat.max.to_string();
                self.commit_target();
            }
        });
        let hint = if let Some(error) = self.field_issues.get(stat.key) {
            error.as_str()
        } else if verified_layout && !selected_mapped {
            "This stat is unavailable on the detected build."
        } else if stat.key == "health" {
            "Current health follows verified maximum health."
        } else {
            "Find applies checked targets once."
        };
        ui.label(RichText::new(language.status(hint)).color(muted_color(ui)));
    }

    fn diagnostic_text(&self) -> String {
        use std::fmt::Write as _;

        let mut report = format!(
            "Trainer {VERSION} / {}\n\nSESSION\n{}\n",
            process::PLATFORM,
            self.status
        );
        if let Some(session) = &self.session {
            let _ = writeln!(
                report,
                "Game PID: {}\nGame executable: {}",
                session.pid,
                session.path.display()
            );
        } else {
            let _ = writeln!(
                report,
                "Game process: {}",
                if self.running {
                    "Selection required"
                } else {
                    "Disconnected"
                }
            );
        }
        if let Some(snapshot) = &self.snapshot {
            let _ = writeln!(
                report,
                "Build: {}\nStat layout check: {:?}",
                snapshot.build, snapshot.stat_layout
            );
            if let Some(character) = &snapshot.character {
                let _ = writeln!(
                    report,
                    "Character: {} (slot {} of {})",
                    character.name,
                    character.index + 1,
                    character.count
                );
            }
        }
        let _ = writeln!(report, "Apply in progress: {}", self.apply.is_some());
        if let Some(scan) = &self.scan {
            let _ = writeln!(
                report,
                "Find active: {}s elapsed",
                scan.started.elapsed().as_secs()
            );
        }
        if let Some(error) = &self.last_apply_error {
            let _ = writeln!(report, "Last Apply error: {error}");
        }
        if let Some(error) = &self.error {
            let _ = writeln!(report, "Application error: {error}");
        }
        let _ = writeln!(report, "\nSTATS");
        for (index, stat) in STATS.iter().enumerate() {
            let current = self.stat_display(stat);
            let enabled = if self.enabled[index] { "on" } else { "off" };
            let _ = write!(
                report,
                "{} | current {} | target {} | enabled {enabled}",
                stat.label,
                current.as_deref().unwrap_or("Unavailable"),
                stat.target(self.targets[index])
            );
            if let Some(issue) = self.field_issues.get(stat.key) {
                let _ = write!(report, " | {issue}");
            }
            report.push('\n');
        }
        let _ = writeln!(report, "\nUPDATER\n{}", self.update_status);
        if let Some(error) = &self.update_error {
            let _ = writeln!(report, "Updater error: {error}");
        }
        if let Ok(directory) = crate::settings::state_directory() {
            let _ = writeln!(
                report,
                "\nBACKGROUND LOG\n{}",
                directory.join("reborn-rust.log").display()
            );
        }
        if let Some(stats) = &self.screen_stats {
            let _ = writeln!(report, "\nSCREEN STATS (OCR)\n{stats}");
        }
        let _ = writeln!(report, "\nRECENT LOG (up to 40 lines)\n{}", self.log_tail);
        report
    }
}

impl eframe::App for TrainerApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            if !self.native_chrome_installed {
                self.native_chrome_installed = crate::macos_chrome::install(_frame);
            }
            let (chinese, theme) = crate::macos_chrome::preferences();
            self.language.chinese = chinese;
            if chinese && !self.chinese_font_loaded {
                self.chinese_font_loaded = crate::macos_chrome::install_chinese_font(context);
            }
            context.set_theme(theme);
        }
        let language = self.language;
        self.poll(context);
        let frame = egui::Frame::new()
            .fill(context.style().visuals.panel_fill)
            .inner_margin(egui::Margin::same(16));
        egui::TopBottomPanel::bottom("trainer-footer")
            .frame(frame)
            .show(context, |ui| self.footer(context, ui));
        egui::CentralPanel::default()
            .frame(frame)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    if let Some(logo) = &self.logo {
                        ui.image((logo.id(), egui::vec2(32.0, 32.0)));
                    }
                    ui.heading(
                        RichText::new(if MACOS { language.text("Reborn Trainer", "Reborn 修改器") } else { "AZEROTH SURVIVORS REBORN" })
                            .size(if MACOS { 22.0 } else { 26.0 })
                            .color(if MACOS { ui.visuals().text_color() } else { GOLD })
                            .strong(),
                    );
                });
                ui.vertical(|ui| {
                    right_aligned_row(ui, ui.spacing().interact_size.y, |ui| {
                        if let Some(session) = &self.session {
                            if MACOS {
                                ui.colored_label(status_color(ui), language.text("Game connected", "已连接游戏"))
                                    .on_hover_text(format!("PID {}", session.pid));
                            } else {
                                ui.colored_label(status_color(ui), format!("Connected · PID {}", session.pid));
                            }
                        } else if self.running {
                            ui.label(
                                language.text("Reborn is running. The process could not be uniquely selected.", "游戏正在运行，暂时无法确定要连接的进程。"),
                            );
                        } else {
                            ui.label(language.text("Waiting for Reborn.", "等待游戏启动。"));
                        }
                    });
                    if self.launch_result.is_some() || self.stop_result.is_some() {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                            ui.allocate_ui(egui::vec2(220.0, 8.0), Self::work_progress);
                        });
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                if self.scan.as_ref().is_some_and(|scan| {
                    scan.waiting_for_panel.load(Ordering::Acquire)
                }) {
                    // A 1.5 second pulse between gold and bright yellow makes the prompt hard to miss.
                    let pulse = ((context.input(|input| input.time) * std::f64::consts::TAU / 1.5)
                        .sin() as f32
                        + 1.0)
                        / 2.0;
                    let glow = Color32::from_rgb(
                        (227.0 + 28.0 * pulse) as u8,
                        (190.0 + 60.0 * pulse) as u8,
                        (105.0 + 120.0 * pulse) as u8,
                    );
                    context.request_repaint_after(Duration::from_millis(50));
                    ui.label(
                        RichText::new(
                            "Open your Character panel (press C) and leave it open until the trainer finds the values.",
                        )
                        .size(22.0)
                        .color(glow)
                        .strong(),
                    );
                }
                ui.add_space(8.0);
                egui::SidePanel::right("selected-stat-editor")
                    .exact_width(EDITOR_WIDTH)
                    .resizable(false)
                    .frame(
                        egui::Frame::new()
                            .fill(ui.visuals().panel_fill)
                            .inner_margin(egui::Margin::symmetric(11, 0)),
                    )
                    .show_inside(ui, |ui| {
                        if MACOS {
                            egui::Frame::new()
                                .fill(ui.visuals().widgets.inactive.bg_fill)
                                .stroke(ui.visuals().widgets.inactive.bg_stroke)
                                .corner_radius(14)
                                .inner_margin(egui::Margin::same(14))
                                .show(ui, |ui| self.stat_editor(context, ui));
                        } else {
                            self.stat_editor(context, ui);
                        }
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show_inside(ui, |ui| {
                        let width = ui.available_width();
                        self.stats_table(context, ui, width);
                    });
            });
        self.stop_confirmation(context);
        if let Some(ready) = self.update_ready.take()
            && let Err(error) = update::confirm_ready(&ready)
        {
            event(
                "update.startup_confirmation_failed",
                format_args!("{error}"),
            );
        }
        if self.show_details {
            if Instant::now() >= self.next_log_read {
                self.next_log_read = Instant::now() + Duration::from_secs(1);
                self.log_tail = crate::settings::state_directory()
                    .and_then(|directory| {
                        crate::diagnostic_log::recent_lines(&directory.join("reborn-rust.log"))
                    })
                    .unwrap_or_else(|error| format!("Log unavailable: {error}"));
            }
            let text = self.diagnostic_text();
            let mut capture = false;
            egui::Window::new(language.text("Detection details", "检测详情"))
                .open(&mut self.show_details)
                .default_width(680.0)
                .show(context, |ui| {
                    if MACOS {
                        ui.label(language.text(
                            "Read current values to check the live game without applying targets.",
                            "使用「读取当前值」可以查看游戏实时数值，避免应用目标值。",
                        ));
                    } else {
                        ui.label("Find locates the Stats panel with OCR, then reads and sets values in memory. Open the Character panel with C.");
                    }
                    ui.horizontal(|ui| {
                        if ui.button(language.text("Copy details", "复制详情")).clicked() {
                            event("diagnostics.copied", format_args!(""));
                            context.copy_text(text.clone());
                        }
                        if !crate::screen_stats::SUPPORTED {
                            return;
                        }
                        let busy = self.screen_stats_result.is_some();
                        let label = if busy {
                            language.text("Reading Stats panel…", "正在读取属性面板…")
                        } else {
                            language.text("Read Stats panel", "读取属性面板")
                        };
                        capture = ui
                            .add_enabled(self.session.is_some() && !busy, Button::new(label))
                            .on_hover_text("Open the Character panel in Reborn first.")
                            .clicked();
                    });
                    egui::ScrollArea::vertical()
                        .max_height(520.0)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(text).monospace()).wrap());
                        });
                });
            if capture && let Some(session) = &self.session {
                let pid = session.pid;
                self.screen_stats_result =
                    Some(task(context, move || crate::screen_stats::read(pid)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_apply_uses_only_checked_custom_targets() {
        let mut enabled = [false; 17];
        enabled[0] = true;
        enabled[2] = true;
        let mut targets = std::array::from_fn(|index| STATS[index].max);
        targets[0] = 222;
        targets[2] = 127;
        assert_eq!(
            checked_targets(&enabled, &targets),
            [("damage", 222), ("crit_damage", 127)].into()
        );
    }
    #[test]
    fn right_aligned_status_rows_leave_room_for_the_main_content() {
        let context = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(980.0, 810.0),
            )),
            ..Default::default()
        };
        let _ = context.run(input, |ctx| {
            let footer = egui::TopBottomPanel::bottom("layout-test-footer").show(ctx, |ui| {
                ui.columns(2, |columns| {
                    columns[0].label("Run Game");
                    update_button_row(&mut columns[1], |ui| {
                        let version = ui.label("Latest");
                        let update = ui.button("Check updates");
                        assert!(version.rect.left() >= update.rect.right());
                        assert!((version.rect.center().y - update.rect.center().y).abs() < 1.0);
                    });
                });
            });
            egui::CentralPanel::default().show(ctx, |ui| {
                right_aligned_row(ui, 27.0, |ui| {
                    ui.label("Waiting for Reborn.");
                });
                assert!(ui.available_height() > 600.0);
                assert!(footer.response.rect.height() < 100.0);
            });
        });
    }

    #[test]
    fn footer_update_button_receives_pointer_clicks() {
        use egui::{Event, PointerButton, Pos2, RawInput, Rect, vec2};
        let context = egui::Context::default();
        let mut position = Pos2::ZERO;
        let mut clicked = false;
        let mut frame = |events| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(980.0, 810.0))),
                events,
                ..Default::default()
            };
            let _ = context.run(input, |ctx| {
                egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
                    ui.columns(2, |columns| {
                        columns[0].label("Game");
                        update_button_row(&mut columns[1], |ui| {
                            ui.label(format!("{VERSION} - Latest"));
                            let response = ui.button("Check updates");
                            position = response.rect.center();
                            clicked |= response.clicked();
                        });
                    });
                });
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Stats");
                });
            });
            position
        };
        frame(vec![]);
        frame(vec![]);
        let center = frame(vec![]);
        frame(vec![Event::PointerMoved(center)]);
        for pressed in [true, false] {
            frame(vec![Event::PointerButton {
                pos: center,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        assert!(clicked);
    }
}
