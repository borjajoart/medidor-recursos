//! Bucle de muestreo: procesos, CPU, RAM, disco, red, sensores, FPS y marcas.

use crate::Config;
use crate::fps::FpsTracker;
use crate::gpu_proc::GpuPerProcess;
use crate::net::Pinger;
use crate::sensors::Sensors;
use chrono::{DateTime, Local};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Uso de CPU (en % del total del equipo) a partir del cual un proceso en
/// segundo plano se registra como "pico".
const SPIKE_CPU: f32 = 15.0;
const AUTOSAVE: Duration = Duration::from_secs(300);
const MAX_SPIKES: usize = 1000;
/// En modo espera: segundos sin el juego abierto para dar la partida por terminada.
const GAME_GONE_SECS: u64 = 60;
const ALERT_COOLDOWN: Duration = Duration::from_secs(600);

#[derive(Serialize, Clone, Default)]
pub struct Sample {
    pub t: f64,
    pub h: String,
    pub cpu: f32,
    pub ram: f32,
    pub ram_gb: f32,
    pub dr: f32,
    pub dw: f32,
    pub ct: Option<f32>,
    pub cc: Option<f32>,
    pub cw: Option<f32>,
    pub gt: Option<f32>,
    pub gh: Option<f32>,
    pub gl: Option<f32>,
    pub gw: Option<f32>,
    pub gc: Option<f32>,
    pub gm: Option<f32>,
    /// VRAM usada (%)
    pub gmp: Option<f32>,
    pub gthr: bool,
    pub ot: BTreeMap<String, f32>,
    /// Ventiladores (RPM)
    pub fan: BTreeMap<String, f32>,
    /// Top 5 procesos: (nombre, %CPU, MB RAM)
    pub top: Vec<(String, f32, f32)>,
    /// Top 3 procesos por uso de GPU: (nombre, %GPU)
    pub gtop: Vec<(String, f32)>,
    pub fg: Option<String>,
    /// FPS, "1 % low", tiempo de fotograma máximo (ms), tirones y proceso medido
    pub fps: Option<f32>,
    pub fl: Option<f32>,
    pub ftm: Option<f32>,
    pub stut: u32,
    pub fpp: Option<String>,
    /// Red (KB/s) y ping (ms)
    pub nrx: f32,
    pub ntx: f32,
    pub ping: Option<f32>,
    pub plost: u32,
}

#[derive(Default, Clone)]
pub struct ProcStats {
    pub samples: u32,
    pub sum_cpu: f64,
    pub max_cpu: f32,
    pub sum_mem: f64,
    pub max_mem: f32,
    pub fg_samples: u32,
    /// CPU acumulada mientras el proceso NO estaba en primer plano
    pub sum_cpu_bg: f64,
    pub max_cpu_bg: f32,
    pub sum_gpu: f64,
    pub max_gpu: f32,
    pub disk_read_mb: f64,
    pub disk_write_mb: f64,
    pub top1: u32,
    pub instances_max: u32,
}

#[derive(Clone)]
pub struct Spike {
    pub name: String,
    pub start: f64,
    pub end: f64,
    pub peak: f32,
    pub hora: String,
    pub fg: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mark {
    pub t: f64,
    pub h: String,
    pub label: String,
}

pub struct Session {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub interval: f64,
    pub cpu_name: String,
    pub gpu_name: Option<String>,
    pub ncpu: usize,
    pub total_ram_gb: f32,
    pub sources: Vec<String>,
    pub has_hwmon: bool,
    pub cpu_limit: f32,
    pub gpu_limit: f32,
    pub samples: Vec<Sample>,
    pub procs: HashMap<String, ProcStats>,
    pub spikes: Vec<Spike>,
    pub marks: Vec<Mark>,
    /// None = se midieron FPS; Some(motivo) si no fue posible
    pub fps_unavailable: Option<String>,
    pub ping_host: Option<String>,
    /// Juego que disparó la sesión (modo espera)
    pub game: Option<String>,
    pub own_cpu_avg: f32,
    pub own_mem_mb: f32,
    pub csv_path: PathBuf,
}

/// Estado compartido con quien controla el monitor (bandeja del sistema).
#[derive(Default)]
pub struct Control {
    pub stop: AtomicBool,
    pub finished: AtomicBool,
    pub live: Mutex<Live>,
    /// Avisos pendientes de mostrar (título, texto)
    pub alerts: Mutex<Vec<(String, String)>>,
}

#[derive(Default, Clone)]
pub struct Live {
    pub waiting: bool,
    pub game: Option<String>,
    pub elapsed: f64,
    pub cpu: f32,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub fps: Option<f32>,
    pub marks: usize,
    pub cpu_limit: f32,
    pub gpu_limit: f32,
}

// ---------------------------------------------------------------------------
// Archivos de control (en %TEMP%): pid/estado, parada y marcas
// ---------------------------------------------------------------------------

fn control_dir() -> PathBuf {
    std::env::temp_dir()
}
fn stop_file() -> PathBuf {
    control_dir().join("medidor-recursos.stop")
}
fn mark_file() -> PathBuf {
    control_dir().join("medidor-recursos.mark")
}
fn pid_file() -> PathBuf {
    control_dir().join("medidor-recursos.pid")
}

fn write_pid_file(report: &Path, csv: &Path, state: &str) {
    let _ = fs::write(
        pid_file(),
        format!("{}\n{}\n{}\n{}\n", std::process::id(), report.display(), csv.display(), state),
    );
}

/// Monitor en marcha según el archivo de control.
pub struct Running {
    pub pid: u32,
    pub report: PathBuf,
    pub csv: PathBuf,
    /// "midiendo" o "esperando:<juegos>"
    pub state: String,
}

impl Running {
    pub fn waiting_for(&self) -> Option<&str> {
        self.state.strip_prefix("esperando:")
    }
}

/// Devuelve el monitor en marcha, si lo hay.
pub fn running_monitor() -> Option<Running> {
    let content = fs::read_to_string(pid_file()).ok()?;
    let mut lines = content.lines();
    let pid: u32 = lines.next()?.trim().parse().ok()?;
    let report = PathBuf::from(lines.next().unwrap_or(""));
    let csv = PathBuf::from(lines.next().unwrap_or(""));
    let state = lines.next().unwrap_or("midiendo").to_string();
    let mut sys = System::new().ok()?;
    let p = Pid::from_u32(pid);
    sys.refresh_processes(ProcessesToUpdate::Some(&[p]), true);
    let proc_ = sys.process(p)?;
    proc_
        .name()
        .to_string_lossy()
        .to_lowercase()
        .contains("medidor")
        .then_some(Running { pid, report, csv, state })
}

pub fn print_status() -> i32 {
    match running_monitor() {
        Some(r) => {
            match r.waiting_for() {
                Some(g) => println!("Monitor en marcha (PID {}), esperando a: {g}", r.pid),
                None => println!("Monitor en marcha (PID {}).\nInforme: {}", r.pid, r.report.display()),
            }
            0
        }
        None => {
            println!("No hay ningún monitor en marcha.");
            1
        }
    }
}

/// Pide al monitor que se detenga y espera a que guarde el informe. Devuelve su ruta.
pub fn stop_and_wait(timeout: Duration) -> Result<PathBuf, String> {
    let Some(r) = running_monitor() else {
        let _ = fs::remove_file(pid_file());
        return Err("No hay ningún monitor en marcha.".into());
    };
    File::create(stop_file()).map_err(|e| format!("No se pudo enviar la orden de parada: {e}"))?;
    let t = Instant::now();
    while t.elapsed() < timeout {
        if running_monitor().is_none() {
            return Ok(r.report);
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(format!("El monitor no ha respondido en {} s.", timeout.as_secs()))
}

pub fn request_stop() -> i32 {
    if running_monitor().is_some() {
        println!("Deteniendo el monitor y generando el informe...");
    }
    match stop_and_wait(Duration::from_secs(60)) {
        Ok(report) => {
            println!("Listo. Informe: {}", report.display());
            0
        }
        Err(e) => {
            println!("{e}");
            1
        }
    }
}

/// Añade una marca en el monitor en marcha (equivale a pulsar Ctrl+Alt+M).
pub fn request_mark() -> Result<(), String> {
    if running_monitor().is_none() {
        return Err("No hay ningún monitor en marcha.".into());
    }
    File::create(mark_file()).map(|_| ()).map_err(|e| e.to_string())
}

/// Lanza este mismo ejecutable como monitor sin ventana (`--hijo`). Devuelve el PID.
pub fn spawn_child(args: &[String]) -> Result<u32, String> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    if let Some(r) = running_monitor() {
        return Err(format!("Ya hay un monitor en marcha (PID {}). Detenlo primero.", r.pid));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let rest: Vec<&String> = args.iter().filter(|a| *a != "-b" && *a != "--segundo-plano").collect();
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    std::process::Command::new(&exe)
        .args(rest)
        .arg("--hijo")
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|c| c.id())
        .map_err(|e| format!("No se pudo iniciar el monitor: {e}"))
}

/// Como `spawn_child`, pero pidiendo permisos de administrador (necesarios para los FPS).
/// Windows muestra el aviso de Control de cuentas de usuario.
pub fn spawn_child_elevated(args: &[String]) -> Result<u32, String> {
    use windows_sys::Win32::System::Threading::GetProcessId;
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    if let Some(r) = running_monitor() {
        return Err(format!("Ya hay un monitor en marcha (PID {}). Detenlo primero.", r.pid));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let quote = |a: &str| if a.contains(' ') || a.is_empty() { format!("\"{a}\"") } else { a.to_string() };
    let params: String = args
        .iter()
        .filter(|a| *a != "-b" && *a != "--segundo-plano")
        .map(|a| quote(a))
        .chain(Some("--hijo".to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    let w = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (verb, file, par) = (w("runas"), w(&exe.to_string_lossy()), w(&params));
    let dir = w(&exe.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default());
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = par.as_ptr();
    info.lpDirectory = dir.as_ptr();
    info.nShow = SW_HIDE;
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        return Err("No se concedieron permisos de administrador.".into());
    }
    let pid = unsafe { GetProcessId(info.hProcess) };
    unsafe { windows_sys::Win32::Foundation::CloseHandle(info.hProcess) };
    Ok(pid)
}

pub fn is_elevated() -> bool {
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut el = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut el as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        windows_sys::Win32::Foundation::CloseHandle(token);
        ok != 0 && el.TokenIsElevated != 0
    }
}

/// Abre un archivo, carpeta o URL con el programa predeterminado.
/// Se hace a través de explorer.exe para que, aunque el monitor sea administrador,
/// el navegador se abra como usuario normal.
pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

fn lower_priority() {
    use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass};
    unsafe {
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}

fn foreground_pid() -> Option<Pid> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        (pid != 0).then(|| Pid::from_u32(pid))
    }
}

/// Tecla rápida global Ctrl+Alt+M para marcar momentos. Va ligada al hilo que la registra.
fn register_hotkey() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey};
    unsafe { RegisterHotKey(std::ptr::null_mut(), 1, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 0x4D) != 0 }
}

/// Cuántas marcas nuevas hay (tecla rápida o archivo de marca).
fn poll_marks() -> usize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY};
    let mut n = 0;
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), WM_HOTKEY, WM_HOTKEY, PM_REMOVE) != 0 {
            n += 1;
        }
    }
    if fs::remove_file(mark_file()).is_ok() {
        n += 1;
    }
    n
}

fn beep() {
    unsafe { windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(0) };
}

/// Número con coma decimal para que Excel en español lo lea bien.
fn num(v: Option<f32>) -> String {
    v.map(|x| format!("{x:.1}").replace('.', ",")).unwrap_or_default()
}

fn fmt_elapsed(secs: f64) -> String {
    let s = secs as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

pub const CSV_HEADER: &str = "hora;segundos;cpu_%;ram_%;ram_gb;cpu_temp;cpu_mhz;cpu_w;gpu_%;gpu_temp;gpu_hotspot;gpu_mhz;gpu_w;gpu_mem_mb;disco_lect_mb_s;disco_escr_mb_s;primer_plano;top1;top1_cpu;top2;top2_cpu;top3;top3_cpu;fps;fps_1pct_low;frametime_max_ms;tirones;fps_proceso;gpu_top1;gpu_top1_pct;red_rx_kb_s;red_tx_kb_s;ping_ms;ping_perdidos;vram_pct;vent_min_rpm;vent_max_rpm;marca";

// ---------------------------------------------------------------------------
// Monitorización
// ---------------------------------------------------------------------------

/// Recursos que se mantienen entre sesiones (modo espera).
struct Ctx {
    sensors: Sensors,
    sys: System,
    networks: Option<Networks>,
    gpu: Option<GpuPerProcess>,
    kind: ProcessRefreshKind,
    ncpu: usize,
    cpu_name: String,
    total_ram_gb: f32,
    own_pid: Pid,
    alerts: HashMap<&'static str, Instant>,
}

impl Ctx {
    fn alert(&mut self, control: &Control, quiet: bool, kind: &'static str, title: &str, text: String) {
        if self.alerts.get(kind).is_some_and(|t| t.elapsed() < ALERT_COOLDOWN) {
            return;
        }
        self.alerts.insert(kind, Instant::now());
        if !quiet {
            println!("\n[aviso] {title}: {text}");
        }
        control.alerts.lock().unwrap().push((title.to_string(), text));
    }
}

/// Ejecuta el monitor hasta que se pida parar (Ctrl+C, `detener`, bandeja o duración).
pub fn run(cfg: Config, control: Arc<Control>) {
    lower_priority();
    {
        let c = control.clone();
        let _ = ctrlc::set_handler(move || c.stop.store(true, Ordering::SeqCst));
    }
    let _ = fs::remove_file(stop_file());
    let _ = fs::remove_file(mark_file());
    let watch_state = format!("esperando:{}", cfg.watch.join(", "));
    write_pid_file(Path::new(""), Path::new(""), if cfg.watch.is_empty() { "midiendo" } else { &watch_state });

    if !cfg.quiet {
        println!("Medidor de recursos: iniciando sensores...");
    }
    let hotkey = register_hotkey();
    let mut sys = System::new().expect("no se pudo leer la información del sistema");
    let kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_disk_usage();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    let mut ctx = Ctx {
        sensors: Sensors::new(),
        ncpu: sys.cpus().len().max(1),
        cpu_name: sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        total_ram_gb: sys.total_memory() as f32 / 1_073_741_824.0,
        sys,
        networks: Networks::new_with_refreshed_list().ok(),
        gpu: GpuPerProcess::new(),
        kind,
        own_pid: Pid::from_u32(std::process::id()),
        alerts: HashMap::new(),
    };
    if !cfg.quiet {
        println!("Fuentes de temperatura: {}", ctx.sensors.sources().join(", "));
        if !ctx.sensors.has_hwmon() {
            println!("Aviso: LibreHardwareMonitor no está abierto. Ábrelo para tener temperaturas completas (ver MANUAL).");
        }
        if hotkey {
            println!("Pulsa Ctrl+Alt+M en cualquier momento (también jugando) para marcar un momento.");
        }
    }

    let stopped = |c: &Control| c.stop.load(Ordering::SeqCst) || stop_file().exists();

    if cfg.watch.is_empty() {
        run_session(&cfg, &control, &mut ctx, None);
    } else {
        loop {
            {
                let mut l = control.live.lock().unwrap();
                l.waiting = true;
                l.game = None;
            }
            write_pid_file(Path::new(""), Path::new(""), &watch_state);
            if !cfg.quiet {
                println!("Esperando a que se abra: {}", cfg.watch.join(", "));
            }
            let Some(game) = wait_for_game(&cfg, &control, &mut ctx.sys) else { break };
            if !cfg.quiet {
                println!("Detectado {game}: empieza la medición.");
            }
            run_session(&cfg, &control, &mut ctx, Some(game));
            if stopped(&control) {
                break;
            }
        }
    }

    let _ = fs::remove_file(stop_file());
    let _ = fs::remove_file(pid_file());
    control.finished.store(true, Ordering::SeqCst);
}

/// Espera (comprobando cada 3 s) a que se abra uno de los juegos. None si se pide parar.
fn wait_for_game(cfg: &Config, control: &Control, sys: &mut System) -> Option<String> {
    loop {
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
        if let Some(name) = sys
            .processes()
            .values()
            .map(|p| p.name().to_string_lossy().into_owned())
            .find(|n| cfg.watch.contains(&n.to_lowercase()))
        {
            return Some(name);
        }
        for _ in 0..15 {
            if control.stop.load(Ordering::SeqCst) || stop_file().exists() {
                return None;
            }
            poll_marks(); // descarta marcas mientras no se mide
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

fn run_session(cfg: &Config, control: &Control, ctx: &mut Ctx, game: Option<String>) {
    let start = Local::now();
    let stamp = start.format("%Y-%m-%d_%H-%M-%S").to_string();
    let _ = fs::create_dir_all(&cfg.out_dir);
    let report_path = cfg.out_dir.join(format!("informe_{stamp}.html"));
    let csv_path = cfg.out_dir.join(format!("datos_{stamp}.csv"));
    let report_abs = std::path::absolute(&report_path).unwrap_or(report_path.clone());
    let csv_abs = std::path::absolute(&csv_path).unwrap_or(csv_path.clone());
    write_pid_file(&report_abs, &csv_abs, "midiendo");

    let mut fps_tracker = None;
    let fps_unavailable = if !cfg.fps {
        Some("desactivado con --sin-fps".to_string())
    } else if !is_elevated() {
        Some("hace falta ejecutar como administrador".to_string())
    } else {
        match FpsTracker::start() {
            Ok(t) => {
                fps_tracker = Some(t);
                None
            }
            Err(e) => Some(format!("no se pudo iniciar la captura ETW ({e})")),
        }
    };
    let pinger = Pinger::start(&cfg.ping, cfg.interval.max(Duration::from_secs(1)));

    let mut session = Session {
        start,
        end: start,
        interval: cfg.interval.as_secs_f64(),
        cpu_name: ctx.cpu_name.clone(),
        gpu_name: ctx.sensors.gpu_name.clone(),
        ncpu: ctx.ncpu,
        total_ram_gb: ctx.total_ram_gb,
        sources: ctx.sensors.sources(),
        has_hwmon: ctx.sensors.has_hwmon(),
        cpu_limit: cfg.cpu_limit,
        gpu_limit: cfg.gpu_limit,
        samples: Vec::new(),
        procs: HashMap::new(),
        spikes: Vec::new(),
        marks: Vec::new(),
        fps_unavailable,
        ping_host: pinger.as_ref().map(|p| p.host.clone()),
        game: game.clone(),
        own_cpu_avg: 0.0,
        own_mem_mb: 0.0,
        csv_path: csv_path.clone(),
    };

    let mut csv = File::create(&csv_path).ok().map(BufWriter::new);
    if let Some(w) = csv.as_mut() {
        let _ = writeln!(w, "{CSV_HEADER}");
    }

    if !cfg.quiet {
        if let Some(why) = &session.fps_unavailable {
            println!("FPS: no se miden ({why}).");
        }
        match cfg.duration {
            Some(d) => println!("Monitorizando durante {}. Ctrl+C para terminar antes.", fmt_elapsed(d.as_secs_f64())),
            None => println!("Monitorizando. Pulsa Ctrl+C para terminar y generar el informe."),
        }
        println!("Informe: {}", report_abs.display());
    }
    {
        let mut l = control.live.lock().unwrap();
        *l = Live { game: game.clone(), cpu_limit: cfg.cpu_limit, gpu_limit: cfg.gpu_limit, ..Default::default() };
    }

    let t_start = Instant::now();
    let mut last_refresh = Instant::now();
    let mut next_tick = Instant::now() + cfg.interval;
    let mut last_save = Instant::now();
    let mut open_spikes: HashMap<String, usize> = HashMap::new();
    let mut own_cpu_sum = 0.0f64;
    let mut game_absent_since: Option<Instant> = None;
    let mut pending_mark: Option<String> = None;
    if let Some(n) = ctx.networks.as_mut() {
            n.refresh(true);
        }

    let should_stop = |c: &Control| {
        c.stop.load(Ordering::SeqCst) || stop_file().exists() || cfg.duration.is_some_and(|d| t_start.elapsed() >= d)
    };

    'outer: loop {
        // Espera troceada para responder rápido a Ctrl+C / detener / marcas.
        while Instant::now() < next_tick {
            if should_stop(control) {
                break 'outer;
            }
            let n = poll_marks();
            for _ in 0..n {
                let label = format!("M{}", session.marks.len() + 1);
                let t = t_start.elapsed().as_secs_f64();
                session.marks.push(Mark { t: (t * 10.0).round() / 10.0, h: Local::now().format("%H:%M:%S").to_string(), label: label.clone() });
                pending_mark = Some(label.clone());
                beep();
                if !cfg.quiet {
                    println!("\nMarca {label} a las {}", Local::now().format("%H:%M:%S"));
                }
            }
            let left = next_tick.saturating_duration_since(Instant::now());
            std::thread::sleep(left.min(Duration::from_millis(100)));
        }
        next_tick += cfg.interval;
        if next_tick < Instant::now() {
            next_tick = Instant::now() + cfg.interval;
        }

        let dt = last_refresh.elapsed().as_secs_f64().max(0.1);
        last_refresh = Instant::now();
        ctx.sys.refresh_cpu_usage();
        ctx.sys.refresh_memory();
        ctx.sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ctx.kind);
        if let Some(n) = ctx.networks.as_mut() {
            n.refresh(true);
        }
        let sys = &ctx.sys;

        let fg_pid = foreground_pid();
        let fg_name = fg_pid.and_then(|p| sys.process(p)).map(|p| p.name().to_string_lossy().into_owned());
        let pid_name = |pid: u32| sys.process(Pid::from_u32(pid)).map(|p| p.name().to_string_lossy().into_owned());

        // GPU por proceso, agrupada por nombre
        let mut gpu_by_name: HashMap<String, f32> = HashMap::new();
        if let Some(g) = ctx.gpu.as_mut() {
            for (pid, v) in g.read() {
                if let Some(n) = pid_name(pid) {
                    let e = gpu_by_name.entry(n).or_default();
                    *e = (*e + v).min(100.0);
                }
            }
        }
        let mut gtop: Vec<(String, f32)> = gpu_by_name.iter().map(|(k, v)| (k.clone(), (v * 10.0).round() / 10.0)).collect();
        gtop.sort_by(|a, b| b.1.total_cmp(&a.1));
        gtop.retain(|x| x.1 >= 0.5);
        gtop.truncate(3);

        // FPS: la ventana activa si presenta fotogramas; si no, el proceso con más FPS.
        let mut fps_s = None;
        if let Some(tr) = fps_tracker.as_mut() {
            let all = tr.collect(dt);
            let candidates: Vec<_> = all
                .into_iter()
                .filter(|s| pid_name(s.pid).is_some_and(|n| !matches!(n.to_lowercase().as_str(), "dwm.exe" | "explorer.exe")))
                .collect();
            fps_s = candidates
                .iter()
                .find(|s| Some(Pid::from_u32(s.pid)) == fg_pid)
                .or_else(|| candidates.iter().max_by(|a, b| a.fps.total_cmp(&b.fps)))
                .cloned();
        }

        // Agrupa por nombre de ejecutable (p.ej. todas las pestañas de chrome.exe juntas).
        struct Agg {
            cpu: f32,
            mem: f32,
            r: f64,
            w: f64,
            n: u32,
        }
        let mut by_name: HashMap<String, Agg> = HashMap::new();
        for (pid, p) in sys.processes() {
            if pid.as_u32() == 0 {
                continue;
            }
            let name = p.name().to_string_lossy();
            if name == "System Idle Process" || name == "Idle" {
                continue;
            }
            let cpu = p.cpu_usage() / ctx.ncpu as f32;
            let du = p.disk_usage();
            if *pid == ctx.own_pid {
                own_cpu_sum += cpu as f64;
                session.own_mem_mb = p.memory() as f32 / 1_048_576.0;
            }
            let e = by_name.entry(name.into_owned()).or_insert(Agg { cpu: 0.0, mem: 0.0, r: 0.0, w: 0.0, n: 0 });
            e.cpu += cpu;
            e.mem += p.memory() as f32 / 1_048_576.0;
            e.r += du.read_bytes as f64 / 1_048_576.0;
            e.w += du.written_bytes as f64 / 1_048_576.0;
            e.n += 1;
        }

        // Modo espera: ¿sigue abierto el juego?
        if let Some(g) = &game {
            let gl = g.to_lowercase();
            if by_name.keys().any(|k| k.to_lowercase() == gl) {
                game_absent_since = None;
            } else if game_absent_since.get_or_insert_with(Instant::now).elapsed() >= Duration::from_secs(GAME_GONE_SECS) {
                break 'outer;
            }
        }

        let mut ranked: Vec<(&String, &Agg)> = by_name.iter().collect();
        ranked.sort_by(|a, b| b.1.cpu.total_cmp(&a.1.cpu));
        let top: Vec<(String, f32, f32)> =
            ranked.iter().take(5).map(|(n, a)| (n.to_string(), (a.cpu * 10.0).round() / 10.0, a.mem.round())).collect();

        let t = t_start.elapsed().as_secs_f64();
        let hora = Local::now().format("%H:%M:%S").to_string();
        let (mut disk_r, mut disk_w) = (0.0, 0.0);

        for (i, (name, a)) in ranked.iter().enumerate() {
            disk_r += a.r;
            disk_w += a.w;
            let st = session.procs.entry(name.to_string()).or_default();
            let is_fg = fg_name.as_deref() == Some(name.as_str());
            let g = gpu_by_name.get(name.as_str()).copied().unwrap_or(0.0);
            st.samples += 1;
            st.sum_cpu += a.cpu as f64;
            st.max_cpu = st.max_cpu.max(a.cpu);
            st.sum_mem += a.mem as f64;
            st.max_mem = st.max_mem.max(a.mem);
            st.sum_gpu += g as f64;
            st.max_gpu = st.max_gpu.max(g);
            st.disk_read_mb += a.r;
            st.disk_write_mb += a.w;
            st.instances_max = st.instances_max.max(a.n);
            if i == 0 {
                st.top1 += 1;
            }
            if is_fg {
                st.fg_samples += 1;
            } else {
                st.sum_cpu_bg += a.cpu as f64;
                st.max_cpu_bg = st.max_cpu_bg.max(a.cpu);
            }
        }

        // Picos de procesos en segundo plano.
        let mut still_open = HashMap::new();
        let mut new_spikes = Vec::new();
        for (name, a) in &ranked {
            if a.cpu < SPIKE_CPU || fg_name.as_deref() == Some(name.as_str()) {
                continue;
            }
            if let Some(&idx) = open_spikes.get(*name) {
                let s = &mut session.spikes[idx];
                s.end = t;
                s.peak = s.peak.max(a.cpu);
                still_open.insert(name.to_string(), idx);
            } else if session.spikes.len() < MAX_SPIKES {
                session.spikes.push(Spike { name: name.to_string(), start: t, end: t, peak: a.cpu, hora: hora.clone(), fg: fg_name.clone() });
                still_open.insert(name.to_string(), session.spikes.len() - 1);
                new_spikes.push((name.to_string(), a.cpu));
            }
        }
        open_spikes = still_open;

        let (rx, tx) = ctx.networks.as_ref().map_or((0, 0), |nets| nets.iter().fold((0u64, 0u64), |(r, w), (_, n)| (r + n.received(), w + n.transmitted())));
        let ping = pinger.as_ref().map(|p| p.take());

        let r = ctx.sensors.read();
        let used = ctx.sys.used_memory() as f32;
        let total = ctx.sys.total_memory().max(1) as f32;
        let sample = Sample {
            t: (t * 10.0).round() / 10.0,
            h: hora,
            cpu: ctx.sys.global_cpu_usage(),
            ram: used / total * 100.0,
            ram_gb: used / 1_073_741_824.0,
            dr: (disk_r / dt) as f32,
            dw: (disk_w / dt) as f32,
            ct: r.cpu_temp,
            cc: r.cpu_clock,
            cw: r.cpu_power,
            gt: r.gpu_temp,
            gh: r.gpu_hot,
            gl: r.gpu_load,
            gw: r.gpu_power,
            gc: r.gpu_clock,
            gm: r.gpu_mem,
            gmp: r.gpu_mem.zip(r.gpu_mem_total).filter(|(_, t)| *t > 0.0).map(|(u, t)| u / t * 100.0),
            gthr: r.gpu_throttle,
            ot: r.others,
            fan: r.fans,
            top,
            gtop,
            fg: fg_name,
            fps: fps_s.as_ref().map(|s| (s.fps * 10.0).round() / 10.0),
            fl: fps_s.as_ref().map(|s| (s.low1 * 10.0).round() / 10.0),
            ftm: fps_s.as_ref().map(|s| (s.frametime_max * 10.0).round() / 10.0),
            stut: fps_s.as_ref().map_or(0, |s| s.stutters),
            fpp: fps_s.as_ref().and_then(|s| pid_name(s.pid)),
            nrx: (rx as f64 / 1024.0 / dt) as f32,
            ntx: (tx as f64 / 1024.0 / dt) as f32,
            ping: ping.as_ref().and_then(|p| p.ms),
            plost: ping.as_ref().map_or(0, |p| p.lost),
        };

        // Avisos (para la bandeja del sistema y la consola)
        if let Some(tc) = sample.ct.filter(|t| *t >= cfg.cpu_limit) {
            ctx.alert(control, cfg.quiet, "cpu", "CPU caliente", format!("{tc:.0} °C (límite {:.0} °C)", cfg.cpu_limit));
        }
        if let Some(tg) = sample.gt.filter(|t| *t >= cfg.gpu_limit) {
            ctx.alert(control, cfg.quiet, "gpu", "GPU caliente", format!("{tg:.0} °C (límite {:.0} °C)", cfg.gpu_limit));
        }
        if sample.ram >= 90.0 {
            ctx.alert(control, cfg.quiet, "ram", "RAM casi llena", format!("{:.0} % en uso", sample.ram));
        }
        if sample.gmp.is_some_and(|v| v >= 95.0) {
            ctx.alert(control, cfg.quiet, "vram", "Memoria de vídeo llena", format!("{:.0} % de la VRAM en uso", sample.gmp.unwrap()));
        }
        if let Some((n, c)) = new_spikes.iter().filter(|(_, c)| *c >= 25.0).max_by(|a, b| a.1.total_cmp(&b.1)) {
            ctx.alert(control, cfg.quiet, "spike", "Pico en segundo plano", format!("{n} usa {c:.0} % de la CPU"));
        }

        if let Some(w) = csv.as_mut() {
            let s = &sample;
            let tp = |i: usize| s.top.get(i).map(|(n, c, _)| format!("{n};{}", num(Some(*c)))).unwrap_or(";".into());
            let fans: Vec<f32> = s.fan.values().copied().filter(|v| *v > 0.0).collect();
            let fmin = fans.iter().copied().reduce(f32::min);
            let fmax = fans.iter().copied().reduce(f32::max);
            let _ = writeln!(
                w,
                "{};{:.0};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{}",
                s.h,
                s.t,
                num(Some(s.cpu)),
                num(Some(s.ram)),
                num(Some(s.ram_gb)),
                num(s.ct),
                num(s.cc),
                num(s.cw),
                num(s.gl),
                num(s.gt),
                num(s.gh),
                num(s.gc),
                num(s.gw),
                num(s.gm),
                num(Some(s.dr)),
                num(Some(s.dw)),
                s.fg.as_deref().unwrap_or(""),
                tp(0),
                tp(1),
                tp(2),
                num(s.fps),
                num(s.fl),
                num(s.ftm),
                if s.fps.is_some() { s.stut.to_string() } else { String::new() },
                s.fpp.as_deref().unwrap_or(""),
                s.gtop.first().map(|g| format!("{};{}", g.0, num(Some(g.1)))).unwrap_or(";".into()),
                num(Some(s.nrx)),
                num(Some(s.ntx)),
                num(s.ping),
                if ping.is_some() { s.plost.to_string() } else { String::new() },
                num(s.gmp),
                num(fmin),
                num(fmax),
                pending_mark.take().unwrap_or_default()
            );
            let _ = w.flush();
        }

        {
            let mut l = control.live.lock().unwrap();
            l.waiting = false;
            l.elapsed = t;
            l.cpu = sample.cpu;
            l.cpu_temp = sample.ct;
            l.gpu_temp = sample.gt;
            l.fps = sample.fps;
            l.marks = session.marks.len();
        }

        if !cfg.quiet {
            let f = |v: Option<f32>, u: &str| v.map(|x| format!("{x:.0}{u}")).unwrap_or("--".into());
            let line = format!(
                "\r[{}] CPU {:>3.0}% {:>5} | GPU {:>4} {:>5} | RAM {:>3.0}% | FPS {:>4} | {} {:.0}%",
                fmt_elapsed(t),
                sample.cpu,
                f(sample.ct, "°C"),
                f(sample.gl, "%"),
                f(sample.gt, "°C"),
                sample.ram,
                f(sample.fps, ""),
                sample.top.first().map(|x| x.0.as_str()).unwrap_or("-"),
                sample.top.first().map(|x| x.1).unwrap_or(0.0),
            );
            print!("{line:<110}");
            let _ = std::io::stdout().flush();
        }

        session.samples.push(sample);

        if last_save.elapsed() >= AUTOSAVE {
            last_save = Instant::now();
            finish_stats(&mut session, own_cpu_sum);
            let _ = crate::report::write(&session, &report_path);
        }
    }

    drop(fps_tracker);
    if !cfg.quiet {
        println!("\nGenerando informe...");
    }
    finish_stats(&mut session, own_cpu_sum);
    if let Some(mut w) = csv {
        let _ = w.flush();
    }
    // En modo espera, una "partida" de menos de 10 muestras (p.ej. un launcher) no merece
    // informe. Si la detiene el usuario, sí se genera: es lo que espera ver.
    let user_stop = control.stop.load(Ordering::SeqCst) || stop_file().exists();
    if game.is_some() && session.samples.len() < 10 && !user_stop {
        let _ = fs::remove_file(&csv_path);
        return;
    }
    match crate::report::write(&session, &report_path) {
        Ok(()) => {
            if !cfg.quiet {
                println!("Informe guardado en: {}", report_abs.display());
                println!("Datos CSV en:        {}", csv_abs.display());
            }
            if cfg.open_report {
                open_path(&report_abs);
            }
        }
        Err(e) => eprintln!("No se pudo guardar el informe: {e}"),
    }
}

fn finish_stats(s: &mut Session, own_cpu_sum: f64) {
    s.end = Local::now();
    s.own_cpu_avg = (own_cpu_sum / s.samples.len().max(1) as f64) as f32;
}
