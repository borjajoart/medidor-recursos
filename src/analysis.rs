//! Análisis de una sesión: estadísticas, episodios, caídas de FPS y sus causas,
//! throttling y conclusiones. Todo son funciones puras para poder probarlas.

use crate::monitor::{Mark, ProcStats, Sample, Session, Spike};
use std::collections::HashMap;

/// Procesos del sistema/escritorio que no cuentan como "aplicación principal".
const SHELL: &[&str] = &[
    "explorer.exe",
    "windowsterminal.exe",
    "conhost.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "medidor-recursos.exe",
    "medidor-gui.exe",
    "searchhost.exe",
    "searchapp.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "lockapp.exe",
    "applicationframehost.exe",
    "taskmgr.exe",
];

pub fn is_shell(name: &str) -> bool {
    SHELL.contains(&name.to_lowercase().as_str())
}

/// Nota breve sobre procesos conocidos que suelen interferir al jugar.
pub fn describe(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "msmpeng.exe" | "mpdefendercoreservice.exe" | "nissrv.exe" => {
            "Antivirus de Windows. Si coincide con tirones, programa el análisis a otra hora o excluye la carpeta del juego."
        }
        "tiworker.exe" | "trustedinstaller.exe" | "wuauclt.exe" | "mousocoreworker.exe" | "usocoreworker.exe" | "sihclient.exe"
        | "wuaueng.exe" => "Windows Update.",
        "searchindexer.exe" | "searchprotocolhost.exe" | "searchfilterhost.exe" => "Indexado de búsqueda de Windows.",
        "onedrive.exe" => "Sincronización de OneDrive.",
        "chrome.exe" | "msedge.exe" | "firefox.exe" | "opera.exe" | "brave.exe" | "msedgewebview2.exe" => "Navegador.",
        "discord.exe" => "Discord. El overlay y la aceleración por hardware tiran de GPU.",
        "steamwebhelper.exe" | "steam.exe" => "Steam (interfaz o descargas).",
        "epicgameslauncher.exe" | "epicwebhelper.exe" => "Launcher de Epic.",
        "battle.net.exe" | "agent.exe" => "Battle.net.",
        "nvidia share.exe" | "nvcontainer.exe" | "nvidia overlay.exe" | "nvsphelper64.exe" => "Overlay o grabación de NVIDIA.",
        "radeonsoftware.exe" | "amdrsserv.exe" | "amdow.exe" => "Software de AMD (overlay o grabación).",
        "dwm.exe" => "Compositor de Windows. Sube con juegos en ventana.",
        "system" => "Núcleo y controladores. Alto de forma sostenida suele ser un driver.",
        "memory compression" => "Compresión de memoria: señal de que falta RAM.",
        "svchost.exe" => "Servicios de Windows.",
        "audiodg.exe" => "Audio de Windows (mejoras de sonido).",
        "wmiprvse.exe" => "Consultas WMI (este monitor u otro software de hardware).",
        "librehardwaremonitor.exe" | "openhardwaremonitor.exe" => "Lector de sensores.",
        "medidor-recursos.exe" | "medidor-gui.exe" => "Este monitor.",
        "wallpaper64.exe" | "wallpaper32.exe" => "Wallpaper Engine.",
        "icue.exe" | "razer synapse 3.exe" | "lghub.exe" | "lghub_agent.exe" | "armourycrate.service.exe" => {
            "Software de periféricos o RGB."
        }
        "obs64.exe" => "OBS.",
        "spotify.exe" => "Spotify.",
        "teams.exe" | "ms-teams.exe" => "Teams.",
        _ => "",
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Stat {
    pub avg: f32,
    pub max: f32,
    pub min: f32,
    pub p95: f32,
    pub max_i: usize,
}

pub fn stat(sm: &[Sample], f: impl Fn(&Sample) -> Option<f32>) -> Option<Stat> {
    let vals: Vec<(usize, f32)> = sm.iter().enumerate().filter_map(|(i, s)| f(s).map(|v| (i, v))).collect();
    if vals.is_empty() {
        return None;
    }
    let mut sorted: Vec<f32> = vals.iter().map(|x| x.1).collect();
    sorted.sort_by(f32::total_cmp);
    let (max_i, max) = vals.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    Some(Stat {
        avg: sorted.iter().sum::<f32>() / sorted.len() as f32,
        max,
        min: sorted[0],
        p95: sorted[((sorted.len() - 1) as f32 * 0.95).round() as usize],
        max_i,
    })
}

pub fn percentile(v: &[f32], p: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    Some(s[((s.len() - 1) as f32 * p).round() as usize])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    FpsDrop,
    CpuHot,
    GpuHot,
    CpuBusy,
    RamFull,
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Kind::FpsDrop => "Caída de FPS",
            Kind::CpuHot => "CPU caliente",
            Kind::GpuHot => "GPU caliente",
            Kind::CpuBusy => "CPU > 90 %",
            Kind::RamFull => "RAM > 90 %",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Episode {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    /// Valor extremo (máximo de temperatura/uso, o mínimo de FPS)
    pub peak: f32,
    pub culprits: Vec<(String, f32)>,
    pub fg: Option<String>,
    pub causes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Grave,
    Aviso,
    Info,
    Ok,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub level: Level,
    pub title: String,
    /// Puede contener HTML sencillo (<b>, <br>); los nombres ya van escapados
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct FpsSummary {
    pub app: String,
    pub avg: f32,
    pub median: f32,
    /// Media de los "1 % low" de cada muestra
    pub low1: f32,
    pub min: f32,
    pub stutters: u32,
    pub samples: usize,
}

#[derive(Debug, Clone)]
pub struct MarkInfo {
    pub mark: Mark,
    pub sample: usize,
    /// Índice del episodio de caída de FPS a menos de 10 s, si lo hay
    pub near_drop: Option<usize>,
    pub causes: Vec<String>,
}

pub struct Analysis {
    pub duration: f64,
    pub cpu: Option<Stat>,
    pub ram: Option<Stat>,
    pub ct: Option<Stat>,
    pub gt: Option<Stat>,
    pub gh: Option<Stat>,
    pub gl: Option<Stat>,
    pub vram: Option<Stat>,
    pub ping: Option<Stat>,
    pub ping_lost: u32,
    pub ping_sent: usize,
    pub fan: Option<(f32, f32)>,
    pub fps: Option<FpsSummary>,
    pub main_app: Option<(String, u32)>,
    pub episodes: Vec<Episode>,
    pub cpu_throttle: usize,
    pub gpu_throttle: usize,
    /// (nombre, % CPU medio en 2º plano, pico)
    pub bg_heavy: Vec<(String, f32, f32)>,
    /// (nombre, % GPU medio, pico) de procesos distintos de la app principal
    pub gpu_bg: Vec<(String, f32, f32)>,
    /// (nombre, nº picos, pico máx, segundos)
    pub spike_groups: Vec<(String, usize, f32, f64)>,
    pub marks: Vec<MarkInfo>,
    pub findings: Vec<Finding>,
    pub verdict: String,
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// "1 caída", "3 caídas".
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

pub fn dur(secs: f64) -> String {
    let s = secs.round() as u64;
    if s >= 3600 {
        format!("{} h {} min", s / 3600, (s / 60) % 60)
    } else if s >= 60 {
        format!("{} min {} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

/// Tramos consecutivos donde se cumple `pred` (se permiten huecos de hasta 2 muestras).
pub fn spans(sm: &[Sample], pred: impl Fn(&Sample) -> bool) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, s) in sm.iter().enumerate() {
        if !pred(s) {
            continue;
        }
        match out.last_mut() {
            Some(e) if i - e.1 <= 3 => e.1 = i,
            _ => out.push((i, i)),
        }
    }
    out
}

/// Procesos que más CPU acumularon (según el top 5 de cada muestra) en un rango.
pub fn culprits(sm: &[Sample], from: usize, to: usize, n: usize) -> Vec<(String, f32)> {
    let mut acc: HashMap<&str, f32> = HashMap::new();
    let len = (to - from + 1) as f32;
    for s in &sm[from..=to] {
        for (name, cpu, _) in &s.top {
            *acc.entry(name.as_str()).or_default() += cpu;
        }
    }
    let mut v: Vec<(String, f32)> = acc.into_iter().map(|(k, c)| (k.to_string(), c / len)).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.truncate(n);
    v
}

pub fn main_fg(sm: &[Sample], from: usize, to: usize) -> Option<String> {
    let mut acc: HashMap<&str, u32> = HashMap::new();
    for s in &sm[from..=to] {
        if let Some(f) = &s.fg {
            *acc.entry(f.as_str()).or_default() += 1;
        }
    }
    acc.into_iter().max_by_key(|(_, c)| *c).map(|(k, _)| k.to_string())
}

/// Aplicación principal: la ventana activa más tiempo, sin contar el escritorio.
pub fn main_app(procs: &HashMap<String, ProcStats>) -> Option<(String, u32)> {
    procs
        .iter()
        .filter(|(k, v)| v.fg_samples > 0 && !is_shell(k))
        .max_by_key(|(k, v)| (v.fg_samples, std::cmp::Reverse((*k).clone())))
        .map(|(k, v)| (k.clone(), v.fg_samples))
}

/// Muestras con la CPU frenada: frecuencia < 80 % de su P95 con carga y temperatura altas.
pub fn cpu_throttle_samples(sm: &[Sample], cpu_limit: f32) -> Vec<usize> {
    let refs: Vec<f32> = sm.iter().filter(|x| x.cpu >= 30.0).filter_map(|x| x.cc).collect();
    let Some(r) = percentile(&refs, 0.95) else { return vec![] };
    sm.iter()
        .enumerate()
        .filter(|(_, x)| x.cpu >= 50.0 && x.cc.is_some_and(|c| c < r * 0.8) && x.ct.is_some_and(|t| t >= cpu_limit - 5.0))
        .map(|(i, _)| i)
        .collect()
}

pub fn gpu_throttle_samples(sm: &[Sample], gpu_limit: f32) -> Vec<usize> {
    let refs: Vec<f32> = sm.iter().filter(|x| x.gl.unwrap_or(0.0) >= 50.0).filter_map(|x| x.gc).collect();
    let r = percentile(&refs, 0.95);
    sm.iter()
        .enumerate()
        .filter(|(_, x)| {
            (x.gthr && x.gl.unwrap_or(0.0) >= 50.0)
                || (x.gl.unwrap_or(0.0) >= 80.0
                    && r.is_some_and(|r| x.gc.is_some_and(|c| c < r * 0.8))
                    && x.gt.is_some_and(|t| t >= gpu_limit - 3.0))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Resumen de FPS del proceso que más tiempo se midió.
pub fn fps_summary(sm: &[Sample]) -> Option<FpsSummary> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for s in sm {
        if let (Some(_), Some(p)) = (s.fps, &s.fpp) {
            *count.entry(p.as_str()).or_default() += 1;
        }
    }
    let app = count.into_iter().max_by_key(|x| x.1)?.0.to_string();
    let mine: Vec<&Sample> = sm.iter().filter(|s| s.fps.is_some() && s.fpp.as_deref() == Some(app.as_str())).collect();
    let fps: Vec<f32> = mine.iter().filter_map(|s| s.fps).collect();
    let lows: Vec<f32> = mine.iter().filter_map(|s| s.fl).collect();
    Some(FpsSummary {
        avg: fps.iter().sum::<f32>() / fps.len() as f32,
        median: percentile(&fps, 0.5)?,
        low1: if lows.is_empty() { 0.0 } else { lows.iter().sum::<f32>() / lows.len() as f32 },
        min: fps.iter().copied().fold(f32::MAX, f32::min),
        stutters: mine.iter().map(|s| s.stut).sum(),
        samples: mine.len(),
        app,
    })
}

/// Muestras con caída de FPS: por debajo del 75 % de la mediana, o con tirones claros.
pub fn fps_drop_spans(sm: &[Sample], f: &FpsSummary) -> Vec<(usize, usize)> {
    let frame_ms = 1000.0 / f.median.max(1.0);
    spans(sm, |s| {
        s.fps.is_some()
            && s.fpp.as_deref() == Some(f.app.as_str())
            && (s.fps.unwrap() < f.median * 0.75 || s.stut >= 2 || s.ftm.is_some_and(|m| m > (frame_ms * 4.0).max(50.0)))
    })
}

/// Qué pasaba en un rango de muestras que pueda explicar una caída de rendimiento.
pub fn explain(s: &Session, from: usize, to: usize, app: Option<&str>, cpu_thr: &[usize], gpu_thr: &[usize]) -> Vec<String> {
    let sm = &s.samples;
    let from = from.saturating_sub(1);
    let to = to.min(sm.len() - 1);
    let win = &sm[from..=to];
    let (t0, t1) = (sm[from].t - s.interval, sm[to].t);
    let mut c = Vec::new();

    if cpu_thr.iter().any(|i| (from..=to).contains(i)) {
        let t = win.iter().filter_map(|x| x.ct).fold(0.0, f32::max);
        let mhz = win.iter().filter_map(|x| x.cc).fold(f32::MAX, f32::min);
        c.push(format!("CPU frenada por temperatura ({t:.0} °C, bajó a {mhz:.0} MHz)"));
    }
    if gpu_thr.iter().any(|i| (from..=to).contains(i)) {
        c.push("GPU frenada por temperatura o consumo".into());
    }
    let is_app = |n: &str| app.is_some_and(|a| a.eq_ignore_ascii_case(n));
    if let Some(sp) = s
        .spikes
        .iter()
        .filter(|sp| sp.start <= t1 && sp.end >= t0 && !is_app(&sp.name))
        .max_by(|a, b| a.peak.total_cmp(&b.peak))
    {
        c.push(format!("pico de {} en segundo plano ({:.0} % de CPU)", esc(&sp.name), sp.peak));
    }
    if let Some((n, v)) = win
        .iter()
        .flat_map(|x| x.gtop.iter())
        .filter(|(n, v)| *v >= 15.0 && !is_app(n) && n.to_lowercase() != "dwm.exe")
        .max_by(|a, b| a.1.total_cmp(&b.1))
    {
        c.push(format!("{} usando {v:.0} % de la GPU", esc(n)));
    }
    if let Some(m) = win.iter().map(|x| x.cpu).reduce(f32::max).filter(|m| *m >= 95.0) {
        c.push(format!("CPU al {m:.0} %"));
    }
    if win.iter().any(|x| x.gmp.is_some_and(|v| v >= 95.0)) {
        c.push("VRAM llena".into());
    }
    if let Some(m) = win.iter().map(|x| x.ram).reduce(f32::max).filter(|m| *m >= 90.0) {
        c.push(format!("RAM al {m:.0} %"));
    }
    if let Some(m) = win.iter().map(|x| x.dr).reduce(f32::max).filter(|m| *m >= 100.0) {
        c.push(format!("lectura de disco de {m:.0} MB/s (posible carga de datos)"));
    }
    if c.is_empty() && win.iter().any(|x| x.gl.is_some_and(|g| g >= 97.0)) {
        c.push("GPU al máximo (escena exigente)".into());
    }
    c.truncate(3);
    c
}

fn bg_spike_groups(spikes: &[Spike], main: Option<&str>, interval: f64) -> Vec<(String, usize, f32, f64)> {
    let mut by: HashMap<&str, (usize, f32, f64)> = HashMap::new();
    for x in spikes.iter().filter(|x| main.is_none_or(|m| x.name != m)) {
        let e = by.entry(x.name.as_str()).or_default();
        e.0 += 1;
        e.1 = e.1.max(x.peak);
        e.2 += x.end - x.start + interval;
    }
    let mut v: Vec<(String, usize, f32, f64)> = by.into_iter().map(|(k, (n, p, s))| (k.to_string(), n, p, s)).collect();
    v.sort_by(|a, b| (b.3 * b.2 as f64).total_cmp(&(a.3 * a.2 as f64)));
    v
}

pub fn analyze(s: &Session) -> Analysis {
    let sm = &s.samples;
    let n = sm.len();
    let nn = n.max(1) as f64;
    let main_app = main_app(&s.procs);
    let main = main_app.as_ref().map(|m| m.0.as_str());

    let cpu_thr = cpu_throttle_samples(sm, s.cpu_limit);
    let gpu_thr = gpu_throttle_samples(sm, s.gpu_limit);
    let fps = fps_summary(sm);

    // ---------- Episodios ----------
    let mut episodes = Vec::new();
    if let Some(f) = &fps {
        for (a, b) in fps_drop_spans(sm, f) {
            let peak = sm[a..=b].iter().filter_map(|x| x.fps).fold(f32::MAX, f32::min);
            episodes.push(Episode {
                kind: Kind::FpsDrop,
                start: a,
                end: b,
                peak,
                culprits: culprits(sm, a, b, 3),
                fg: main_fg(sm, a, b),
                causes: explain(s, a, b, Some(&f.app), &cpu_thr, &gpu_thr),
            });
        }
    }
    let mut push = |kind: Kind, sp: Vec<(usize, usize)>, val: &dyn Fn(&Sample) -> f32, min_len: usize| {
        for (a, b) in sp.into_iter().filter(|(a, b)| b - a + 1 >= min_len) {
            episodes.push(Episode {
                kind,
                start: a,
                end: b,
                peak: sm[a..=b].iter().map(val).fold(f32::MIN, f32::max),
                culprits: culprits(sm, a.saturating_sub(15), b, 4),
                fg: main_fg(sm, a, b),
                causes: Vec::new(),
            });
        }
    };
    push(Kind::CpuHot, spans(sm, |x| x.ct.is_some_and(|t| t >= s.cpu_limit)), &|x| x.ct.unwrap_or(0.0), 1);
    push(Kind::GpuHot, spans(sm, |x| x.gt.is_some_and(|t| t >= s.gpu_limit)), &|x| x.gt.unwrap_or(0.0), 1);
    push(Kind::CpuBusy, spans(sm, |x| x.cpu >= 90.0), &|x| x.cpu, 3);
    push(Kind::RamFull, spans(sm, |x| x.ram >= 90.0), &|x| x.ram, 1);
    episodes.sort_by_key(|e| e.start);

    // ---------- Procesos ----------
    let mut procs: Vec<(&String, &ProcStats)> = s.procs.iter().collect();
    procs.sort_by(|a, b| b.1.sum_cpu.total_cmp(&a.1.sum_cpu));
    let not_main = |k: &str| main.is_none_or(|m| m != k) && !matches!(k, "medidor-recursos.exe" | "medidor-gui.exe");
    let bg_heavy: Vec<(String, f32, f32)> = procs
        .iter()
        .filter(|(k, _)| not_main(k))
        .map(|(k, v)| (k.to_string(), (v.sum_cpu_bg / nn) as f32, v.max_cpu_bg))
        .filter(|x| x.1 >= 3.0)
        .collect();
    let mut gpu_bg: Vec<(String, f32, f32)> = procs
        .iter()
        .filter(|(k, _)| not_main(k) && k.to_lowercase() != "dwm.exe")
        .map(|(k, v)| (k.to_string(), (v.sum_gpu / nn) as f32, v.max_gpu))
        .filter(|x| x.1 >= 3.0 || x.2 >= 30.0)
        .collect();
    gpu_bg.sort_by(|a, b| b.1.total_cmp(&a.1));
    let spike_groups = bg_spike_groups(&s.spikes, main, s.interval);

    // ---------- Marcas ----------
    let marks = s
        .marks
        .iter()
        .map(|m| {
            let i = sm.iter().position(|x| x.t >= m.t).unwrap_or(n.saturating_sub(1));
            let near_drop = episodes
                .iter()
                .position(|e| e.kind == Kind::FpsDrop && sm[e.start].t - 10.0 <= m.t && sm[e.end].t + 10.0 >= m.t);
            let causes = if n == 0 {
                vec![]
            } else {
                let (a, b) = near_drop.map(|d| (episodes[d].start, episodes[d].end)).unwrap_or((i.saturating_sub(5), i));
                explain(s, a, b, fps.as_ref().map(|f| f.app.as_str()).or(main), &cpu_thr, &gpu_thr)
            };
            MarkInfo { mark: m.clone(), sample: i, near_drop, causes }
        })
        .collect();

    let fans: Vec<f32> = sm.iter().flat_map(|x| x.fan.values().copied()).collect();
    let pings: Vec<&Sample> = sm.iter().filter(|x| x.ping.is_some() || x.plost > 0).collect();

    let mut a = Analysis {
        duration: sm.last().map(|x| x.t).unwrap_or(0.0),
        cpu: stat(sm, |x| Some(x.cpu)),
        ram: stat(sm, |x| Some(x.ram)),
        ct: stat(sm, |x| x.ct),
        gt: stat(sm, |x| x.gt),
        gh: stat(sm, |x| x.gh),
        gl: stat(sm, |x| x.gl),
        vram: stat(sm, |x| x.gmp),
        ping: stat(sm, |x| x.ping),
        ping_lost: sm.iter().map(|x| x.plost).sum(),
        ping_sent: pings.len(),
        fan: (!fans.is_empty()).then(|| (fans.iter().copied().fold(f32::MAX, f32::min), fans.iter().copied().fold(0.0, f32::max))),
        fps,
        main_app,
        episodes,
        cpu_throttle: cpu_thr.len(),
        gpu_throttle: gpu_thr.len(),
        bg_heavy,
        gpu_bg,
        spike_groups,
        marks,
        findings: Vec::new(),
        verdict: String::new(),
    };
    a.findings = findings(s, &a);
    a.verdict = verdict(s, &a);
    a
}

fn count(a: &Analysis, k: Kind) -> usize {
    a.episodes.iter().filter(|e| e.kind == k).count()
}

fn secs_in(a: &Analysis, s: &Session, k: Kind) -> f64 {
    a.episodes
        .iter()
        .filter(|e| e.kind == k)
        .map(|e| s.samples[e.end].t - s.samples[e.start].t + s.interval)
        .sum()
}

fn findings(s: &Session, a: &Analysis) -> Vec<Finding> {
    let sm = &s.samples;
    let mut f = Vec::new();
    let hhmm = |i: usize| sm.get(i).map(|x| x.h.clone()).unwrap_or_default();

    // FPS
    if let Some(fp) = &a.fps {
        let drops = count(a, Kind::FpsDrop);
        // causa más repetida entre las caídas
        let mut causes: HashMap<String, usize> = HashMap::new();
        for e in a.episodes.iter().filter(|e| e.kind == Kind::FpsDrop) {
            for c in &e.causes {
                // agrupa por la parte antes del paréntesis
                let key = c.split(" (").next().unwrap_or(c).to_string();
                *causes.entry(key).or_default() += 1;
            }
        }
        let mut cv: Vec<_> = causes.into_iter().collect();
        cv.sort_by(|x, y| y.1.cmp(&x.1));
        let detail = format!(
            "{}: media {:.0} FPS, 1 % low {:.0}, mínimo {:.0}. {} (fotogramas de más de 2,5 veces lo normal).{}",
            esc(&fp.app),
            fp.avg,
            fp.low1,
            fp.min,
            plural(fp.stutters as usize, "tirón", "tirones"),
            if drops == 0 {
                String::new()
            } else {
                format!(
                    " {}, {} en total.{}",
                    plural(drops, "caída", "caídas"),
                    dur(secs_in(a, s, Kind::FpsDrop)),
                    if cv.is_empty() {
                        " En los datos no aparece una causa clara.".to_string()
                    } else {
                        format!(
                            " Coinciden sobre todo con: {}.",
                            cv.iter().take(3).map(|(k, n)| format!("{k} ({n})")).collect::<Vec<_>>().join(", ")
                        )
                    }
                )
            }
        );
        let level = if drops >= 5 || fp.low1 < fp.median * 0.5 { Level::Aviso } else { Level::Info };
        f.push(Finding { level, title: if drops > 0 { "Caídas de FPS".into() } else { "FPS estables".into() }, detail });
    } else if let Some(why) = &s.fps_unavailable {
        f.push(Finding {
            level: Level::Info,
            title: "FPS no medidos".into(),
            detail: format!(
                "Motivo: {why}. Para medirlos, inicia la monitorización con la opción «Medir FPS» (pide permiso de administrador)."
            ),
        });
    }

    // CPU
    match a.ct {
        None => f.push(Finding {
            level: Level::Info,
            title: "Sin temperatura de CPU".into(),
            detail: "Abre LibreHardwareMonitor como administrador antes de empezar para tener temperaturas, frecuencias y ventiladores.".into(),
        }),
        Some(ct) => {
            let who = culprits(sm, ct.max_i.saturating_sub(15), ct.max_i, 3)
                .iter()
                .map(|(k, c)| format!("{} {c:.0} %", esc(k)))
                .collect::<Vec<_>>()
                .join(", ");
            let (level, title) = if ct.max >= 95.0 {
                (Level::Grave, "CPU a temperatura crítica")
            } else if ct.max >= s.cpu_limit {
                (Level::Aviso, "CPU caliente")
            } else {
                (Level::Ok, "Temperatura de CPU normal")
            };
            let mut d = format!(
                "Máx. {:.0} °C a las {}, media {:.0} °C. {} por encima de {:.0} °C.",
                ct.max,
                hhmm(ct.max_i),
                ct.avg,
                dur(secs_in(a, s, Kind::CpuHot)),
                s.cpu_limit
            );
            if !who.is_empty() && ct.max >= s.cpu_limit {
                d += &format!(" Consumo de CPU en ese momento: {who}.");
            }
            if ct.max >= s.cpu_limit {
                d += " Revisar polvo en disipador y ventiladores, pasta térmica y flujo de aire de la caja.";
            }
            f.push(Finding { level, title: title.into(), detail: d });
            if ct.max_i < sm.len() && sm.iter().filter_map(|x| x.ct).all(|t| (t - ct.max).abs() < 0.5) && sm.len() > 10 && !s.has_hwmon {
                f.push(Finding {
                    level: Level::Info,
                    title: "La temperatura de Windows no varía".into(),
                    detail: "Es la zona térmica ACPI y en sobremesa suele dar un valor fijo. No es fiable; usa LibreHardwareMonitor.".into(),
                });
            }
        }
    }

    // GPU
    if let Some(gt) = a.gt {
        let (level, title) = if gt.max >= 90.0 {
            (Level::Grave, "GPU a temperatura crítica")
        } else if gt.max >= s.gpu_limit {
            (Level::Aviso, "GPU caliente")
        } else {
            (Level::Ok, "Temperatura de GPU normal")
        };
        let mut d = format!("Máx. {:.0} °C a las {}, media {:.0} °C.", gt.max, hhmm(gt.max_i), gt.avg);
        if let Some(h) = a.gh {
            d += &format!(" Hot spot máx. {:.0} °C.", h.max);
        }
        if gt.max >= s.gpu_limit {
            d += " Limpiar los ventiladores de la gráfica o subir su curva de ventilación.";
        }
        f.push(Finding { level, title: title.into(), detail: d });
    }

    if a.cpu_throttle > 0 {
        f.push(Finding {
            level: Level::Grave,
            title: "La CPU se frenó por calor".into(),
            detail: format!(
                "En {} muestras ({}) la frecuencia cayó más de un 20 % con carga y temperatura altas.",
                a.cpu_throttle,
                dur(a.cpu_throttle as f64 * s.interval)
            ),
        });
    }
    if a.gpu_throttle > 0 {
        f.push(Finding {
            level: Level::Grave,
            title: "La GPU se frenó".into(),
            detail: format!("Durante {} bajó su frecuencia por temperatura o consumo.", dur(a.gpu_throttle as f64 * s.interval)),
        });
    }

    let busy = count(a, Kind::CpuBusy);
    if busy > 0 {
        let e = a.episodes.iter().filter(|e| e.kind == Kind::CpuBusy).max_by_key(|e| e.end - e.start).unwrap();
        let who = e.culprits.iter().take(3).map(|(k, c)| format!("{} {c:.0} %", esc(k))).collect::<Vec<_>>().join(", ");
        f.push(Finding {
            level: Level::Aviso,
            title: "CPU al límite".into(),
            detail: format!("{busy} tramos por encima del 90 %. En el más largo: {who}."),
        });
    }

    if let Some(v) = a.vram.filter(|v| v.max >= 90.0) {
        f.push(Finding {
            level: if v.max >= 97.0 { Level::Aviso } else { Level::Info },
            title: "Memoria de vídeo casi llena".into(),
            detail: format!("Máx. {:.0} % de la VRAM. Con la VRAM llena aparecen tirones; bajar la calidad de texturas ayuda.", v.max),
        });
    }

    if let Some(r) = a.ram {
        if r.max >= 90.0 {
            f.push(Finding {
                level: Level::Grave,
                title: "RAM casi llena".into(),
                detail: format!("Máx. {:.0} % de {:.0} GB. Windows empieza a tirar del disco y aparecen tirones.", r.max, s.total_ram_gb),
            });
        } else if r.max >= 80.0 {
            f.push(Finding {
                level: Level::Info,
                title: "RAM alta".into(),
                detail: format!("Máx. {:.0} % de {:.0} GB.", r.max, s.total_ram_gb),
            });
        }
    }

    if !a.bg_heavy.is_empty() {
        let list = a
            .bg_heavy
            .iter()
            .take(5)
            .map(|(k, avg, mx)| {
                let d = describe(k);
                format!("<b>{}</b> {avg:.1} % de media, pico {mx:.0} %{}", esc(k), if d.is_empty() { String::new() } else { format!(". {d}") })
            })
            .collect::<Vec<_>>()
            .join("<br>");
        f.push(Finding { level: Level::Aviso, title: "Consumo de CPU en segundo plano".into(), detail: list });
    }

    if !a.spike_groups.is_empty() {
        let list = a
            .spike_groups
            .iter()
            .take(5)
            .map(|(k, n, peak, secs)| {
                let d = describe(k);
                format!(
                    "<b>{}</b> {n} {}, hasta {peak:.0} %, {}{}",
                    esc(k),
                    if *n == 1 { "pico" } else { "picos" },
                    dur(*secs),
                    if d.is_empty() { String::new() } else { format!(". {d}") }
                )
            })
            .collect::<Vec<_>>()
            .join("<br>");
        f.push(Finding { level: Level::Aviso, title: "Picos en segundo plano".into(), detail: list });
    }

    if !a.gpu_bg.is_empty() {
        let list = a
            .gpu_bg
            .iter()
            .take(4)
            .map(|(k, avg, mx)| format!("<b>{}</b> {avg:.0} % de media, pico {mx:.0} %", esc(k)))
            .collect::<Vec<_>>()
            .join("<br>");
        f.push(Finding { level: Level::Aviso, title: "Otros programas usando la GPU".into(), detail: list });
    }

    if let Some(p) = a.ping {
        let loss = if a.ping_sent > 0 { a.ping_lost as f32 / (a.ping_sent as f32 + a.ping_lost as f32) * 100.0 } else { 0.0 };
        if p.p95 >= 100.0 || loss >= 2.0 {
            f.push(Finding {
                level: Level::Aviso,
                title: "Conexión inestable".into(),
                detail: format!(
                    "Ping a {}: media {:.0} ms, P95 {:.0} ms, máx. {:.0} ms, {:.1} % de paquetes perdidos. En juegos online esto se nota como tirones aunque los FPS estén bien.",
                    esc(s.ping_host.as_deref().unwrap_or("?")),
                    p.avg,
                    p.p95,
                    p.max,
                    loss
                ),
            });
        }
    }

    // Ventilador parado justo cuando hay calor (en reposo es normal: muchos se paran solos).
    let mut stopped: Vec<&str> = Vec::new();
    for x in sm.iter().filter(|x| x.ct.is_some_and(|t| t >= s.cpu_limit) || x.gt.is_some_and(|t| t >= s.gpu_limit)) {
        for (k, v) in &x.fan {
            if *v == 0.0 && !stopped.contains(&k.as_str()) {
                stopped.push(k);
            }
        }
    }
    if !stopped.is_empty() {
        f.push(Finding {
            level: Level::Aviso,
            title: "Ventilador a 0 rpm con temperatura alta".into(),
            detail: format!(
                "{}. Puede estar parado o mal conectado. Si ese conector no tiene ventilador, ignóralo.",
                stopped.iter().map(|k| esc(k)).collect::<Vec<_>>().join(", ")
            ),
        });
    }

    f.sort_by_key(|x| x.level);
    f
}

fn verdict(s: &Session, a: &Analysis) -> String {
    let mut parts: Vec<String> = Vec::new();
    if a.cpu_throttle > 0 {
        parts.push(format!("La CPU llegó a {:.0} °C y bajó su frecuencia para protegerse.", a.ct.map_or(0.0, |c| c.max)));
    } else if a.ct.is_some_and(|c| c.max >= s.cpu_limit) {
        parts.push(format!("La CPU se calentó hasta {:.0} °C.", a.ct.unwrap().max));
    }
    if a.gpu_throttle > 0 {
        parts.push("La GPU se frenó por temperatura o consumo.".into());
    } else if a.gt.is_some_and(|g| g.max >= s.gpu_limit) {
        parts.push(format!("La GPU llegó a {:.0} °C.", a.gt.unwrap().max));
    }
    let drops: Vec<&Episode> = a.episodes.iter().filter(|e| e.kind == Kind::FpsDrop).collect();
    if !drops.is_empty() {
        let with_cause = drops.iter().filter(|e| !e.causes.is_empty()).count();
        parts.push(match (drops.len(), with_cause) {
            (1, 1) => "Hubo una caída de FPS y en los datos se ve qué la provocó.".to_string(),
            (1, _) => "Hubo una caída de FPS sin causa clara en los datos.".to_string(),
            (n, 0) => format!("Hubo {n} caídas de FPS, ninguna con una causa clara en los datos."),
            (n, c) if c == n => format!("Hubo {n} caídas de FPS; todas coinciden con algo visible en los datos."),
            (n, c) => format!("Hubo {n} caídas de FPS; {c} coinciden con algo visible en los datos."),
        });
    }
    if let Some((name, ..)) = a.spike_groups.first() {
        parts.push(format!("El proceso en segundo plano que más molestó fue {}.", esc(name)));
    } else if let Some((name, ..)) = a.bg_heavy.first() {
        parts.push(format!("En segundo plano destacó {}.", esc(name)));
    }
    if a.ram.is_some_and(|r| r.max >= 90.0) {
        parts.push("La RAM se llenó.".into());
    }
    if parts.is_empty() {
        "Nada fuera de lo normal en esta sesión.".into()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use chrono::Local;

    pub fn session(samples: Vec<Sample>) -> Session {
        let now = Local::now();
        Session {
            start: now,
            end: now,
            interval: 2.0,
            cpu_name: "CPU".into(),
            gpu_name: None,
            ncpu: 8,
            total_ram_gb: 16.0,
            sources: vec![],
            has_hwmon: true,
            cpu_limit: 85.0,
            gpu_limit: 83.0,
            samples,
            procs: HashMap::new(),
            spikes: vec![],
            marks: vec![],
            fps_unavailable: None,
            ping_host: Some("1.1.1.1".into()),
            game: None,
            own_cpu_avg: 0.0,
            own_mem_mb: 0.0,
            csv_path: "x.csv".into(),
        }
    }

    pub fn sample(i: usize) -> Sample {
        Sample { t: i as f64 * 2.0, h: format!("{i}"), cpu: 40.0, ram: 50.0, ..Default::default() }
    }

    #[test]
    fn tramos_con_huecos() {
        let sm: Vec<Sample> = (0..20).map(sample).collect();
        let hot = [3usize, 4, 6, 12];
        let r = spans(&sm, |s| hot.contains(&((s.t / 2.0) as usize)));
        assert_eq!(r, vec![(3, 6), (12, 12)]);
    }

    #[test]
    fn app_principal_ignora_escritorio() {
        let mut p = HashMap::new();
        p.insert("explorer.exe".to_string(), ProcStats { fg_samples: 100, ..Default::default() });
        p.insert("juego.exe".to_string(), ProcStats { fg_samples: 40, ..Default::default() });
        assert_eq!(main_app(&p).unwrap().0, "juego.exe");
    }

    #[test]
    fn throttling_cpu() {
        let mut sm: Vec<Sample> = (0..30)
            .map(|i| Sample { cpu: 70.0, cc: Some(4000.0), ct: Some(80.0), ..sample(i) })
            .collect();
        for s in &mut sm[20..25] {
            s.cc = Some(2800.0);
            s.ct = Some(96.0);
        }
        assert_eq!(cpu_throttle_samples(&sm, 85.0), vec![20, 21, 22, 23, 24]);
    }

    #[test]
    fn caida_de_fps_y_su_causa() {
        let mut sm: Vec<Sample> = (0..60)
            .map(|i| Sample { fps: Some(120.0), fl: Some(100.0), fpp: Some("juego.exe".into()), fg: Some("juego.exe".into()), ..sample(i) })
            .collect();
        for s in &mut sm[30..33] {
            s.fps = Some(60.0);
            s.stut = 3;
        }
        let mut s = session(sm);
        s.spikes.push(Spike { name: "MsMpEng.exe".into(), start: 58.0, end: 66.0, peak: 35.0, hora: "x".into(), fg: None });
        s.marks.push(Mark { t: 62.0, h: "x".into(), label: "M1".into() });
        let a = analyze(&s);
        let f = a.fps.as_ref().unwrap();
        assert_eq!(f.app, "juego.exe");
        assert_eq!(f.median, 120.0);
        let drops: Vec<_> = a.episodes.iter().filter(|e| e.kind == Kind::FpsDrop).collect();
        assert_eq!(drops.len(), 1);
        assert_eq!((drops[0].start, drops[0].end), (30, 32));
        assert!(drops[0].causes.iter().any(|c| c.contains("MsMpEng.exe")), "{:?}", drops[0].causes);
        assert_eq!(a.marks[0].near_drop, Some(0));
        assert!(a.verdict.contains("una caída de FPS y en los datos se ve qué la provocó"), "{}", a.verdict);
    }

    #[test]
    fn sin_problemas() {
        let s = session((0..30).map(|i| Sample { ct: Some(60.0), ..sample(i) }).collect());
        let a = analyze(&s);
        assert!(a.episodes.is_empty());
        assert_eq!(a.verdict, "Nada fuera de lo normal en esta sesión.");
        assert!(a.findings.iter().all(|f| f.level >= Level::Info));
    }
}
