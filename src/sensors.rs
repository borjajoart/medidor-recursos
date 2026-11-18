//! Lectura de temperaturas, frecuencias y uso de GPU.
//!
//! Orden de preferencia:
//! 1. LibreHardwareMonitor / OpenHardwareMonitor (vía WMI) si se está ejecutando:
//!    da CPU, GPU (cualquier marca), discos, frecuencias y consumo.
//! 2. NVIDIA NVML (si hay gráfica NVIDIA) para la GPU.
//! 3. Zona térmica ACPI de Windows (requiere administrador, poco precisa) para la CPU.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use wmi::WMIConnection;

#[derive(Default, Clone, Debug)]
pub struct Readings {
    pub cpu_temp: Option<f32>,
    pub cpu_clock: Option<f32>,
    pub cpu_power: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub gpu_hot: Option<f32>,
    pub gpu_load: Option<f32>,
    pub gpu_power: Option<f32>,
    pub gpu_clock: Option<f32>,
    pub gpu_mem: Option<f32>,
    /// true si la GPU (NVIDIA) informa de limitación térmica/energética en este instante
    pub gpu_throttle: bool,
    /// Otras temperaturas (discos, placa base...)
    pub others: BTreeMap<String, f32>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct HwSensor {
    identifier: String,
    name: String,
    sensor_type: String,
    value: f32,
    parent: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct HwHardware {
    identifier: String,
    name: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename = "MSAcpi_ThermalZoneTemperature")]
#[serde(rename_all = "PascalCase")]
struct AcpiZone {
    current_temperature: u32,
}

struct HwMonitor {
    conn: WMIConnection,
    namespace: &'static str,
    hw_names: BTreeMap<String, String>,
}

pub struct Sensors {
    hwmon: Option<HwMonitor>,
    last_hwmon_try: Instant,
    acpi: Option<WMIConnection>,
    nvml: Option<nvml_wrapper::Nvml>,
    pub gpu_name: Option<String>,
}

const NAMESPACES: [&str; 2] = ["root\\LibreHardwareMonitor", "root\\OpenHardwareMonitor"];

fn connect_hwmon() -> Option<HwMonitor> {
    for ns in NAMESPACES {
        let Ok(conn) = WMIConnection::with_namespace_path(ns) else { continue };
        let Ok(sensors) = conn.raw_query::<HwSensor>("SELECT Identifier, Name, SensorType, Value, Parent FROM Sensor") else {
            continue;
        };
        if sensors.is_empty() {
            continue;
        }
        let hw_names = conn
            .raw_query::<HwHardware>("SELECT Identifier, Name FROM Hardware")
            .unwrap_or_default()
            .into_iter()
            .map(|h| (h.identifier, h.name))
            .collect();
        return Some(HwMonitor { conn, namespace: ns, hw_names });
    }
    None
}

fn is_cpu(id: &str) -> bool {
    id.starts_with("/intelcpu") || id.starts_with("/amdcpu") || id.starts_with("/cpu")
}

/// Prioridad de GPU: dedicada (NVIDIA/AMD) antes que integrada (Intel).
fn gpu_rank(id: &str) -> Option<u8> {
    if id.starts_with("/gpu-nvidia") || id.starts_with("/nvidiagpu") {
        Some(0)
    } else if id.starts_with("/gpu-amd") || id.starts_with("/atigpu") {
        Some(1)
    } else if id.starts_with("/gpu-intel") || id.starts_with("/gpu") {
        Some(2)
    } else {
        None
    }
}

fn is_storage(id: &str) -> bool {
    id.starts_with("/hdd") || id.starts_with("/nvme") || id.starts_with("/ssd") || id.starts_with("/storage")
}

impl Sensors {
    pub fn new() -> Self {
        let hwmon = connect_hwmon();
        let acpi = WMIConnection::with_namespace_path("root\\WMI").ok().filter(|c| {
            c.raw_query::<AcpiZone>("SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature")
                .map(|v| !v.is_empty())
                .unwrap_or(false)
        });
        let nvml = nvml_wrapper::Nvml::init().ok().filter(|n| n.device_by_index(0).is_ok());
        let gpu_name = nvml.as_ref().and_then(|n| n.device_by_index(0).ok()?.name().ok());
        Sensors { hwmon, last_hwmon_try: Instant::now(), acpi, nvml, gpu_name }
    }

    /// Descripción de las fuentes de datos disponibles (para el informe).
    pub fn sources(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(h) = &self.hwmon {
            v.push(format!("{} (WMI {})", if h.namespace.contains("Libre") { "LibreHardwareMonitor" } else { "OpenHardwareMonitor" }, h.namespace));
        }
        if self.nvml.is_some() {
            v.push("NVIDIA NVML".into());
        }
        if self.acpi.is_some() {
            v.push("Zona térmica ACPI de Windows (aproximada)".into());
        }
        if v.is_empty() {
            v.push("ninguna fuente de temperatura disponible".into());
        }
        v
    }

    pub fn has_hwmon(&self) -> bool {
        self.hwmon.is_some()
    }

    pub fn read(&mut self) -> Readings {
        let mut r = Readings::default();

        // Si LibreHardwareMonitor no estaba abierto, reintenta cada 30 s por si se abre después.
        if self.hwmon.is_none() && self.last_hwmon_try.elapsed() > Duration::from_secs(30) {
            self.last_hwmon_try = Instant::now();
            self.hwmon = connect_hwmon();
        }

        if let Some(h) = &self.hwmon {
            match h.conn.raw_query::<HwSensor>("SELECT Identifier, Name, SensorType, Value, Parent FROM Sensor") {
                Ok(list) if !list.is_empty() => fill_from_hwmon(&mut r, &list, &h.hw_names),
                _ => {
                    // LibreHardwareMonitor se ha cerrado
                    self.hwmon = None;
                    self.last_hwmon_try = Instant::now();
                }
            }
        }

        if let Some(nvml) = &self.nvml {
            if let Ok(dev) = nvml.device_by_index(0) {
                use nvml_wrapper::bitmasks::device::ThrottleReasons as T;
                use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
                if r.gpu_temp.is_none() {
                    r.gpu_temp = dev.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f32);
                }
                if r.gpu_load.is_none() {
                    r.gpu_load = dev.utilization_rates().ok().map(|u| u.gpu as f32);
                }
                if r.gpu_power.is_none() {
                    r.gpu_power = dev.power_usage().ok().map(|mw| mw as f32 / 1000.0);
                }
                if r.gpu_clock.is_none() {
                    r.gpu_clock = dev.clock_info(Clock::Graphics).ok().map(|c| c as f32);
                }
                if r.gpu_mem.is_none() {
                    r.gpu_mem = dev.memory_info().ok().map(|m| (m.used / 1_048_576) as f32);
                }
                if let Ok(reasons) = dev.current_throttle_reasons() {
                    r.gpu_throttle = reasons.intersects(
                        T::SW_THERMAL_SLOWDOWN | T::HW_THERMAL_SLOWDOWN | T::HW_SLOWDOWN | T::HW_POWER_BRAKE_SLOWDOWN,
                    );
                }
            }
        }

        if r.cpu_temp.is_none() {
            if let Some(acpi) = &self.acpi {
                if let Ok(zones) = acpi.raw_query::<AcpiZone>("SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature") {
                    r.cpu_temp = zones
                        .iter()
                        .map(|z| z.current_temperature as f32 / 10.0 - 273.15)
                        .filter(|t| *t > 0.0 && *t < 130.0)
                        .reduce(f32::max);
                }
            }
        }
        r
    }
}

fn fill_from_hwmon(r: &mut Readings, list: &[HwSensor], hw_names: &BTreeMap<String, String>) {
    // --- CPU ---
    let cpu: Vec<&HwSensor> = list.iter().filter(|s| is_cpu(&s.identifier)).collect();
    let cpu_temps: Vec<&&HwSensor> = cpu
        .iter()
        .filter(|s| s.sensor_type == "Temperature" && !s.name.contains("Distance") && s.value > 0.0)
        .collect();
    let pick = |names: &[&str]| cpu_temps.iter().find(|s| names.iter().any(|n| s.name.contains(n))).map(|s| s.value);
    r.cpu_temp = pick(&["Package", "Tctl", "Tdie"])
        .or_else(|| pick(&["Core Max"]))
        .or_else(|| cpu_temps.iter().map(|s| s.value).reduce(f32::max));

    let clocks: Vec<f32> = cpu
        .iter()
        .filter(|s| s.sensor_type == "Clock" && s.name.starts_with("Core") && s.value > 0.0)
        .map(|s| s.value)
        .collect();
    if !clocks.is_empty() {
        r.cpu_clock = Some(clocks.iter().sum::<f32>() / clocks.len() as f32);
    }
    r.cpu_power = cpu
        .iter()
        .find(|s| s.sensor_type == "Power" && s.name.contains("Package"))
        .map(|s| s.value);

    // --- GPU (la de mayor prioridad: dedicada) ---
    let best_gpu = list
        .iter()
        .filter_map(|s| gpu_rank(&s.identifier).map(|rank| (rank, s.parent.clone())))
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, p)| p);
    if let Some(parent) = best_gpu {
        let g: Vec<&HwSensor> = list.iter().filter(|s| s.parent == parent).collect();
        let find = |ty: &str, names: &[&str]| {
            g.iter()
                .find(|s| s.sensor_type == ty && names.iter().any(|n| s.name.contains(n)))
                .map(|s| s.value)
        };
        r.gpu_temp = find("Temperature", &["GPU Core", "Core"]).or_else(|| find("Temperature", &["GPU"]));
        r.gpu_hot = find("Temperature", &["Hot Spot", "Hotspot", "Junction"]);
        r.gpu_load = find("Load", &["GPU Core", "D3D 3D", "Core"]);
        r.gpu_power = find("Power", &["GPU Package", "GPU Power", "Package", "Total"]);
        r.gpu_clock = find("Clock", &["GPU Core", "Core"]);
        r.gpu_mem = find("SmallData", &["GPU Memory Used", "D3D Dedicated Memory Used", "Memory Used"]);
    }

    // --- Otras temperaturas: discos y placa base ---
    for s in list.iter().filter(|s| s.sensor_type == "Temperature" && s.value > 0.0 && s.value < 150.0) {
        let id = &s.identifier;
        if is_cpu(id) || gpu_rank(id).is_some() {
            continue;
        }
        let hw = hw_names.get(&s.parent).cloned().unwrap_or_else(|| s.parent.clone());
        let label = if is_storage(id) { format!("Disco {hw}") } else { format!("{hw} · {}", s.name) };
        if r.others.len() < 8 {
            r.others.insert(label, s.value);
        }
    }
}
