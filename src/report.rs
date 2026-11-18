//! Análisis de la sesión y generación del informe HTML (autocontenido, sin internet).

use crate::monitor::{ProcStats, Sample, Session};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

const MAX_POINTS: usize = 2000;

/// Procesos del propio sistema/escritorio que no cuentan como "aplicación principal".
const SHELL: &[&str] = &[
    "explorer.exe",
    "windowsterminal.exe",
    "conhost.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "medidor-recursos.exe",
    "searchhost.exe",
    "searchapp.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "lockapp.exe",
    "applicationframehost.exe",
    "taskmgr.exe",
];

/// Explicación de procesos habituales que suelen molestar mientras se juega.
fn describe(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "msmpeng.exe" | "mpdefendercoreservice.exe" | "nissrv.exe" => {
            "Antivirus Microsoft Defender analizando. Programa el análisis fuera de horas de juego o excluye la carpeta del juego."
        }
        "tiworker.exe" | "trustedinstaller.exe" | "wuauclt.exe" | "mousocoreworker.exe" | "usocoreworker.exe" | "sihclient.exe"
        | "wuaueng.exe" => "Windows Update descargando/instalando actualizaciones. Deja que termine o pausa las actualizaciones.",
        "searchindexer.exe" | "searchprotocolhost.exe" | "searchfilterhost.exe" => "Indexador de búsqueda de Windows.",
        "onedrive.exe" => "Sincronización de OneDrive.",
        "chrome.exe" | "msedge.exe" | "firefox.exe" | "opera.exe" | "brave.exe" | "msedgewebview2.exe" => {
            "Navegador o componente web (pestañas, vídeos). Ciérralo mientras juegas."
        }
        "discord.exe" => "Discord (overlay, streams y aceleración por hardware pueden consumir).",
        "steamwebhelper.exe" | "steam.exe" => "Steam (interfaz web o descargas/actualizaciones en curso).",
        "epicgameslauncher.exe" | "epicwebhelper.exe" => "Launcher de Epic Games (descargas/actualizaciones).",
        "battle.net.exe" | "agent.exe" => "Battle.net (descargas/actualizaciones).",
        "nvidia share.exe" | "nvcontainer.exe" | "nvidia overlay.exe" | "nvsphelper64.exe" => {
            "Overlay/grabación de NVIDIA (ShadowPlay / Instant Replay)."
        }
        "radeonsoftware.exe" | "amdrsserv.exe" | "amdow.exe" => "Software AMD Adrenalin (overlay/grabación).",
        "dwm.exe" => "Gestor de ventanas de Windows. Es normal que use algo; alto con juegos en modo ventana.",
        "system" => "Núcleo de Windows y controladores. Si es alto de forma sostenida, puede ser un driver problemático.",
        "memory compression" => "Compresión de memoria de Windows: si consume mucho indica falta de RAM.",
        "svchost.exe" => "Servicios de Windows (genérico: actualizaciones, red, etc.).",
        "audiodg.exe" => "Audio de Windows (efectos/mejoras de sonido).",
        "wmiprvse.exe" => "WMI: consultas de hardware (este monitor, LibreHardwareMonitor u otro software).",
        "librehardwaremonitor.exe" | "openhardwaremonitor.exe" => "Lector de sensores usado por este monitor.",
        "medidor-recursos.exe" => "Este monitor.",
        "wallpaper64.exe" | "wallpaper32.exe" => "Wallpaper Engine (fondos animados).",
        "icue.exe" | "razer synapse 3.exe" | "lghub.exe" | "lghub_agent.exe" | "armourycrate.service.exe" => {
            "Software de periféricos/RGB."
        }
        "obs64.exe" => "OBS (grabación/streaming).",
        "spotify.exe" => "Spotify.",
        "teams.exe" | "ms-teams.exe" => "Microsoft Teams.",
        _ => "",
    }
}

#[derive(Serialize)]
struct Band {
    a: f64,
    b: f64,
    kind: &'static str,
}

struct Episode {
    start: usize,
    end: usize,
    max: f32,
}

struct Finding {
    level: &'static str,
    title: String,
    detail: String,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn series(samples: &[Sample], f: impl Fn(&Sample) -> Option<f32>) -> Vec<f32> {
    samples.iter().filter_map(f).collect()
}

fn avg(v: &[f32]) -> Option<f32> {
    (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
}

fn max(v: &[f32]) -> Option<f32> {
    v.iter().copied().reduce(f32::max)
}

fn pct(v: &[f32], p: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    let i = ((s.len() - 1) as f32 * p).round() as usize;
    Some(s[i])
}

fn o(v: Option<f32>, unit: &str) -> String {
    v.map(|x| format!("{x:.0}{unit}")).unwrap_or_else(|| "—".into())
}

fn dur(secs: f64) -> String {
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
fn episodes(samples: &[Sample], pred: impl Fn(&Sample) -> bool, value: impl Fn(&Sample) -> f32) -> Vec<Episode> {
    let mut out: Vec<Episode> = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        if !pred(s) {
            continue;
        }
        let v = value(s);
        match out.last_mut() {
            Some(e) if i - e.end <= 3 => {
                e.end = i;
                e.max = e.max.max(v);
            }
            _ => out.push(Episode { start: i, end: i, max: v }),
        }
    }
    out
}

/// Procesos que más CPU acumularon (según el top 5 de cada muestra) en un rango.
fn culprits(samples: &[Sample], from: usize, to: usize, n: usize) -> Vec<(String, f32)> {
    let mut acc: HashMap<&str, f32> = HashMap::new();
    let len = (to - from + 1) as f32;
    for s in &samples[from..=to] {
        for (name, cpu, _) in &s.top {
            *acc.entry(name.as_str()).or_default() += cpu;
        }
    }
    let mut v: Vec<(String, f32)> = acc.into_iter().map(|(k, c)| (k.to_string(), c / len)).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.truncate(n);
    v
}

fn main_fg(samples: &[Sample], from: usize, to: usize) -> Option<String> {
    let mut acc: HashMap<&str, u32> = HashMap::new();
    for s in &samples[from..=to] {
        if let Some(f) = &s.fg {
            *acc.entry(f.as_str()).or_default() += 1;
        }
    }
    acc.into_iter().max_by_key(|(_, c)| *c).map(|(k, _)| k.to_string())
}

fn is_shell(name: &str) -> bool {
    SHELL.contains(&name.to_lowercase().as_str())
}

pub fn write(s: &Session, path: &Path, _final: bool) -> std::io::Result<()> {
    std::fs::write(path, build(s))
}

fn build(s: &Session) -> String {
    let sm = &s.samples;
    let n = sm.len();
    let total_t = sm.last().map(|x| x.t).unwrap_or(0.0);

    let cpu = series(sm, |x| Some(x.cpu));
    let ram = series(sm, |x| Some(x.ram));
    let ct = series(sm, |x| x.ct);
    let gt = series(sm, |x| x.gt);
    let gl = series(sm, |x| x.gl);
    let gh = series(sm, |x| x.gh);

    let argmax = |f: &dyn Fn(&Sample) -> Option<f32>| {
        sm.iter()
            .enumerate()
            .filter_map(|(i, x)| f(x).map(|v| (i, v)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let ct_max_i = argmax(&|x| x.ct);
    let gt_max_i = argmax(&|x| x.gt);

    // Aplicación principal (probablemente el juego).
    let main_app = s
        .procs
        .iter()
        .filter(|(k, v)| v.fg_samples > 0 && !is_shell(k))
        .max_by_key(|(_, v)| v.fg_samples)
        .map(|(k, v)| (k.clone(), v.fg_samples));

    // ---------- Episodios ----------
    let cpu_hot = episodes(sm, |x| x.ct.is_some_and(|t| t >= s.cpu_limit), |x| x.ct.unwrap_or(0.0));
    let gpu_hot = episodes(sm, |x| x.gt.is_some_and(|t| t >= s.gpu_limit), |x| x.gt.unwrap_or(0.0));
    let cpu_sat = episodes(sm, |x| x.cpu >= 90.0, |x| x.cpu)
        .into_iter()
        .filter(|e| e.end - e.start >= 2)
        .collect::<Vec<_>>();
    let ram_full = episodes(sm, |x| x.ram >= 90.0, |x| x.ram);

    let mut bands = Vec::new();
    for (eps, kind) in [(&cpu_hot, "cpu"), (&gpu_hot, "gpu"), (&cpu_sat, "sat")] {
        for e in eps.iter() {
            bands.push(Band { a: sm[e.start].t, b: sm[e.end].t, kind });
        }
    }

    // ---------- Throttling ----------
    let cc_ref = pct(&series(sm, |x| if x.cpu >= 30.0 { x.cc } else { None }), 0.95);
    let cpu_throttle = cc_ref.map_or(0, |r| {
        sm.iter()
            .filter(|x| {
                x.cpu >= 50.0 && x.cc.is_some_and(|c| c < r * 0.8) && x.ct.is_some_and(|t| t >= s.cpu_limit - 5.0)
            })
            .count()
    });
    let gc_ref = pct(&series(sm, |x| if x.gl.unwrap_or(0.0) >= 50.0 { x.gc } else { None }), 0.95);
    let gpu_throttle = sm
        .iter()
        .filter(|x| {
            x.gthr && x.gl.unwrap_or(0.0) >= 50.0
                || (x.gl.unwrap_or(0.0) >= 80.0
                    && gc_ref.is_some_and(|r| x.gc.is_some_and(|c| c < r * 0.8))
                    && x.gt.is_some_and(|t| t >= s.gpu_limit - 3.0))
        })
        .count();

    // ---------- Procesos ----------
    let mut procs: Vec<(&String, &ProcStats)> = s.procs.iter().collect();
    let nn = n.max(1) as f64;
    procs.sort_by(|a, b| b.1.sum_cpu.total_cmp(&a.1.sum_cpu));
    let bg_heavy: Vec<(&String, f32, f32)> = procs
        .iter()
        .filter(|(k, _)| main_app.as_ref().is_none_or(|m| &m.0 != *k) && k.as_str() != "medidor-recursos.exe")
        .map(|(k, v)| (*k, (v.sum_cpu_bg / nn) as f32, v.max_cpu_bg))
        .filter(|(_, a, _)| *a >= 3.0)
        .collect();

    // ---------- Diagnóstico ----------
    let mut f: Vec<Finding> = Vec::new();
    let hhmm = |i: Option<usize>| i.map(|i| sm[i].h.clone()).unwrap_or_default();

    match max(&ct) {
        None => f.push(Finding {
            level: "info",
            title: "No se pudo leer la temperatura de la CPU".into(),
            detail: "Abre LibreHardwareMonitor (como administrador) antes de iniciar el monitor para obtener temperaturas, frecuencias y consumo de todos los componentes. Ver el MANUAL.".into(),
        }),
        Some(m) => {
            let ep = cpu_hot.iter().map(|e| sm[e.end].t - sm[e.start].t + s.interval).sum::<f64>();
            let who = ct_max_i
                .map(|i| culprits(sm, i.saturating_sub(15), i, 3))
                .unwrap_or_default()
                .iter()
                .map(|(k, c)| format!("{k} ({c:.0}%)"))
                .collect::<Vec<_>>()
                .join(", ");
            let base = format!(
                "Máxima {m:.0} °C a las {}. Media {}. Tiempo por encima de {:.0} °C: {}. En el momento del máximo consumían más CPU: {}.",
                hhmm(ct_max_i),
                o(avg(&ct), " °C"),
                s.cpu_limit,
                dur(ep),
                if who.is_empty() { "—".into() } else { who }
            );
            let (level, title) = if m >= 95.0 {
                ("grave", "La CPU llega a temperatura crítica")
            } else if m >= s.cpu_limit {
                ("aviso", "La CPU se calienta más de lo recomendable")
            } else {
                ("ok", "Temperatura de CPU correcta")
            };
            let extra = if m >= s.cpu_limit {
                " Revisa: polvo en disipador y ventiladores, pasta térmica (si tiene años), flujo de aire de la caja y curva de ventiladores en la BIOS."
            } else {
                ""
            };
            f.push(Finding { level, title: title.into(), detail: format!("{base}{extra}") });
        }
    }
    if s.cpu_limit > 0.0 && ct.iter().all(|&t| (t - ct[0]).abs() < 0.5) && ct.len() > 10 && !s.has_hwmon {
        f.push(Finding {
            level: "info",
            title: "La temperatura ACPI no cambia".into(),
            detail: "Windows está dando un valor fijo (habitual en sobremesa). No es fiable: usa LibreHardwareMonitor.".into(),
        });
    }

    if let Some(m) = max(&gt) {
        let hot = max(&gh).map(|h| format!(" Hot spot máximo: {h:.0} °C.")).unwrap_or_default();
        let (level, title) = if m >= 90.0 {
            ("grave", "La GPU llega a temperatura crítica")
        } else if m >= s.gpu_limit {
            ("aviso", "La GPU se calienta bastante")
        } else {
            ("ok", "Temperatura de GPU correcta")
        };
        let extra = if m >= s.gpu_limit {
            " Limpia los ventiladores de la gráfica, mejora la ventilación de la caja o ajusta su curva de ventilador (MSI Afterburner / Adrenalin)."
        } else {
            ""
        };
        f.push(Finding {
            level,
            title: title.into(),
            detail: format!("Máxima {m:.0} °C a las {}. Media {}.{hot}{extra}", hhmm(gt_max_i), o(avg(&gt), " °C")),
        });
    } else {
        f.push(Finding {
            level: "info",
            title: "Sin datos de la GPU".into(),
            detail: "Con gráficas AMD/Intel hace falta LibreHardwareMonitor abierto; con NVIDIA se lee directamente del driver.".into(),
        });
    }

    if cpu_throttle > 0 {
        f.push(Finding {
            level: "grave",
            title: "Posible thermal throttling de la CPU".into(),
            detail: format!(
                "En {} muestras ({}) la frecuencia de la CPU cayó más de un 20 % con carga alta y temperatura alta. Esto produce bajadas de FPS directamente.",
                cpu_throttle,
                dur(cpu_throttle as f64 * s.interval)
            ),
        });
    }
    if gpu_throttle > 0 {
        f.push(Finding {
            level: "grave",
            title: "La GPU se limitó por temperatura/energía".into(),
            detail: format!(
                "Detectado en {} muestras ({}). La gráfica bajó su frecuencia para protegerse, lo que reduce los FPS.",
                gpu_throttle,
                dur(gpu_throttle as f64 * s.interval)
            ),
        });
    }

    if let Some(p95) = pct(&cpu, 0.95) {
        if p95 >= 90.0 || !cpu_sat.is_empty() {
            let (a, b) = cpu_sat.iter().max_by_key(|e| e.end - e.start).map(|e| (e.start, e.end)).unwrap_or((0, n - 1));
            let who = culprits(sm, a, b, 3).iter().map(|(k, c)| format!("{k} ({c:.0}%)")).collect::<Vec<_>>().join(", ");
            f.push(Finding {
                level: "aviso",
                title: "La CPU llega al 100 % de uso".into(),
                detail: format!(
                    "{} episodios de CPU por encima del 90 %. Procesos responsables: {who}. Si el juego es uno de ellos, la CPU es el cuello de botella; si no, ciérralos.",
                    cpu_sat.len()
                ),
            });
        }
    }

    if let Some(a) = avg(&series(sm, |x| if main_app.as_ref().is_some_and(|m| x.fg.as_ref() == Some(&m.0)) { x.gl } else { None })) {
        if a >= 95.0 {
            f.push(Finding {
                level: "info",
                title: "La GPU va al máximo mientras juegas".into(),
                detail: format!(
                    "Uso medio de GPU con {} en primer plano: {a:.0} %. Los FPS están limitados por la gráfica (es normal); bajar calidad gráfica o resolución sube los FPS.",
                    main_app.as_ref().map(|m| m.0.as_str()).unwrap_or("")
                ),
            });
        }
    }

    if let Some(m) = max(&ram) {
        if m >= 90.0 {
            f.push(Finding {
                level: "grave",
                title: "Memoria RAM casi llena".into(),
                detail: format!(
                    "Llegó al {m:.0} % ({} episodios). Cuando la RAM se llena Windows usa el disco y aparecen tirones. Cierra programas o amplía la RAM ({:.0} GB actuales).",
                    ram_full.len(),
                    s.total_ram_gb
                ),
            });
        } else if m >= 80.0 {
            f.push(Finding {
                level: "aviso",
                title: "Uso de RAM alto".into(),
                detail: format!("Máximo {m:.0} % de {:.0} GB. Vigila los programas abiertos mientras juegas.", s.total_ram_gb),
            });
        }
    }

    if !bg_heavy.is_empty() {
        let list = bg_heavy
            .iter()
            .take(6)
            .map(|(k, a, mx)| {
                let d = describe(k);
                format!("<b>{}</b>: media {a:.1} %, pico {mx:.0} %{}", esc(k), if d.is_empty() { String::new() } else { format!(" — {d}") })
            })
            .collect::<Vec<_>>()
            .join("<br>");
        f.push(Finding {
            level: "aviso",
            title: "Procesos en segundo plano consumiendo CPU".into(),
            detail: list,
        });
    }

    let bg_spikes: Vec<_> = s.spikes.iter().filter(|x| main_app.as_ref().is_none_or(|m| x.name != m.0)).collect();
    if !bg_spikes.is_empty() {
        // nombre -> (nº de picos, pico máximo, segundos totales)
        let mut by: HashMap<&str, (usize, f32, f64)> = HashMap::new();
        for x in &bg_spikes {
            let e = by.entry(x.name.as_str()).or_default();
            e.0 += 1;
            e.1 = e.1.max(x.peak);
            e.2 += x.end - x.start + s.interval;
        }
        let mut c: Vec<_> = by.into_iter().collect();
        c.sort_by(|a, b| (b.1.2 * b.1.1 as f64).total_cmp(&(a.1.2 * a.1.1 as f64)));
        let list = c
            .iter()
            .take(5)
            .map(|(k, (n, peak, secs))| {
                let d = describe(k);
                format!(
                    "<b>{}</b>: {n} pico(s), hasta {peak:.0} %, {} en total{}",
                    esc(k),
                    dur(*secs),
                    if d.is_empty() { String::new() } else { format!(" — {d}") }
                )
            })
            .collect::<Vec<_>>()
            .join("<br>");
        f.push(Finding {
            level: "aviso",
            title: "Picos de CPU en segundo plano (posibles tirones)".into(),
            detail: format!(
                "Procesos que no eran la ventana activa y superaron el 15 % de la CPU total. Estos picos suelen coincidir con tirones o bajadas de FPS:<br>{list}"
            ),
        });
    }

    let disk_bg: Vec<_> = procs
        .iter()
        .filter(|(k, v)| (v.disk_read_mb + v.disk_write_mb) > 2000.0 && main_app.as_ref().is_none_or(|m| &m.0 != *k))
        .map(|(k, v)| format!("{} ({:.1} GB)", esc(k), (v.disk_read_mb + v.disk_write_mb) / 1024.0))
        .take(4)
        .collect();
    if !disk_bg.is_empty() {
        f.push(Finding {
            level: "info",
            title: "Mucha actividad de disco en segundo plano".into(),
            detail: format!(
                "Procesos con más de 2 GB leídos/escritos: {}. En discos HDD esto provoca tirones en juegos que cargan datos.",
                disk_bg.join(", ")
            ),
        });
    }

    if f.iter().all(|x| x.level == "ok" || x.level == "info") {
        f.push(Finding {
            level: "ok",
            title: "No se detectan problemas claros".into(),
            detail: "Si hubo bajadas de FPS, compáralas con la hora en las gráficas (pasa el ratón por encima). Prueba también a monitorizar una sesión más larga.".into(),
        });
    }
    // grave primero
    let rank = |l: &str| match l {
        "grave" => 0,
        "aviso" => 1,
        "info" => 2,
        _ => 3,
    };
    f.sort_by_key(|x| rank(x.level));

    // ---------- HTML: piezas ----------
    let mut findings_html = String::new();
    for x in &f {
        let _ = write!(
            findings_html,
            r#"<div class="find {}"><span class="tag">{}</span><div><h3>{}</h3><p>{}</p></div></div>"#,
            x.level,
            match x.level {
                "grave" => "Grave",
                "aviso" => "Aviso",
                "ok" => "OK",
                _ => "Info",
            },
            esc(&x.title),
            x.detail
        );
    }

    let kpi = |label: &str, value: String, sub: String| {
        format!(r#"<div class="kpi"><div class="kl">{label}</div><div class="kv">{value}</div><div class="ks">{sub}</div></div>"#)
    };
    let mut kpis = String::new();
    kpis += &kpi("Temp. CPU máx.", o(max(&ct), " °C"), format!("media {}", o(avg(&ct), " °C")));
    kpis += &kpi("Temp. GPU máx.", o(max(&gt), " °C"), format!("media {}", o(avg(&gt), " °C")));
    kpis += &kpi("Uso CPU", o(avg(&cpu), " %"), format!("máx. {}", o(max(&cpu), " %")));
    kpis += &kpi("Uso GPU", o(avg(&gl), " %"), format!("máx. {}", o(max(&gl), " %")));
    kpis += &kpi("RAM máx.", o(max(&ram), " %"), format!("de {:.0} GB", s.total_ram_gb));
    kpis += &kpi(
        "App principal",
        main_app.as_ref().map(|m| esc(&m.0)).unwrap_or("—".into()),
        main_app.as_ref().map(|m| format!("{} en primer plano", dur(m.1 as f64 * s.interval))).unwrap_or_default(),
    );

    // Tabla de episodios de calor y saturación
    let mut ep_rows = String::new();
    let mut push_ep = |kind: &str, unit: &str, e: &Episode| {
        let who = culprits(sm, e.start.saturating_sub(15), e.end, 4)
            .iter()
            .map(|(k, c)| format!("{} <span class=dim>{c:.0}%</span>", esc(k)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(
            ep_rows,
            "<tr><td>{kind}</td><td>{}</td><td>{}</td><td class=n>{:.0}{unit}</td><td>{}</td><td>{who}</td></tr>",
            sm[e.start].h,
            dur(sm[e.end].t - sm[e.start].t + s.interval),
            e.max,
            main_fg(sm, e.start, e.end).map(|x| esc(&x)).unwrap_or("—".into())
        );
    };
    for e in &cpu_hot {
        push_ep("🔥 CPU caliente", " °C", e);
    }
    for e in &gpu_hot {
        push_ep("🔥 GPU caliente", " °C", e);
    }
    for e in &cpu_sat {
        push_ep("⚙️ CPU al 90 %+", " %", e);
    }
    for e in &ram_full {
        push_ep("💾 RAM llena", " %", e);
    }
    let ep_section = if ep_rows.is_empty() {
        "<p class=dim>No hubo episodios de temperatura alta, CPU saturada ni RAM llena.</p>".to_string()
    } else {
        format!(
            "<p class=dim>Los responsables se calculan con el consumo de CPU desde 30 s antes del inicio hasta el final del episodio.</p><div class=tw><table><tr><th>Tipo</th><th>Inicio</th><th>Duración</th><th>Máx.</th><th>Ventana activa</th><th>Procesos que más CPU usaron</th></tr>{ep_rows}</table></div>"
        )
    };

    // Tabla de procesos
    let mut proc_rows = String::new();
    for (k, v) in procs.iter().take(25) {
        let avg_cpu = v.sum_cpu / nn;
        if avg_cpu < 0.05 && v.max_cpu < 1.0 {
            continue;
        }
        let d = describe(k);
        let _ = write!(
            proc_rows,
            "<tr><td><b>{}</b>{}{}</td><td class=n>{:.1} %</td><td class=n>{:.0} %</td><td class=n>{:.1} %</td><td class=n>{:.0} MB</td><td class=n>{:.0} MB</td><td class=n>{:.0} %</td></tr>",
            esc(k),
            if v.instances_max > 1 { format!(" <span class=dim>×{}</span>", v.instances_max) } else { String::new() },
            if d.is_empty() { String::new() } else { format!("<div class=desc>{d}</div>") },
            avg_cpu,
            v.max_cpu,
            v.sum_cpu_bg / nn,
            v.max_mem,
            v.disk_read_mb + v.disk_write_mb,
            v.fg_samples as f64 / nn * 100.0
        );
    }

    let mut mem_rows = String::new();
    let mut by_mem = procs.clone();
    by_mem.sort_by(|a, b| b.1.max_mem.total_cmp(&a.1.max_mem));
    for (k, v) in by_mem.iter().take(10) {
        let _ = write!(
            mem_rows,
            "<tr><td>{}</td><td class=n>{:.0} MB</td><td class=n>{:.0} MB</td></tr>",
            esc(k),
            v.sum_mem / v.samples.max(1) as f64,
            v.max_mem
        );
    }

    // Picos en segundo plano
    let mut spikes = s.spikes.clone();
    spikes.sort_by(|a, b| b.peak.total_cmp(&a.peak));
    let mut spike_rows = String::new();
    for x in spikes.iter().take(20) {
        let _ = write!(
            spike_rows,
            "<tr><td>{}</td><td><b>{}</b></td><td class=n>{:.0} %</td><td>{}</td><td>{}</td></tr>",
            x.hora,
            esc(&x.name),
            x.peak,
            dur(x.end - x.start + s.interval),
            x.fg.as_deref().map(esc).unwrap_or("—".into())
        );
    }
    let spike_section = if spike_rows.is_empty() {
        "<p class=dim>Ningún proceso en segundo plano superó el 15 % de CPU.</p>".into()
    } else {
        format!("<div class=tw><table><tr><th>Hora</th><th>Proceso</th><th>Pico CPU</th><th>Duración</th><th>Ventana activa</th></tr>{spike_rows}</table></div>")
    };

    // Aplicaciones en primer plano
    let mut fg: Vec<_> = s.procs.iter().filter(|(_, v)| v.fg_samples > 0).collect();
    fg.sort_by(|a, b| b.1.fg_samples.cmp(&a.1.fg_samples));
    let mut fg_rows = String::new();
    for (k, v) in fg.iter().take(10) {
        let _ = write!(
            fg_rows,
            "<tr><td>{}</td><td class=n>{}</td><td class=n>{:.0} %</td></tr>",
            esc(k),
            dur(v.fg_samples as f64 * s.interval),
            v.fg_samples as f64 / nn * 100.0
        );
    }

    // Datos para las gráficas (submuestreados conservando los picos)
    let step = n.div_ceil(MAX_POINTS).max(1);
    let pts: Vec<&Sample> = sm
        .chunks(step)
        .map(|c| {
            c.iter()
                .max_by(|a, b| {
                    let ka = a.ct.or(a.gt).unwrap_or(a.cpu);
                    let kb = b.ct.or(b.gt).unwrap_or(b.cpu);
                    ka.total_cmp(&kb)
                })
                .unwrap()
        })
        .collect();
    let data = serde_json::json!({
        "s": pts,
        "bands": bands,
        "cpuLimit": s.cpu_limit,
        "gpuLimit": s.gpu_limit,
    });
    let data_js = data.to_string().replace("</", "<\\/");

    let equipo = format!(
        "{} · {} núcleos lógicos · {:.0} GB RAM{}",
        esc(&s.cpu_name),
        s.ncpu,
        s.total_ram_gb,
        s.gpu_name.as_deref().map(|g| format!(" · {}", esc(g))).unwrap_or_default()
    );

    TEMPLATE
        .replace("{{FECHA}}", &s.start.format("%d/%m/%Y").to_string())
        .replace("{{INICIO}}", &s.start.format("%H:%M:%S").to_string())
        .replace("{{FIN}}", &s.end.format("%H:%M:%S").to_string())
        .replace("{{DURACION}}", &dur(total_t))
        .replace("{{MUESTRAS}}", &n.to_string())
        .replace("{{INTERVALO}}", &format!("{}", s.interval))
        .replace("{{EQUIPO}}", &equipo)
        .replace("{{FUENTES}}", &esc(&s.sources.join(", ")))
        .replace("{{FINDINGS}}", &findings_html)
        .replace("{{KPIS}}", &kpis)
        .replace("{{EPISODIOS}}", &ep_section)
        .replace("{{PROCS}}", &proc_rows)
        .replace("{{MEM}}", &mem_rows)
        .replace("{{SPIKES}}", &spike_section)
        .replace("{{FG}}", &fg_rows)
        .replace("{{OWN}}", &format!("{:.2} % de CPU de media y {:.0} MB de RAM", s.own_cpu_avg, s.own_mem_mb))
        .replace("{{CSV}}", &esc(&s.csv_path.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default()))
        .replace("{{DATA}}", &data_js)
}

const TEMPLATE: &str = include_str!("informe.html");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::Spike;
    use chrono::Local;
    use std::collections::BTreeMap;

    /// Genera un informe de ejemplo con datos simulados (juego que calienta la CPU
    /// y un análisis del antivirus en segundo plano) en `ejemplo/informe_ejemplo.html`.
    #[test]
    fn informe_ejemplo() {
        let mut samples = Vec::new();
        let mut procs: HashMap<String, ProcStats> = HashMap::new();
        for i in 0..900 {
            let t = i as f64 * 2.0;
            let game = i > 60;
            let av = (400..470).contains(&i);
            let heat = if game { 62.0 + (i.min(500) as f32 - 60.0) / 14.0 } else { 45.0 };
            let ct = heat + if av { 9.0 } else { 0.0 } + ((i * 7) % 5) as f32 * 0.6;
            let game_cpu = if game { 48.0 + ((i * 13) % 9) as f32 } else { 0.0 };
            let av_cpu = if av { 28.0 } else { 0.4 };
            let cpu = 6.0 + game_cpu + av_cpu;
            let mut top = vec![("juego.exe".to_string(), game_cpu, 5200.0), ("MsMpEng.exe".to_string(), av_cpu, 310.0), ("chrome.exe".to_string(), 3.1, 1400.0)];
            top.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut ot = BTreeMap::new();
            ot.insert("Disco Samsung SSD 860".to_string(), 38.0 + i as f32 / 200.0);
            let cc = if ct > 90.0 { 2600.0 } else { 3500.0 };
            let fg = if game { "juego.exe" } else { "explorer.exe" };
            samples.push(Sample {
                t,
                h: format!("20:{:02}:{:02}", (i * 2 / 60) % 60, (i * 2) % 60),
                cpu,
                ram: 62.0 + if av { 6.0 } else { 0.0 },
                ram_gb: 9.9,
                dr: if av { 140.0 } else { 3.0 },
                dw: 1.5,
                ct: Some(ct),
                cc: Some(cc),
                cw: Some(cpu * 0.9),
                gt: Some(if game { 70.0 + i.min(300) as f32 / 30.0 } else { 40.0 }),
                gh: Some(if game { 82.0 + i.min(300) as f32 / 30.0 } else { 48.0 }),
                gl: Some(if game { 97.0 } else { 3.0 }),
                gw: Some(if game { 150.0 } else { 15.0 }),
                gc: Some(if game { 1850.0 } else { 300.0 }),
                gm: Some(if game { 6100.0 } else { 600.0 }),
                gthr: false,
                ot,
                top: top.clone(),
                fg: Some(fg.to_string()),
            });
            for (n, c, m) in &top {
                let st = procs.entry(n.clone()).or_default();
                st.samples += 1;
                st.sum_cpu += *c as f64;
                st.max_cpu = st.max_cpu.max(*c);
                st.sum_mem += *m as f64;
                st.max_mem = st.max_mem.max(*m);
                if n == fg {
                    st.fg_samples += 1;
                } else {
                    st.sum_cpu_bg += *c as f64;
                    st.max_cpu_bg = st.max_cpu_bg.max(*c);
                }
                st.instances_max = if n == "chrome.exe" { 14 } else { 1 };
            }
            if av {
                procs.get_mut("MsMpEng.exe").unwrap().disk_read_mb += 280.0;
            }
        }
        procs.entry("explorer.exe".into()).or_default().fg_samples = 61;
        let now = Local::now();
        let s = Session {
            start: now,
            end: now,
            interval: 2.0,
            cpu_name: "CPU de ejemplo 6 núcleos".into(),
            gpu_name: Some("GPU de ejemplo".into()),
            ncpu: 12,
            total_ram_gb: 16.0,
            sources: vec!["datos simulados".into()],
            has_hwmon: true,
            cpu_limit: 85.0,
            gpu_limit: 83.0,
            samples,
            procs,
            spikes: vec![Spike { name: "MsMpEng.exe".into(), start: 800.0, end: 940.0, peak: 31.0, hora: "20:13:20".into(), fg: Some("juego.exe".into()) }],
            own_cpu_avg: 0.3,
            own_mem_mb: 9.0,
            csv_path: "datos_ejemplo.csv".into(),
        };
        let html = build(&s);
        assert!(html.contains("Antivirus Microsoft Defender"));
        assert!(html.contains("thermal throttling"));
        assert!(!html.contains("{{"));
        std::fs::create_dir_all("ejemplo").unwrap();
        std::fs::write("ejemplo/informe_ejemplo.html", html).unwrap();
    }
}
