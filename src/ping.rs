//! Game-bar ping: one ICMP echo per second to `ping_host` on a background thread (no admin
//! rights needed). Only runs while the game bar shows the ping readout.

use crate::ffi::*;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// No reply yet / host unreachable.
pub const NONE: u32 = u32::MAX;

pub struct Pinger {
    rtt: Arc<AtomicU32>,
    gen: Arc<AtomicU64>,
    host: String,
    on: bool,
}

fn resolve(host: &str) -> Option<u32> {
    (host, 0).to_socket_addrs().ok()?.find_map(|a| match a.ip() {
        IpAddr::V4(v4) => Some(u32::from_ne_bytes(v4.octets())), // IPAddr is network byte order
        IpAddr::V6(_) => None,
    })
}

fn echo(h: HANDLE, dest: u32) -> Option<u32> {
    let data = *b"notch-island-ping";
    // ICMP_ECHO_REPLY (40 bytes on x64) + payload + room for an ICMP error.
    let mut reply = [0u64; 16];
    let n = unsafe {
        IcmpSendEcho(
            h,
            dest,
            data.as_ptr().cast(),
            data.len() as u16,
            std::ptr::null(),
            reply.as_mut_ptr().cast(),
            128,
            1000,
        )
    };
    // Layout: Address u32, Status u32, RoundTripTime u32.
    let words = reply.as_ptr() as *const u32;
    let (status, rtt) = unsafe { (*words.add(1), *words.add(2)) };
    (n > 0 && status == 0).then_some(rtt)
}

impl Pinger {
    pub fn new() -> Self {
        Self { rtt: Arc::new(AtomicU32::new(NONE)), gen: Arc::new(AtomicU64::new(0)), host: String::new(), on: false }
    }

    /// Start, stop or retarget the worker. Cheap to call every second.
    pub fn set(&mut self, on: bool, host: &str) {
        if on == self.on && host == self.host {
            return;
        }
        let gen = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.on = on;
        self.host = host.to_string();
        self.rtt.store(NONE, Ordering::Relaxed);
        if !on {
            return;
        }
        let (rtt, cur, host) = (self.rtt.clone(), self.gen.clone(), self.host.clone());
        let _ = std::thread::Builder::new().name("ping".into()).spawn(move || {
            let h = unsafe { IcmpCreateFile() };
            if h == 0 || h == -1 {
                return;
            }
            let mut dest = None;
            while cur.load(Ordering::SeqCst) == gen {
                if dest.is_none() {
                    dest = resolve(&host);
                }
                let r = dest.and_then(|d| echo(h, d));
                if cur.load(Ordering::SeqCst) != gen {
                    break;
                }
                rtt.store(r.unwrap_or(NONE), Ordering::Relaxed);
                if r.is_none() {
                    dest = None; // re-resolve next time (DNS change, network switch)
                }
                std::thread::sleep(Duration::from_millis(1000));
            }
            unsafe { IcmpCloseHandle(h) };
        });
    }

    /// Last round-trip time in ms, `None` while unknown or unreachable.
    pub fn ms(&self) -> Option<u32> {
        let v = self.rtt.load(Ordering::Relaxed);
        (self.on && v != NONE).then_some(v)
    }
}

impl Drop for Pinger {
    fn drop(&mut self) {
        self.gen.fetch_add(1, Ordering::SeqCst);
    }
}
