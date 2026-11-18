//! Ping (ICMP) en un hilo aparte, para no bloquear el muestreo.

use std::net::{Ipv4Addr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default, Clone, Copy)]
struct PingState {
    last_ms: Option<f32>,
    sent: u32,
    lost: u32,
}

pub struct Pinger {
    pub host: String,
    state: Arc<Mutex<PingState>>,
    stop: Arc<AtomicBool>,
}

/// Lectura del ping desde la última llamada.
pub struct PingReading {
    pub ms: Option<f32>,
    pub lost: u32,
    pub sent: u32,
}

fn resolve(host: &str) -> Option<Ipv4Addr> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Some(ip);
    }
    (host, 0).to_socket_addrs().ok()?.find_map(|a| match a.ip() {
        std::net::IpAddr::V4(v4) => Some(v4),
        _ => None,
    })
}

fn ping_once(handle: windows_sys::Win32::Foundation::HANDLE, ip: Ipv4Addr) -> Option<f32> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{ICMP_ECHO_REPLY, IcmpSendEcho};
    let data = [0x61u8; 32];
    let mut reply = vec![0u8; std::mem::size_of::<ICMP_ECHO_REPLY>() + data.len() + 8];
    let addr = u32::from_ne_bytes(ip.octets());
    let n = unsafe {
        IcmpSendEcho(
            handle,
            addr,
            data.as_ptr() as *const _,
            data.len() as u16,
            std::ptr::null(),
            reply.as_mut_ptr() as *mut _,
            reply.len() as u32,
            1000,
        )
    };
    if n == 0 {
        return None;
    }
    let r = unsafe { &*(reply.as_ptr() as *const ICMP_ECHO_REPLY) };
    (r.Status == 0).then_some(r.RoundTripTime as f32)
}

impl Pinger {
    /// `host` vacío o "no" desactiva el ping.
    pub fn start(host: &str, every: Duration) -> Option<Self> {
        let host = host.trim();
        if host.is_empty() || host.eq_ignore_ascii_case("no") {
            return None;
        }
        let ip = resolve(host)?;
        let state = Arc::new(Mutex::new(PingState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            use windows_sys::Win32::NetworkManagement::IpHelper::{IcmpCloseHandle, IcmpCreateFile};
            let h = unsafe { IcmpCreateFile() };
            if h.is_null() || h as isize == -1 {
                return;
            }
            while !sp.load(Ordering::Relaxed) {
                let r = ping_once(h, ip);
                if let Ok(mut s) = st.lock() {
                    s.sent += 1;
                    match r {
                        Some(ms) => s.last_ms = Some(ms),
                        None => s.lost += 1,
                    }
                }
                std::thread::sleep(every);
            }
            unsafe { IcmpCloseHandle(h) };
        });
        Some(Pinger { host: host.to_string(), state, stop })
    }

    pub fn take(&self) -> PingReading {
        let mut s = self.state.lock().unwrap();
        let r = PingReading { ms: s.last_ms, lost: s.lost, sent: s.sent };
        *s = PingState::default();
        r
    }
}

impl Drop for Pinger {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
