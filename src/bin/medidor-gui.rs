//! Interfaz gráfica nativa de Windows (sin consola).
//!
//! La ventana solo lanza y controla el monitor: este se ejecuta como un proceso
//! aparte sin ventana (`medidor-gui.exe --hijo ...`), así que se puede cerrar la
//! interfaz y la monitorización continúa.

#![windows_subsystem = "windows"]

use medidor_recursos::{cli, monitor, sensors};
use native_windows_gui as nwg;
use std::cell::Cell;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const LHM_URL: &str = "https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases";
const REPO_URL: &str = "https://github.com/borjajoart/medidor-recursos";

const DURATIONS: [(&str, Option<&str>); 6] = [
    ("Hasta que lo detenga", None),
    ("15 minutos", Some("15m")),
    ("30 minutos", Some("30m")),
    ("1 hora", Some("1h")),
    ("2 horas", Some("2h")),
    ("4 horas", Some("4h")),
];
const INTERVALS: [(&str, &str); 3] = [("1 segundo", "1"), ("2 segundos (recomendado)", "2"), ("5 segundos", "5")];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Proceso hijo: el monitor propiamente dicho, sin ventana.
    if args.iter().any(|a| a == "--hijo") {
        if let Ok(cli::Command::Run { cfg, .. }) = cli::parse(&args) {
            let _ = fs::create_dir_all(&cfg.out_dir);
            monitor::run(cfg);
        }
        return;
    }
    if let Err(e) = gui() {
        nwg::error_message("Medidor de recursos", &format!("No se pudo abrir la ventana: {e}"));
    }
}

#[derive(Default)]
struct Ui {
    window: nwg::Window,
    title: nwg::Label,
    subtitle: nwg::Label,
    status_title: nwg::Label,
    status: nwg::Label,
    sensors: nwg::Label,
    btn_lhm: nwg::Button,
    opts_title: nwg::Label,
    lbl_dur: nwg::Label,
    dur: nwg::ComboBox<&'static str>,
    lbl_int: nwg::Label,
    int: nwg::ComboBox<&'static str>,
    lbl_cpu: nwg::Label,
    cpu_limit: nwg::TextInput,
    lbl_gpu: nwg::Label,
    gpu_limit: nwg::TextInput,
    open_report: nwg::CheckBox,
    btn_start: nwg::Button,
    btn_stop: nwg::Button,
    btn_last: nwg::Button,
    btn_folder: nwg::Button,
    btn_manual: nwg::Button,
    footer: nwg::Label,
    timer: nwg::AnimationTimer,
    f_title: nwg::Font,
    f_bold: nwg::Font,
    f_small: nwg::Font,
}

/// Estado compartido con los hilos auxiliares.
#[derive(Default)]
struct Shared {
    /// 0 = sin comprobar, 1 = detectado, 2 = no detectado
    lhm: AtomicU8,
    lhm_checking: AtomicBool,
    stopping: AtomicBool,
    stop_result: Mutex<Option<Result<PathBuf, String>>>,
}

struct App {
    ui: Ui,
    shared: Arc<Shared>,
    ticks: Cell<u32>,
    message: std::cell::RefCell<String>,
}

fn font(f: &mut nwg::Font, size: u32, weight: u32) -> Result<(), nwg::NwgError> {
    nwg::Font::builder().family("Segoe UI").size(size).weight(weight).build(f)
}

fn label(l: &mut nwg::Label, parent: &nwg::Window, text: &str, pos: (i32, i32), size: (i32, i32), f: Option<&nwg::Font>) -> Result<(), nwg::NwgError> {
    nwg::Label::builder()
        .text(text)
        .position(pos)
        .size(size)
        .font(f)
        .v_align(nwg::VTextAlign::Top)
        .parent(parent)
        .build(l)
}

fn button(b: &mut nwg::Button, parent: &nwg::Window, text: &str, pos: (i32, i32), size: (i32, i32)) -> Result<(), nwg::NwgError> {
    nwg::Button::builder().text(text).position(pos).size(size).parent(parent).build(b)
}

fn gui() -> Result<(), nwg::NwgError> {
    // El soporte de DPI y los controles v6 vienen del manifiesto (ver build.rs).
    nwg::init()?;
    let mut normal = nwg::Font::default();
    font(&mut normal, 17, 400)?;
    nwg::Font::set_global_default(Some(normal));

    let mut ui = Ui::default();
    font(&mut ui.f_title, 28, 700)?;
    font(&mut ui.f_bold, 18, 700)?;
    font(&mut ui.f_small, 15, 400)?;

    const W: i32 = 500;
    nwg::Window::builder()
        .title("Medidor de recursos")
        .size((W, 640))
        .center(true)
        .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::VISIBLE)
        .build(&mut ui.window)?;
    let w = &ui.window;
    let x = 20;
    let full = W - 2 * x;

    label(&mut ui.title, w, "Medidor de recursos", (x, 14), (full, 36), Some(&ui.f_title))?;
    label(
        &mut ui.subtitle,
        w,
        "Vigila temperaturas y procesos mientras juegas y genera un informe con gráficas.",
        (x, 52),
        (full, 22),
        Some(&ui.f_small),
    )?;

    label(&mut ui.status_title, w, "Estado", (x, 86), (full, 22), Some(&ui.f_bold))?;
    label(&mut ui.status, w, "Comprobando...", (x, 110), (full, 92), None)?;

    label(&mut ui.sensors, w, "Sensores: comprobando LibreHardwareMonitor...", (x, 208), (full, 40), Some(&ui.f_small))?;
    button(&mut ui.btn_lhm, w, "Descargar LibreHardwareMonitor", (x, 250), (240, 28))?;

    label(&mut ui.opts_title, w, "Opciones", (x, 296), (full, 22), Some(&ui.f_bold))?;
    let lx = x;
    let cx = 230;
    let cw = W - cx - x;
    label(&mut ui.lbl_dur, w, "Duración", (lx, 327), (200, 22), None)?;
    nwg::ComboBox::builder()
        .collection(DURATIONS.iter().map(|d| d.0).collect())
        .selected_index(Some(0))
        .position((cx, 322))
        .size((cw, 26))
        .parent(w)
        .build(&mut ui.dur)?;
    label(&mut ui.lbl_int, w, "Tomar una muestra cada", (lx, 361), (200, 22), None)?;
    nwg::ComboBox::builder()
        .collection(INTERVALS.iter().map(|d| d.0).collect())
        .selected_index(Some(1))
        .position((cx, 356))
        .size((cw, 26))
        .parent(w)
        .build(&mut ui.int)?;
    label(&mut ui.lbl_cpu, w, "Temperatura alta de CPU (°C)", (lx, 395), (205, 22), None)?;
    nwg::TextInput::builder().text("85").limit(3).position((cx, 391)).size((60, 26)).parent(w).build(&mut ui.cpu_limit)?;
    label(&mut ui.lbl_gpu, w, "Temperatura alta de GPU (°C)", (lx, 429), (205, 22), None)?;
    nwg::TextInput::builder().text("83").limit(3).position((cx, 425)).size((60, 26)).parent(w).build(&mut ui.gpu_limit)?;
    nwg::CheckBox::builder()
        .text("Abrir el informe al terminar")
        .check_state(nwg::CheckBoxState::Checked)
        .position((lx, 461))
        .size((full, 24))
        .parent(w)
        .build(&mut ui.open_report)?;

    let half = (full - 10) / 2;
    button(&mut ui.btn_start, w, "▶  Iniciar monitorización", (x, 500), (half, 40))?;
    button(&mut ui.btn_stop, w, "■  Detener y generar informe", (x + half + 10, 500), (half, 40))?;
    let third = (full - 20) / 3;
    button(&mut ui.btn_last, w, "Último informe", (x, 550), (third, 30))?;
    button(&mut ui.btn_folder, w, "Carpeta de informes", (x + third + 10, 550), (third, 30))?;
    button(&mut ui.btn_manual, w, "Manual", (x + 2 * (third + 10), 550), (third, 30))?;
    label(
        &mut ui.footer,
        w,
        "Puedes cerrar esta ventana: la monitorización sigue en segundo plano.",
        (x, 594),
        (full, 22),
        Some(&ui.f_small),
    )?;

    nwg::AnimationTimer::builder().parent(w).interval(Duration::from_millis(1500)).active(true).build(&mut ui.timer)?;

    let app = Rc::new(App { ui, shared: Arc::new(Shared::default()), ticks: Cell::new(0), message: Default::default() });
    app.check_lhm();
    app.refresh();

    let a = app.clone();
    let handler = nwg::full_bind_event_handler(&app.ui.window.handle, move |evt, _data, handle| {
        use nwg::Event as E;
        let ui = &a.ui;
        match evt {
            E::OnWindowClose if handle == ui.window.handle => nwg::stop_thread_dispatch(),
            E::OnTimerTick if handle == ui.timer.handle => a.tick(),
            E::OnButtonClick => {
                if handle == ui.btn_start.handle {
                    a.start();
                } else if handle == ui.btn_stop.handle {
                    a.stop();
                } else if handle == ui.btn_last.handle {
                    a.open_last();
                } else if handle == ui.btn_folder.handle {
                    let dir = cli::default_out_dir();
                    let _ = fs::create_dir_all(&dir);
                    monitor::open_path(&dir);
                } else if handle == ui.btn_manual.handle {
                    a.open_manual();
                } else if handle == ui.btn_lhm.handle {
                    monitor::open_path(Path::new(LHM_URL));
                }
            }
            _ => {}
        }
    });

    nwg::dispatch_thread_events();
    nwg::unbind_event_handler(&handler);
    Ok(())
}

impl App {
    fn tick(&self) {
        let t = self.ticks.get().wrapping_add(1);
        self.ticks.set(t);
        if t % 10 == 0 {
            self.check_lhm();
        }
        self.refresh();
    }

    /// Comprueba LibreHardwareMonitor en otro hilo (WMI puede tardar y usa COM).
    fn check_lhm(&self) {
        if self.shared.lhm_checking.swap(true, Ordering::SeqCst) {
            return;
        }
        let sh = self.shared.clone();
        std::thread::spawn(move || {
            let ok = sensors::hwmon_available();
            sh.lhm.store(if ok { 1 } else { 2 }, Ordering::SeqCst);
            sh.lhm_checking.store(false, Ordering::SeqCst);
        });
    }

    fn refresh(&self) {
        let ui = &self.ui;

        if let Some(res) = self.shared.stop_result.lock().unwrap().take() {
            self.shared.stopping.store(false, Ordering::SeqCst);
            *self.message.borrow_mut() = match res {
                Ok(p) => format!("Informe guardado: {}", p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
                Err(e) => e,
            };
        }

        let stopping = self.shared.stopping.load(Ordering::SeqCst);
        let running = monitor::running_monitor();
        let text = match (&running, stopping) {
            (Some(_), true) => "◌  Deteniendo y generando el informe...".to_string(),
            (Some(r), false) => live_status(r),
            (None, _) => {
                let msg = self.message.borrow();
                format!(
                    "○  Detenido.\r\nPulsa «Iniciar monitorización» y juega con normalidad. Al terminar, pulsa «Detener y generar informe».{}",
                    if msg.is_empty() { String::new() } else { format!("\r\n\r\n{msg}") }
                )
            }
        };
        if ui.status.text() != text {
            ui.status.set_text(&text);
        }

        let idle = running.is_none() && !stopping;
        ui.btn_start.set_enabled(idle);
        ui.btn_stop.set_enabled(running.is_some() && !stopping);
        for c in [&ui.dur as &dyn Enable, &ui.int, &ui.cpu_limit, &ui.gpu_limit, &ui.open_report] {
            c.enable(idle);
        }

        let s = match self.shared.lhm.load(Ordering::SeqCst) {
            1 => "Sensores: ✔ LibreHardwareMonitor detectado. Se leerán temperaturas, frecuencias y consumo.",
            2 => "Sensores: ✖ LibreHardwareMonitor no está abierto (ábrelo como administrador). Sin él solo hay temperatura de GPU NVIDIA.",
            _ => "Sensores: comprobando LibreHardwareMonitor...",
        };
        if ui.sensors.text() != s {
            ui.sensors.set_text(s);
        }
    }

    fn start(&self) {
        let ui = &self.ui;
        let parse_limit = |t: &nwg::TextInput, what: &str| -> Result<String, String> {
            let v = t.text().trim().to_string();
            match v.parse::<f32>() {
                Ok(n) if (40.0..=110.0).contains(&n) => Ok(v),
                _ => Err(format!("La temperatura alta de {what} debe ser un número entre 40 y 110.")),
            }
        };
        let limits = parse_limit(&ui.cpu_limit, "CPU").and_then(|c| parse_limit(&ui.gpu_limit, "GPU").map(|g| (c, g)));
        let (cpu, gpu) = match limits {
            Ok(v) => v,
            Err(e) => {
                nwg::modal_error_message(&ui.window, "Medidor de recursos", &e);
                return;
            }
        };
        let mut args: Vec<String> = vec![];
        if let Some(d) = DURATIONS[ui.dur.selection().unwrap_or(0)].1 {
            args.extend(["--duracion".into(), d.into()]);
        }
        args.extend(["--intervalo".into(), INTERVALS[ui.int.selection().unwrap_or(1)].1.into()]);
        args.extend(["--umbral-cpu".into(), cpu, "--umbral-gpu".into(), gpu]);
        if ui.open_report.check_state() != nwg::CheckBoxState::Checked {
            args.push("--no-abrir".into());
        }
        match monitor::spawn_child(&args) {
            Ok(_) => {
                self.message.borrow_mut().clear();
                // Da tiempo al monitor a escribir su archivo de control.
                std::thread::sleep(Duration::from_millis(700));
                self.refresh();
            }
            Err(e) => {
                nwg::modal_error_message(&ui.window, "Medidor de recursos", &e);
            }
        }
    }

    fn stop(&self) {
        if self.shared.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        let sh = self.shared.clone();
        std::thread::spawn(move || {
            let r = monitor::stop_and_wait(Duration::from_secs(90));
            *sh.stop_result.lock().unwrap() = Some(r);
        });
        self.refresh();
    }

    fn open_last(&self) {
        let newest = fs::read_dir(cli::default_out_dir()).ok().and_then(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.starts_with("informe_") && n.ends_with(".html")
                })
                .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
                .map(|e| e.path())
        });
        match newest {
            Some(p) => monitor::open_path(&p),
            None => {
                nwg::modal_info_message(&self.ui.window, "Medidor de recursos", "Todavía no hay ningún informe. Inicia una monitorización primero.");
            }
        }
    }

    fn open_manual(&self) {
        let manual = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("MANUAL.md")));
        match manual.filter(|m| m.exists()) {
            Some(m) => {
                let _ = std::process::Command::new("notepad.exe").arg(m).spawn();
            }
            None => monitor::open_path(Path::new(REPO_URL)),
        }
    }
}

/// Activa/desactiva controles de distinto tipo con la misma llamada.
trait Enable {
    fn enable(&self, v: bool);
}
impl Enable for nwg::ComboBox<&'static str> {
    fn enable(&self, v: bool) {
        self.set_enabled(v)
    }
}
impl Enable for nwg::TextInput {
    fn enable(&self, v: bool) {
        self.set_enabled(v)
    }
}
impl Enable for nwg::CheckBox {
    fn enable(&self, v: bool) {
        self.set_enabled(v)
    }
}

/// Lee la última línea del CSV que va escribiendo el monitor.
fn last_csv_line(path: &Path) -> Option<Vec<String>> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(4096))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().rev().find(|l| !l.trim().is_empty() && !l.starts_with("hora;"))?;
    Some(line.split(';').map(str::to_string).collect())
}

fn live_status(r: &monitor::Running) -> String {
    let head = format!("●  Monitorizando (PID {})", r.pid);
    let Some(c) = last_csv_line(&r.csv) else {
        return format!("{head}\r\nIniciando sensores...");
    };
    let g = |i: usize| c.get(i).map(String::as_str).unwrap_or("");
    let n = |i: usize, unit: &str| {
        g(i).replace(',', ".").parse::<f32>().map(|v| format!("{v:.0}{unit}")).unwrap_or_else(|_| "--".into())
    };
    // "34 %" o "34 % · 72 °C" según haya temperatura
    let pair = |load: usize, temp: usize| {
        let t = n(temp, " °C");
        if t == "--" { n(load, " %") } else { format!("{} · {t}", n(load, " %")) }
    };
    let secs = g(1).parse::<u64>().unwrap_or(0);
    let top = if g(17).is_empty() { "-".to_string() } else { format!("{} ({})", g(17), n(18, " %")) };
    let fg = if g(16).is_empty() { "-" } else { g(16) };
    format!(
        "{head}  ·  {:02}:{:02}:{:02}\r\nCPU {}      GPU {}      RAM {}\r\nMás CPU: {top}\r\nVentana activa: {fg}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        pair(2, 5),
        pair(8, 9),
        n(3, " %"),
    )
}
