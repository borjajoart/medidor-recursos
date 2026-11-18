//! Configuración y análisis de argumentos (compartido por la consola y la interfaz).

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

pub enum Command {
    Run { cfg: Config, background: bool, child: bool },
    Stop,
    Status,
    Help,
}

pub const HELP: &str = "\
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

Para usarlo sin consola, abre medidor-gui.exe.
";

pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim().to_lowercase();
    if let Ok(n) = s.parse::<f64>() {
        return (n > 0.0).then(|| Duration::from_secs_f64(n * 60.0));
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

/// Carpeta 'informes' junto al ejecutable.
pub fn default_out_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("informes")))
        .unwrap_or_else(|| PathBuf::from("informes"))
}

pub fn parse(args: &[String]) -> Result<Command, String> {
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
        let mut val = || it.next().cloned().ok_or_else(|| format!("falta el valor de {a}"));
        match a.as_str() {
            "detener" | "stop" => return Ok(Command::Stop),
            "estado" | "status" => return Ok(Command::Status),
            "-h" | "--ayuda" | "--help" | "/?" => return Ok(Command::Help),
            "-d" | "--duracion" => {
                let v = val()?;
                cfg.duration = Some(parse_duration(&v).ok_or_else(|| format!("duración no válida: {v}"))?);
            }
            "-i" | "--intervalo" => {
                let s: f64 = val()?
                    .parse()
                    .ok()
                    .filter(|s| (0.5..=600.0).contains(s))
                    .ok_or("intervalo entre 0.5 y 600 s")?;
                cfg.interval = Duration::from_secs_f64(s);
            }
            "-o" | "--salida" => cfg.out_dir = PathBuf::from(val()?),
            "--umbral-cpu" => cfg.cpu_limit = val()?.parse().map_err(|_| "umbral de CPU no válido")?,
            "--umbral-gpu" => cfg.gpu_limit = val()?.parse().map_err(|_| "umbral de GPU no válido")?,
            "-b" | "--segundo-plano" => background = true,
            "--no-abrir" => cfg.open_report = false,
            "--hijo" => child = true,
            other => return Err(format!("opción desconocida: {other}")),
        }
    }
    cfg.quiet = child;
    Ok(Command::Run { cfg, background, child })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90s"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("30m"), Some(Duration::from_secs(1800)));
        assert_eq!(parse_duration("1h30m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("2"), Some(Duration::from_secs(120)));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("10x"), None);
        assert_eq!(parse_duration("10m5"), None);
        assert_eq!(parse_duration("0"), None);
    }

    #[test]
    fn args() {
        let a: Vec<String> = ["-d", "30m", "--umbral-cpu", "80", "--hijo", "--no-abrir"].iter().map(|s| s.to_string()).collect();
        match parse(&a) {
            Ok(Command::Run { cfg, background, child }) => {
                assert_eq!(cfg.duration, Some(Duration::from_secs(1800)));
                assert_eq!(cfg.cpu_limit, 80.0);
                assert!(!cfg.open_report && cfg.quiet && child && !background);
            }
            _ => panic!("debería ser Run"),
        }
        assert!(matches!(parse(&["detener".to_string()]), Ok(Command::Stop)));
        assert!(parse(&["--xyz".to_string()]).is_err());
        assert!(parse(&["-i".to_string(), "0.1".to_string()]).is_err());
    }
}
