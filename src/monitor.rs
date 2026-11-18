//! Bucle de muestreo: procesos, CPU, RAM, disco y sensores.

use crate::Config;
use crate::sensors::Sensors;
use chrono::{DateTime, Local};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Uso de CPU (en % del total del equipo) a partir del cual un proceso en
/// segundo plano se registra como "pico".
const SPIKE_CPU: f32 = 15.0;
const AUTOSAVE: Duration = Duration::from_secs(300);
const MAX_SPIKES: usize = 1000;

#[derive(Serialize, Clone)]
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
    pub gthr: bool,
    pub ot: BTreeMap<String, f32>,
    /// Top 5 procesos: (nombre, %CPU, MB RAM)
    pub top: Vec<(String, f32, f32)>,
    pub fg: Option<String>,
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
    pub own_cpu_avg: f32,
    pub own_mem_mb: f32,
    pub csv_path: PathBuf,
}

fn control_dir() -> PathBuf {
    std::env::temp_dir()
}
fn stop_file() -> PathBuf {
    control_dir().join("medidor-recursos.stop")
}
fn pid_file() -> PathBuf {
    control_dir().join("medidor-recursos.pid")
}

/// Monitor en marcha según el archivo de control.
pub struct Running {
    pub pid: u32,
    pub report: PathBuf,
    pub csv: PathBuf,
}

/// Devuelve el monitor en marcha, si lo hay.
pub fn running_monitor() -> Option<Running> {
    let content = fs::read_to_string(pid_file()).ok()?;
    let mut lines = content.lines();
    let pid: u32 = lines.next()?.trim().parse().ok()?;
    let report = PathBuf::from(lines.next().unwrap_or(""));
    let csv = PathBuf::from(lines.next().unwrap_or(""));
    let mut sys = System::new().ok()?;
    let p = Pid::from_u32(pid);
    sys.refresh_processes(ProcessesToUpdate::Some(&[p]), true);
    let proc_ = sys.process(p)?;
    proc_.name().to_string_lossy().to_lowercase().contains("medidor").then_some(Running { pid, report, csv })
}

pub fn print_status() -> i32 {
    match running_monitor() {
        Some(r) => {
            println!("Monitor en marcha (PID {}).\nInforme: {}", r.pid, r.report.display());
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

/// Número con coma decimal para que Excel en español lo lea bien.
fn num(v: Option<f32>) -> String {
    v.map(|x| format!("{x:.1}").replace('.', ",")).unwrap_or_default()
}

fn fmt_elapsed(secs: f64) -> String {
    let s = secs as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

pub fn run(cfg: Config) {
    lower_priority();

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst));
    }
    let _ = fs::remove_file(stop_file());

    let start = Local::now();
    let stamp = start.format("%Y-%m-%d_%H-%M-%S").to_string();
    let report_path = cfg.out_dir.join(format!("informe_{stamp}.html"));
    let csv_path = cfg.out_dir.join(format!("datos_{stamp}.csv"));
    let report_abs = std::path::absolute(&report_path).unwrap_or(report_path.clone());
    let csv_abs = std::path::absolute(&csv_path).unwrap_or(csv_path.clone());
    let _ = fs::write(pid_file(), format!("{}\n{}\n{}\n", std::process::id(), report_abs.display(), csv_abs.display()));

    let say = |s: &str| {
        if !cfg.quiet {
            println!("{s}");
        }
    };
    say("Medidor de recursos — iniciando sensores...");

    let mut sensors = Sensors::new();
    let mut sys = System::new().expect("no se pudo leer la información del sistema");
    let kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_disk_usage();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);

    let ncpu = sys.cpus().len().max(1);
    let cpu_name = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
    let total_ram_gb = sys.total_memory() as f32 / 1_073_741_824.0;
    let own_pid = Pid::from_u32(std::process::id());

    let mut session = Session {
        start,
        end: start,
        interval: cfg.interval.as_secs_f64(),
        cpu_name,
        gpu_name: sensors.gpu_name.clone(),
        ncpu,
        total_ram_gb,
        sources: sensors.sources(),
        has_hwmon: sensors.has_hwmon(),
        cpu_limit: cfg.cpu_limit,
        gpu_limit: cfg.gpu_limit,
        samples: Vec::new(),
        procs: HashMap::new(),
        spikes: Vec::new(),
        own_cpu_avg: 0.0,
        own_mem_mb: 0.0,
        csv_path: csv_path.clone(),
    };

    let mut csv = File::create(&csv_path).ok().map(BufWriter::new);
    if let Some(w) = csv.as_mut() {
        let _ = writeln!(
            w,
            "hora;segundos;cpu_%;ram_%;ram_gb;cpu_temp;cpu_mhz;cpu_w;gpu_%;gpu_temp;gpu_hotspot;gpu_mhz;gpu_w;gpu_mem_mb;disco_lect_mb_s;disco_escr_mb_s;primer_plano;top1;top1_cpu;top2;top2_cpu;top3;top3_cpu"
        );
    }

    say(&format!("Fuentes de temperatura: {}", session.sources.join(", ")));
    if !session.has_hwmon {
        say("Aviso: LibreHardwareMonitor no está abierto. Ábrelo para tener temperaturas completas (ver MANUAL).");
    }
    match cfg.duration {
        Some(d) => say(&format!("Monitorizando durante {}. Ctrl+C para terminar antes.", fmt_elapsed(d.as_secs_f64()))),
        None => say("Monitorizando. Pulsa Ctrl+C para terminar y generar el informe."),
    }
    say(&format!("Informe: {}", report_abs.display()));

    let t_start = Instant::now();
    let mut last_refresh = Instant::now();
    let mut next_tick = Instant::now() + cfg.interval;
    let mut last_save = Instant::now();
    let mut open_spikes: HashMap<String, usize> = HashMap::new();
    let mut own_cpu_sum = 0.0f64;

    let should_stop = |stop: &AtomicBool| {
        stop.load(Ordering::SeqCst)
            || stop_file().exists()
            || cfg.duration.is_some_and(|d| t_start.elapsed() >= d)
    };

    'outer: loop {
        // Espera troceada para responder rápido a Ctrl+C / detener.
        while Instant::now() < next_tick {
            if should_stop(&stop) {
                break 'outer;
            }
            let left = next_tick.saturating_duration_since(Instant::now());
            std::thread::sleep(left.min(Duration::from_millis(200)));
        }
        next_tick += cfg.interval;
        if next_tick < Instant::now() {
            next_tick = Instant::now() + cfg.interval;
        }

        let dt = last_refresh.elapsed().as_secs_f64().max(0.1);
        last_refresh = Instant::now();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);

        let fg_name = foreground_pid().and_then(|p| sys.process(p)).map(|p| p.name().to_string_lossy().into_owned());

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
            let cpu = p.cpu_usage() / ncpu as f32;
            let du = p.disk_usage();
            if *pid == own_pid {
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

        let mut ranked: Vec<(&String, &Agg)> = by_name.iter().collect();
        ranked.sort_by(|a, b| b.1.cpu.total_cmp(&a.1.cpu));
        let top: Vec<(String, f32, f32)> = ranked
            .iter()
            .take(5)
            .map(|(n, a)| (n.to_string(), (a.cpu * 10.0).round() / 10.0, a.mem.round()))
            .collect();

        let t = t_start.elapsed().as_secs_f64();
        let hora = Local::now().format("%H:%M:%S").to_string();
        let (mut disk_r, mut disk_w) = (0.0, 0.0);

        for (i, (name, a)) in ranked.iter().enumerate() {
            disk_r += a.r;
            disk_w += a.w;
            let st = session.procs.entry(name.to_string()).or_default();
            let is_fg = fg_name.as_deref() == Some(name.as_str());
            st.samples += 1;
            st.sum_cpu += a.cpu as f64;
            st.max_cpu = st.max_cpu.max(a.cpu);
            st.sum_mem += a.mem as f64;
            st.max_mem = st.max_mem.max(a.mem);
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
                session.spikes.push(Spike {
                    name: name.to_string(),
                    start: t,
                    end: t,
                    peak: a.cpu,
                    hora: hora.clone(),
                    fg: fg_name.clone(),
                });
                still_open.insert(name.to_string(), session.spikes.len() - 1);
            }
        }
        open_spikes = still_open;

        let r = sensors.read();
        let used = sys.used_memory() as f32;
        let total = sys.total_memory().max(1) as f32;
        let sample = Sample {
            t: (t * 10.0).round() / 10.0,
            h: hora,
            cpu: sys.global_cpu_usage(),
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
            gthr: r.gpu_throttle,
            ot: r.others,
            top,
            fg: fg_name,
        };

        if let Some(w) = csv.as_mut() {
            let s = &sample;
            let tp = |i: usize| s.top.get(i).map(|(n, c, _)| format!("{n};{}", num(Some(*c)))).unwrap_or(";".into());
            let _ = writeln!(
                w,
                "{};{:.0};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{}",
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
                tp(2)
            );
            let _ = w.flush();
        }

        if !cfg.quiet {
            let f = |v: Option<f32>, u: &str| v.map(|x| format!("{x:.0}{u}")).unwrap_or("--".into());
            let line = format!(
                "\r[{}] CPU {:>3.0}% {:>5} | GPU {:>4} {:>5} | RAM {:>3.0}% | {} {:.0}%",
                fmt_elapsed(t),
                sample.cpu,
                f(sample.ct, "°C"),
                f(sample.gl, "%"),
                f(sample.gt, "°C"),
                sample.ram,
                sample.top.first().map(|x| x.0.as_str()).unwrap_or("-"),
                sample.top.first().map(|x| x.1).unwrap_or(0.0),
            );
            print!("{line:<100}");
            let _ = std::io::stdout().flush();
        }

        session.samples.push(sample);

        if last_save.elapsed() >= AUTOSAVE {
            last_save = Instant::now();
            finish_stats(&mut session, own_cpu_sum);
            let _ = crate::report::write(&session, &report_path, false);
        }
    }

    if !cfg.quiet {
        println!("\nGenerando informe...");
    }
    finish_stats(&mut session, own_cpu_sum);
    if let Some(mut w) = csv {
        let _ = w.flush();
    }
    let result = crate::report::write(&session, &report_path, true);
    let _ = fs::remove_file(stop_file());
    let _ = fs::remove_file(pid_file());
    match result {
        Ok(()) => {
            if !cfg.quiet {
                println!("Informe guardado en: {}", report_abs.display());
                println!("Datos CSV en:        {}", std::path::absolute(&csv_path).unwrap_or(csv_path).display());
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

/// Abre un archivo o carpeta con el programa predeterminado de Windows.
pub fn open_path(path: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
