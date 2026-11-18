//! Atajo global (macOS): funciona aunque Colibrí no tenga el foco.
//! Usa los hotkeys de Carbon, que solo avisan de esa combinación concreta: la app no ve
//! el resto de lo que tecleas y no hace falta el permiso de Accesibilidad.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, Key};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::config::Shortcut;

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: Handler,
        count: usize,
        types: *const EventTypeSpec,
        user: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
}

/// 'keyb' y kEventHotKeyPressed.
const CLASS_KEYBOARD: u32 = 0x6B65_7962;
const HOTKEY_PRESSED: u32 = 5;
/// 'clbr': identifica nuestros hotkeys.
const SIGNATURE: u32 = 0x636C_6272;

static PRESSED: AtomicBool = AtomicBool::new(false);
static CTX: OnceLock<egui::Context> = OnceLock::new();
/// El hotkey registrado (un `EventHotKeyRef`), o 0 si no hay ninguno.
static CURRENT: Mutex<usize> = Mutex::new(0);

/// Corre en el hilo principal, dentro del bucle de eventos de la app.
extern "C" fn on_hotkey(_next: *mut c_void, _event: *mut c_void, _user: *mut c_void) -> i32 {
    PRESSED.store(true, Ordering::Relaxed);
    unsafe {
        // Traer la app al frente aquí mismo: en segundo plano la ventana puede no repintar.
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, activateIgnoringOtherApps: true];
        }
    }
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
    0
}

/// Registra `text` ("ctrl+alt+cmd+j") como atajo global, sustituyendo al anterior.
/// Vacío o no reconocido = sin atajo global. Devuelve false si no se pudo registrar.
/// Debe llamarse desde el hilo principal.
pub fn set(ctx: &egui::Context, text: &str) -> bool {
    let mut current = CURRENT.lock().unwrap();
    unsafe {
        if *current != 0 {
            UnregisterEventHotKey(*current as *mut c_void);
            *current = 0;
        }
        let Some((code, modifiers)) = Shortcut::parse(text).and_then(|sc| carbon(&sc)) else {
            return text.trim().is_empty();
        };
        if CTX.set(ctx.clone()).is_ok() {
            let spec = EventTypeSpec {
                class: CLASS_KEYBOARD,
                kind: HOTKEY_PRESSED,
            };
            InstallEventHandler(
                GetApplicationEventTarget(),
                on_hotkey,
                1,
                &spec,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }
        let mut out = std::ptr::null_mut();
        let status = RegisterEventHotKey(
            code,
            modifiers,
            EventHotKeyId {
                signature: SIGNATURE,
                id: 1,
            },
            GetApplicationEventTarget(),
            0,
            &mut out,
        );
        if status != 0 {
            return false;
        }
        *current = out as usize;
        true
    }
}

/// True una vez por cada pulsación del atajo global.
pub fn take_pressed() -> bool {
    PRESSED.swap(false, Ordering::Relaxed)
}

/// Código de tecla virtual y modificadores de Carbon para un atajo. Un atajo global sin
/// modificadores se rechaza: le quitaría esa tecla a todas las aplicaciones.
fn carbon(sc: &Shortcut) -> Option<(u32, u32)> {
    let mut modifiers = 0;
    if sc.cmd {
        modifiers |= 0x0100;
    }
    if sc.shift {
        modifiers |= 0x0200;
    }
    if sc.alt {
        modifiers |= 0x0800;
    }
    if sc.ctrl {
        modifiers |= 0x1000;
    }
    if modifiers & !0x0200 == 0 {
        return None;
    }
    let code = match sc.key {
        Key::A => 0x00,
        Key::S => 0x01,
        Key::D => 0x02,
        Key::F => 0x03,
        Key::H => 0x04,
        Key::G => 0x05,
        Key::Z => 0x06,
        Key::X => 0x07,
        Key::C => 0x08,
        Key::V => 0x09,
        Key::B => 0x0B,
        Key::Q => 0x0C,
        Key::W => 0x0D,
        Key::E => 0x0E,
        Key::R => 0x0F,
        Key::Y => 0x10,
        Key::T => 0x11,
        Key::Num1 => 0x12,
        Key::Num2 => 0x13,
        Key::Num3 => 0x14,
        Key::Num4 => 0x15,
        Key::Num6 => 0x16,
        Key::Num5 => 0x17,
        Key::Num9 => 0x19,
        Key::Num7 => 0x1A,
        Key::Num8 => 0x1C,
        Key::Num0 => 0x1D,
        Key::O => 0x1F,
        Key::U => 0x20,
        Key::I => 0x22,
        Key::P => 0x23,
        Key::Enter => 0x24,
        Key::L => 0x25,
        Key::J => 0x26,
        Key::K => 0x28,
        Key::N => 0x2D,
        Key::M => 0x2E,
        Key::Tab => 0x30,
        Key::Space => 0x31,
        Key::Escape => 0x35,
        Key::ArrowLeft => 0x7B,
        Key::ArrowRight => 0x7C,
        Key::ArrowDown => 0x7D,
        Key::ArrowUp => 0x7E,
        _ => return None,
    };
    Some((code, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carbon_codes() {
        let sc = Shortcut::parse("ctrl+alt+cmd+j").unwrap();
        assert_eq!(carbon(&sc), Some((0x26, 0x0100 | 0x0800 | 0x1000)));
        // Sin modificadores (o solo shift) no se registra.
        assert_eq!(carbon(&Shortcut::parse("j").unwrap()), None);
        assert_eq!(carbon(&Shortcut::parse("shift+j").unwrap()), None);
    }
}
