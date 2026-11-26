//! La terminal (`view.rs`, `session.rs`): lo que Colibrí pinta junto a la rejilla y el
//! nombre genérico de un agente. El resto de lo que se ve en la rejilla viene del programa
//! de la terminal y no se traduce.

pub struct T {
    // Agente reconocido sin nombre propio (`Agent::Other`).
    pub other_agent: &'static str,
    // Junto a las cajas de un prompt: cómo acabó el comando anterior.
    /// Lo que tardó, a partir de los milisegundos: «850 ms», «2.5 s», «3 min 7 s».
    pub duration: fn(u64) -> String,
    /// El comando falló: código de salida y duración ya formateada.
    pub failed: fn(i32, &str) -> String,
    /// Recuadro de un enlace web bajo el puntero: qué hace un clic.
    pub link_open: &'static str,
}

fn duration_es(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f32 / 1000.0)
    } else {
        format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000)
    }
}

/// Las unidades son las mismas en inglés.
fn duration_en(ms: u64) -> String {
    duration_es(ms)
}

fn failed_es(status: i32, time: &str) -> String {
    format!("falló ({status})  ·  {time}")
}

fn failed_en(status: i32, time: &str) -> String {
    format!("failed ({status})  ·  {time}")
}

pub const ES: T = T {
    other_agent: "Agente",
    duration: duration_es,
    failed: failed_es,
    link_open: "Clic para abrir",
};

pub const EN: T = T {
    other_agent: "Agent",
    duration: duration_en,
    failed: failed_en,
    link_open: "Click to open",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duraciones_en_los_dos_idiomas() {
        let cases = [850, 2_500, 187_000];
        let es: Vec<String> = cases.iter().map(|ms| (ES.duration)(*ms)).collect();
        let en: Vec<String> = cases.iter().map(|ms| (EN.duration)(*ms)).collect();
        assert_eq!(es, ["850 ms", "2.5 s", "3 min 7 s"]);
        assert_eq!(en, ["850 ms", "2.5 s", "3 min 7 s"]);
    }

    #[test]
    fn comando_que_falla() {
        assert_eq!((ES.failed)(127, "850 ms"), "falló (127)  ·  850 ms");
        assert_eq!((EN.failed)(127, "850 ms"), "failed (127)  ·  850 ms");
    }
}
