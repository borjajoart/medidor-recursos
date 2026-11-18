//! FPS y tiempos de fotograma mediante ETW (los mismos eventos que usa PresentMon).
//!
//! Se escuchan los eventos `Present` de DXGI (DirectX 10/11/12) y de Direct3D 9.
//! Requiere permisos de administrador. No cubre Vulkan/OpenGL puros.

use ferrisetw::EventRecord;
use ferrisetw::provider::Provider;
use ferrisetw::schema_locator::SchemaLocator;
use ferrisetw::trace::{UserTrace, stop_trace_by_name};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const SESSION: &str = "MedidorRecursosFps";
const DXGI: &str = "CA11C036-0102-4A2D-A6AD-F03CFED5D3C9"; // Microsoft-Windows-DXGI
const D3D9: &str = "783ACA0A-790E-4D7F-8451-AA850511C6B9"; // Microsoft-Windows-D3D9
const DXGI_PRESENT_START: u16 = 42;
const D3D9_PRESENT_START: u16 = 1;

type Presents = Arc<Mutex<HashMap<u32, Vec<i64>>>>;

pub struct FpsTracker {
    _trace: UserTrace,
    presents: Presents,
    last: HashMap<u32, i64>,
    qpc_freq: f64,
}

/// Resultado de FPS de un intervalo para un proceso.
#[derive(Debug, Clone, PartialEq)]
pub struct FpsSample {
    pub pid: u32,
    pub fps: f32,
    /// "1 % low": FPS equivalentes al percentil 99 del tiempo de fotograma
    pub low1: f32,
    pub frametime_max: f32,
    /// Fotogramas que tardaron más de 2,5 veces la mediana (tirones)
    pub stutters: u32,
}

fn qpc_frequency() -> f64 {
    let mut f = 0i64;
    unsafe { windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut f) };
    if f > 0 { f as f64 } else { 10_000_000.0 }
}

impl FpsTracker {
    pub fn start() -> Result<Self, String> {
        // Por si quedó una sesión huérfana de una ejecución anterior que se cerró a la fuerza.
        let _ = stop_trace_by_name(SESSION);
        let presents: Presents = Arc::new(Mutex::new(HashMap::new()));
        let mk = |id: u16, guid: &str| {
            let p = presents.clone();
            Provider::by_guid(guid)
                .add_callback(move |r: &EventRecord, _: &SchemaLocator| {
                    if r.event_id() == id {
                        if let Ok(mut m) = p.lock() {
                            m.entry(r.process_id()).or_default().push(r.raw_timestamp());
                        }
                    }
                })
                .build()
        };
        let trace = UserTrace::new()
            .named(SESSION.to_string())
            .enable(mk(DXGI_PRESENT_START, DXGI))
            .enable(mk(D3D9_PRESENT_START, D3D9))
            .start_and_process()
            .map_err(|e| format!("{e:?}"))?;
        Ok(FpsTracker { _trace: trace, presents, last: HashMap::new(), qpc_freq: qpc_frequency() })
    }

    /// Recoge los eventos desde la última llamada y calcula los FPS de cada proceso.
    pub fn collect(&mut self, dt_secs: f64) -> Vec<FpsSample> {
        let batch = std::mem::take(&mut *self.presents.lock().unwrap());
        let mut out = Vec::new();
        for (pid, mut ts) in batch {
            ts.sort_unstable();
            let prev = self.last.get(&pid).copied();
            if let Some(&l) = ts.last() {
                self.last.insert(pid, l);
            }
            if let Some(s) = compute(pid, prev, &ts, dt_secs, self.qpc_freq) {
                out.push(s);
            }
        }
        // olvida procesos que ya no presentan fotogramas
        if self.last.len() > 256 {
            self.last.clear();
        }
        out
    }
}

/// Cálculo puro (testeable) a partir de marcas de tiempo QPC.
pub fn compute(pid: u32, prev: Option<i64>, ts: &[i64], dt_secs: f64, freq: f64) -> Option<FpsSample> {
    if ts.is_empty() {
        return None;
    }
    let mut frames: Vec<f32> = Vec::with_capacity(ts.len());
    let mut last = prev;
    for &t in ts {
        if let Some(l) = last {
            let ms = ((t - l) as f64 / freq * 1000.0) as f32;
            if ms > 0.0 && ms < 5000.0 {
                frames.push(ms);
            }
        }
        last = Some(t);
    }
    // Con tiempos de fotograma se usa su media: contar eventos por ventana de muestreo
    // da saltos (49, 61...) según dónde caiga el corte.
    if frames.is_empty() {
        let fps = (ts.len() as f64 / dt_secs.max(0.1)) as f32;
        return Some(FpsSample { pid, fps, low1: fps, frametime_max: 0.0, stutters: 0 });
    }
    let fps = 1000.0 * frames.len() as f32 / frames.iter().sum::<f32>();
    let mut sorted = frames.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    let p99 = sorted[((sorted.len() - 1) as f32 * 0.99).round() as usize];
    let max = *sorted.last().unwrap();
    let stutters = frames.iter().filter(|&&f| f > median * 2.5 && f > 20.0).count() as u32;
    Some(FpsSample { pid, fps, low1: 1000.0 / p99, frametime_max: max, stutters })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_estables() {
        // 60 fotogramas a 16,67 ms en 1 s, reloj de 10 MHz
        let ts: Vec<i64> = (0..60).map(|i| i * 166_667).collect();
        let s = compute(1, None, &ts, 1.0, 10_000_000.0).unwrap();
        assert!((s.fps - 60.0).abs() < 0.5);
        assert!((s.low1 - 60.0).abs() < 1.0);
        assert_eq!(s.stutters, 0);
    }

    #[test]
    fn detecta_tiron() {
        let mut ts: Vec<i64> = (0..50).map(|i| i * 166_667).collect();
        // un fotograma de 100 ms
        let last = *ts.last().unwrap();
        ts.push(last + 1_000_000);
        let s = compute(1, None, &ts, 1.0, 10_000_000.0).unwrap();
        assert_eq!(s.stutters, 1);
        assert!(s.frametime_max > 99.0);
        assert!(s.low1 < 15.0);
    }
}
