//! Cromo de la ventana (`main.rs`): barra de pestañas, botón «Actualizar», paneles de
//! Puertos y Procesos, avisos y notificaciones.

pub struct T {
    // Barra de pestañas: menú contextual de una pestaña.
    pub color: &'static str,
    /// Nombre de cada color, en el orden de `TAB_COLORS` (en `session.toml` va otro, fijo).
    pub color_names: [&'static str; 7],
    pub no_color: &'static str,
    pub rename: &'static str,
    pub close_tab: &'static str,
    // Barra de pestañas: botones y pistas.
    pub new_tab_hint: &'static str,
    pub procs_hint: &'static str,
    pub ports_hint: &'static str,
    pub update: &'static str,
    pub update_hint: &'static str,
    /// «Colibrí 0.3.0 disponible»: hay una versión publicada más nueva (`updates.rs`).
    pub update_available: fn(&str) -> String,
    /// Qué hace el clic si Colibrí se instaló con Homebrew…
    pub update_available_brew: &'static str,
    /// …y si no.
    pub update_available_web: &'static str,
    // Pista de una pestaña al pasar el ratón.
    pub running: &'static str,
    /// «Terminó hace 5 min», a partir de los segundos transcurridos.
    pub finished_ago: fn(u64) -> String,
    /// Tiempo transcurrido suelto, «5 min» (también en el panel de agentes).
    pub ago: fn(u64) -> String,

    // Apartados laterales: lo común.
    pub hide: &'static str,
    // Panel de Puertos.
    pub ports: &'static str,
    /// «3 servidores web», en la cabecera.
    pub web_servers: fn(usize) -> String,
    pub refresh_now: &'static str,
    pub no_web_servers: &'static str,
    /// «4 puertos más (no web)», plegable.
    pub more_ports: fn(usize) -> String,
    pub stopping: &'static str,
    pub force: &'static str,
    pub stop: &'static str,
    pub stop_process: &'static str,
    pub copy_url: &'static str,
    pub open_browser: &'static str,
    pub go_to_tab: &'static str,
    // Panel de Procesos.
    pub processes: &'static str,
    /// Cabecera: CPU total, RAM usada y total en GB.
    pub procs_detail: fn(f32, f64, f64) -> String,
    pub search: &'static str,
    pub col_cpu: &'static str,
    pub col_memory: &'static str,
    pub sort_by: &'static str,
    pub reading_procs: &'static str,
    pub no_match: &'static str,
    /// CPU de un proceso con tantos decimales.
    pub percent: fn(f32, usize) -> String,
    /// Pista del control de detener, ya armado: «Confirmar: terminar pid 42».
    pub confirm_stop_pid: fn(i32) -> String,
    /// Pista del control de detener: «Detener proceso (pid 42)».
    pub stop_pid: fn(i32) -> String,

    // Notificación de algo que terminó.
    /// Lo que terminó si no era un agente.
    pub command: &'static str,
    /// Título: «Claude Code terminó».
    pub finished_title: fn(&str) -> String,
    /// Cuerpo, con la carpeta de la pestaña.
    pub finished_body: fn(&str) -> String,

    // Sugerencia en gris junto al cursor (retomar Claude Code).
    pub suggestion_keys: &'static str,
}

fn ago_es(secs: u64) -> String {
    match secs {
        0..=59 => "un momento".into(),
        60..=3599 => format!("{} min", secs / 60),
        _ => format!("{} h", secs / 3600),
    }
}

fn ago_en(secs: u64) -> String {
    match secs {
        0..=59 => "a moment".into(),
        60..=3599 => format!("{} min", secs / 60),
        _ => format!("{} h", secs / 3600),
    }
}

fn finished_ago_es(secs: u64) -> String {
    format!("Terminó hace {}", ago_es(secs))
}

fn finished_ago_en(secs: u64) -> String {
    format!("Finished {} ago", ago_en(secs))
}

fn web_servers_es(n: usize) -> String {
    match n {
        1 => "1 servidor web".to_string(),
        n => format!("{n} servidores web"),
    }
}

fn web_servers_en(n: usize) -> String {
    match n {
        1 => "1 web server".to_string(),
        n => format!("{n} web servers"),
    }
}

fn more_ports_es(n: usize) -> String {
    format!("{n} puertos más (no web)")
}

fn more_ports_en(n: usize) -> String {
    match n {
        1 => "1 more port (not web)".to_string(),
        n => format!("{n} more ports (not web)"),
    }
}

fn procs_detail_es(cpu: f32, used: f64, total: f64) -> String {
    format!("CPU {cpu:.0} %  ·  RAM {used:.0}/{total:.0} GB")
}

fn procs_detail_en(cpu: f32, used: f64, total: f64) -> String {
    format!("CPU {cpu:.0}%  ·  RAM {used:.0}/{total:.0} GB")
}

fn percent_es(value: f32, decimals: usize) -> String {
    format!("{value:.decimals$} %")
}

fn percent_en(value: f32, decimals: usize) -> String {
    format!("{value:.decimals$}%")
}

fn confirm_stop_pid_es(pid: i32) -> String {
    format!("Confirmar: terminar pid {pid}")
}

fn confirm_stop_pid_en(pid: i32) -> String {
    format!("Confirm: terminate pid {pid}")
}

fn stop_pid_es(pid: i32) -> String {
    format!("Detener proceso (pid {pid})")
}

fn stop_pid_en(pid: i32) -> String {
    format!("Stop process (pid {pid})")
}

fn finished_title_es(what: &str) -> String {
    format!("{what} terminó")
}

fn finished_title_en(what: &str) -> String {
    format!("{what} finished")
}

fn finished_body_es(project: &str) -> String {
    format!("{project} — clic o ⌘J para ir")
}

fn finished_body_en(project: &str) -> String {
    format!("{project} — click or ⌘J to go there")
}

pub const ES: T = T {
    color: "Color",
    color_names: [
        "rojo", "naranja", "amarillo", "verde", "azul", "morado", "rosa",
    ],
    no_color: "sin color",
    rename: "Renombrar…",
    close_tab: "Cerrar pestaña",
    new_tab_hint: "Nueva pestaña  ⌘T",
    procs_hint: "Procesos  ⌘⇧M",
    ports_hint: "Puertos  ⌘⇧P",
    update: "Actualizar",
    update_hint: "Hay una compilación nueva. Reinicia Colibrí y reabre tus pestañas;\nlo que esté corriendo en ellas se detiene.",
    update_available: |v| format!("Colibrí {v} disponible"),
    update_available_brew: "Abre una pestaña con «brew upgrade --cask colibri». Al terminar,\neste botón pasa a «Actualizar» para reiniciar con la versión nueva.",
    update_available_web: "Abre en el navegador la página de la versión para descargarla.",
    running: "Corriendo…",
    finished_ago: finished_ago_es,
    ago: ago_es,

    hide: "Ocultar",
    ports: "Puertos",
    web_servers: web_servers_es,
    refresh_now: "Actualizar ahora",
    no_web_servers: "No hay servidores web corriendo",
    more_ports: more_ports_es,
    stopping: "Deteniendo…",
    force: "Forzar",
    stop: "Detener",
    stop_process: "Detener el proceso",
    copy_url: "Copiar la URL",
    open_browser: "Abrir en el navegador",
    go_to_tab: "Ir a la pestaña que lo lanzó",
    processes: "Procesos",
    procs_detail: procs_detail_es,
    search: "Buscar…",
    col_cpu: "CPU",
    col_memory: "Memoria",
    sort_by: "Ordenar por esta columna",
    reading_procs: "Leyendo procesos…",
    no_match: "Nada coincide con la búsqueda",
    percent: percent_es,
    confirm_stop_pid: confirm_stop_pid_es,
    stop_pid: stop_pid_es,

    command: "Comando",
    finished_title: finished_title_es,
    finished_body: finished_body_es,

    suggestion_keys: "↩ ejecutar  ·  ⇥ escribir",
};

pub const EN: T = T {
    color: "Color",
    color_names: ["Red", "Orange", "Yellow", "Green", "Blue", "Purple", "Pink"],
    no_color: "No Color",
    rename: "Rename…",
    close_tab: "Close Tab",
    new_tab_hint: "New Tab  ⌘T",
    procs_hint: "Processes  ⌘⇧M",
    ports_hint: "Ports  ⌘⇧P",
    update: "Update",
    update_hint: "A new build is available. Restart Colibrí and reopen your tabs;\nanything running in them will stop.",
    update_available: |v| format!("Colibrí {v} available"),
    update_available_brew: "Opens a tab running “brew upgrade --cask colibri”. When it finishes,\nthis button turns into “Update” to restart with the new version.",
    update_available_web: "Opens the release page in your browser to download it.",
    running: "Running…",
    finished_ago: finished_ago_en,
    ago: ago_en,

    hide: "Hide",
    ports: "Ports",
    web_servers: web_servers_en,
    refresh_now: "Refresh now",
    no_web_servers: "No web servers running",
    more_ports: more_ports_en,
    stopping: "Stopping…",
    force: "Force",
    stop: "Stop",
    stop_process: "Stop the process",
    copy_url: "Copy the URL",
    open_browser: "Open in browser",
    go_to_tab: "Go to the tab that started it",
    processes: "Processes",
    procs_detail: procs_detail_en,
    search: "Search…",
    col_cpu: "CPU",
    col_memory: "Memory",
    sort_by: "Sort by this column",
    reading_procs: "Reading processes…",
    no_match: "Nothing matches your search",
    percent: percent_en,
    confirm_stop_pid: confirm_stop_pid_en,
    stop_pid: stop_pid_en,

    command: "Command",
    finished_title: finished_title_en,
    finished_body: finished_body_en,

    suggestion_keys: "↩ run  ·  ⇥ insert",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hace_cuanto_en_los_dos_idiomas() {
        let cases = [30, 5 * 60, 2 * 3600, 3 * 86_400];
        let es: Vec<String> = cases.iter().map(|s| (ES.ago)(*s)).collect();
        let en: Vec<String> = cases.iter().map(|s| (EN.ago)(*s)).collect();
        assert_eq!(es, ["un momento", "5 min", "2 h", "72 h"]);
        assert_eq!(en, ["a moment", "5 min", "2 h", "72 h"]);
        let es: Vec<String> = cases.iter().map(|s| (ES.finished_ago)(*s)).collect();
        let en: Vec<String> = cases.iter().map(|s| (EN.finished_ago)(*s)).collect();
        assert_eq!(
            es,
            [
                "Terminó hace un momento",
                "Terminó hace 5 min",
                "Terminó hace 2 h",
                "Terminó hace 72 h"
            ]
        );
        assert_eq!(
            en,
            [
                "Finished a moment ago",
                "Finished 5 min ago",
                "Finished 2 h ago",
                "Finished 72 h ago"
            ]
        );
    }

    #[test]
    fn plurales() {
        assert_eq!((ES.web_servers)(0), "0 servidores web");
        assert_eq!((ES.web_servers)(1), "1 servidor web");
        assert_eq!((ES.web_servers)(3), "3 servidores web");
        assert_eq!((EN.web_servers)(0), "0 web servers");
        assert_eq!((EN.web_servers)(1), "1 web server");
        assert_eq!((EN.web_servers)(3), "3 web servers");
        assert_eq!((ES.more_ports)(4), "4 puertos más (no web)");
        assert_eq!((EN.more_ports)(1), "1 more port (not web)");
        assert_eq!((EN.more_ports)(4), "4 more ports (not web)");
    }

    #[test]
    fn textos_con_datos() {
        assert_eq!(
            (ES.procs_detail)(12.4, 8.2, 16.0),
            "CPU 12 %  ·  RAM 8/16 GB"
        );
        assert_eq!(
            (EN.procs_detail)(12.4, 8.2, 16.0),
            "CPU 12%  ·  RAM 8/16 GB"
        );
        assert_eq!((ES.percent)(42.0, 0), "42 %");
        assert_eq!((ES.percent)(3.21, 1), "3.2 %");
        assert_eq!((EN.percent)(3.21, 1), "3.2%");
        assert_eq!((ES.confirm_stop_pid)(42), "Confirmar: terminar pid 42");
        assert_eq!((EN.confirm_stop_pid)(42), "Confirm: terminate pid 42");
        assert_eq!((ES.stop_pid)(42), "Detener proceso (pid 42)");
        assert_eq!((EN.stop_pid)(42), "Stop process (pid 42)");
        assert_eq!((ES.finished_title)("Claude Code"), "Claude Code terminó");
        assert_eq!((EN.finished_title)("Claude Code"), "Claude Code finished");
        assert_eq!((ES.finished_body)("colibri"), "colibri — clic o ⌘J para ir");
        assert_eq!(
            (EN.finished_body)("colibri"),
            "colibri — click or ⌘J to go there"
        );
    }
}
