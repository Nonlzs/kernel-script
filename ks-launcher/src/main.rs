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

const DRIVER_SERVICE: &str = "KsDriver";
const BACKEND_SERVICE: &str = "KsService";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
    driver: LaunchState,
    service: LaunchState,
    gui: LaunchState,
    busy: bool,
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
        let mut app = Self {
            base_dir,
            log: Arc::new(Mutex::new(log)),
            events,
            event_tx,
            driver: LaunchState::Unknown,
            service: LaunchState::Unknown,
            gui: LaunchState::Unknown,
            busy: false,
        };
        app.log("launcher started");
        app.log(format!("launcher directory: {}", app.base_dir.display()));
        app.log(format!(
            "driver path: {}",
            app.base_dir.join("ks-driver.sys").display()
        ));
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
        self.driver = service_state(DRIVER_SERVICE);
        self.service = service_state(BACKEND_SERVICE);
        self.gui = process_state("ks-gui.exe");
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
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = match target {
                Target::Driver => start_driver(&base_dir, &log),
                Target::Service => start_service(&base_dir, &log),
                Target::Gui => start_gui(&base_dir, &log),
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
            events
                .send(LaunchEvent {
                    target,
                    state: service_or_process_state(target),
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
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = match target {
                Target::Driver => stop_service(DRIVER_SERVICE, &log),
                Target::Service => stop_service(BACKEND_SERVICE, &log),
                Target::Gui => stop_gui(&log),
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
        while let Ok(event) = self.events.try_recv() {
            self.log(format!("{:?}: {}", event.target, event.message));
            *self.state_mut(event.target) = event.state;
            if event.state != LaunchState::Starting {
                self.busy = false;
            }
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_events();
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

fn start_driver(base_dir: &Path, log: &Arc<Mutex<File>>) -> Result<String, String> {
    let path = base_dir.join("ks-driver.sys");
    if !path.is_file() {
        return Err(format!("driver file not found: {}", path.display()));
    }
    // Command arguments already preserve the path as one value. Do not embed
    // quote characters in the value: SCM would store those quotes in ImagePath
    // instead of normalizing it to the native \??\ path form.
    let image_path = path.to_string_lossy().into_owned();
    if service_state(DRIVER_SERVICE) != LaunchState::Running {
        delete_service_if_present(DRIVER_SERVICE, log);
        run_sc(
            log,
            &[
                "create",
                DRIVER_SERVICE,
                "type=",
                "kernel",
                "start=",
                "demand",
                "binPath=",
                &image_path,
            ],
        )?;
        run_sc(log, &["start", DRIVER_SERVICE])?;
    }
    if service_state(DRIVER_SERVICE) != LaunchState::Running {
        return Err("driver did not reach RUNNING state".to_owned());
    }
    Ok("driver started".to_owned())
}

fn start_service(base_dir: &Path, log: &Arc<Mutex<File>>) -> Result<String, String> {
    let path = base_dir.join("ks-service.exe");
    if !path.is_file() {
        return Err(format!("service file not found: {}", path.display()));
    }
    if service_state(BACKEND_SERVICE) != LaunchState::Running {
        let quoted = format!("\"{}\"", path.display());
        delete_service_if_present(BACKEND_SERVICE, log);
        run_sc(
            log,
            &[
                "create",
                BACKEND_SERVICE,
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
        run_sc(log, &["sidtype", BACKEND_SERVICE, "unrestricted"])?;
        run_sc(log, &["start", BACKEND_SERVICE])?;
    }
    if service_state(BACKEND_SERVICE) != LaunchState::Running {
        return Err("backend service did not reach RUNNING state".to_owned());
    }
    Ok("service started".to_owned())
}

fn start_gui(base_dir: &Path, log: &Arc<Mutex<File>>) -> Result<String, String> {
    let path = base_dir.join("ks-gui.exe");
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

fn stop_gui(log: &Arc<Mutex<File>>) -> Result<String, String> {
    write_log(log, "> taskkill /IM ks-gui.exe /T");
    let output = Command::new("taskkill")
        .args(["/IM", "ks-gui.exe", "/T"])
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

fn service_or_process_state(target: Target) -> LaunchState {
    match target {
        Target::Driver => service_state(DRIVER_SERVICE),
        Target::Service => service_state(BACKEND_SERVICE),
        Target::Gui => process_state("ks-gui.exe"),
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
