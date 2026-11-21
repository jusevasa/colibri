//! Integración con macOS que no cabe en otro módulo: carpetas que llegan de Finder
//! («Abrir con», arrastrar al icono del Dock, el servicio «Nueva pestaña de Colibrí aquí»),
//! el número del icono del Dock, la entrada segura de teclado y saber si la ventana se ve.

use std::path::PathBuf;
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{AllocAnyThread, class, define_class, msg_send, sel};
use objc2_foundation::{NSPoint, NSRect, NSString};

static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

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
    crate::shell::wake_all();
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

/// «Reducir movimiento» (Ajustes → Accesibilidad → Pantalla). Hilo principal.
pub fn reduce_motion() -> bool {
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            return false;
        }
        msg_send![workspace, accessibilityDisplayShouldReduceMotion]
    }
}

/// Altura de la pantalla principal (la primera de `NSScreen.screens`, la de la barra de
/// menús, no `mainScreen`): es la que fija el origen de las coordenadas de Cocoa.
fn primary_height() -> Option<f64> {
    unsafe {
        let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
        if screens.is_null() {
            return None;
        }
        let count: usize = msg_send![screens, count];
        if count == 0 {
            return None;
        }
        let first: *mut AnyObject = msg_send![screens, objectAtIndex: 0usize];
        let frame: NSRect = msg_send![first, frame];
        Some(frame.size.height)
    }
}

/// Dónde está el puntero en la pantalla, en puntos con origen arriba a la izquierda de la
/// pantalla principal (como `outer_position` de winit y `Win::frame`). Hilo principal.
pub fn pointer_on_screen() -> Option<egui::Pos2> {
    let height = primary_height()?;
    let at: NSPoint = unsafe { msg_send![class!(NSEvent), mouseLocation] };
    Some(egui::pos2(
        at.x as f32,
        crate::drag::flip_y(at.y, height) as f32,
    ))
}

/// Número de la ventana que se ve en ese punto (de cualquier aplicación: respeta qué ventana
/// tapa a cuál), o nada si no hay ninguna. Punto como el de `pointer_on_screen`.
pub fn window_number_at(p: egui::Pos2) -> Option<isize> {
    let height = primary_height()?;
    let at = NSPoint::new(p.x as f64, crate::drag::flip_y(p.y as f64, height));
    let number: isize = unsafe {
        msg_send![
            class!(NSWindow),
            windowNumberAtPoint: at,
            belowWindowWithWindowNumber: 0isize
        ]
    };
    (number > 0).then_some(number)
}

/// Si el botón principal del ratón está pulsado ahora mismo, según el sistema y no según
/// los eventos recibidos: al soltar fuera de la ventana sin mover más el ratón, egui-winit
/// no entrega el soltar y egui seguiría creyendo que está pulsado.
pub fn primary_button_down() -> bool {
    let buttons: usize = unsafe { msg_send![class!(NSEvent), pressedMouseButtons] };
    buttons & 1 != 0
}

/// Zona útil (sin barra de menús ni Dock) de la pantalla que contiene el punto, en puntos con
/// origen arriba a la izquierda de la pantalla principal; si ninguna lo contiene, la de la
/// principal. Para que una ventana nueva no nazca medio fuera. Hilo principal.
pub fn visible_frame_at(p: egui::Pos2) -> Option<egui::Rect> {
    let height = primary_height()?;
    let at = NSPoint::new(p.x as f64, crate::drag::flip_y(p.y as f64, height));
    unsafe {
        let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
        let count: usize = msg_send![screens, count];
        let mut chosen = None;
        for i in 0..count {
            let screen: *mut AnyObject = msg_send![screens, objectAtIndex: i];
            let frame: NSRect = msg_send![screen, frame];
            let inside = at.x >= frame.origin.x
                && at.x < frame.origin.x + frame.size.width
                && at.y >= frame.origin.y
                && at.y < frame.origin.y + frame.size.height;
            if inside || (i == 0 && chosen.is_none()) {
                chosen = Some(screen);
            }
            if inside {
                break;
            }
        }
        let visible: NSRect = msg_send![chosen?, visibleFrame];
        let top = crate::drag::flip_y(visible.origin.y + visible.size.height, height);
        Some(egui::Rect::from_min_size(
            egui::pos2(visible.origin.x as f32, top as f32),
            egui::vec2(visible.size.width as f32, visible.size.height as f32),
        ))
    }
}
