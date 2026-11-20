//! Entrada «Reiniciar Colibrí» en el menú de la aplicación (el que lleva su nombre en la
//! barra de menús de macOS), junto a «Quit».

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{AllocAnyThread, class, define_class, msg_send, sel};
use objc2_foundation::NSString;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static RESTART: AtomicBool = AtomicBool::new(false);

define_class!(
    // SAFETY: NSObject no impone requisitos de subclase y Target no implementa Drop.
    #[unsafe(super(NSObject))]
    #[name = "ColibriMenuTarget"]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(restart:))]
        fn restart(&self, _sender: *mut AnyObject) {
            RESTART.store(true, Ordering::Relaxed);
            crate::shell::wake_all();
        }
    }
);

/// Añade la entrada si aún no está. El menú lo crea el sistema de ventanas al arrancar, así
/// que puede no existir todavía: se llama en cada cuadro hasta que lo consigue (después no
/// cuesta nada). Debe llamarse desde el hilo principal.
pub fn install() {
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }
        let bar: *mut AnyObject = msg_send![app, mainMenu];
        if bar.is_null() {
            return;
        }
        let count: isize = msg_send![bar, numberOfItems];
        if count == 0 {
            return;
        }
        let first: *mut AnyObject = msg_send![bar, itemAtIndex: 0isize];
        let menu: *mut AnyObject = msg_send![first, submenu];
        if menu.is_null() {
            return;
        }
        let target: Retained<Target> = msg_send![Target::alloc(), init];
        let title = NSString::from_str("Reiniciar Colibrí");
        let key = NSString::from_str("");
        let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let item: *mut AnyObject = msg_send![
            item,
            initWithTitle: &*title,
            action: sel!(restart:),
            keyEquivalent: &*key,
        ];
        let _: () = msg_send![item, setTarget: &*target];
        // El menú guarda el destino como referencia débil: debe vivir toda la ejecución.
        std::mem::forget(target);
        // Justo antes de «Quit», que es la última entrada.
        let items: isize = msg_send![menu, numberOfItems];
        let _: () = msg_send![menu, insertItem: item, atIndex: (items - 1).max(0)];
    }
    INSTALLED.store(true, Ordering::Relaxed);
}

/// True una vez si se eligió «Reiniciar Colibrí» en el menú.
pub fn take_restart() -> bool {
    RESTART.swap(false, Ordering::Relaxed)
}
