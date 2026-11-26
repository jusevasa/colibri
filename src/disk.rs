//! Qué rutas se pueden mirar en el disco sin esperar a la red: lo que Colibrí mira por su
//! cuenta en el hilo de la interfaz (el color de las rutas del comando, el panel de
//! detalles) se limita a discos de este Mac. Una unidad de red que no responde dejaría la
//! interfaz parada segundos, y autofs (`/net`) montaría el recurso solo por mirarlo.

use std::path::{Component, Path, PathBuf};

/// Puntos de montaje y si son locales, leídos de la tabla del sistema sin esperar a ningún
/// servidor (`getmntinfo` con `MNT_NOWAIT`).
pub fn mounts() -> Vec<(PathBuf, bool)> {
    use std::os::unix::ffi::OsStrExt;
    let mut list: *mut libc::statfs = std::ptr::null_mut();
    // SAFETY: getmntinfo deja en `list` un búfer suyo con `n` entradas válidas (no se libera).
    let n = unsafe { libc::getmntinfo(&mut list, libc::MNT_NOWAIT) };
    if n <= 0 || list.is_null() {
        return Vec::new();
    }
    // SAFETY: `n` entradas a partir de `list`, recién devueltas por getmntinfo.
    let entries = unsafe { std::slice::from_raw_parts(list, n as usize) };
    entries
        .iter()
        .map(|m| {
            // SAFETY: f_mntonname es una cadena C terminada en NUL dentro del struct.
            let name = unsafe { std::ffi::CStr::from_ptr(m.f_mntonname.as_ptr()) };
            let local = m.f_flags & libc::MNT_LOCAL as u32 != 0;
            (
                PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())),
                local,
            )
        })
        .collect()
}

/// `path` está en un disco de este Mac: el montaje más largo que lo contiene es local (si no
/// hay ninguno, por ejemplo porque no se pudo leer la tabla, se da por no local), y no
/// cuelga de `/net` ni de `/Network` (autofs montaría el recurso de red solo por mirarlo).
/// Mirar una ruta en una unidad de red puede tardar segundos si el servidor no responde, y
/// el color de las rutas se calcula en el hilo de la interfaz.
pub fn on_local_disk(path: &Path, mounts: &[(PathBuf, bool)]) -> bool {
    if under(path, Path::new("/net")) || under(path, Path::new("/Network")) {
        return false;
    }
    mounts
        .iter()
        .filter(|(at, _)| under(path, at))
        .max_by_key(|(at, _)| at.as_os_str().len())
        // Sin montaje que la contenga (la tabla vino vacía) no se sabe si es local: no.
        .is_some_and(|(_, local)| *local)
}

/// `path` está dentro de `dir`, comparando tramo a tramo sin distinguir mayúsculas: el
/// volumen de arranque APFS no las distingue, así que `/volumes/nas` llega a `/Volumes/NAS`.
/// En un volumen que sí las distingue, el error va hacia el lado seguro (no mirar).
fn under(path: &Path, dir: &Path) -> bool {
    let mut parts = path.components();
    dir.components().all(|d| {
        parts.next().is_some_and(|p| {
            p.as_os_str().to_string_lossy().to_lowercase()
                == d.as_os_str().to_string_lossy().to_lowercase()
        })
    })
}

/// Llegar a `path` no pasa por ninguna unidad de red, ni siquiera a través de enlaces
/// simbólicos: se recorre tramo a tramo y cada enlace se resuelve con `readlink` (que no
/// sigue el enlace) antes de mirar nada detrás de él. Cada `lstat` se hace sobre una ruta
/// cuyos tramos anteriores ya se comprobaron, así que ninguno cruza un montaje de red. Lo que
/// no existe deja de recorrerse: más allá no hay nada que mirar.
pub fn local_route(path: &Path, mounts: &[(PathBuf, bool)]) -> bool {
    let mut pending: std::collections::VecDeque<PathBuf> = path
        .components()
        .map(|c| PathBuf::from(c.as_os_str()))
        .collect();
    let mut current = PathBuf::new();
    let mut links = 0;
    while let Some(part) = pending.pop_front() {
        match part.components().next() {
            Some(Component::RootDir) => current = PathBuf::from("/"),
            Some(Component::CurDir) | None => {}
            Some(Component::ParentDir) => {
                current.pop();
            }
            Some(_) => {
                current.push(&part);
                if !on_local_disk(&current, mounts) {
                    return false;
                }
                match std::fs::symlink_metadata(&current) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        links += 1;
                        let Some(target) =
                            std::fs::read_link(&current).ok().filter(|_| links <= 32)
                        else {
                            return false;
                        };
                        current.pop();
                        let mut next: std::collections::VecDeque<PathBuf> = target
                            .components()
                            .map(|c| PathBuf::from(c.as_os_str()))
                            .collect();
                        next.extend(pending);
                        pending = next;
                    }
                    Ok(_) => {}
                    Err(_) => return true,
                }
            }
        }
    }
    // Lo recorrido ya se comprobó tramo a tramo; esto cubre lo que no tiene tramos («/»).
    on_local_disk(&current, mounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rutas_en_unidades_de_red_no_se_miran() {
        let mounts = [
            (PathBuf::from("/"), true),
            (PathBuf::from("/System/Volumes/Data"), true),
            (PathBuf::from("/Volumes/NAS"), false),
        ];
        assert!(on_local_disk(Path::new("/Users/yo/src"), &mounts));
        assert!(!on_local_disk(Path::new("/Volumes/NAS/proyecto"), &mounts));
        // Se compara por componentes: «/Volumes/NASX» no está dentro de «/Volumes/NAS».
        assert!(on_local_disk(Path::new("/Volumes/NASX/a"), &mounts));
        // autofs montaría el recurso de red solo por mirarlo.
        assert!(!on_local_disk(Path::new("/net/servidor/x"), &mounts));
        assert!(!on_local_disk(Path::new("/Network/Servers/x"), &mounts));
        // La raíz de este Mac siempre es local.
        assert!(
            super::mounts()
                .iter()
                .any(|(at, local)| at == Path::new("/") && *local)
        );
    }

    #[test]
    fn montajes_sin_distinguir_mayusculas() {
        // El volumen de arranque APFS no distingue mayúsculas: `/volumes/nas` es `/Volumes/NAS`.
        let mounts = [
            (PathBuf::from("/"), true),
            (PathBuf::from("/Volumes/NAS"), false),
        ];
        assert!(!on_local_disk(Path::new("/volumes/nas/x"), &mounts));
        assert!(!on_local_disk(Path::new("/NET/servidor"), &mounts));
        assert!(!on_local_disk(Path::new("/network/Servers/x"), &mounts));
        assert!(on_local_disk(Path::new("/volumes/nasx/a"), &mounts));
        assert!(on_local_disk(Path::new("/Users/yo"), &mounts));
    }

    #[test]
    fn sin_tabla_de_montajes_no_se_mira_nada() {
        // Si getmntinfo fallara no se puede saber qué es local: no se mira el disco.
        assert!(!on_local_disk(Path::new("/Users/yo/src"), &[]));
        assert!(!local_route(Path::new("/Users/yo/src"), &[]));
        // También la raíz sola.
        assert!(!local_route(Path::new("/"), &[]));
        assert!(local_route(Path::new("/"), &[(PathBuf::from("/"), true)]));
    }

    #[test]
    fn enlace_a_red_no_se_sigue() {
        // Resuelta, como los montajes de `getmntinfo` (en macOS `/var` es `/private/var`).
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("colibri-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("local/sub")).unwrap();
        std::fs::create_dir_all(dir.join("red/x")).unwrap();
        // `red` hace de montaje de red; `local` es un disco de este Mac.
        let mounts = [(PathBuf::from("/"), true), (dir.join("red"), false)];
        std::os::unix::fs::symlink(dir.join("red/x"), dir.join("local/absoluto")).unwrap();
        std::os::unix::fs::symlink("../red/x", dir.join("local/relativo")).unwrap();
        std::os::unix::fs::symlink("sub", dir.join("local/dentro")).unwrap();
        std::os::unix::fs::symlink("bucle", dir.join("local/bucle")).unwrap();
        assert!(local_route(&dir.join("local/sub/a.txt"), &mounts));
        assert!(!local_route(&dir.join("local/absoluto/a.txt"), &mounts));
        assert!(!local_route(&dir.join("local/relativo"), &mounts));
        // Un enlace que se queda en el disco local se sigue.
        assert!(local_route(&dir.join("local/dentro/a.txt"), &mounts));
        assert!(!local_route(&dir.join("local/bucle/a"), &mounts));
        assert!(local_route(&dir.join("local/no-existe/a"), &mounts));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
