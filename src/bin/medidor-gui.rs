//! Interfaz gráfica nativa de Windows (sin consola).
//!
//! Dos modos en el mismo ejecutable:
//! - Panel (sin argumentos): la ventana con las opciones. Solo lanza y controla el monitor.
//! - Monitor (`--hijo ...`): mide en segundo plano y muestra un icono en la bandeja del
//!   sistema (color según la temperatura, menú para marcar o detener y avisos).
//!   Se puede cerrar el panel y la medición continúa.
//! - `--autoarranque`: lo que se ejecuta al iniciar Windows (si se activó en el panel).

#![windows_subsystem = "windows"]

use medidor_recursos::{cli, compare, monitor, sensors};
use native_windows_gui as nwg;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const APP: &str = "Medidor de recursos";
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

// Iconos incrustados (res/recursos.rc)
const ICO_APP: usize = 1;
const ICO_OK: usize = 2;
const ICO_WARM: usize = 3;
const ICO_HOT: usize = 4;
const ICO_IDLE: usize = 5;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--hijo") {
        tray_child(&args);
    } else if args.iter().any(|a| a == "--autoarranque") {
        autostart();
    } else if let Err(e) = panel() {
        nwg::error_message(APP, &format!("No se pudo abrir la ventana: {e}"));
    }
}

// ---------------------------------------------------------------------------
// Ajustes guardados (%APPDATA%\MedidorRecursos\config.json)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    duration: usize,
    interval: usize,
    cpu_limit: String,
    gpu_limit: String,
    ping: String,
    fps: bool,
    watch: bool,
    games: String,
    open_report: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            duration: 0,
            interval: 1,
            cpu_limit: "85".into(),
            gpu_limit: "83".into(),
            ping: cli::DEFAULT_PING.into(),
            fps: false,
            watch: false,
            games: String::new(),
            open_report: true,
        }
    }
}

fn settings_path() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("MedidorRecursos").join("config.json")
}

impl Settings {
    fn load() -> Self {
        fs::read_to_string(settings_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn save(&self) {
        let p = settings_path();
        if let Some(d) = p.parent() {
            let _ = fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = fs::write(p, t);
        }
    }

    /// Argumentos para el monitor. Error si algo no es válido.
    fn to_args(&self, force_watch: bool) -> Result<Vec<String>, String> {
        let limit = |v: &str, what: &str| -> Result<String, String> {
            let v = v.trim();
            match v.parse::<f32>() {
                Ok(n) if (40.0..=110.0).contains(&n) => Ok(v.to_string()),
                _ => Err(format!("La temperatura alta de {what} debe ser un número entre 40 y 110.")),
            }
        };
        let mut a: Vec<String> = vec![];
        if let Some(d) = DURATIONS.get(self.duration).and_then(|d| d.1) {
            a.extend(["--duracion".into(), d.into()]);
        }
        a.extend(["--intervalo".into(), INTERVALS.get(self.interval).unwrap_or(&INTERVALS[1]).1.into()]);
        a.extend(["--umbral-cpu".into(), limit(&self.cpu_limit, "CPU")?]);
        a.extend(["--umbral-gpu".into(), limit(&self.gpu_limit, "GPU")?]);
        let ping = self.ping.trim();
        a.extend(["--ping".into(), if ping.is_empty() { "no".into() } else { ping.to_string() }]);
        if !self.fps {
            a.push("--sin-fps".into());
        }
        if self.watch || force_watch {
            let games = cli::parse_exe_list(&self.games);
            if games.is_empty() {
                return Err("Escribe al menos un juego (el nombre del .exe, por ejemplo: FortniteClient-Win64-Shipping.exe).".into());
            }
            a.extend(["--esperar".into(), games.join(",")]);
        }
        if !self.open_report {
            a.push("--no-abrir".into());
        }
        Ok(a)
    }
}

fn spawn(settings: &Settings, args: &[String]) -> Result<u32, String> {
    if settings.fps && !monitor::is_elevated() { monitor::spawn_child_elevated(args) } else { monitor::spawn_child(args) }
}

// ---------------------------------------------------------------------------
// Inicio con Windows (HKCU\...\Run)
// ---------------------------------------------------------------------------

mod autorun {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW};

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "MedidorRecursos";

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn enabled() -> bool {
        let mut size = 0u32;
        unsafe {
            RegGetValueW(HKEY_CURRENT_USER, w(KEY).as_ptr(), w(NAME).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) == 0
        }
    }

    pub fn set(on: bool) -> Result<(), String> {
        let r = unsafe {
            if on {
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let cmd = w(&format!("\"{}\" --autoarranque", exe.display()));
                RegSetKeyValueW(HKEY_CURRENT_USER, w(KEY).as_ptr(), w(NAME).as_ptr(), REG_SZ, cmd.as_ptr().cast(), (cmd.len() * 2) as u32)
            } else {
                let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, w(KEY).as_ptr(), w(NAME).as_ptr());
                if r == 2 { 0 } else { r } // ERROR_FILE_NOT_FOUND: ya no estaba
            }
        };
        if r == 0 { Ok(()) } else { Err(format!("No se pudo cambiar el inicio con Windows (error {r}).")) }
    }
}

/// Al iniciar Windows: espera a los juegos configurados (en la bandeja).
fn autostart() {
    let s = Settings::load();
    if monitor::running_monitor().is_some() {
        return;
    }
    if let Ok(args) = s.to_args(true) {
        let _ = spawn(&s, &args);
    }
}

// ---------------------------------------------------------------------------
// Monitor con icono en la bandeja
// ---------------------------------------------------------------------------

fn tray_child(args: &[String]) {
    let Ok(cli::Command::Run { cfg, .. }) = cli::parse(args) else { return };
    let _ = fs::create_dir_all(&cfg.out_dir);
    let control = Arc::new(monitor::Control::default());
    let c = control.clone();
    let worker = std::thread::spawn(move || monitor::run(cfg, c));
    // Si no se puede crear el icono, el monitor sigue igual (se detiene desde el panel).
    let _ = tray(&control);
    let _ = worker.join();
}

#[derive(Default)]
struct Tray {
    window: nwg::MessageWindow,
    icon: nwg::TrayNotification,
    menu: nwg::Menu,
    m_mark: nwg::MenuItem,
    m_panel: nwg::MenuItem,
    m_sep: nwg::MenuSeparator,
    m_stop: nwg::MenuItem,
    timer: nwg::AnimationTimer,
    icons: Vec<nwg::Icon>,
}

fn tray(control: &Arc<monitor::Control>) -> Result<(), nwg::NwgError> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    nwg::init()?;
    let mut t = Tray::default();
    let embed = nwg::EmbedResource::load(None)?;
    let sm = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    for id in [ICO_IDLE, ICO_OK, ICO_WARM, ICO_HOT] {
        t.icons.push(embed.icon(id, Some((sm, sm))).unwrap_or_default());
    }
    nwg::MessageWindow::builder().build(&mut t.window)?;
    if monitor::is_elevated() {
        // Un proceso de administrador no recibe por defecto los mensajes del Explorador
        // (clics en el icono). Se permiten de forma explícita.
        use windows_sys::Win32::UI::WindowsAndMessaging::{ChangeWindowMessageFilterEx, MSGFLT_ALLOW, RegisterWindowMessageW, WM_USER};
        if let Some(h) = t.window.handle.hwnd() {
            let taskbar: Vec<u16> = "TaskbarCreated".encode_utf16().chain(Some(0)).collect();
            unsafe {
                ChangeWindowMessageFilterEx(h as _, WM_USER + 102, MSGFLT_ALLOW, std::ptr::null_mut());
                ChangeWindowMessageFilterEx(h as _, RegisterWindowMessageW(taskbar.as_ptr()), MSGFLT_ALLOW, std::ptr::null_mut());
            }
        }
    }
    nwg::TrayNotification::builder()
        .parent(&t.window)
        .icon(Some(&t.icons[0]))
        .tip(Some("Medidor de recursos: iniciando..."))
        .build(&mut t.icon)?;
    nwg::Menu::builder().popup(true).parent(&t.window).build(&mut t.menu)?;
    nwg::MenuItem::builder().text("Marcar este momento\tCtrl+Alt+M").parent(&t.menu).build(&mut t.m_mark)?;
    nwg::MenuItem::builder().text("Abrir el panel").parent(&t.menu).build(&mut t.m_panel)?;
    nwg::MenuSeparator::builder().parent(&t.menu).build(&mut t.m_sep)?;
    nwg::MenuItem::builder().text("Detener y generar informe").parent(&t.menu).build(&mut t.m_stop)?;
    nwg::AnimationTimer::builder().parent(&t.window).interval(Duration::from_millis(1000)).active(true).build(&mut t.timer)?;

    let t = Rc::new(t);
    let state = Rc::new(Cell::new(usize::MAX));
    let tip = Rc::new(RefCell::new(String::new()));
    let ctl = control.clone();
    let tt = t.clone();
    let handler = nwg::full_bind_event_handler(&t.window.handle, move |evt, _, handle| {
        use nwg::Event as E;
        match evt {
            E::OnContextMenu if handle == tt.icon.handle => {
                let mut p = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p) };
                tt.menu.popup(p.x, p.y);
            }
            E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) if handle == tt.icon.handle => open_panel(),
            E::OnMenuItemSelected => {
                if handle == tt.m_mark.handle {
                    let _ = monitor::request_mark();
                } else if handle == tt.m_panel.handle {
                    open_panel();
                } else if handle == tt.m_stop.handle {
                    ctl.stop.store(true, Ordering::SeqCst);
                    tt.icon.set_tip("Medidor de recursos: generando el informe...");
                }
            }
            E::OnTimerTick if handle == tt.timer.handle => {
                if ctl.finished.load(Ordering::SeqCst) {
                    nwg::stop_thread_dispatch();
                    return;
                }
                let live = ctl.live.lock().unwrap().clone();
                let (idx, text) = tray_state(&live);
                if state.get() != idx {
                    state.set(idx);
                    tt.icon.set_icon(&tt.icons[idx]);
                }
                if *tip.borrow() != text {
                    tt.icon.set_tip(&text);
                    *tip.borrow_mut() = text;
                }
                let alerts: Vec<_> = std::mem::take(&mut *ctl.alerts.lock().unwrap());
                if let Some((title, text)) = alerts.into_iter().last() {
                    tt.icon.show(&text, Some(&title), Some(nwg::TrayNotificationFlags::WARNING_ICON), None);
                }
            }
            _ => {}
        }
    });
    nwg::dispatch_thread_events();
    nwg::unbind_event_handler(&handler);
    Ok(())
}

/// Icono (índice en `Tray::icons`) y texto del tooltip según el estado.
fn tray_state(l: &monitor::Live) -> (usize, String) {
    if l.waiting {
        return (0, "Medidor de recursos\nEsperando a que se abra el juego".into());
    }
    let ratio = |t: Option<f32>, lim: f32| t.map(|t| t - lim);
    let worst = [ratio(l.cpu_temp, l.cpu_limit), ratio(l.gpu_temp, l.gpu_limit)].into_iter().flatten().fold(f32::MIN, f32::max);
    let idx = if worst >= 0.0 {
        3
    } else if worst >= -8.0 {
        2
    } else {
        1
    };
    let s = l.elapsed as u64;
    let mut parts = vec![];
    if let Some(t) = l.cpu_temp {
        parts.push(format!("CPU {t:.0} °C"));
    } else {
        parts.push(format!("CPU {:.0} %", l.cpu));
    }
    if let Some(t) = l.gpu_temp {
        parts.push(format!("GPU {t:.0} °C"));
    }
    if let Some(f) = l.fps {
        parts.push(format!("{f:.0} FPS"));
    }
    let head = match &l.game {
        Some(g) => format!("Midiendo {g}"),
        None => "Midiendo".into(),
    };
    let mut text = format!("{head} · {:02}:{:02}:{:02}\n{}", s / 3600, (s / 60) % 60, s % 60, parts.join(" · "));
    if l.marks > 0 {
        text.push_str(&format!("\n{} marca{}", l.marks, if l.marks == 1 { "" } else { "s" }));
    }
    text.truncate(120);
    (idx, text)
}

/// Abre el panel como usuario normal aunque el monitor sea administrador.
fn open_panel() {
    if let Ok(exe) = std::env::current_exe() {
        monitor::open_path(&exe);
    }
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Ui {
    window: nwg::Window,
    icon: nwg::Icon,
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
    lbl_temp: nwg::Label,
    lbl_cpu: nwg::Label,
    cpu_limit: nwg::TextInput,
    lbl_gpu: nwg::Label,
    gpu_limit: nwg::TextInput,
    lbl_ping: nwg::Label,
    ping: nwg::TextInput,
    lbl_ping_hint: nwg::Label,
    fps: nwg::CheckBox,
    watch: nwg::CheckBox,
    games: nwg::TextInput,
    open_report: nwg::CheckBox,
    autostart: nwg::CheckBox,
    btn_start: nwg::Button,
    btn_mark: nwg::Button,
    btn_stop: nwg::Button,
    btn_last: nwg::Button,
    btn_folder: nwg::Button,
    btn_compare: nwg::Button,
    btn_manual: nwg::Button,
    footer: nwg::Label,
    timer: nwg::AnimationTimer,
    dialog: nwg::FileDialog,
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
    message: RefCell<String>,
}

fn font(f: &mut nwg::Font, size: u32, weight: u32) -> Result<(), nwg::NwgError> {
    nwg::Font::builder().family("Segoe UI").size(size).weight(weight).build(f)
}

fn label(l: &mut nwg::Label, parent: &nwg::Window, text: &str, pos: (i32, i32), size: (i32, i32), f: Option<&nwg::Font>) -> Result<(), nwg::NwgError> {
    nwg::Label::builder().text(text).position(pos).size(size).font(f).v_align(nwg::VTextAlign::Top).parent(parent).build(l)
}

fn button(b: &mut nwg::Button, parent: &nwg::Window, text: &str, pos: (i32, i32), size: (i32, i32)) -> Result<(), nwg::NwgError> {
    nwg::Button::builder().text(text).position(pos).size(size).parent(parent).build(b)
}

fn check(c: &mut nwg::CheckBox, parent: &nwg::Window, text: &str, on: bool, pos: (i32, i32), w: i32) -> Result<(), nwg::NwgError> {
    let st = if on { nwg::CheckBoxState::Checked } else { nwg::CheckBoxState::Unchecked };
    nwg::CheckBox::builder().text(text).check_state(st).position(pos).size((w, 24)).parent(parent).build(c)
}

fn input(t: &mut nwg::TextInput, parent: &nwg::Window, text: &str, limit: usize, pos: (i32, i32), w: i32) -> Result<(), nwg::NwgError> {
    nwg::TextInput::builder().text(text).limit(limit).position(pos).size((w, 26)).parent(parent).build(t)
}

fn panel() -> Result<(), nwg::NwgError> {
    // El soporte de DPI y los controles v6 vienen del manifiesto (ver build.rs).
    nwg::init()?;
    let mut normal = nwg::Font::default();
    font(&mut normal, 17, 400)?;
    nwg::Font::set_global_default(Some(normal));
    let st = Settings::load();

    let mut ui = Ui::default();
    font(&mut ui.f_title, 28, 700)?;
    font(&mut ui.f_bold, 18, 700)?;
    font(&mut ui.f_small, 15, 400)?;
    if let Ok(embed) = nwg::EmbedResource::load(None) {
        ui.icon = embed.icon(ICO_APP, None).unwrap_or_default();
    }

    const W: i32 = 520;
    nwg::Window::builder()
        .title(APP)
        .size((W, 736))
        .center(true)
        .icon(Some(&ui.icon))
        .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::VISIBLE)
        .build(&mut ui.window)?;
    let w = &ui.window;
    let x = 20;
    let full = W - 2 * x;

    label(&mut ui.title, w, APP, (x, 14), (full, 36), Some(&ui.f_title))?;
    label(&mut ui.subtitle, w, "Vigila temperaturas, procesos y FPS mientras juegas y genera un informe con gráficas.", (x, 52), (full, 22), Some(&ui.f_small))?;

    label(&mut ui.status_title, w, "Estado", (x, 84), (full, 22), Some(&ui.f_bold))?;
    label(&mut ui.status, w, "Comprobando...", (x, 108), (full, 100), None)?;

    label(&mut ui.sensors, w, "Sensores: comprobando LibreHardwareMonitor...", (x, 212), (full, 38), Some(&ui.f_small))?;
    button(&mut ui.btn_lhm, w, "Descargar LibreHardwareMonitor", (x, 252), (240, 26))?;

    label(&mut ui.opts_title, w, "Opciones", (x, 290), (full, 22), Some(&ui.f_bold))?;
    let cx = 230;
    let cw = W - cx - x;
    label(&mut ui.lbl_dur, w, "Duración", (x, 321), (200, 22), None)?;
    nwg::ComboBox::builder()
        .collection(DURATIONS.iter().map(|d| d.0).collect())
        .selected_index(Some(st.duration.min(DURATIONS.len() - 1)))
        .position((cx, 316))
        .size((cw, 26))
        .parent(w)
        .build(&mut ui.dur)?;
    label(&mut ui.lbl_int, w, "Tomar una muestra cada", (x, 353), (200, 22), None)?;
    nwg::ComboBox::builder()
        .collection(INTERVALS.iter().map(|d| d.0).collect())
        .selected_index(Some(st.interval.min(INTERVALS.len() - 1)))
        .position((cx, 348))
        .size((cw, 26))
        .parent(w)
        .build(&mut ui.int)?;
    label(&mut ui.lbl_temp, w, "Temperatura alta (°C)", (x, 385), (200, 22), None)?;
    label(&mut ui.lbl_cpu, w, "CPU", (cx, 385), (36, 22), None)?;
    input(&mut ui.cpu_limit, w, &st.cpu_limit, 3, (cx + 38, 380), 52)?;
    label(&mut ui.lbl_gpu, w, "GPU", (cx + 112, 385), (36, 22), None)?;
    input(&mut ui.gpu_limit, w, &st.gpu_limit, 3, (cx + 150, 380), 52)?;
    label(&mut ui.lbl_ping, w, "Medir el ping a", (x, 417), (200, 22), None)?;
    input(&mut ui.ping, w, &st.ping, 64, (cx, 412), 130)?;
    label(&mut ui.lbl_ping_hint, w, "vacío = no medir", (cx + 138, 417), (cw - 138, 22), Some(&ui.f_small))?;

    check(&mut ui.fps, w, "Medir FPS (Windows pedirá permiso de administrador)", st.fps, (x, 450), full)?;
    check(&mut ui.watch, w, "Esperar a que se abra un juego y medir solo mientras está abierto:", st.watch, (x, 476), full)?;
    nwg::TextInput::builder()
        .text(&st.games)
        .placeholder_text(Some("ejemplo: cs2.exe, FortniteClient-Win64-Shipping.exe"))
        .position((x + 22, 502))
        .size((full - 22, 26))
        .parent(w)
        .build(&mut ui.games)?;
    check(&mut ui.open_report, w, "Abrir el informe al terminar", st.open_report, (x, 536), full)?;
    check(&mut ui.autostart, w, "Iniciar con Windows (espera a los juegos de la lista)", autorun::enabled(), (x, 562), full)?;

    let small = 120;
    let big = (full - small - 20) / 2;
    button(&mut ui.btn_start, w, "▶  Iniciar", (x, 600), (big, 40))?;
    button(&mut ui.btn_mark, w, "Marcar momento", (x + big + 10, 600), (small, 40))?;
    button(&mut ui.btn_stop, w, "■  Detener e informe", (x + big + small + 20, 600), (big, 40))?;
    let q = (full - 30) / 4;
    button(&mut ui.btn_last, w, "Último informe", (x, 650), (q, 30))?;
    button(&mut ui.btn_folder, w, "Carpeta", (x + q + 10, 650), (q, 30))?;
    button(&mut ui.btn_compare, w, "Comparar...", (x + 2 * (q + 10), 650), (q, 30))?;
    button(&mut ui.btn_manual, w, "Manual", (x + 3 * (q + 10), 650), (q, 30))?;
    label(
        &mut ui.footer,
        w,
        "Puedes cerrar esta ventana: la medición sigue (icono junto al reloj). Ctrl+Alt+M marca un momento.",
        (x, 692),
        (full, 40),
        Some(&ui.f_small),
    )?;

    nwg::AnimationTimer::builder().parent(w).interval(Duration::from_millis(1500)).active(true).build(&mut ui.timer)?;
    nwg::FileDialog::builder()
        .title("Elige dos archivos de datos (datos_*.csv) con Ctrl+clic")
        .action(nwg::FileDialogAction::Open)
        .multiselect(true)
        .filters("Datos de sesión(*.csv)|Todos(*.*)")
        .build(&mut ui.dialog)?;

    let app = Rc::new(App { ui, shared: Arc::new(Shared::default()), ticks: Cell::new(0), message: Default::default() });
    app.check_lhm();
    app.refresh();

    let a = app.clone();
    let handler = nwg::full_bind_event_handler(&app.ui.window.handle, move |evt, _data, handle| {
        use nwg::Event as E;
        let ui = &a.ui;
        match evt {
            E::OnWindowClose if handle == ui.window.handle => {
                a.settings().save();
                nwg::stop_thread_dispatch();
            }
            E::OnTimerTick if handle == ui.timer.handle => a.tick(),
            E::OnButtonClick => {
                if handle == ui.btn_start.handle {
                    a.start();
                } else if handle == ui.btn_stop.handle {
                    a.stop();
                } else if handle == ui.btn_mark.handle {
                    *a.message.borrow_mut() = match monitor::request_mark() {
                        Ok(()) => "Marca añadida.".into(),
                        Err(e) => e,
                    };
                    a.refresh();
                } else if handle == ui.btn_last.handle {
                    a.open_last();
                } else if handle == ui.btn_folder.handle {
                    let dir = cli::default_out_dir();
                    let _ = fs::create_dir_all(&dir);
                    monitor::open_path(&dir);
                } else if handle == ui.btn_compare.handle {
                    a.compare();
                } else if handle == ui.btn_manual.handle {
                    a.open_manual();
                } else if handle == ui.btn_lhm.handle {
                    monitor::open_path(Path::new(LHM_URL));
                } else if handle == ui.autostart.handle {
                    a.toggle_autostart();
                } else if handle == ui.watch.handle {
                    a.refresh();
                }
            }
            _ => {}
        }
    });

    nwg::dispatch_thread_events();
    nwg::unbind_event_handler(&handler);
    Ok(())
}

fn checked(c: &nwg::CheckBox) -> bool {
    c.check_state() == nwg::CheckBoxState::Checked
}

impl App {
    fn settings(&self) -> Settings {
        let ui = &self.ui;
        Settings {
            duration: ui.dur.selection().unwrap_or(0),
            interval: ui.int.selection().unwrap_or(1),
            cpu_limit: ui.cpu_limit.text().trim().into(),
            gpu_limit: ui.gpu_limit.text().trim().into(),
            ping: ui.ping.text().trim().into(),
            fps: checked(&ui.fps),
            watch: checked(&ui.watch),
            games: ui.games.text().trim().into(),
            open_report: checked(&ui.open_report),
        }
    }

    fn error(&self, e: &str) {
        nwg::modal_error_message(&self.ui.window, APP, e);
    }

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
                Ok(p) if p.as_os_str().is_empty() => "Detenido. No llegó a abrirse ningún juego, así que no hay informe.".into(),
                Ok(p) => format!("Informe guardado: {}", p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
                Err(e) => e,
            };
        }

        let stopping = self.shared.stopping.load(Ordering::SeqCst);
        let running = monitor::running_monitor();
        let msg = self.message.borrow().clone();
        let text = match (&running, stopping) {
            (Some(_), true) => "◌  Deteniendo y generando el informe...".to_string(),
            (Some(r), false) => {
                let mut t = live_status(r);
                if !msg.is_empty() {
                    t.push_str(&format!("\r\n{msg}"));
                }
                t
            }
            (None, _) => format!(
                "○  Detenido.\r\nPulsa «Iniciar» y juega con normalidad. Al terminar, pulsa «Detener e informe» (o usa el icono junto al reloj).{}",
                if msg.is_empty() { String::new() } else { format!("\r\n\r\n{msg}") }
            ),
        };
        if ui.status.text() != text {
            ui.status.set_text(&text);
        }

        let idle = running.is_none() && !stopping;
        ui.btn_start.set_enabled(idle);
        ui.btn_stop.set_enabled(running.is_some() && !stopping);
        ui.btn_mark.set_enabled(running.as_ref().is_some_and(|r| r.waiting_for().is_none()) && !stopping);
        for c in [&ui.dur as &dyn Enable, &ui.int, &ui.cpu_limit, &ui.gpu_limit, &ui.ping, &ui.fps, &ui.watch, &ui.open_report] {
            c.enable(idle);
        }
        ui.games.set_enabled(idle && checked(&ui.watch));

        let s = match self.shared.lhm.load(Ordering::SeqCst) {
            1 => "Sensores: LibreHardwareMonitor detectado. Se leerán temperaturas, frecuencias, consumo y ventiladores.",
            2 => "Sensores: LibreHardwareMonitor no está abierto (ábrelo como administrador). Sin él solo hay temperatura de GPU NVIDIA.",
            _ => "Sensores: comprobando LibreHardwareMonitor...",
        };
        if ui.sensors.text() != s {
            ui.sensors.set_text(s);
        }
    }

    fn start(&self) {
        let st = self.settings();
        let args = match st.to_args(false) {
            Ok(a) => a,
            Err(e) => return self.error(&e),
        };
        st.save();
        match spawn(&st, &args) {
            Ok(_) => {
                self.message.borrow_mut().clear();
                // Da tiempo al monitor a escribir su archivo de control.
                std::thread::sleep(Duration::from_millis(700));
                self.refresh();
            }
            Err(e) => self.error(&e),
        }
    }

    fn stop(&self) {
        if self.shared.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        self.message.borrow_mut().clear();
        let sh = self.shared.clone();
        std::thread::spawn(move || {
            let r = monitor::stop_and_wait(Duration::from_secs(90));
            *sh.stop_result.lock().unwrap() = Some(r);
        });
        self.refresh();
    }

    fn toggle_autostart(&self) {
        let ui = &self.ui;
        let on = checked(&ui.autostart);
        let st = self.settings();
        if on {
            if let Err(e) = st.to_args(true) {
                ui.autostart.set_check_state(nwg::CheckBoxState::Unchecked);
                return self.error(&format!("Para iniciar con Windows hace falta la lista de juegos a esperar.\n\n{e}"));
            }
        }
        st.save();
        if let Err(e) = autorun::set(on) {
            ui.autostart.set_check_state(if on { nwg::CheckBoxState::Unchecked } else { nwg::CheckBoxState::Checked });
            self.error(&e);
        } else if on {
            let extra = if st.fps { "\n\nComo está activado «Medir FPS», Windows pedirá permiso de administrador al iniciar sesión." } else { "" };
            nwg::modal_info_message(
                &ui.window,
                APP,
                &format!("Al iniciar sesión se quedará esperando (icono gris junto al reloj) a que abras alguno de estos juegos, con las opciones actuales.{extra}"),
            );
        }
    }

    fn compare(&self) {
        let ui = &self.ui;
        let _ = ui.dialog.set_default_folder(&cli::default_out_dir().to_string_lossy());
        if !ui.dialog.run(Some(&ui.window)) {
            return;
        }
        let mut files: Vec<PathBuf> = ui.dialog.get_selected_items().unwrap_or_default().into_iter().map(PathBuf::from).collect();
        if files.len() != 2 {
            return self.error("Selecciona exactamente dos archivos datos_*.csv (mantén Ctrl pulsado al hacer clic).");
        }
        files.sort(); // por nombre = por fecha: A es la más antigua
        match compare::compare_files(&files[0], &files[1], None) {
            Ok(p) => monitor::open_path(&std::path::absolute(&p).unwrap_or(p)),
            Err(e) => self.error(&e),
        }
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
                nwg::modal_info_message(&self.ui.window, APP, "Todavía no hay ningún informe. Inicia una medición primero.");
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
macro_rules! enable_impl {
    ($($t:ty),*) => { $(impl Enable for $t { fn enable(&self, v: bool) { self.set_enabled(v) } })* };
}
enable_impl!(nwg::ComboBox<&'static str>, nwg::TextInput, nwg::CheckBox);

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
    if let Some(games) = r.waiting_for() {
        return format!("●  Esperando a que se abra: {games}\r\nLa medición empieza sola al abrir el juego y termina al cerrarlo (un informe por partida).");
    }
    let head = "●  Midiendo";
    let Some(c) = last_csv_line(&r.csv) else {
        return format!("{head}\r\nIniciando sensores...");
    };
    let g = |i: usize| c.get(i).map(String::as_str).unwrap_or("");
    let n = |i: usize, unit: &str| g(i).replace(',', ".").parse::<f32>().map(|v| format!("{v:.0}{unit}")).unwrap_or_else(|_| "--".into());
    // "34 %" o "34 % · 72 °C" según haya temperatura
    let pair = |load: usize, temp: usize| {
        let t = n(temp, " °C");
        if t == "--" { n(load, " %") } else { format!("{} · {t}", n(load, " %")) }
    };
    let secs = g(1).parse::<u64>().unwrap_or(0);
    let top = if g(17).is_empty() { "-".to_string() } else { format!("{} ({})", g(17), n(18, " %")) };
    let fg = if g(16).is_empty() { "-" } else { g(16) };
    let fps = if g(23).is_empty() { String::new() } else { format!("  ·  {} FPS (1 % low {})", n(23, ""), n(24, "")) };
    format!(
        "{head}  ·  {:02}:{:02}:{:02}{fps}\r\nCPU {}      GPU {}      RAM {}\r\nMás CPU: {top}\r\nVentana activa: {fg}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        pair(2, 5),
        pair(8, 9),
        n(3, " %"),
    )
}
