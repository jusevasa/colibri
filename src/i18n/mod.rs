//! Textos de la interfaz en cada idioma.
//!
//! Colibrí sale en el idioma del sistema: inglés si el primero de los preferidos es inglés
//! y español en cualquier otro caso. El idioma se decide una vez al arrancar (`init`);
//! `COLIBRI_LANG=es|en` lo fuerza para probar.
//!
//! Los textos van en un struct por zona de la interfaz (`menu`, `overlay`, `chrome`,
//! `agents`, `term`), cada uno con sus constantes `ES` y `EN`: si a un idioma le falta un
//! texto, no compila. Los que llevan datos (plurales, «hace 3 min») son funciones, para
//! que cada idioma ordene las palabras a su manera. Un idioma nuevo es otra constante en
//! cada archivo de esta carpeta y una rama en `pick`.
//!
//! No se traduce lo que no ve el usuario (stderr de depuración) ni lo que viene de fuera
//! (títulos que pone el programa de la terminal, carpetas, ramas, salida de comandos).

pub mod agents;
pub mod chrome;
pub mod menu;
pub mod overlay;
pub mod term;

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

pub struct Texts {
    pub menu: menu::T,
    pub overlay: overlay::T,
    pub chrome: chrome::T,
    pub agents: agents::T,
    pub term: term::T,
}

static ES: Texts = Texts {
    menu: menu::ES,
    overlay: overlay::ES,
    chrome: chrome::ES,
    agents: agents::ES,
    term: term::ES,
};

static EN: Texts = Texts {
    menu: menu::EN,
    overlay: overlay::EN,
    chrome: chrome::EN,
    agents: agents::EN,
    term: term::EN,
};

static LANG: OnceLock<Lang> = OnceLock::new();

/// Decide el idioma: `COLIBRI_LANG` si está, si no el primero de `AppleLanguages` (con el
/// del dominio de la app delante: así vale el idioma elegido para Colibrí en Ajustes).
pub fn init() {
    LANG.get_or_init(|| {
        if let Ok(forced) = std::env::var("COLIBRI_LANG") {
            return pick(&forced);
        }
        // El guion de pruebas corre en español en cualquier Mac (sus pasos `menu <título>`
        // y las capturas de referencia lo dan por hecho), salvo que se pida otro idioma.
        if std::env::var_os("COLIBRI_SCRIPT").is_some() {
            return Lang::Es;
        }
        pick(&system_language().unwrap_or_default())
    });
}

/// El idioma de la interfaz.
pub fn lang() -> Lang {
    *LANG.get_or_init(|| Lang::Es)
}

/// Los textos del idioma de la interfaz.
pub fn t() -> &'static Texts {
    texts(lang())
}

/// Los textos de un idioma concreto (para las pruebas).
pub fn texts(lang: Lang) -> &'static Texts {
    match lang {
        Lang::Es => &ES,
        Lang::En => &EN,
    }
}

/// El idioma que corresponde a una etiqueta como `en-US` o `es-CO`. Español por defecto.
fn pick(tag: &str) -> Lang {
    let main = tag.split(['-', '_']).next().unwrap_or("");
    if main.eq_ignore_ascii_case("en") {
        Lang::En
    } else {
        Lang::Es
    }
}

fn system_language() -> Option<String> {
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    let list = defaults.stringArrayForKey(objc2_foundation::ns_string!("AppleLanguages"))?;
    let first = list.iter().next()?;
    Some(first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elige_idioma() {
        assert_eq!(pick("en-US"), Lang::En);
        assert_eq!(pick("en"), Lang::En);
        assert_eq!(pick("en_GB"), Lang::En);
        assert_eq!(pick("es-CO"), Lang::Es);
        assert_eq!(pick("fr-FR"), Lang::Es);
        assert_eq!(pick(""), Lang::Es);
    }
}
