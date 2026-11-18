//! Uso de GPU por proceso con los contadores de rendimiento de Windows
//! (`\GPU Engine(*)\Utilization Percentage`, lo mismo que muestra el Administrador de tareas).

use std::collections::HashMap;
use windows_sys::Win32::System::Performance::*;

pub struct GpuPerProcess {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

impl GpuPerProcess {
    pub fn new() -> Option<Self> {
        unsafe {
            let mut query: PDH_HQUERY = std::ptr::null_mut();
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let mut counter: PDH_HCOUNTER = std::ptr::null_mut();
            let path = wide("\\GPU Engine(*)\\Utilization Percentage");
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                PdhCloseQuery(query);
                return None;
            }
            PdhCollectQueryData(query);
            Some(GpuPerProcess { query, counter })
        }
    }

    /// % de GPU por PID (máximo entre motores: 3D, copia, vídeo...), como el Administrador de tareas.
    pub fn read(&mut self) -> HashMap<u32, f32> {
        let mut per_engine: HashMap<(u32, String), f32> = HashMap::new();
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return HashMap::new();
            }
            let mut size = 0u32;
            let mut count = 0u32;
            let r = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut());
            if r != PDH_MORE_DATA as u32 || size == 0 {
                return HashMap::new();
            }
            let mut buf = vec![0u8; size as usize];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                return HashMap::new();
            }
            for i in 0..count as usize {
                let it = &*items.add(i);
                let v = it.FmtValue.Anonymous.doubleValue as f32;
                if v <= 0.0 {
                    continue;
                }
                let mut len = 0;
                while *it.szName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, len));
                if let Some((pid, eng)) = parse_instance(&name) {
                    *per_engine.entry((pid, eng)).or_default() += v;
                }
            }
        }
        let mut out: HashMap<u32, f32> = HashMap::new();
        for ((pid, _), v) in per_engine {
            let e = out.entry(pid).or_default();
            *e = e.max(v.min(100.0));
        }
        out
    }
}

impl Drop for GpuPerProcess {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// "pid_1234_luid_0x0_0x1_phys_0_eng_3_engtype_3D" -> (1234, "3D")
pub fn parse_instance(name: &str) -> Option<(u32, String)> {
    let rest = name.strip_prefix("pid_")?;
    let pid: u32 = rest.split('_').next()?.parse().ok()?;
    let eng = name.rsplit_once("engtype_").map(|(_, e)| e.to_string()).unwrap_or_default();
    Some((pid, eng))
}

#[cfg(test)]
mod tests {
    #[test]
    fn instancia() {
        assert_eq!(
            super::parse_instance("pid_1234_luid_0x00000000_0x0000D1B5_phys_0_eng_0_engtype_3D"),
            Some((1234, "3D".to_string()))
        );
        assert_eq!(super::parse_instance("_Total"), None);
    }
}
