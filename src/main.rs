mod monitor;
mod report;
mod sensors;

use std::path::PathBuf;
use std::time::Duration;

pub struct Config {
    pub duration: Option<Duration>,
    pub interval: Duration,
    pub out_dir: PathBuf,
    pub cpu_limit: f32,
    pub gpu_limit: f32,
    pub open_report: bool,
    pub quiet: bool,
}

const HELP: &str = "\
medidor-recursos — monitor ligero de procesos y temperaturas

USO:
  medidor-recursos [opciones]          Monitoriza hasta Ctrl+C (o hasta --duracion)
  medidor-recursos detener             Detiene el monitor que corre en segundo plano
  medidor-recursos estado              Indica si hay un monitor en marcha

OPCIONES:
  -d, --duracion <t>     Tiempo de monitorización: 90s, 30m, 2h, 1h30m (número solo = minutos)
  -i, --intervalo <s>    Segundos entre muestras (por defecto 2)
  -o, --salida <dir>     Carpeta de los informes (por defecto: 'informes' junto al .exe)
      --umbral-cpu <°C>  Temperatura de CPU considerada alta (por defecto 85)
      --umbral-gpu <°C>  Temperatura de GPU considerada alta (por defecto 83)
  -b, --segundo-plano    Se ejecuta sin ventana; se detiene con 'medidor-recursos detener'
      --no-abrir         No abre el informe en el navegador al terminar
  -h, --ayuda            Muestra esta ayuda
";

fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim().to_lowercase();
    if let Ok(n) = s.parse::<f64>() {
        return Some(Duration::from_secs_f64(n * 60.0));
    }
    let mut total = 0.0;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            let n: f64 = num.parse().ok()?;
            num.clear();
            total += n * match c {
                's' => 1.0,
                'm' => 60.0,
                'h' => 3600.0,
                _ => return None,
            };
        }
    }
    if !num.is_empty() {
        return None;
    }
    (total > 0.0).then(|| Duration::from_secs_f64(total))
}

fn default_out_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("informes")))
        .unwrap_or_else(|| PathBuf::from("informes"))
}

fn fail(msg: &str) -> ! {
    eprintln!("Error: {msg}\n\n{HELP}");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = Config {
        duration: None,
        interval: Duration::from_secs(2),
        out_dir: default_out_dir(),
        cpu_limit: 85.0,
        gpu_limit: 83.0,
        open_report: true,
        quiet: false,
    };
    let mut background = false;
    let mut child = false;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().unwrap_or_else(|| fail(&format!("falta el valor de {name}")));
        match a.as_str() {
            "detener" | "stop" => std::process::exit(monitor::request_stop()),
            "estado" | "status" => std::process::exit(monitor::print_status()),
            "-h" | "--ayuda" | "--help" | "/?" => {
                println!("{HELP}");
                return;
            }
            "-d" | "--duracion" => {
                let v = val(a);
                cfg.duration = Some(parse_duration(&v).unwrap_or_else(|| fail(&format!("duración no válida: {v}"))));
            }
            "-i" | "--intervalo" => {
                let v = val(a);
                let s: f64 = v.parse().ok().filter(|s| *s >= 0.5 && *s <= 600.0).unwrap_or_else(|| fail("intervalo entre 0.5 y 600 s"));
                cfg.interval = Duration::from_secs_f64(s);
            }
            "-o" | "--salida" => cfg.out_dir = PathBuf::from(val(a)),
            "--umbral-cpu" => cfg.cpu_limit = val(a).parse().unwrap_or_else(|_| fail("umbral de CPU no válido")),
            "--umbral-gpu" => cfg.gpu_limit = val(a).parse().unwrap_or_else(|_| fail("umbral de GPU no válido")),
            "-b" | "--segundo-plano" => background = true,
            "--no-abrir" => cfg.open_report = false,
            "--hijo" => child = true,
            other => fail(&format!("opción desconocida: {other}")),
        }
    }

    if background && !child {
        std::process::exit(monitor::spawn_background(&args));
    }
    cfg.quiet = child;
    if let Err(e) = std::fs::create_dir_all(&cfg.out_dir) {
        fail(&format!("no se puede crear la carpeta {}: {e}", cfg.out_dir.display()));
    }
    monitor::run(cfg);
}

#[cfg(test)]
mod tests {
    use super::parse_duration;
    use std::time::Duration;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90s"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("30m"), Some(Duration::from_secs(1800)));
        assert_eq!(parse_duration("1h30m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("2"), Some(Duration::from_secs(120)));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("10x"), None);
        assert_eq!(parse_duration("10m5"), None);
    }
}
