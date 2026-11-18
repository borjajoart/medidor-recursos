//! Comparación de dos sesiones a partir de sus CSV (p. ej. antes y después de limpiar el PC).

use crate::analysis::{dur, esc, percentile};
use crate::report::{CHARTS_JS, CSS};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

const TEMPLATE: &str = include_str!("comparar.html");

/// Columnas que se comparan: (clave CSV, etiqueta, unidad, más alto es mejor)
const METRICS: &[(&str, &str, &str, bool)] = &[
    ("fps", "FPS medios", "", true),
    ("fps_1pct_low", "FPS 1 % low", "", true),
    ("cpu_temp", "Temperatura CPU", " °C", false),
    ("gpu_temp", "Temperatura GPU", " °C", false),
    ("gpu_hotspot", "GPU hot spot", " °C", false),
    ("cpu_%", "Uso de CPU", " %", false),
    ("gpu_%", "Uso de GPU", " %", false),
    ("cpu_mhz", "Frecuencia CPU", " MHz", true),
    ("ram_%", "RAM", " %", false),
    ("vram_pct", "VRAM", " %", false),
    ("ping_ms", "Ping", " ms", false),
    ("vent_max_rpm", "Ventilador más rápido", " rpm", false),
];

pub struct Csv {
    pub name: String,
    pub start: String,
    pub t: Vec<f64>,
    pub cols: HashMap<String, Vec<Option<f32>>>,
    pub tirones: u32,
}

fn parse_num(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() { None } else { s.replace(',', ".").parse().ok() }
}

pub fn read_csv(path: &Path) -> Result<Csv, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_csv(&text, &path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

pub fn parse_csv(text: &str, name: &str) -> Result<Csv, String> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().ok_or("CSV vacío")?.split(';').collect();
    let idx = |k: &str| header.iter().position(|h| *h == k);
    let ti = idx("segundos").ok_or("el CSV no tiene la columna 'segundos' (¿es un CSV del medidor?)")?;
    let hi = idx("hora");
    let wanted: Vec<(&str, usize)> = METRICS.iter().filter_map(|m| idx(m.0).map(|i| (m.0, i))).collect();
    let st = idx("tirones");
    let mut c = Csv { name: name.to_string(), start: String::new(), t: vec![], cols: HashMap::new(), tirones: 0 };
    for line in lines.filter(|l| !l.trim().is_empty()) {
        // los nombres de proceso no contienen ';', así que el split es fiable
        let f: Vec<&str> = line.split(';').collect();
        let Some(t) = f.get(ti).and_then(|x| x.trim().parse::<f64>().ok()) else { continue };
        if c.t.is_empty() {
            c.start = hi.and_then(|i| f.get(i)).map(|s| s.to_string()).unwrap_or_default();
        }
        c.t.push(t);
        for (k, i) in &wanted {
            c.cols.entry(k.to_string()).or_default().push(f.get(*i).and_then(|x| parse_num(x)));
        }
        if let Some(i) = st {
            c.tirones += f.get(i).and_then(|x| x.trim().parse::<u32>().ok()).unwrap_or(0);
        }
    }
    if c.t.is_empty() {
        return Err(format!("{name}: el CSV no tiene muestras"));
    }
    Ok(c)
}

fn vals(c: &Csv, k: &str) -> Vec<f32> {
    c.cols.get(k).map(|v| v.iter().flatten().copied().collect()).unwrap_or_default()
}

pub fn build(a: &Csv, b: &Csv) -> String {
    let mut rows = String::new();
    for (k, label, unit, higher_better) in METRICS {
        let (va, vb) = (vals(a, k), vals(b, k));
        if va.is_empty() && vb.is_empty() {
            continue;
        }
        let avg = |v: &[f32]| (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32);
        let mx = |v: &[f32]| percentile(v, 1.0);
        let (aa, ab, ma, mb) = (avg(&va), avg(&vb), mx(&va), mx(&vb));
        let f = |v: Option<f32>| v.map(|x| format!("{x:.0}{unit}")).unwrap_or("–".into());
        let diff = match (aa, ab) {
            (Some(x), Some(y)) => {
                let d = y - x;
                let better = if *higher_better { d > 0.0 } else { d < 0.0 };
                let cls = if d.abs() < 0.5 { "" } else if better { "better" } else { "worse" };
                format!("<span class=\"{cls}\">{}{d:.0}{unit}</span>", if d > 0.0 { "+" } else { "" })
            }
            _ => "–".into(),
        };
        let _ = write!(
            rows,
            "<tr><td>{label}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{diff}</td></tr>",
            f(aa),
            f(ma),
            f(ab),
            f(mb)
        );
    }
    let _ = write!(rows, "<tr><td>Tirones</td><td class=\"n\">{}</td><td></td><td class=\"n\">{}</td><td></td><td></td></tr>", a.tirones, b.tirones);

    let series = |c: &Csv, k: &str| -> Vec<[f64; 2]> {
        c.cols
            .get(k)
            .map(|v| c.t.iter().zip(v).filter_map(|(t, x)| x.map(|x| [*t, x as f64])).collect())
            .unwrap_or_default()
    };
    let mut data = serde_json::Map::new();
    for (k, ..) in METRICS {
        data.insert(format!("a_{k}"), serde_json::json!(series(a, k)));
        data.insert(format!("b_{k}"), serde_json::json!(series(b, k)));
    }
    let span = |c: &Csv| c.t.last().copied().unwrap_or(0.0);

    TEMPLATE
        .replace("{{CSS}}", CSS)
        .replace("{{A}}", &esc(&a.name))
        .replace("{{B}}", &esc(&b.name))
        .replace("{{AINFO}}", &format!("desde {} · {}", esc(&a.start), dur(span(a))))
        .replace("{{BINFO}}", &format!("desde {} · {}", esc(&b.start), dur(span(b))))
        .replace("{{ROWS}}", &rows)
        .replace("{{CHARTS}}", CHARTS_JS)
        .replace("{{DATA}}", &serde_json::Value::Object(data).to_string().replace("</", "<\\/"))
}

/// Compara dos CSV y escribe el HTML. Devuelve la ruta del informe.
pub fn compare_files(a: &Path, b: &Path, out: Option<&Path>) -> Result<std::path::PathBuf, String> {
    let (ca, cb) = (read_csv(a)?, read_csv(b)?);
    let out = out.map(Path::to_path_buf).unwrap_or_else(|| {
        a.parent().unwrap_or(Path::new(".")).join(format!("comparacion_{}_vs_{}.html", ca.name.trim_start_matches("datos_"), cb.name.trim_start_matches("datos_")))
    });
    std::fs::write(&out, build(&ca, &cb)).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "hora;segundos;cpu_%;cpu_temp;fps;tirones\n20:00:00;0;50,0;90,5;100,0;1\n20:00:02;2;52,0;91,5;102,0;0\n";
    const B: &str = "hora;segundos;cpu_%;cpu_temp;fps;tirones\n21:00:00;0;48,0;70,0;120,0;0\n";

    #[test]
    fn lee_y_compara() {
        let a = parse_csv(A, "antes").unwrap();
        let b = parse_csv(B, "despues").unwrap();
        assert_eq!(a.t, vec![0.0, 2.0]);
        assert_eq!(vals(&a, "cpu_temp"), vec![90.5, 91.5]);
        assert_eq!(a.tirones, 1);
        let html = build(&a, &b);
        assert!(html.contains("Temperatura CPU"));
        assert!(html.contains("<span class=\"better\">-21 °C</span>"), "la temperatura bajó 21 °C");
        assert!(html.contains("<span class=\"better\">+19</span>"), "los FPS subieron");
        assert!(!html.contains("{{"));
    }

    #[test]
    fn rechaza_csv_ajeno() {
        assert!(parse_csv("a;b\n1;2\n", "x").is_err());
    }
}
