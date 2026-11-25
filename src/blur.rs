//! Desenfoque de lo que hay detrás de la ventana (macOS).
//!
//! Antes se usaba la API privada de CoreGraphics que usan Terminal.app, Alacritty o kitty
//! (`CGSSetWindowBackgroundBlurRadius`), pero desde macOS 26 el servidor de ventanas
//! desenfoca con un radio de esquina menor que el de la ventana: en las esquinas
//! redondeadas asomaba un chaflán oscuro y un borde claro por fuera de la curva (comprobado
//! con capturas de pantalla sobre un fondo blanco con rayas, en macOS 27.2; sin desenfoque
//! las esquinas salían limpias). Ahora el desenfoque es un `NSVisualEffectView` detrás de la
//! vista de contenido, dentro del marco de la ventana: AppKit lo recorta con las mismas
//! esquinas que el resto. Para que se vea como antes (solo lo de detrás, desenfocado) se
//! ocultan sus capas de tinte y se fija el radio de su capa `backdrop`.

use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSRect, NSString};

/// Identificador de nuestra vista de desenfoque, para encontrarla en el marco de la ventana.
const IDENT: &str = "colibri-blur";

/// Decide si macOS puede mover la ventana al arrastrar su franja superior. Con el contenido
/// a ventana completa esa franja sigue siendo «barra de título» y el sistema se queda con
/// el arrastre antes de que llegue a la app; hay que apagarlo donde queremos arrastrar algo
/// nuestro (las pestañas). Debe llamarse desde el hilo principal.
pub fn set_movable(movable: bool) -> bool {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return false;
        }
        let windows: *mut AnyObject = msg_send![app, windows];
        if windows.is_null() {
            return false;
        }
        let count: usize = msg_send![windows, count];
        for i in 0..count {
            let window: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            let _: () = msg_send![window, setMovable: movable];
        }
        count > 0
    }
}

/// Aplica el radio de desenfoque (0 = sin desenfoque) a las ventanas de la app.
/// Devuelve false si todavía no hay ninguna ventana. Debe llamarse desde el hilo principal.
pub fn set(radius: i64) -> bool {
    each_window(|window| unsafe { apply(window, radius) })
}

/// Vuelve a dejar solo el desenfoque en las vistas que ya existen: AppKit puede rehacer las
/// capas de tinte del `NSVisualEffectView` (al cambiar la apariencia, por ejemplo). Barato:
/// unas pocas capas por ventana. Debe llamarse desde el hilo principal.
pub fn refresh(radius: i64) {
    each_window(|window| unsafe {
        if let Some(view) = find(window) {
            clean(view, radius);
        }
    });
}

fn each_window(mut f: impl FnMut(*mut AnyObject)) -> bool {
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return false;
        }
        let windows: *mut AnyObject = msg_send![app, windows];
        if windows.is_null() {
            return false;
        }
        let count: usize = msg_send![windows, count];
        for i in 0..count {
            let window: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            if !window.is_null() {
                f(window);
            }
        }
        count > 0
    }
}

/// El marco de la ventana (la supervista de la de contenido), donde va el desenfoque.
unsafe fn frame_view(window: *mut AnyObject) -> Option<*mut AnyObject> {
    let content: *mut AnyObject = unsafe { msg_send![window, contentView] };
    if content.is_null() {
        return None;
    }
    let frame: *mut AnyObject = unsafe { msg_send![content, superview] };
    (!frame.is_null()).then_some(frame)
}

/// Nuestra vista de desenfoque en esa ventana, si ya está puesta.
unsafe fn find(window: *mut AnyObject) -> Option<*mut AnyObject> {
    let frame = unsafe { frame_view(window) }?;
    let ident = NSString::from_str(IDENT);
    let subviews: *mut AnyObject = unsafe { msg_send![frame, subviews] };
    let count: usize = unsafe { msg_send![subviews, count] };
    (0..count).find_map(|i| {
        let view: *mut AnyObject = unsafe { msg_send![subviews, objectAtIndex: i] };
        let id: *mut NSString = unsafe { msg_send![view, identifier] };
        let same = !id.is_null() && unsafe { (*id).isEqualToString(&ident) };
        same.then_some(view)
    })
}

unsafe fn apply(window: *mut AnyObject, radius: i64) {
    let existing = unsafe { find(window) };
    if radius <= 0 {
        if let Some(view) = existing {
            let _: () = unsafe { msg_send![view, removeFromSuperview] };
        }
        return;
    }
    let view = match existing {
        Some(view) => view,
        None => {
            let Some(frame) = (unsafe { frame_view(window) }) else {
                return;
            };
            let content: *mut AnyObject = unsafe { msg_send![window, contentView] };
            let bounds: NSRect = unsafe { msg_send![frame, bounds] };
            let view: *mut AnyObject = unsafe { msg_send![class!(NSVisualEffectView), alloc] };
            let view: *mut AnyObject = unsafe { msg_send![view, initWithFrame: bounds] };
            if view.is_null() {
                return;
            }
            unsafe {
                // NSVisualEffectBlendingModeBehindWindow = 0, NSVisualEffectStateActive = 1
                // (también con la ventana sin foco), NSVisualEffectMaterialUnderWindowBackground = 21.
                let _: () = msg_send![view, setBlendingMode: 0isize];
                let _: () = msg_send![view, setState: 1isize];
                let _: () = msg_send![view, setMaterial: 21isize];
                // NSViewWidthSizable | NSViewHeightSizable: sigue el tamaño de la ventana.
                let _: () = msg_send![view, setAutoresizingMask: 2usize | 16usize];
                let _: () = msg_send![view, setIdentifier: &*NSString::from_str(IDENT)];
                // NSWindowBelow = -1: debajo de la vista de contenido (que pinta Metal).
                let _: () =
                    msg_send![frame, addSubview: view, positioned: -1isize, relativeTo: content];
                // `addSubview` la retiene; soltamos la de `alloc`.
                let _: () = msg_send![view, release];
            }
            view
        }
    };
    unsafe { clean(view, radius) };
}

/// Deja del `NSVisualEffectView` solo la capa que desenfoca (`backdrop`), con nuestro radio:
/// las demás (relleno, tono, tinte del escritorio) oscurecían y teñían lo de detrás.
unsafe fn clean(view: *mut AnyObject, radius: i64) {
    let layer: *mut AnyObject = unsafe { msg_send![view, layer] };
    if layer.is_null() {
        return;
    }
    let backdrop = NSString::from_str("backdrop");
    let key = NSString::from_str("filters.gaussianBlur.inputRadius");
    let materials: *mut AnyObject = unsafe { msg_send![layer, sublayers] };
    if materials.is_null() {
        return;
    }
    let count: usize = unsafe { msg_send![materials, count] };
    for i in 0..count {
        let material: *mut AnyObject = unsafe { msg_send![materials, objectAtIndex: i] };
        let layers: *mut AnyObject = unsafe { msg_send![material, sublayers] };
        if layers.is_null() {
            continue;
        }
        let n: usize = unsafe { msg_send![layers, count] };
        for j in 0..n {
            let sub: *mut AnyObject = unsafe { msg_send![layers, objectAtIndex: j] };
            let name: *mut NSString = unsafe { msg_send![sub, name] };
            let is_backdrop = !name.is_null() && unsafe { (*name).isEqualToString(&backdrop) };
            unsafe {
                if is_backdrop {
                    // Solo si cambió: escribirlo en cada pasada haría trabajar a Core Animation.
                    let current: *mut AnyObject = msg_send![sub, valueForKeyPath: &*key];
                    let same = !current.is_null() && {
                        let v: f64 = msg_send![current, doubleValue];
                        v == radius as f64
                    };
                    if same {
                        continue;
                    }
                    let value: *mut AnyObject =
                        msg_send![class!(NSNumber), numberWithDouble: radius as f64];
                    let _: () = msg_send![sub, setValue: value, forKeyPath: &*key];
                } else {
                    let hidden: bool = msg_send![sub, isHidden];
                    if !hidden {
                        let _: () = msg_send![sub, setHidden: true];
                    }
                }
            }
        }
    }
}
