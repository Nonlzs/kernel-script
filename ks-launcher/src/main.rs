#![windows_subsystem = "windows"]

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui;
use egui::FontFamily;

const DEFAULT_DRIVER_FILE: &str = "ks-driver.sys";
const DEFAULT_SERVICE_FILE: &str = "ks-service.exe";
const DEFAULT_GUI_FILE: &str = "ks-gui.exe";
const DEFAULT_DRIVER_SERVICE: &str = "KsDriver";
const DEFAULT_BACKEND_SERVICE: &str = "KsService";
const STATE_FILE: &str = "ks-launcher.state";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Random per-run identities. Each component's file is renamed to a fresh
/// random name when its Start button is clicked, and renamed back to the
/// canonical name once that component stops. Every service creation also
/// registers a fresh random SCM name. Everything is recorded in a small
/// state file so stop/cleanup keeps working across launcher restarts.
#[derive(Clone)]
struct LaunchNames {
    driver_file: String,
    service_file: String,
    gui_file: String,
    driver: String,
    service: String,
}

impl LaunchNames {
    fn load(base_dir: &Path) -> Self {
        let mut names = Self {
            driver_file: DEFAULT_DRIVER_FILE.to_owned(),
            service_file: DEFAULT_SERVICE_FILE.to_owned(),
            gui_file: DEFAULT_GUI_FILE.to_owned(),
            driver: DEFAULT_DRIVER_SERVICE.to_owned(),
            service: DEFAULT_BACKEND_SERVICE.to_owned(),
        };
        let Ok(text) = std::fs::read_to_string(base_dir.join(STATE_FILE)) else {
            return names;
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if !is_valid_name(value) {
                continue;
            }
            match key {
                "driver_file" => names.driver_file = value.to_owned(),
                "service_file" => names.service_file = value.to_owned(),
                "gui_file" => names.gui_file = value.to_owned(),
                "driver" => names.driver = value.to_owned(),
                "service" => names.service = value.to_owned(),
                _ => {}
            }
        }
        names
    }

    fn save(&self, base_dir: &Path) {
        let text = format!(
            "driver_file={}\nservice_file={}\ngui_file={}\ndriver={}\nservice={}\n",
            self.driver_file, self.service_file, self.gui_file, self.driver, self.service
        );
        if let Err(error) = std::fs::write(base_dir.join(STATE_FILE), text) {
            eprintln!("failed to write {STATE_FILE}: {error}");
        }
    }
}

/// Eight lowercase alphanumeric characters; a neutral name with no
/// project-identifying prefix, valid as both a file stem and an SCM name.
fn random_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let mut state =
        nanos ^ ((std::process::id() as u64) << 32) ^ nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    if state == 0 {
        state = 0x853C_49E6_748F_EA9B;
    }
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut token = String::with_capacity(8);
    for _ in 0..8 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        token.push(ALPHABET[(state >> 33) as usize % ALPHABET.len()] as char);
    }
    token
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// Renames one component file to a fresh random name. The recorded state
/// name wins when it still exists on disk (already renamed), otherwise the
/// canonical default name is used. A rename failure (file locked by a loaded
/// driver or a running process) keeps the current name.
fn rename_component(
    base_dir: &Path,
    recorded: &mut String,
    default_name: &str,
    extension: &str,
    running: bool,
    log: &Arc<Mutex<File>>,
) {
    let current = if base_dir.join(recorded.as_str()).is_file() {
        recorded.clone()
    } else if base_dir.join(default_name).is_file() {
        default_name.to_owned()
    } else {
        write_log(log, format!("note: {default_name} not found"));
        return;
    };
    if running {
        write_log(log, format!("{current} is running; keeping its name"));
        *recorded = current;
        return;
    }
    for _ in 0..8 {
        let candidate = format!("{}.{}", random_token(), extension);
        if base_dir.join(&candidate).exists() {
            continue;
        }
        match std::fs::rename(base_dir.join(&current), base_dir.join(&candidate)) {
            Ok(()) => {
                write_log(log, format!("renamed {current} -> {candidate}"));
                *recorded = candidate;
            }
            Err(error) => {
                write_log(
                    log,
                    format!("[warning] rename {current} failed: {error}; keeping its name"),
                );
                *recorded = current;
            }
        }
        return;
    }
    *recorded = current;
}

/// Restores the canonical file names after their component has stopped.
fn restore_component(
    base_dir: &Path,
    recorded: &mut String,
    default_name: &str,
    log: &Arc<Mutex<File>>,
) {
    if *recorded == default_name {
        return;
    }
    let current = base_dir.join(recorded.as_str());
    if !current.is_file() {
        // The random-named file is gone; drop the stale name from the state.
        write_log(
            log,
            format!("note: {recorded} missing; recording {default_name}"),
        );
        *recorded = default_name.to_owned();
        return;
    }
    let target = base_dir.join(default_name);
    if target.exists() {
        write_log(
            log,
            format!("[warning] cannot restore {default_name}: already exists"),
        );
        return;
    }
    match std::fs::rename(&current, &target) {
        Ok(()) => {
            write_log(log, format!("restored {} -> {default_name}", *recorded));
            *recorded = default_name.to_owned();
        }
        Err(error) => {
            write_log(
                log,
                format!("[warning] restore {default_name} failed: {error}"),
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LaunchState {
    Unknown,
    Stopped,
    Starting,
    Running,
    Failed,
}

struct LaunchEvent {
    target: Target,
    state: LaunchState,
    message: String,
}

#[derive(Clone, Copy, Debug)]
enum Target {
    Driver,
    Service,
    Gui,
}

struct LauncherApp {
    base_dir: PathBuf,
    log: Arc<Mutex<File>>,
    events: Receiver<LaunchEvent>,
    event_tx: Sender<LaunchEvent>,
    names: LaunchNames,
    driver: LaunchState,
    service: LaunchState,
    gui: LaunchState,
    busy: bool,
    last_restore_attempt: Option<std::time::Instant>,
}

impl LauncherApp {
    fn new() -> Self {
        let base_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        let log_path = base_dir.join("ks-launcher.log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap_or_else(|error| panic!("cannot open {}: {error}", log_path.display()));
        let (event_tx, events) = channel();
        let names = LaunchNames::load(&base_dir);
        let mut app = Self {
            base_dir,
            log: Arc::new(Mutex::new(log)),
            events,
            event_tx,
            names,
            driver: LaunchState::Unknown,
            service: LaunchState::Unknown,
            gui: LaunchState::Unknown,
            busy: false,
            last_restore_attempt: None,
        };
        app.log("launcher started");
        app.log(format!("launcher directory: {}", app.base_dir.display()));
        app.refresh_states();
        app
    }

    fn log(&self, message: impl AsRef<str>) {
        if let Ok(mut file) = self.log.lock() {
            let _ = writeln!(file, "{}", message.as_ref());
            let _ = file.flush();
        }
    }

    fn refresh_states(&mut self) {
        self.driver = service_state(&self.names.driver);
        self.service = service_state(&self.names.service);
        self.gui = process_state(&self.names.gui_file);
    }

    fn state_mut(&mut self, target: Target) -> &mut LaunchState {
        match target {
            Target::Driver => &mut self.driver,
            Target::Service => &mut self.service,
            Target::Gui => &mut self.gui,
        }
    }

    fn start(&mut self, ctx: &egui::Context, target: Target) {
        if !self.can_start(target) {
            return;
        }
        self.busy = true;
        *self.state_mut(target) = LaunchState::Starting;
        let base_dir = self.base_dir.clone();
        let log = Arc::clone(&self.log);
        let events = self.event_tx.clone();
        let names = self.names.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let mut names = names;
            let result = match target {
                Target::Driver => start_driver(&base_dir, &log, &mut names),
                Target::Service => start_service(&base_dir, &log, &mut names),
                Target::Gui => start_gui(&base_dir, &log, &mut names),
            };
            let (state, message) = match result {
                Ok(message) => (LaunchState::Running, message),
                Err(error) => (LaunchState::Failed, format!("[error] {error}")),
            };
            let _ = events.send(LaunchEvent {
                target,
                state,
                message,
            });
            // The start helpers persisted fresh random SCM names; query the
            // state with the reloaded names, not the stale pre-start clone.
            let fresh = LaunchNames::load(&base_dir);
            let refreshed = match target {
                Target::Driver => service_state(&fresh.driver),
                Target::Service => service_state(&fresh.service),
                Target::Gui => process_state(&fresh.gui_file),
            };
            events
                .send(LaunchEvent {
                    target,
                    state: refreshed,
                    message: "state refreshed".to_owned(),
                })
                .ok();
            repaint.request_repaint();
        });
    }

    fn stop(&mut self, ctx: &egui::Context, target: Target) {
        if self.busy {
            return;
        }
        self.busy = true;
        *self.state_mut(target) = LaunchState::Starting;
        let log = Arc::clone(&self.log);
        let events = self.event_tx.clone();
        let names = self.names.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = match target {
                Target::Driver => stop_service(&names.driver, &log),
                Target::Service => stop_service(&names.service, &log),
                Target::Gui => stop_gui(&names.gui_file, &log),
            };
            let (state, message) = match result {
                Ok(message) => (LaunchState::Stopped, message),
                Err(error) => (LaunchState::Failed, format!("[error] {error}")),
            };
            let _ = events.send(LaunchEvent {
                target,
                state,
                message,
            });
            repaint.request_repaint();
        });
    }

    fn can_start(&self, target: Target) -> bool {
        if self.busy {
            return false;
        }
        match target {
            Target::Driver => self.driver != LaunchState::Running,
            Target::Service => {
                self.driver == LaunchState::Running && self.service != LaunchState::Running
            }
            Target::Gui => {
                self.driver == LaunchState::Running
                    && self.service == LaunchState::Running
                    && self.gui != LaunchState::Running
            }
        }
    }

    fn can_stop(&self, target: Target) -> bool {
        if self.busy || self.state(target) != LaunchState::Running {
            return false;
        }
        match target {
            // The GUI must be closed before stopping the backend it uses.
            Target::Gui => true,
            // The backend must be stopped before unloading the driver device.
            Target::Service => self.gui != LaunchState::Running,
            Target::Driver => {
                self.gui != LaunchState::Running && self.service != LaunchState::Running
            }
        }
    }

    fn state(&self, target: Target) -> LaunchState {
        match target {
            Target::Driver => self.driver,
            Target::Service => self.service,
            Target::Gui => self.gui,
        }
    }

    fn update_events(&mut self) {
        let mut processed = false;
        while let Ok(event) = self.events.try_recv() {
            self.log(format!("{:?}: {}", event.target, event.message));
            *self.state_mut(event.target) = event.state;
            if event.state != LaunchState::Starting {
                self.busy = false;
            }
            processed = true;
        }
        // start_driver/start_service persist fresh random SCM names and file
        // names; reload them so later state queries and stop/cleanup address
        // the new registrations instead of the names this session started
        // with.
        if processed {
            self.names = LaunchNames::load(&self.base_dir);
        }
    }

    /// Renames the random component file back to the canonical name once its
    /// component has stopped. Retried at most once per second until it
    /// succeeds (a driver image can stay locked for a short moment after the
    /// service reports STOPPED).
    fn maybe_restore_file_names(&mut self) {
        if self.busy {
            return;
        }
        let now = std::time::Instant::now();
        if let Some(last) = self.last_restore_attempt {
            if now.duration_since(last) < Duration::from_secs(1) {
                return;
            }
        }
        let mut touched = false;
        let attempts = [
            (
                self.driver == LaunchState::Stopped,
                &mut self.names.driver_file,
                DEFAULT_DRIVER_FILE,
            ),
            (
                self.service == LaunchState::Stopped,
                &mut self.names.service_file,
                DEFAULT_SERVICE_FILE,
            ),
            (
                self.gui == LaunchState::Stopped,
                &mut self.names.gui_file,
                DEFAULT_GUI_FILE,
            ),
        ];
        for (stopped, recorded, default_name) in attempts {
            if stopped && recorded != default_name {
                restore_component(&self.base_dir, recorded, default_name, &self.log);
                touched = true;
            }
        }
        if touched {
            self.last_restore_attempt = Some(now);
            self.names.save(&self.base_dir);
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_events();
        self.maybe_restore_file_names();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                if launch_button(
                    ui,
                    "Driver",
                    self.driver,
                    self.can_start(Target::Driver),
                    self.can_stop(Target::Driver),
                )
                .clicked()
                {
                    if self.driver == LaunchState::Running {
                        self.stop(ctx, Target::Driver);
                    } else {
                        self.start(ctx, Target::Driver);
                    }
                }
                if launch_button(
                    ui,
                    "Service",
                    self.service,
                    self.can_start(Target::Service),
                    self.can_stop(Target::Service),
                )
                .clicked()
                {
                    if self.service == LaunchState::Running {
                        self.stop(ctx, Target::Service);
                    } else {
                        self.start(ctx, Target::Service);
                    }
                }
                if launch_button(
                    ui,
                    "GUI",
                    self.gui,
                    self.can_start(Target::Gui),
                    self.can_stop(Target::Gui),
                )
                .clicked()
                {
                    if self.gui == LaunchState::Running {
                        self.stop(ctx, Target::Gui);
                    } else {
                        self.start(ctx, Target::Gui);
                    }
                }
            });
        });
        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

fn launch_button(
    ui: &mut egui::Ui,
    target: &str,
    state: LaunchState,
    start_enabled: bool,
    stop_enabled: bool,
) -> egui::Response {
    let running = state == LaunchState::Running;
    let label = if running {
        format!("Stop {target}")
    } else {
        format!("Start {target}")
    };
    let button =
        egui::Button::new(egui::RichText::new(&label).size(16.0)).min_size(egui::vec2(220.0, 48.0));
    let response = ui.add_enabled(if running { stop_enabled } else { start_enabled }, button);
    if running {
        let rect = response.rect;
        ui.painter()
            .rect_filled(rect, 4.0, egui::Color32::from_rgb(190, 55, 55));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("Stop {target}"),
            egui::FontId::proportional(16.0),
            egui::Color32::WHITE,
        );
    }
    response
}

fn start_driver(
    base_dir: &Path,
    log: &Arc<Mutex<File>>,
    names: &mut LaunchNames,
) -> Result<String, String> {
    // Rename the file to a fresh random name for this run before the service
    // registration references it.
    rename_component(
        base_dir,
        &mut names.driver_file,
        DEFAULT_DRIVER_FILE,
        "sys",
        false,
        log,
    );
    names.save(base_dir);
    let path = base_dir.join(&names.driver_file);
    if !path.is_file() {
        return Err(format!("driver file not found: {}", path.display()));
    }
    // Every creation uses a fresh random SCM name; the previous registration
    // (recorded in the state file) is deleted first, and the new name is
    // persisted before `sc create` so a crash cannot orphan it.
    let service_name = random_token();
    let mut updated = names.clone();
    updated.driver = service_name.clone();
    updated.save(base_dir);
    // Command arguments already preserve the path as one value. Do not embed
    // quote characters in the value: SCM would store those quotes in ImagePath
    // instead of normalizing it to the native \??\ path form.
    let image_path = path.to_string_lossy().into_owned();
    delete_service_if_present(&names.driver, log);
    run_sc(
        log,
        &[
            "create",
            &service_name,
            "type=",
            "kernel",
            "start=",
            "demand",
            "binPath=",
            &image_path,
        ],
    )?;
    run_sc(log, &["start", &service_name])?;
    if !wait_for_running(&service_name) {
        return Err("driver did not reach RUNNING state".to_owned());
    }
    Ok(format!("driver started as {service_name}"))
}

fn start_service(
    base_dir: &Path,
    log: &Arc<Mutex<File>>,
    names: &mut LaunchNames,
) -> Result<String, String> {
    // Rename the file to a fresh random name for this run before the service
    // registration references it.
    rename_component(
        base_dir,
        &mut names.service_file,
        DEFAULT_SERVICE_FILE,
        "exe",
        false,
        log,
    );
    names.save(base_dir);
    let path = base_dir.join(&names.service_file);
    if !path.is_file() {
        return Err(format!("service file not found: {}", path.display()));
    }
    let service_name = random_token();
    let mut updated = names.clone();
    updated.service = service_name.clone();
    updated.save(base_dir);
    if service_state(&service_name) != LaunchState::Running {
        // The registered SCM name must match the name the windows-service
        // dispatcher expects, so it is passed to the binary through binPath.
        let quoted = format!("\"{}\" --service-name {}", path.display(), service_name);
        delete_service_if_present(&names.service, log);
        run_sc(
            log,
            &[
                "create",
                &service_name,
                "type=",
                "own",
                "start=",
                "demand",
                "obj=",
                "LocalSystem",
                "binPath=",
                &quoted,
            ],
        )?;
        run_sc(log, &["sidtype", &service_name, "unrestricted"])?;
        run_sc(log, &["start", &service_name])?;
    }
    if !wait_for_running(&service_name) {
        return Err("backend service did not reach RUNNING state".to_owned());
    }
    Ok(format!("service started as {service_name}"))
}

/// `sc start` returns while the service is still START_PENDING; poll until it
/// reaches RUNNING (about 3s at most) so the reported state is final.
fn wait_for_running(name: &str) -> bool {
    for _ in 0..30 {
        match service_state(name) {
            LaunchState::Running => return true,
            LaunchState::Starting => thread::sleep(Duration::from_millis(100)),
            _ => return false,
        }
    }
    false
}

fn start_gui(
    base_dir: &Path,
    log: &Arc<Mutex<File>>,
    names: &mut LaunchNames,
) -> Result<String, String> {
    // Rename the file to a fresh random name for this run. The Start button
    // is only enabled while the GUI is not running, so the rename cannot hit
    // a live process image.
    rename_component(
        base_dir,
        &mut names.gui_file,
        DEFAULT_GUI_FILE,
        "exe",
        false,
        log,
    );
    names.save(base_dir);
    let path = base_dir.join(&names.gui_file);
    if !path.is_file() {
        return Err(format!("GUI file not found: {}", path.display()));
    }
    Command::new(&path)
        .current_dir(base_dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("failed to start GUI: {error}"))?;
    write_log(log, "GUI process started");
    Ok("GUI started".to_owned())
}

fn stop_service(name: &str, log: &Arc<Mutex<File>>) -> Result<String, String> {
    if service_state(name) == LaunchState::Running {
        run_sc(log, &["stop", name])?;
    }
    run_sc(log, &["delete", name])?;
    Ok(format!("{name} stopped and deleted"))
}

fn delete_service_if_present(name: &str, log: &Arc<Mutex<File>>) {
    write_log(log, format!("recreating service {name}"));
    if service_state(name) == LaunchState::Running {
        if let Err(error) = run_sc(log, &["stop", name]) {
            write_log(log, format!("[warning] failed to stop {name}: {error}"));
        }
    }
    if let Err(error) = run_sc(log, &["delete", name]) {
        write_log(log, format!("[info] delete {name}: {error}"));
    }
}

fn stop_gui(image_name: &str, log: &Arc<Mutex<File>>) -> Result<String, String> {
    write_log(log, format!("> taskkill /IM {image_name} /T"));
    let output = Command::new("taskkill")
        .args(["/IM", image_name, "/T"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("failed to run taskkill: {error}"))?;
    write_log(log, String::from_utf8_lossy(&output.stdout));
    write_log(log, String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(format!(
            "taskkill failed with exit code {:?}",
            output.status.code()
        ));
    }
    Ok("GUI stopped".to_owned())
}

fn run_sc(log: &Arc<Mutex<File>>, args: &[&str]) -> Result<(), String> {
    write_log(log, format!("> sc.exe {}", args.join(" ")));
    let output = Command::new("sc.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("failed to run sc.exe: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    write_log(log, stdout.trim());
    if !stderr.trim().is_empty() {
        write_log(log, stderr.trim());
    }
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "sc.exe failed with exit code {:?}",
            output.status.code()
        ))
    }
}

fn write_log(log: &Arc<Mutex<File>>, message: impl AsRef<str>) {
    if let Ok(mut file) = log.lock() {
        let _ = writeln!(file, "{}", message.as_ref());
        let _ = file.flush();
    }
}

fn service_state(name: &str) -> LaunchState {
    let Ok(output) = Command::new("sc.exe")
        .args(["query", name])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return LaunchState::Unknown;
    };
    if !output.status.success() {
        return LaunchState::Stopped;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("RUNNING") {
        LaunchState::Running
    } else if text.contains("START_PENDING") || text.contains("STOP_PENDING") {
        LaunchState::Starting
    } else {
        LaunchState::Stopped
    }
}

fn process_state(name: &str) -> LaunchState {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {name}")])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(output) if String::from_utf8_lossy(&output.stdout).contains(name) => {
            LaunchState::Running
        }
        Ok(_) => LaunchState::Stopped,
        Err(_) => LaunchState::Unknown,
    }
}

fn install_chinese_font(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    let Some(path) = candidates.iter().find(|path| Path::new(path).is_file()) else {
        return;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "ks-launcher-cjk".to_owned(),
        egui::FontData::from_owned(bytes),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "ks-launcher-cjk".to_owned());
    }
    ctx.set_fonts(fonts);
}

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kernel Script Launcher")
            .with_inner_size([360.0, 300.0])
            .with_resizable(false),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "Kernel Script Launcher",
        options,
        Box::new(|creation_context| {
            install_chinese_font(&creation_context.egui_ctx);
            Box::new(LauncherApp::new())
        }),
    );
}
