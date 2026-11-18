//! Informe HTML de una sesión (un único archivo, sin internet).

use crate::analysis::{Analysis, Episode, Kind, Level, analyze, describe, dur, esc};
use crate::monitor::{Sample, Session};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::Path;

const MAX_POINTS: usize = 2000;
pub const CSS: &str = include_str!("estilo.css");
pub const CHARTS_JS: &str = include_str!("charts.js");
const TEMPLATE: &str = include_str!("informe.html");

#[derive(Serialize)]
struct Band {
    a: f64,
    b: f64,
    kind: &'static str,
}

pub fn write(s: &Session, path: &Path) -> std::io::Result<()> {
    std::fs::write(path, build(s))
}

fn o(v: Option<f32>, unit: &str) -> String {
    v.map(|x| format!("{x:.0}{unit}")).unwrap_or_else(|| "–".into())
}

fn kv(k: &str, v: String) -> String {
    format!("<div><dt>{k}</dt><dd>{v}</dd></div>")
}

fn causes_html(c: &[String]) -> String {
    if c.is_empty() { "<span class=\"muted\">nada destacable en los datos</span>".into() } else { c.join("; ") }
}

/// Submuestrea a MAX_POINTS conservando, en cada tramo, la muestra más "interesante".
fn downsample(sm: &[Sample]) -> Vec<&Sample> {
    let step = sm.len().div_ceil(MAX_POINTS).max(1);
    sm.chunks(step)
        .map(|c| {
            c.iter()
                .max_by(|a, b| {
                    let k = |x: &Sample| x.fps.map(|f| -f).or(x.ct).or(x.gt).unwrap_or(x.cpu);
                    k(a).total_cmp(&k(b))
                })
                .unwrap()
        })
        .collect()
}

pub fn build(s: &Session) -> String {
    let a = analyze(s);
    let sm = &s.samples;
    let n = sm.len();
    let nn = n.max(1) as f64;

    // --- Cabecera ---
    let titulo_app = s.game.clone().or_else(|| a.fps.as_ref().map(|f| f.app.clone())).or_else(|| a.main_app.as_ref().map(|m| m.0.clone()));
    let h1 = match &titulo_app {
        Some(app) => format!("{} · {}", esc(app.trim_end_matches(".exe")), s.start.format("%d/%m/%Y")),
        None => format!("Sesión del {}", s.start.format("%d/%m/%Y")),
    };
    let sub = format!(
        "{}–{} · {} · una muestra cada {} s · {} muestras{}",
        s.start.format("%H:%M"),
        s.end.format("%H:%M"),
        dur(a.duration),
        s.interval,
        n,
        if s.marks.is_empty() { String::new() } else { format!(" · {}", crate::analysis::plural(s.marks.len(), "marca", "marcas")) }
    );

    // --- Resumen ---
    let at = |i: usize| sm.get(i).map(|x| format!(" <small>{}</small>", x.h)).unwrap_or_default();
    let mut k = String::new();
    if let Some(f) = &a.fps {
        k += &kv("FPS medios", format!("{:.0} <small>1 % low {:.0} · mín. {:.0}</small>", f.avg, f.low1, f.min));
        k += &kv("Tirones", format!("{} <small>caídas {}</small>", f.stutters, a.episodes.iter().filter(|e| e.kind == Kind::FpsDrop).count()));
    } else {
        k += &kv("FPS", "<small>no medidos</small>".into());
    }
    k += &kv("CPU temp. máx.", a.ct.map(|c| format!("{:.0} °C{}", c.max, at(c.max_i))).unwrap_or("–".into()));
    k += &kv("GPU temp. máx.", a.gt.map(|c| format!("{:.0} °C{}", c.max, at(c.max_i))).unwrap_or("–".into()));
    k += &kv("CPU uso medio", format!("{} <small>máx. {}</small>", o(a.cpu.map(|c| c.avg), " %"), o(a.cpu.map(|c| c.max), " %")));
    k += &kv("GPU uso medio", format!("{} <small>máx. {}</small>", o(a.gl.map(|c| c.avg), " %"), o(a.gl.map(|c| c.max), " %")));
    k += &kv("RAM máx.", format!("{} <small>de {:.0} GB</small>", o(a.ram.map(|c| c.max), " %"), s.total_ram_gb));
    k += &kv("VRAM máx.", o(a.vram.map(|c| c.max), " %"));
    k += &kv(
        "Ping medio",
        a.ping
            .map(|p| format!("{:.0} ms <small>P95 {:.0} · perdidos {}</small>", p.avg, p.p95, a.ping_lost))
            .unwrap_or("–".into()),
    );
    k += &kv("Ventiladores", a.fan.map(|(lo, hi)| format!("{lo:.0}–{hi:.0} rpm")).unwrap_or("–".into()));
    k += &kv(
        "Ventana principal",
        a.main_app.as_ref().map(|m| format!("{} <small>{}</small>", esc(&m.0), dur(m.1 as f64 * s.interval))).unwrap_or("–".into()),
    );
    k += &kv("CPU frenada por calor", if a.cpu_throttle > 0 { dur(a.cpu_throttle as f64 * s.interval) } else { "no".into() });

    // --- Hallazgos ---
    let mut fh = String::new();
    for x in &a.findings {
        let cls = match x.level {
            Level::Grave => "grave",
            Level::Aviso => "aviso",
            Level::Info => "info",
            Level::Ok => "ok",
        };
        let _ = write!(fh, r#"<li class="{cls}"><b class="t">{}</b><div class="d">{}</div></li>"#, esc(&x.title), x.detail);
    }

    // --- Marcas ---
    let marcas = if a.marks.is_empty() {
        String::new()
    } else {
        let mut rows = String::new();
        for m in &a.marks {
            let x = &sm[m.sample.min(n.saturating_sub(1))];
            let _ = write!(
                rows,
                "<tr><td class=\"mono\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td>{}{}</td></tr>",
                esc(&m.mark.label),
                m.mark.h,
                o(x.fps, ""),
                o(x.ct, " °C"),
                o(x.gt, " °C"),
                if m.near_drop.is_some() { "<span class=\"tag\">caída de FPS</span> " } else { "" },
                causes_html(&m.causes)
            );
        }
        format!(
            "<h2>Marcas</h2><p class=\"muted\">Momentos marcados con Ctrl+Alt+M. Se muestra lo que pasaba en los segundos previos.</p><div class=\"tw\"><table><tr><th>Marca</th><th class=\"n\">Hora</th><th class=\"n\">FPS</th><th class=\"n\">CPU</th><th class=\"n\">GPU</th><th>Qué pasaba</th></tr>{rows}</table></div>"
        )
    };

    // --- Caídas de FPS ---
    let drops: Vec<&Episode> = a.episodes.iter().filter(|e| e.kind == Kind::FpsDrop).collect();
    let caidas = if a.fps.is_none() {
        String::new()
    } else if drops.is_empty() {
        "<h2>Caídas de FPS</h2><p class=\"muted\">No hubo caídas por debajo del 75 % de los FPS habituales ni tirones repetidos.</p>".into()
    } else {
        let f = a.fps.as_ref().unwrap();
        let mut rows = String::new();
        let mut sorted = drops.clone();
        sorted.sort_by(|x, y| x.peak.total_cmp(&y.peak));
        for e in sorted.iter().take(30) {
            let stut: u32 = sm[e.start..=e.end].iter().map(|x| x.stut).sum();
            let _ = write!(
                rows,
                "<tr><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{:.0}</td><td class=\"n\">{}</td><td>{}</td></tr>",
                sm[e.start].h,
                dur(sm[e.end].t - sm[e.start].t + s.interval),
                e.peak,
                stut,
                causes_html(&e.causes)
            );
        }
        format!(
            "<h2>Caídas de FPS</h2><p class=\"muted\">Tramos por debajo de {:.0} FPS (75 % de la mediana, {:.0}) o con tirones. Ordenadas de peor a menos mala{}.</p><div class=\"tw\"><table><tr><th class=\"n\">Inicio</th><th class=\"n\">Duración</th><th class=\"n\">FPS mín.</th><th class=\"n\">Tirones</th><th>Qué pasaba</th></tr>{rows}</table></div>",
            f.median * 0.75,
            f.median,
            if drops.len() > 30 { format!("; se muestran 30 de {}", drops.len()) } else { String::new() }
        )
    };

    // --- Episodios de temperatura/carga ---
    let mut ep_rows = String::new();
    for e in a.episodes.iter().filter(|e| e.kind != Kind::FpsDrop) {
        let unit = if matches!(e.kind, Kind::CpuHot | Kind::GpuHot) { " °C" } else { " %" };
        let who = e.culprits.iter().map(|(k, c)| format!("{} <span class=\"muted\">{c:.0} %</span>", esc(k))).collect::<Vec<_>>().join(", ");
        let _ = write!(
            ep_rows,
            "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{:.0}{unit}</td><td>{}</td><td>{who}</td></tr>",
            e.kind.label(),
            sm[e.start].h,
            dur(sm[e.end].t - sm[e.start].t + s.interval),
            e.peak,
            e.fg.as_deref().map(esc).unwrap_or("–".into())
        );
    }
    let episodios = if ep_rows.is_empty() {
        "<p class=\"muted\">Ninguno: sin temperaturas por encima del límite, CPU por debajo del 90 % y RAM por debajo del 90 %.</p>".to_string()
    } else {
        format!(
            "<p class=\"muted\">Consumo de CPU contado desde 30 s antes del inicio de cada episodio.</p><div class=\"tw\"><table><tr><th>Tipo</th><th class=\"n\">Inicio</th><th class=\"n\">Duración</th><th class=\"n\">Máx.</th><th>Ventana activa</th><th>Procesos con más CPU</th></tr>{ep_rows}</table></div>"
        )
    };

    // --- Procesos ---
    let mut procs: Vec<_> = s.procs.iter().collect();
    procs.sort_by(|x, y| (y.1.sum_cpu + y.1.sum_gpu).total_cmp(&(x.1.sum_cpu + x.1.sum_gpu)));
    let mut proc_rows = String::new();
    for (k, v) in procs.iter().take(25) {
        let (cpu, gpu) = (v.sum_cpu / nn, v.sum_gpu / nn);
        if cpu < 0.05 && v.max_cpu < 1.0 && v.max_gpu < 1.0 {
            continue;
        }
        let d = describe(k);
        let _ = write!(
            proc_rows,
            "<tr><td>{}{}{}</td><td class=\"n\">{cpu:.1}</td><td class=\"n\">{:.0}</td><td class=\"n\">{:.1}</td><td class=\"n\">{gpu:.1}</td><td class=\"n\">{:.0}</td><td class=\"n\">{:.0} MB</td><td class=\"n\">{:.0} MB</td><td class=\"n\">{:.0} %</td></tr>",
            esc(k),
            if v.instances_max > 1 { format!(" <span class=\"muted\">×{}</span>", v.instances_max) } else { String::new() },
            if d.is_empty() { String::new() } else { format!("<span class=\"note\">{d}</span>") },
            v.max_cpu,
            v.sum_cpu_bg / nn,
            v.max_gpu,
            v.max_mem,
            v.disk_read_mb + v.disk_write_mb,
            v.fg_samples as f64 / nn * 100.0
        );
    }
    let mut mem_rows = String::new();
    let mut by_mem = procs.clone();
    by_mem.sort_by(|x, y| y.1.max_mem.total_cmp(&x.1.max_mem));
    for (k, v) in by_mem.iter().take(8) {
        let _ = write!(
            mem_rows,
            "<tr><td>{}</td><td class=\"n\">{:.0} MB</td><td class=\"n\">{:.0} MB</td></tr>",
            esc(k),
            v.sum_mem / v.samples.max(1) as f64,
            v.max_mem
        );
    }

    let mut spikes = s.spikes.clone();
    spikes.sort_by(|x, y| y.peak.total_cmp(&x.peak));
    let mut spike_rows = String::new();
    for x in spikes.iter().take(15) {
        let _ = write!(
            spike_rows,
            "<tr><td class=\"n\">{}</td><td>{}</td><td class=\"n\">{:.0} %</td><td class=\"n\">{}</td></tr>",
            x.hora,
            esc(&x.name),
            x.peak,
            dur(x.end - x.start + s.interval)
        );
    }
    let spike_section = if spike_rows.is_empty() {
        "<p class=\"muted\">Ningún proceso en segundo plano pasó del 15 % de CPU.</p>".to_string()
    } else {
        format!("<div class=\"tw\"><table><tr><th class=\"n\">Hora</th><th>Proceso</th><th class=\"n\">Pico</th><th class=\"n\">Duración</th></tr>{spike_rows}</table></div>")
    };

    let mut fg: Vec<_> = s.procs.iter().filter(|(_, v)| v.fg_samples > 0).collect();
    fg.sort_by(|x, y| y.1.fg_samples.cmp(&x.1.fg_samples));
    let mut fg_rows = String::new();
    for (k, v) in fg.iter().take(8) {
        let _ = write!(
            fg_rows,
            "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{:.0} %</td></tr>",
            esc(k),
            dur(v.fg_samples as f64 * s.interval),
            v.fg_samples as f64 / nn * 100.0
        );
    }

    // --- Pie: equipo y fuentes ---
    let mut foot = String::new();
    let _ = write!(foot, "<dt>Equipo</dt><dd>{} · {} hilos · {:.0} GB RAM{}</dd>", esc(&s.cpu_name), s.ncpu, s.total_ram_gb,
        s.gpu_name.as_deref().map(|g| format!(" · {}", esc(g))).unwrap_or_default());
    let _ = write!(foot, "<dt>Sensores</dt><dd>{}</dd>", esc(&s.sources.join(", ")));
    let _ = write!(foot, "<dt>FPS</dt><dd>{}</dd>", match &s.fps_unavailable {
        None => "ETW (eventos Present de DXGI y D3D9)".to_string(),
        Some(w) => format!("no medidos: {}", esc(w)),
    });
    let _ = write!(foot, "<dt>Ping</dt><dd>{}</dd>", s.ping_host.as_deref().map(esc).unwrap_or("desactivado".into()));
    let _ = write!(foot, "<dt>Límites</dt><dd>CPU {:.0} °C · GPU {:.0} °C</dd>", s.cpu_limit, s.gpu_limit);
    let _ = write!(foot, "<dt>Este monitor</dt><dd>{:.2} % de CPU de media, {:.0} MB de RAM</dd>", s.own_cpu_avg, s.own_mem_mb);
    let _ = write!(foot, "<dt>Datos</dt><dd>{}</dd>", esc(&s.csv_path.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default()));

    // --- Datos de las gráficas ---
    let mut bands = Vec::new();
    for e in &a.episodes {
        let kind = match e.kind {
            Kind::FpsDrop => "fps",
            Kind::CpuHot => "cpu",
            Kind::GpuHot => "gpu",
            Kind::CpuBusy => "busy",
            Kind::RamFull => continue,
        };
        bands.push(Band { a: sm[e.start].t - s.interval / 2.0, b: sm[e.end].t + s.interval / 2.0, kind });
    }
    let data = serde_json::json!({
        "s": downsample(sm),
        "marks": s.marks,
        "bands": bands,
        "cpuLimit": s.cpu_limit,
        "gpuLimit": s.gpu_limit,
        "interval": s.interval,
        "clock": s.start.format("%H:%M:%S").to_string(),
        "pingHost": s.ping_host,
    });

    TEMPLATE
        .replace("{{CSS}}", CSS)
        .replace("{{TITULO}}", &format!("{} · informe", titulo_app.as_deref().map(|x| x.trim_end_matches(".exe")).unwrap_or("Sesión")))
        .replace("{{FECHA}}", &s.start.format("%d/%m/%Y %H:%M").to_string())
        .replace("{{H1}}", &h1)
        .replace("{{SUB}}", &sub)
        .replace("{{VEREDICTO}}", &verdict_html(&a))
        .replace("{{KV}}", &k)
        .replace("{{FINDINGS}}", &fh)
        .replace("{{MARCAS}}", &marcas)
        .replace("{{CAIDAS}}", &caidas)
        .replace("{{EPISODIOS}}", &episodios)
        .replace("{{PROCS}}", &proc_rows)
        .replace("{{MEM}}", &mem_rows)
        .replace("{{SPIKES}}", &spike_section)
        .replace("{{FG}}", &fg_rows)
        .replace("{{FOOT}}", &foot)
        .replace("{{CHARTS}}", CHARTS_JS)
        .replace("{{DATA}}", &data.to_string().replace("</", "<\\/"))
}

fn verdict_html(a: &Analysis) -> String {
    a.verdict.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::tests::session;
    use crate::monitor::{Mark, ProcStats, Spike};
    use std::collections::{BTreeMap, HashMap};

    /// Informe de ejemplo con datos simulados en `ejemplo/informe_ejemplo.html`:
    /// un juego que calienta la CPU hasta frenarla, el antivirus metiendo un pico
    /// a mitad de partida (con su caída de FPS y una marca) y Discord usando GPU.
    #[test]
    fn informe_ejemplo() {
        let mut samples = Vec::new();
        let mut procs: HashMap<String, ProcStats> = HashMap::new();
        for i in 0..900usize {
            let game = i > 60;
            let av = (400..430).contains(&i);
            let heat = if game { 62.0 + (i.min(500) as f32 - 60.0) / 14.0 } else { 45.0 };
            let ct = heat + if av { 9.0 } else { 0.0 } + ((i * 7) % 5) as f32 * 0.6;
            let game_cpu = if game { 48.0 + ((i * 13) % 9) as f32 } else { 0.0 };
            let av_cpu = if av { 28.0 } else { 0.4 };
            let cpu = 6.0 + game_cpu + av_cpu;
            let mut top = vec![
                ("juego.exe".to_string(), game_cpu, 5200.0),
                ("MsMpEng.exe".to_string(), av_cpu, 310.0),
                ("chrome.exe".to_string(), 3.1, 1400.0),
                ("Discord.exe".to_string(), 1.2, 450.0),
            ];
            top.sort_by(|a, b| b.1.total_cmp(&a.1));
            let throttled = ct > 92.0;
            let fps = if !game { None } else if av { Some(71.0 + (i % 3) as f32) } else if throttled { Some(98.0) } else { Some(140.0 + (i % 5) as f32) };
            let mut ot = BTreeMap::new();
            ot.insert("Disco Samsung SSD 860".to_string(), 38.0 + i as f32 / 200.0);
            let mut fan = BTreeMap::new();
            fan.insert("Placa base · CPU Fan".to_string(), 900.0 + (ct - 45.0) * 18.0);
            fan.insert("GPU · GPU Fan".to_string(), if game { 1700.0 } else { 0.0 });
            let fg = if game { "juego.exe" } else { "explorer.exe" };
            let gtop = if game { vec![("juego.exe".to_string(), 92.0), ("Discord.exe".to_string(), 7.0)] } else { vec![] };
            samples.push(Sample {
                t: i as f64 * 2.0,
                h: format!("21:{:02}:{:02}", (i * 2 / 60) % 60, (i * 2) % 60),
                cpu,
                ram: 62.0 + if av { 6.0 } else { 0.0 },
                ram_gb: 9.9,
                dr: if av { 140.0 } else { 3.0 },
                dw: 1.5,
                ct: Some(ct),
                cc: Some(if throttled { 2600.0 } else { 3500.0 }),
                cw: Some(cpu * 0.9),
                gt: Some(if game { 70.0 + i.min(300) as f32 / 30.0 } else { 40.0 }),
                gh: Some(if game { 82.0 + i.min(300) as f32 / 30.0 } else { 48.0 }),
                gl: Some(if game { 97.0 } else { 3.0 }),
                gw: Some(if game { 150.0 } else { 15.0 }),
                gc: Some(if game { 1850.0 } else { 300.0 }),
                gm: Some(if game { 6100.0 } else { 600.0 }),
                gmp: Some(if game { 76.0 } else { 8.0 }),
                ot,
                fan,
                top: top.clone(),
                gtop: gtop.clone(),
                fg: Some(fg.to_string()),
                fps,
                fl: fps.map(|f| if av { f * 0.5 } else { f * 0.8 }),
                ftm: fps.map(|f| if av { 95.0 } else { 1000.0 / f * 1.4 }),
                stut: if av { 4 } else { 0 },
                fpp: fps.map(|_| "juego.exe".to_string()),
                nrx: if game { 180.0 } else { 5.0 },
                ntx: if game { 40.0 } else { 2.0 },
                ping: Some(if (600..610).contains(&i) { 180.0 } else { 24.0 + (i % 4) as f32 }),
                plost: if (600..605).contains(&i) { 1 } else { 0 },
                ..Default::default()
            });
            for (nme, c, m) in &top {
                let st = procs.entry(nme.clone()).or_default();
                let g = gtop.iter().find(|x| &x.0 == nme).map_or(0.0, |x| x.1);
                st.samples += 1;
                st.sum_cpu += *c as f64;
                st.max_cpu = st.max_cpu.max(*c);
                st.sum_mem += *m as f64;
                st.max_mem = st.max_mem.max(*m);
                st.sum_gpu += g as f64;
                st.max_gpu = st.max_gpu.max(g);
                if nme == fg {
                    st.fg_samples += 1;
                } else {
                    st.sum_cpu_bg += *c as f64;
                    st.max_cpu_bg = st.max_cpu_bg.max(*c);
                }
                st.instances_max = if nme == "chrome.exe" { 14 } else { 1 };
            }
            if av {
                procs.get_mut("MsMpEng.exe").unwrap().disk_read_mb += 280.0;
            }
        }
        procs.entry("explorer.exe".into()).or_default().fg_samples = 61;
        let mut s = session(samples);
        s.cpu_name = "CPU de ejemplo 6 núcleos".into();
        s.gpu_name = Some("GPU de ejemplo".into());
        s.sources = vec!["datos simulados".into()];
        s.procs = procs;
        s.spikes = vec![Spike { name: "MsMpEng.exe".into(), start: 800.0, end: 858.0, peak: 31.0, hora: "21:13:20".into(), fg: Some("juego.exe".into()) }];
        s.marks = vec![
            Mark { t: 812.0, h: "21:13:32".into(), label: "M1".into() },
            Mark { t: 1500.0, h: "21:25:00".into(), label: "M2".into() },
        ];
        s.game = Some("juego.exe".into());
        s.own_cpu_avg = 0.4;
        s.own_mem_mb = 16.0;
        s.csv_path = "datos_ejemplo.csv".into();

        let html = build(&s);
        assert!(html.contains("Antivirus de Windows"));
        assert!(html.contains("La CPU se frenó por calor"));
        assert!(html.contains("pico de MsMpEng.exe"));
        assert!(html.contains("Caídas de FPS"));
        assert!(!html.contains("{{"));
        assert!(!html.contains('—'), "sin rayas largas en el informe");
        std::fs::create_dir_all("ejemplo").unwrap();
        std::fs::write("ejemplo/informe_ejemplo.html", html).unwrap();
    }
}
