//! Consultas baratas al sistema sobre procesos (macOS).

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::path::PathBuf;

/// PID del grupo en primer plano de la terminal.
pub fn foreground_pid(master_fd: c_int) -> Option<i32> {
    let pid = unsafe { libc::tcgetpgrp(master_fd) };
    (pid > 0).then_some(pid)
}

/// Si la terminal está pidiendo una contraseña: modo por líneas con el eco apagado, que es
/// lo que ponen `sudo`, `ssh`, `passwd` o `read -s` mientras se escribe la clave (zle, vim o
/// Claude Code apagan el eco pero leen tecla a tecla). El maestro ve el modo del esclavo.
/// Es el mismo criterio que usa Ghostty para su entrada segura automática.
pub fn password_mode(master_fd: c_int) -> bool {
    if master_fd < 0 {
        return false;
    }
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(master_fd, &mut t) } != 0 {
        return false;
    }
    t.c_lflag & libc::ICANON != 0 && t.c_lflag & libc::ECHO == 0
}

pub fn name(pid: i32) -> String {
    let mut buf = [0u8; 256];
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf8_lossy(&buf[..n as usize]).into_owned()
}

pub fn cwd(pid: i32) -> Option<PathBuf> {
    unsafe {
        let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as c_int;
        let n = libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            &mut info as *mut _ as *mut c_void,
            size,
        );
        if n < size {
            return None;
        }
        let ptr = &info.pvi_cdir.vip_path as *const _ as *const c_char;
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        (!s.is_empty()).then(|| PathBuf::from(s))
    }
}

/// PID del proceso padre.
pub fn parent(pid: i32) -> Option<i32> {
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as c_int;
        let n = libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut c_void,
            size,
        );
        (n == size && info.pbi_ppid > 0).then_some(info.pbi_ppid as i32)
    }
}

/// Argumentos del proceso (argv), vía sysctl KERN_PROCARGS2.
pub fn args(pid: i32) -> Vec<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return vec![];
        }
        let mut buf = vec![0u8; size];
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return vec![];
        }
        buf.truncate(size);
        if buf.len() < 4 {
            return vec![];
        }
        let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]).max(0) as usize;
        let rest = &buf[4..];
        // Ruta del ejecutable, luego relleno de NULs, luego argv.
        let exec_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        let mut i = exec_end;
        while i < rest.len() && rest[i] == 0 {
            i += 1;
        }
        rest[i..]
            .split(|&b| b == 0)
            .take(argc)
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect()
    }
}

/// "~/Dev/proyecto" en lugar de "/Users/yo/Dev/proyecto".
pub fn pretty_path(p: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = p.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~/{}", rest.display())
            };
        }
    }
    p.display().to_string()
}

pub fn base_name(p: &std::path::Path) -> String {
    match p.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => pretty_path(p),
    }
}

#[cfg(test)]
mod tests {
    use super::password_mode;

    /// Un PTY de verdad: el modo se cambia en el esclavo (lo que hacen `sudo` o `ssh`) y se
    /// lee desde el maestro, que es lo único que tiene Colibrí.
    #[test]
    fn contrasena_se_ve_desde_el_maestro() {
        let (mut master, mut slave) = (0, 0);
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0);
        let set = |echo: bool, canonical: bool| unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(slave, &mut t), 0);
            for (flag, on) in [(libc::ECHO, echo), (libc::ICANON, canonical)] {
                if on {
                    t.c_lflag |= flag;
                } else {
                    t.c_lflag &= !flag;
                }
            }
            assert_eq!(libc::tcsetattr(slave, libc::TCSANOW, &t), 0);
        };
        // El shell esperando una orden normal.
        set(true, true);
        assert!(!password_mode(master));
        // `sudo`, `ssh`, `read -s`: sin eco, pero por líneas.
        set(false, true);
        assert!(password_mode(master));
        // zle, vim o Claude Code: sin eco y tecla a tecla. No es una contraseña.
        set(false, false);
        assert!(!password_mode(master));
        unsafe {
            libc::close(slave);
            libc::close(master);
        }
        // Un descriptor cerrado no es una contraseña.
        assert!(!password_mode(-1));
    }
}
