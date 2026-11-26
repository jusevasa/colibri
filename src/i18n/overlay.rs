//! Ventanas flotantes (`overlay.rs`): paleta de comandos, Preferencias, renombrar pestaña,
//! historial, desplegables de las cajas del prompt y sugerencias con Tab.

pub struct T {
    // Paleta de comandos: entradas.
    pub new_tab: &'static str,
    pub close_tab: &'static str,
    pub new_window: &'static str,
    pub close_window: &'static str,
    pub reopen_closed: &'static str,
    pub next_tab: &'static str,
    pub prev_tab: &'static str,
    pub move_tab_left: &'static str,
    pub move_tab_right: &'static str,
    pub rename_tab: &'static str,
    pub next_agent: &'static str,
    pub prev_agent: &'static str,
    pub jump_finished: &'static str,
    pub prefs: &'static str,
    pub agents_panel: &'static str,
    pub search_agents: &'static str,
    pub ports_panel: &'static str,
    pub procs_panel: &'static str,
    pub toggle_panel: &'static str,
    pub font_bigger: &'static str,
    pub font_smaller: &'static str,
    pub font_reset: &'static str,
    pub clear: &'static str,
    pub open_config: &'static str,
    pub restart: &'static str,
    pub history: &'static str,
    /// «Ir a: <pestaña>».
    pub go_to: fn(&str) -> String,
    // Paleta de comandos: buscador.
    pub palette_placeholder: &'static str,
    pub no_results: &'static str,

    // Preferencias: título y apartados.
    pub prefs_title: &'static str,
    pub sec_appearance: &'static str,
    pub sec_terminal: &'static str,
    pub sec_panels: &'static str,
    pub sec_advanced: &'static str,
    // Preferencias: etiquetas (una por fila de `PREF_ROWS`).
    pub pref_theme: &'static str,
    pub pref_font: &'static str,
    pub pref_font_size: &'static str,
    pub pref_line_height: &'static str,
    pub pref_padding: &'static str,
    pub pref_cursor: &'static str,
    pub pref_cursor_blink: &'static str,
    pub pref_opacity: &'static str,
    pub pref_blur: &'static str,
    pub pref_scrollback: &'static str,
    pub pref_restore_tabs: &'static str,
    pub pref_notify: &'static str,
    pub pref_history_popup: &'static str,
    pub pref_complete_popup: &'static str,
    pub pref_link_click: &'static str,
    pub pref_status_bar: &'static str,
    pub pref_shell_integration: &'static str,
    pub pref_editor: &'static str,
    pub pref_secure_input: &'static str,
    pub pref_check_updates: &'static str,
    pub pref_show_agents: &'static str,
    pub pref_agents_side: &'static str,
    pub pref_show_ports: &'static str,
    pub pref_show_processes: &'static str,
    pub pref_open_file: &'static str,
    // Preferencias: valores mostrados.
    pub custom_theme: &'static str,
    pub custom_font: &'static str,
    pub cursor_bar: &'static str,
    pub cursor_underline: &'static str,
    pub cursor_block: &'static str,
    /// Opacidad, «85 %».
    pub percent: fn(f32) -> String,
    /// Historial por pestaña, «5000 líneas».
    pub lines: fn(usize) -> String,
    pub editor_default: &'static str,
    pub side_right: &'static str,
    pub side_left: &'static str,
    /// Entrada segura: apagada, solo con las contraseñas o siempre.
    pub secure_off: &'static str,
    pub secure_passwords: &'static str,
    pub secure_always: &'static str,
    // Preferencias: pista del pie según la fila elegida.
    pub hint_font: &'static str,
    pub hint_opacity: &'static str,
    pub hint_blur: &'static str,
    pub hint_scrollback: &'static str,
    pub hint_theme: &'static str,
    pub hint_panels: &'static str,
    pub hint_agents_side: &'static str,
    pub hint_secure_input: &'static str,
    pub hint_default: &'static str,

    // Renombrar pestaña.
    pub rename_title: &'static str,
    pub rename_placeholder: &'static str,
    pub rename_keys: &'static str,

    // Historial de comandos.
    /// Hace cuánto se usó, a partir de los segundos transcurridos.
    pub ago: fn(u64) -> String,
    /// «3 de 40»: posición del elegido en la lista (también en las sugerencias).
    pub of: fn(usize, usize) -> String,
    /// «empiezan por «git»»: lo escrito antes de abrir la lista.
    pub starting_with: fn(&str) -> String,
    pub history_keys: &'static str,

    // Sugerencias con Tab.
    pub kind_command: &'static str,
    pub kind_folder: &'static str,
    pub kind_file: &'static str,
    pub kind_branch: &'static str,
    pub kind_current_branch: &'static str,
    /// Rama de un remoto que aún no tiene rama local.
    pub kind_remote_branch: &'static str,
    /// Panel de detalles de las sugerencias: «alias de git checkout».
    pub detail_alias: fn(&str) -> String,
    pub detail_builtin: &'static str,
    /// Elementos de una carpeta.
    pub detail_entries: fn(usize) -> String,
    /// Tamaño y hace cuánto se modificó un archivo.
    pub detail_file: fn(&str, &str) -> String,
    /// «12 opciones», sin nada elegido.
    pub options: fn(usize) -> String,
    pub complete_keys: &'static str,

    // Desplegables de las cajas del prompt (ramas y carpetas).
    pub search_branch: &'static str,
    pub new_tab_folder: &'static str,
    pub search_files: &'static str,
    pub no_matches: &'static str,
    pub current_branch: &'static str,
    pub parent_folder: &'static str,
}

fn go_to_es(tab: &str) -> String {
    format!("Ir a: {tab}")
}

fn go_to_en(tab: &str) -> String {
    format!("Go to: {tab}")
}

fn percent_es(value: f32) -> String {
    format!("{value:.0} %")
}

fn percent_en(value: f32) -> String {
    format!("{value:.0}%")
}

fn lines_es(n: usize) -> String {
    if n == 1 {
        "1 línea".to_string()
    } else {
        format!("{n} líneas")
    }
}

fn lines_en(n: usize) -> String {
    if n == 1 {
        "1 line".to_string()
    } else {
        format!("{n} lines")
    }
}

fn ago_es(secs: u64) -> String {
    match secs {
        s if s < 60 => "ahora".to_string(),
        s if s < 3600 => format!("hace {} min", s / 60),
        s if s < 86_400 => format!("hace {} h", s / 3600),
        s => format!("hace {} d", s / 86_400),
    }
}

fn ago_en(secs: u64) -> String {
    match secs {
        s if s < 60 => "now".to_string(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

fn of_es(i: usize, n: usize) -> String {
    format!("{i} de {n}")
}

fn of_en(i: usize, n: usize) -> String {
    format!("{i} of {n}")
}

fn starting_with_es(text: &str) -> String {
    format!("empiezan por «{text}»")
}

fn starting_with_en(text: &str) -> String {
    format!("starting with “{text}”")
}

fn options_es(n: usize) -> String {
    if n == 1 {
        "1 opción".to_string()
    } else {
        format!("{n} opciones")
    }
}

fn options_en(n: usize) -> String {
    if n == 1 {
        "1 option".to_string()
    } else {
        format!("{n} options")
    }
}

pub const ES: T = T {
    new_tab: "Nueva pestaña",
    close_tab: "Cerrar pestaña",
    new_window: "Nueva ventana",
    close_window: "Cerrar ventana",
    reopen_closed: "Reabrir lo último cerrado",
    next_tab: "Pestaña siguiente",
    prev_tab: "Pestaña anterior",
    move_tab_left: "Mover pestaña a la izquierda",
    move_tab_right: "Mover pestaña a la derecha",
    rename_tab: "Renombrar pestaña…",
    next_agent: "Agente siguiente",
    prev_agent: "Agente anterior",
    jump_finished: "Ir al agente que terminó",
    prefs: "Preferencias: tema, fuente, tamaño…",
    agents_panel: "Panel de agentes",
    search_agents: "Buscar agentes",
    ports_panel: "Panel de puertos",
    procs_panel: "Panel de procesos (CPU y memoria)",
    toggle_panel: "Mostrar u ocultar paneles",
    font_bigger: "Agrandar texto",
    font_smaller: "Reducir texto",
    font_reset: "Tamaño de texto original",
    clear: "Limpiar pantalla e historial",
    open_config: "Abrir config.toml en el editor",
    restart: "Reiniciar Colibrí con la última compilación",
    history: "Historial de comandos",
    go_to: go_to_es,
    palette_placeholder: "Escribe un comando o el nombre de una pestaña",
    no_results: "Sin resultados",

    prefs_title: "Preferencias",
    sec_appearance: "APARIENCIA",
    sec_terminal: "TERMINAL",
    sec_panels: "PANELES VISIBLES AL ABRIR",
    sec_advanced: "AVANZADO",
    pref_theme: "Tema",
    pref_font: "Fuente",
    pref_font_size: "Tamaño de fuente",
    pref_line_height: "Interlineado",
    pref_padding: "Margen",
    pref_cursor: "Cursor",
    pref_cursor_blink: "Cursor parpadeante",
    pref_opacity: "Opacidad del fondo",
    pref_blur: "Desenfocar el fondo",
    pref_scrollback: "Historial por pestaña",
    pref_restore_tabs: "Recuperar pestañas al abrir",
    pref_notify: "Notificar cuando algo termina",
    pref_history_popup: "Historial de comandos con ↑",
    pref_complete_popup: "Sugerir carpetas y archivos con ⇥",
    pref_link_click: "Abrir enlaces con un clic",
    pref_status_bar: "Carpeta y rama en cajas, en el prompt",
    pref_shell_integration: "Integración con zsh (pestañas nuevas)",
    pref_editor: "Abrir archivos (⌘+clic) con",
    pref_secure_input: "Entrada segura de teclado",
    pref_check_updates: "Buscar actualizaciones",
    pref_show_agents: "Agentes",
    pref_agents_side: "Lado del panel de agentes",
    pref_show_ports: "Puertos",
    pref_show_processes: "Procesos",
    pref_open_file: "Abrir config.toml en el editor…",
    custom_theme: "Personalizado",
    custom_font: "Personalizada",
    cursor_bar: "Barra",
    cursor_underline: "Subrayado",
    cursor_block: "Bloque",
    percent: percent_es,
    lines: lines_es,
    editor_default: "Lo que diga macOS",
    side_right: "Derecha",
    side_left: "Izquierda",
    secure_off: "No",
    secure_passwords: "En contraseñas",
    secure_always: "Siempre",
    hint_font: "Menlo, Hack (incluida) y las monoespaciadas de tu Mac. Otras: font_path en el archivo.",
    hint_opacity: "Menos de 100 % deja ver lo que hay detrás de la ventana.",
    hint_blur: "Solo se nota con la opacidad por debajo de 100 %.",
    hint_scrollback: "Se aplica a las pestañas que abras a partir de ahora.",
    hint_theme: "Colores a medida: sección [theme] del archivo.",
    hint_panels: "Qué apartados aparecen abiertos al iniciar Colibrí.",
    hint_agents_side: "Puertos y Procesos van en el lado contrario.",
    hint_secure_input: "Siempre: lo que abras desde la terminal (open) no podrá pasar al frente.",
    hint_default: "Los cambios se guardan al momento.",

    rename_title: "Nombre de la pestaña",
    rename_placeholder: "automático (carpeta o agente)",
    rename_keys: "↩ guardar  ·  vacío = automático  ·  esc cancelar",

    ago: ago_es,
    of: of_es,
    starting_with: starting_with_es,
    history_keys: "↑↓ elegir  ·  ↩ ejecutar  ·  ⇥ editar  ·  esc cancelar",

    kind_command: "Comando",
    kind_folder: "Carpeta",
    kind_file: "Archivo",
    kind_branch: "Rama",
    kind_current_branch: "Rama actual",
    kind_remote_branch: "Rama remota",
    detail_alias: |v| format!("alias de {v}"),
    detail_builtin: "Orden interna del shell",
    detail_entries: |n| {
        if n == 1 {
            "1 elemento".to_string()
        } else {
            format!("{n} elementos")
        }
    },
    detail_file: |size, ago| format!("{size} · modificado {ago}"),
    options: options_es,
    complete_keys: "⇥ ↑↓ elegir  ·  escribe para filtrar  ·  esc cerrar",

    search_branch: "Buscar rama…",
    new_tab_folder: "Carpeta para la sesión nueva…  ·  → entra  ·  ↩ abre aquí",
    search_files: "Buscar carpeta o archivo…  ·  espacio: vista previa",
    no_matches: "Nada coincide",
    current_branch: "actual",
    parent_folder: ".. (carpeta superior)",
};

pub const EN: T = T {
    new_tab: "New Tab",
    close_tab: "Close Tab",
    new_window: "New Window",
    close_window: "Close Window",
    reopen_closed: "Reopen Closed Tab",
    next_tab: "Next Tab",
    prev_tab: "Previous Tab",
    move_tab_left: "Move Tab Left",
    move_tab_right: "Move Tab Right",
    rename_tab: "Rename Tab…",
    next_agent: "Next Agent",
    prev_agent: "Previous Agent",
    jump_finished: "Go to Finished Agent",
    prefs: "Settings: Theme, Font, Size…",
    agents_panel: "Agents Panel",
    search_agents: "Search Agents",
    ports_panel: "Ports Panel",
    procs_panel: "Processes Panel (CPU and Memory)",
    toggle_panel: "Show or Hide Panels",
    font_bigger: "Bigger Text",
    font_smaller: "Smaller Text",
    font_reset: "Actual Text Size",
    clear: "Clear Screen and Scrollback",
    open_config: "Open config.toml in Editor",
    restart: "Restart Colibrí with Latest Build",
    history: "Command History",
    go_to: go_to_en,
    palette_placeholder: "Type a command or a tab name",
    no_results: "No results",

    prefs_title: "Settings",
    sec_appearance: "APPEARANCE",
    sec_terminal: "TERMINAL",
    sec_panels: "PANELS SHOWN AT LAUNCH",
    sec_advanced: "ADVANCED",
    pref_theme: "Theme",
    pref_font: "Font",
    pref_font_size: "Font size",
    pref_line_height: "Line spacing",
    pref_padding: "Padding",
    pref_cursor: "Cursor",
    pref_cursor_blink: "Blinking cursor",
    pref_opacity: "Background opacity",
    pref_blur: "Blur background",
    pref_scrollback: "Scrollback per tab",
    pref_restore_tabs: "Restore tabs at launch",
    pref_notify: "Notify when something finishes",
    pref_history_popup: "Command history with ↑",
    pref_complete_popup: "Suggest folders and files with ⇥",
    pref_link_click: "Open links with one click",
    pref_status_bar: "Folder and branch as chips in the prompt",
    pref_shell_integration: "zsh integration (new tabs)",
    pref_editor: "Open files (⌘+click) with",
    pref_secure_input: "Secure keyboard entry",
    pref_check_updates: "Check for updates",
    pref_show_agents: "Agents",
    pref_agents_side: "Agents panel side",
    pref_show_ports: "Ports",
    pref_show_processes: "Processes",
    pref_open_file: "Open config.toml in editor…",
    custom_theme: "Custom",
    custom_font: "Custom",
    cursor_bar: "Bar",
    cursor_underline: "Underline",
    cursor_block: "Block",
    percent: percent_en,
    lines: lines_en,
    editor_default: "macOS default",
    side_right: "Right",
    side_left: "Left",
    secure_off: "Off",
    secure_passwords: "For passwords",
    secure_always: "Always",
    hint_font: "Menlo, Hack (built in) and your Mac's monospaced fonts. Others: font_path in the file.",
    hint_opacity: "Below 100% you can see what's behind the window.",
    hint_blur: "Only noticeable with opacity below 100%.",
    hint_scrollback: "Applies to tabs you open from now on.",
    hint_theme: "Custom colors: [theme] section of the file.",
    hint_panels: "Which panels are open when Colibrí starts.",
    hint_agents_side: "Ports and Processes go on the opposite side.",
    hint_secure_input: "Always: apps you open from the terminal (open) can't come to the front.",
    hint_default: "Changes are saved right away.",

    rename_title: "Tab name",
    rename_placeholder: "automatic (folder or agent)",
    rename_keys: "↩ save  ·  empty = automatic  ·  esc cancel",

    ago: ago_en,
    of: of_en,
    starting_with: starting_with_en,
    history_keys: "↑↓ choose  ·  ↩ run  ·  ⇥ edit  ·  esc cancel",

    kind_command: "Command",
    kind_folder: "Folder",
    kind_file: "File",
    kind_branch: "Branch",
    kind_current_branch: "Current branch",
    kind_remote_branch: "Remote branch",
    detail_alias: |v| format!("alias for {v}"),
    detail_builtin: "Shell builtin",
    detail_entries: |n| {
        if n == 1 {
            "1 item".to_string()
        } else {
            format!("{n} items")
        }
    },
    detail_file: |size, ago| format!("{size} · modified {ago}"),
    options: options_en,
    complete_keys: "⇥ ↑↓ choose  ·  type to filter  ·  esc close",

    search_branch: "Search branches…",
    new_tab_folder: "Folder for the new session…  ·  → go in  ·  ↩ open here",
    search_files: "Search folders and files…  ·  space: preview",
    no_matches: "No matches",
    current_branch: "current",
    parent_folder: ".. (parent folder)",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hace_cuanto_en_los_dos_idiomas() {
        let cases = [30, 120, 2 * 3600, 3 * 86_400];
        let es: Vec<String> = cases.iter().map(|s| (ES.ago)(*s)).collect();
        let en: Vec<String> = cases.iter().map(|s| (EN.ago)(*s)).collect();
        assert_eq!(es, ["ahora", "hace 2 min", "hace 2 h", "hace 3 d"]);
        assert_eq!(en, ["now", "2 min ago", "2 h ago", "3 d ago"]);
    }

    #[test]
    fn plurales() {
        assert_eq!((ES.options)(5), "5 opciones");
        assert_eq!((EN.options)(1), "1 option");
        assert_eq!((EN.options)(2), "2 options");
        assert_eq!((ES.lines)(5000), "5000 líneas");
        assert_eq!((EN.lines)(1), "1 line");
        assert_eq!((EN.lines)(5000), "5000 lines");
        assert_eq!((ES.percent)(85.0), "85 %");
        assert_eq!((EN.percent)(85.0), "85%");
    }
}
