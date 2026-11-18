//! Desenfoque de lo que hay detrás de la ventana (macOS).
//! Usa la misma API privada de CoreGraphics que Terminal.app, Alacritty o kitty.

use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGSMainConnectionID() -> *mut AnyObject;
    fn CGSSetWindowBackgroundBlurRadius(
        connection: *mut AnyObject,
        window: isize,
        radius: i64,
    ) -> i32;
}

/// Aplica el radio de desenfoque (0 = sin desenfoque) a las ventanas de la app.
/// Devuelve false si todavía no hay ninguna ventana. Debe llamarse desde el hilo principal.
pub fn set(radius: i64) -> bool {
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
        let connection = CGSMainConnectionID();
        for i in 0..count {
            let window: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            let number: isize = msg_send![window, windowNumber];
            CGSSetWindowBackgroundBlurRadius(connection, number, radius);
        }
        count > 0
    }
}
