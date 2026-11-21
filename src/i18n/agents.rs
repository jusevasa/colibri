//! Panel de agentes (`agents.rs`): botones de arriba, buscador, filtros, filas y pistas.

pub struct T {
    // Botones de arriba.
    pub new_session: &'static str,
    pub new_session_in_folder: &'static str,
    /// Pista de cada icono de abajo: «Nueva sesión con Claude Code».
    pub new_session_with: fn(&str) -> String,
    // Buscador y filtros (en una fila de píldoras con su contador al lado).
    pub search: &'static str,
    pub filter_all: &'static str,
    pub filter_waiting: &'static str,
    pub filter_working: &'static str,
    // Lista vacía.
    pub empty: &'static str,
    pub no_match: &'static str,
    // Estado de cada fila.
    pub working: &'static str,
    pub done: &'static str,
    pub waiting: &'static str,
    // Botón de ocultar o mostrar el panel.
    /// Pista: si el panel se ve y el atajo, «Ocultar panel de agentes  ⌥⌘A».
    pub toggle_hint: fn(bool, &str) -> String,
}

fn new_session_with_es(agent: &str) -> String {
    format!("Nueva sesión con {agent}")
}

fn new_session_with_en(agent: &str) -> String {
    format!("New Session with {agent}")
}

fn toggle_hint_es(shown: bool, key: &str) -> String {
    let verb = if shown { "Ocultar" } else { "Mostrar" };
    format!("{verb} panel de agentes  {key}")
}

fn toggle_hint_en(shown: bool, key: &str) -> String {
    let verb = if shown { "Hide" } else { "Show" };
    format!("{verb} Agents Panel  {key}")
}

pub const ES: T = T {
    new_session: "Nueva sesión",
    new_session_in_folder: "Nueva sesión en otra carpeta…",
    new_session_with: new_session_with_es,
    search: "Buscar agentes",
    filter_all: "Todos",
    filter_waiting: "Esperando",
    filter_working: "Trabajando",
    empty: "Sin agentes. Abre uno con los iconos de abajo.",
    no_match: "Nada coincide.",
    working: "Trabajando…",
    done: "Terminó",
    waiting: "Esperando tu mensaje",
    toggle_hint: toggle_hint_es,
};

pub const EN: T = T {
    new_session: "New Session",
    new_session_in_folder: "New Session in Another Folder…",
    new_session_with: new_session_with_en,
    search: "Search agents",
    filter_all: "All",
    filter_waiting: "Waiting",
    filter_working: "Working",
    empty: "No agents. Start one with the icons below.",
    no_match: "Nothing matches.",
    working: "Working…",
    done: "Finished",
    waiting: "Waiting for your message",
    toggle_hint: toggle_hint_en,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textos_con_datos() {
        assert_eq!(
            (ES.new_session_with)("Claude Code"),
            "Nueva sesión con Claude Code"
        );
        assert_eq!(
            (EN.new_session_with)("Claude Code"),
            "New Session with Claude Code"
        );
        assert_eq!(
            (ES.toggle_hint)(true, "⌥⌘A"),
            "Ocultar panel de agentes  ⌥⌘A"
        );
        assert_eq!(
            (ES.toggle_hint)(false, "⌥⌘A"),
            "Mostrar panel de agentes  ⌥⌘A"
        );
        assert_eq!((EN.toggle_hint)(true, "⌥⌘A"), "Hide Agents Panel  ⌥⌘A");
        assert_eq!((EN.toggle_hint)(false, "⌥⌘A"), "Show Agents Panel  ⌥⌘A");
    }
}
