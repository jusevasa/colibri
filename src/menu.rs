//! La barra de menús de macOS: Colibrí, Archivo, Edición, Visualización y Ventana.
//!
//! Sustituye a la que pone winit (solo «Quit» y «Hide»). Cada entrada lleva el atajo de la
//! configuración como equivalente de teclado, así que el menú enseña los atajos reales.
//! Ojo: macOS da las teclas a la barra de menús antes que a la ventana; un atajo que está en
//! el menú ya no llega a egui como tecla, sino como la acción de su entrada. Por eso cada
//! entrada hace exactamente lo mismo que su atajo:
//! - las acciones de Colibrí (`Cmd`) van a `App::run`, como hace `App::shortcuts`;
//! - las de Edición (⌘C, ⌘X, ⌘V, ⌘A) se vuelven a inyectar como teclas en la ventana del
//!   frente, para que sigan su camino de siempre (`clip::translate`, la selección de zle…);
//! - las del sistema (ocultar, salir, minimizar, pantalla completa) llaman a su selector de
//!   AppKit.
//!
//! Se reconstruye entera cuando cambian los atajos.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{AllocAnyThread, class, define_class, msg_send, sel};
use objc2_foundation::NSString;

use crate::Cmd;
use crate::config::Shortcut;

/// Lo que hace una entrada.
#[derive(Clone, Copy)]
pub enum Action {
    /// Una acción de Colibrí, la misma que su atajo.
    Cmd(Cmd),
    /// Una tecla que se inyecta en la ventana del frente como si se hubiera pulsado.
    Key(egui::Key, egui::Modifiers),
    /// Un selector de AppKit que atiende el sistema (`hide:`, `terminate:`…).
    System(Sel),
}

pub enum Entry {
    Item {
        title: String,
        action: Action,
        shortcut: Option<Shortcut>,
    },
    Separator,
    /// El submenú «Servicios» del sistema.
    Services,
}

impl Entry {
    pub fn item(title: &str, action: Action, shortcut: Option<Shortcut>) -> Self {
        Entry::Item {
            title: title.into(),
            action,
            shortcut,
        }
    }
}

/// Un menú de la barra. El primero es el de la aplicación (macOS le pone el nombre de la
/// app); el que se llama «Ventana» recibe además la lista de ventanas abiertas.
pub struct Menu {
    pub title: String,
    pub entries: Vec<Entry>,
}

/// Lo que hace cada entrada de las nuestras, por su `tag`, y dónde está (para el guion).
struct Built {
    actions: Vec<Action>,
    /// Título, menú (`NSMenu *`) e índice de cada entrada con acción.
    places: Vec<(String, usize, isize)>,
}

static BUILT: Mutex<Built> = Mutex::new(Built {
    actions: Vec::new(),
    places: Vec::new(),
});
/// Acciones elegidas en el menú que la aplicación aún no ha atendido.
static CHOSEN: Mutex<Vec<Action>> = Mutex::new(Vec::new());
static TARGET_MADE: AtomicBool = AtomicBool::new(false);
static TARGET: Mutex<usize> = Mutex::new(0);

define_class!(
    // SAFETY: NSObject no impone requisitos de subclase y Target no implementa Drop.
    #[unsafe(super(NSObject))]
    #[name = "ColibriMenuTarget"]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(choose:))]
        fn choose(&self, sender: *mut AnyObject) {
            if sender.is_null() {
                return;
            }
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = BUILT
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .actions
                .get(tag as usize)
                .copied();
            if let Some(action) = action {
                CHOSEN
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(action);
                crate::shell::wake_all();
            }
        }
    }
);

/// El destino de nuestras entradas. El menú lo guarda como referencia débil, así que vive
/// toda la ejecución.
fn target() -> *mut AnyObject {
    let mut ptr = TARGET.lock().unwrap_or_else(|e| e.into_inner());
    if !TARGET_MADE.swap(true, Ordering::Relaxed) {
        let target: Retained<Target> = unsafe { msg_send![Target::alloc(), init] };
        *ptr = Retained::into_raw(target) as usize;
    }
    *ptr as *mut AnyObject
}

/// Lo que se ve en el menú para la tecla de un atajo, o nada si no tiene equivalente.
fn key_equivalent(key: egui::Key) -> Option<String> {
    use egui::Key as K;
    let special = |c: u32| char::from_u32(c).map(String::from);
    match key {
        K::ArrowUp => special(0xF700),
        K::ArrowDown => special(0xF701),
        K::ArrowLeft => special(0xF702),
        K::ArrowRight => special(0xF703),
        K::Delete => special(0xF728),
        K::Home => special(0xF729),
        K::End => special(0xF72B),
        K::PageUp => special(0xF72C),
        K::PageDown => special(0xF72D),
        K::Space => Some(" ".into()),
        K::Tab => Some("\t".into()),
        K::Enter => Some("\r".into()),
        K::Escape => Some("\u{1b}".into()),
        K::Backspace => Some("\u{8}".into()),
        _ => {
            let name = key.symbol_or_name();
            (name.chars().count() == 1).then(|| name.to_lowercase())
        }
    }
}

/// `NSEventModifierFlags` de un atajo.
fn modifier_mask(sc: &Shortcut) -> usize {
    const SHIFT: usize = 1 << 17;
    const CONTROL: usize = 1 << 18;
    const OPTION: usize = 1 << 19;
    const COMMAND: usize = 1 << 20;
    let mut mask = 0;
    if sc.shift {
        mask |= SHIFT;
    }
    if sc.ctrl {
        mask |= CONTROL;
    }
    if sc.alt {
        mask |= OPTION;
    }
    if sc.cmd {
        mask |= COMMAND;
    }
    mask
}

/// Pone `menus` como barra de menús. Devuelve false si macOS aún no ha terminado de
/// arrancar (winit pone su menú al acabar el lanzamiento y pisaría el nuestro): hay que
/// volver a intentarlo. Hilo principal.
pub fn build(menus: &[Menu]) -> bool {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return false;
        }
        // Mientras no esté el menú de winit, la aplicación no ha terminado de arrancar.
        let current: *mut AnyObject = msg_send![app, mainMenu];
        if current.is_null() {
            return false;
        }
        let count: isize = msg_send![current, numberOfItems];
        if count == 0 {
            return false;
        }

        let target = target();
        let mut actions = Vec::new();
        let mut places = Vec::new();
        let bar = new_menu("");
        for menu in menus {
            let sub = new_menu(&menu.title);
            for entry in &menu.entries {
                match entry {
                    Entry::Separator => {
                        let sep: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
                        let _: () = msg_send![sub, addItem: sep];
                    }
                    Entry::Services => {
                        let services = new_menu("Servicios");
                        let item = new_item("Servicios", None, "");
                        let _: () = msg_send![item, setSubmenu: services];
                        let _: () = msg_send![sub, addItem: item];
                        let _: () = msg_send![app, setServicesMenu: services];
                    }
                    Entry::Item {
                        title,
                        action,
                        shortcut,
                    } => {
                        let key = shortcut
                            .as_ref()
                            .and_then(|sc| key_equivalent(sc.key))
                            .unwrap_or_default();
                        let selector = match action {
                            Action::System(sel) => *sel,
                            _ => sel!(choose:),
                        };
                        let item = new_item(title, Some(selector), &key);
                        if !key.is_empty()
                            && let Some(sc) = shortcut
                        {
                            let _: () =
                                msg_send![item, setKeyEquivalentModifierMask: modifier_mask(sc)];
                        }
                        if !matches!(action, Action::System(_)) {
                            let _: () = msg_send![item, setTarget: target];
                            let _: () = msg_send![item, setTag: actions.len() as isize];
                            actions.push(*action);
                        }
                        let index: isize = msg_send![sub, numberOfItems];
                        places.push((title.clone(), sub as usize, index));
                        let _: () = msg_send![sub, addItem: item];
                    }
                }
            }
            let holder = new_item(&menu.title, None, "");
            let _: () = msg_send![holder, setSubmenu: sub];
            let _: () = msg_send![bar, addItem: holder];
            if menu.title == "Ventana" {
                let _: () = msg_send![app, setWindowsMenu: sub];
            }
        }
        let _: () = msg_send![app, setMainMenu: bar];
        *BUILT.lock().unwrap_or_else(|e| e.into_inner()) = Built { actions, places };
    }
    true
}

unsafe fn new_menu(title: &str) -> *mut AnyObject {
    let title = NSString::from_str(title);
    unsafe {
        let menu: *mut AnyObject = msg_send![class!(NSMenu), alloc];
        msg_send![menu, initWithTitle: &*title]
    }
}

unsafe fn new_item(title: &str, action: Option<Sel>, key: &str) -> *mut AnyObject {
    let title = NSString::from_str(title);
    let key = NSString::from_str(key);
    unsafe {
        let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        msg_send![item, initWithTitle: &*title, action: action, keyEquivalent: &*key]
    }
}

/// Lo elegido en el menú desde la última vez.
pub fn take() -> Vec<Action> {
    std::mem::take(&mut *CHOSEN.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Elige la entrada con ese título como si se hiciera clic en ella (para el guion).
/// Devuelve false si no hay ninguna. Hilo principal.
pub fn perform(title: &str) -> bool {
    let place = BUILT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .places
        .iter()
        .find(|(t, _, _)| t == title)
        .map(|(_, menu, index)| (*menu, *index));
    let Some((menu, index)) = place else {
        return false;
    };
    unsafe {
        let _: () = msg_send![menu as *mut AnyObject, performActionForItemAtIndex: index];
    }
    true
}
