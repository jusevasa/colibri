//! Integración con macOS que no cabe en otro módulo: carpetas que llegan de Finder
//! («Abrir con», arrastrar al icono del Dock, el servicio «Nueva pestaña de Colibrí aquí»),
//! el número del icono del Dock, la entrada segura de teclado y saber si la ventana se ve.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{AllocAnyThread, class, define_class, msg_send, sel};
use objc2_foundation::NSString;

static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn EnableSecureEventInput() -> i32;
    fn DisableSecureEventInput() -> i32;
}

/// 'aevt' / 'odoc': el evento con el que Finder pide abrir archivos; '----' es su lista.
const CORE_EVENT_CLASS: u32 = 0x6165_7674;
const OPEN_DOCUMENTS: u32 = 0x6F64_6F63;
const DIRECT_OBJECT: u32 = 0x2D2D_2D2D;

fn push(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    OPENED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(paths);
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// Ruta de un `NSString *` (o nada si es nulo).
unsafe fn path_of(string: *mut AnyObject) -> Option<PathBuf> {
    let string = unsafe { (string as *const NSString).as_ref() }?;
    Some(PathBuf::from(string.to_string()))
}

define_class!(
    // SAFETY: NSObject no impone requisitos de subclase y Opener no implementa Drop.
    #[unsafe(super(NSObject))]
    #[name = "ColibriOpener"]
    struct Opener;

    unsafe impl NSObjectProtocol for Opener {}

    impl Opener {
        /// La aplicación está a punto de terminar de arrancar: es el momento de quedarse
        /// con «abrir documentos» (antes lo pisaría AppKit; después, un `open -a Colibri
        /// carpeta` en frío ya habría pasado) y de ofrecer el servicio.
        #[unsafe(method(launching:))]
        fn launching(&self, _note: *mut AnyObject) {
            unsafe {
                let manager: *mut AnyObject =
                    msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
                let _: () = msg_send![
                    manager,
                    setEventHandler: self,
                    andSelector: sel!(open:reply:),
                    forEventClass: CORE_EVENT_CLASS,
                    andEventID: OPEN_DOCUMENTS,
                ];
                let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app, setServicesProvider: self];
            }
        }

        #[unsafe(method(open:reply:))]
        fn open(&self, event: *mut AnyObject, _reply: *mut AnyObject) {
            let mut paths = Vec::new();
            unsafe {
                let list: *mut AnyObject =
                    msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
                let count: isize = if list.is_null() {
                    0
                } else {
                    msg_send![list, numberOfItems]
                };
                for i in 1..=count {
                    let item: *mut AnyObject = msg_send![list, descriptorAtIndex: i];
                    if item.is_null() {
                        continue;
                    }
                    let url: *mut AnyObject = msg_send![item, fileURLValue];
                    if url.is_null() {
                        continue;
                    }
                    let path: *mut AnyObject = msg_send![url, path];
                    paths.extend(path_of(path));
                }
            }
            push(paths);
        }

        /// Servicio «Nueva pestaña de Colibrí aquí» (clic derecho sobre una carpeta en
        /// Finder): las rutas llegan en el portapapeles del servicio.
        #[unsafe(method(newTab:userData:error:))]
        fn new_tab(&self, board: *mut AnyObject, _data: *mut AnyObject, _error: *mut AnyObject) {
            let mut paths = Vec::new();
            unsafe {
                let kind = NSString::from_str("NSFilenamesPboardType");
                let names: *mut AnyObject = msg_send![board, propertyListForType: &*kind];
                let count: usize = if names.is_null() {
                    0
                } else {
                    msg_send![names, count]
                };
                for i in 0..count {
                    let name: *mut AnyObject = msg_send![names, objectAtIndex: i];
                    paths.extend(path_of(name));
                }
                let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app, activateIgnoringOtherApps: true];
            }
            push(paths);
        }
    }
);

/// Deja preparado el recibir carpetas de Finder. Hay que llamarlo antes de arrancar la
/// interfaz, desde el hilo principal.
pub fn install() {
    unsafe {
        let opener: Retained<Opener> = msg_send![Opener::alloc(), init];
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![
            center,
            addObserver: &*opener,
            selector: sel!(launching:),
            name: &*name,
            object: nil,
        ];
        // Nadie más lo retiene (observador y proveedor son referencias débiles).
        std::mem::forget(opener);
    }
}

/// A quién avisar cuando llegue una carpeta con la app ya abierta.
pub fn set_context(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

/// Lo que Finder pidió abrir desde la última vez.
pub fn take_opened() -> Vec<PathBuf> {
    std::mem::take(&mut *OPENED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Número sobre el icono del Dock (0 lo quita). Hilo principal.
pub fn set_badge(count: usize) {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }
        let tile: *mut AnyObject = msg_send![app, dockTile];
        let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
        let label: *const NSString = label.as_deref().map_or(std::ptr::null(), |l| l);
        let _: () = msg_send![tile, setBadgeLabel: label];
    }
}

/// Entrada segura de teclado: mientras está puesta, ninguna otra aplicación puede leer lo
/// que se teclea. Es de todo el sistema, así que solo debe estar puesta con Colibrí al
/// frente (quien llama la quita al perder el foco).
pub fn secure_input(on: bool) {
    unsafe {
        if on {
            EnableSecureEventInput();
        } else {
            DisableSecureEventInput();
        }
    }
}

/// True si no se ve nada de la aplicación: minimizada, tapada del todo por otras
/// ventanas, en otro escritorio o con la pantalla apagada. Hilo principal.
pub fn hidden() -> bool {
    const VISIBLE: usize = 1 << 1;
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return false;
        }
        let state: usize = msg_send![app, occlusionState];
        state & VISIBLE == 0
    }
}
