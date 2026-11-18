//! Monitor de procesos: `ps` + `vm_stat` cada 2 s, solo mientras el apartado está visible.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, PartialEq)]
pub struct Proc {
    pub pid: i32,
    pub cpu: f32,
    pub mem: u64,
    pub name: String,
    /// App a la que pertenece ("Helium" para "Helium Helper"), si aplica.
    pub app: String,
}

#[derive(Clone, Default, PartialEq)]
pub struct Snapshot {
    pub procs: Vec<Proc>,
    /// Uso total de CPU, 0–100.
    pub cpu_total: f32,
    pub mem_used: u64,
    pub mem_total: u64,
}

pub struct Monitor {
    pub data: Arc<Mutex<Snapshot>>,
    pub enabled: Arc<AtomicBool>,
}

impl Monitor {
    pub fn start(ctx: eframe::egui::Context) -> Self {
        let data = Arc::new(Mutex::new(Snapshot::default()));
        let enabled = Arc::new(AtomicBool::new(false));
        let (d, e) = (data.clone(), enabled.clone());
        std::thread::Builder::new()
            .name("colibri-procs".into())
            .spawn(move || {
                let mem_total = mem_total();
                let cores = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1) as f32;
                loop {
                    if e.load(Ordering::Relaxed) {
                        let procs = scan();
                        let cpu_total =
                            (procs.iter().map(|p| p.cpu).sum::<f32>() / cores).min(100.0);
                        let snap = Snapshot {
                            procs,
                            cpu_total,
                            mem_used: mem_used(),
                            mem_total,
                        };
                        let mut guard = d.lock().unwrap();
                        if *guard != snap {
                            *guard = snap;
                            drop(guard);
                            ctx.request_repaint();
                        }
                        std::thread::sleep(Duration::from_secs(2));
                    } else {
                        std::thread::sleep(Duration::from_millis(400));
                    }
                }
            })
            .ok();
        Self { data, enabled }
    }
}

fn scan() -> Vec<Proc> {
    let out = match Command::new("/bin/ps")
        .env("LC_ALL", "C")
        .args(["-axo", "pid=,pcpu=,rss=,comm="])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => return vec![],
    };
    out.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, rest) = line.split_once(char::is_whitespace)?;
            let (cpu, rest) = rest.trim_start().split_once(char::is_whitespace)?;
            let (rss, path) = rest.trim_start().split_once(char::is_whitespace)?;
            let path = path.trim();
            let name = path.rsplit('/').next().unwrap_or(path).to_string();
            let app = path
                .split('/')
                .find_map(|part| part.strip_suffix(".app"))
                .filter(|a| *a != name)
                .unwrap_or("")
                .to_string();
            Some(Proc {
                pid: pid.parse().ok()?,
                cpu: cpu.parse().ok()?,
                mem: rss.parse::<u64>().ok()? * 1024,
                name,
                app,
            })
        })
        .collect()
}

fn mem_total() -> u64 {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            &mut value as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        );
    }
    value
}

/// Memoria en uso al estilo del Monitor de Actividad: activa + cableada + comprimida.
fn mem_used() -> u64 {
    let out = match Command::new("/usr/bin/vm_stat").output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => return 0,
    };
    let mut page = 16384u64;
    let mut pages = 0u64;
    for line in out.lines() {
        if let Some(rest) = line.split("page size of ").nth(1) {
            page = rest
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(page);
        }
        let counted = [
            "Pages active:",
            "Pages wired down:",
            "Pages occupied by compressor:",
        ];
        if counted.iter().any(|k| line.starts_with(k)) {
            let n = line
                .rsplit(':')
                .next()
                .unwrap_or("")
                .trim()
                .trim_end_matches('.');
            pages += n.parse::<u64>().unwrap_or(0);
        }
    }
    pages * page
}

/// Pide al proceso que termine (SIGTERM), o lo fuerza (SIGKILL) si ya ignoró la petición.
/// Nunca toca a launchd ni a Colibrí.
pub fn stop(pid: i32, force: bool) {
    if pid > 1 && pid != std::process::id() as i32 {
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        unsafe { libc::kill(pid, signal) };
    }
}

pub fn alive(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

pub fn human(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}
