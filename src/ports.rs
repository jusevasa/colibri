//! Puertos TCP en escucha en esta máquina. Todo local: `lsof` + una sonda HTTP a 127.0.0.1.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, PartialEq)]
pub struct Port {
    pub port: u16,
    pub pid: i32,
    pub command: String,
    pub project: String,
    /// Carpeta de trabajo del proceso, abreviada ("~/Dev/app").
    pub cwd: String,
    pub web: bool,
}

pub struct Ports {
    pub list: Arc<Mutex<Vec<Port>>>,
    pub enabled: Arc<AtomicBool>,
    /// Pide un escaneo inmediato.
    pub refresh: Arc<AtomicBool>,
}

impl Ports {
    pub fn start() -> Self {
        let list = Arc::new(Mutex::new(Vec::new()));
        let enabled = Arc::new(AtomicBool::new(true));
        let refresh = Arc::new(AtomicBool::new(false));
        let (l, e, r) = (list.clone(), enabled.clone(), refresh.clone());
        std::thread::Builder::new()
            .name("colibri-ports".into())
            .spawn(move || {
                let mut web_cache: HashMap<(i32, u16), bool> = HashMap::new();
                loop {
                    if e.load(Ordering::Relaxed) {
                        let found = scan(&mut web_cache);
                        let mut guard = l.lock().unwrap();
                        if *guard != found {
                            *guard = found;
                            drop(guard);
                            crate::shell::wake_all();
                        }
                    }
                    // Espera 3 s, pero despierta enseguida si piden refrescar.
                    for _ in 0..30 {
                        if r.swap(false, Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            })
            .ok();
        Self {
            list,
            enabled,
            refresh,
        }
    }
}

fn scan(web_cache: &mut HashMap<(i32, u16), bool>) -> Vec<Port> {
    let out = match Command::new("/usr/sbin/lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcn"])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => return vec![],
    };
    let mut ports: Vec<Port> = Vec::new();
    let (mut pid, mut command) = (0i32, String::new());
    for line in out.lines() {
        let Some(tag) = line.chars().next() else {
            continue;
        };
        let rest = &line[tag.len_utf8()..];
        match tag {
            'p' => pid = rest.parse().unwrap_or(0),
            'c' => command = rest.to_string(),
            'n' => {
                let Some(port) = rest.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) else {
                    continue;
                };
                if ports.iter().any(|p| p.port == port) {
                    continue;
                }
                let web = *web_cache
                    .entry((pid, port))
                    .or_insert_with(|| is_http(port));
                let dir = crate::proc::cwd(pid);
                let project = dir
                    .as_deref()
                    .map(crate::proc::base_name)
                    .unwrap_or_default();
                let cwd = dir
                    .as_deref()
                    .map(crate::proc::pretty_path)
                    .unwrap_or_default();
                ports.push(Port {
                    port,
                    pid,
                    command: command.clone(),
                    project,
                    cwd,
                    web,
                });
            }
            _ => {}
        }
    }
    web_cache.retain(|k, _| ports.iter().any(|p| (p.pid, p.port) == *k));
    ports.sort_by_key(|p| p.port);
    ports
}

fn is_http(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(150)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
    let _ = s.set_write_timeout(Some(Duration::from_millis(150)));
    if s.write_all(b"HEAD / HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 5];
    matches!(s.read(&mut buf), Ok(n) if n >= 5 && &buf == b"HTTP/")
}
