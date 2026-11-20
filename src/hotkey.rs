//! Atajo global (macOS): funciona aunque Colibrí no tenga el foco.
//! Usa los hotkeys de Carbon, que solo avisan de esa combinación concreta: la app no ve
//! el resto de lo que tecleas y no hace falta el permiso de Accesibilidad.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use egui::{self, Key};
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
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        kind: u32,
        actual_kind: *mut u32,
        size: usize,
        actual_size: *mut usize,
        out: *mut c_void,
    ) -> i32;
}

/// 'keyb' y kEventHotKeyPressed.
const CLASS_KEYBOARD: u32 = 0x6B65_7962;
const HOTKEY_PRESSED: u32 = 5;
/// 'clbr': identifica nuestros hotkeys.
const SIGNATURE: u32 = 0x636C_6272;

static PRESSED: AtomicBool = AtomicBool::new(false);
/// El manejador de Carbon ya está instalado (una vez para los dos atajos).
static HANDLER: OnceLock<()> = OnceLock::new();
/// Los hotkeys registrados (un `EventHotKeyRef` cada uno), o 0 donde no hay.
static CURRENT: Mutex<[usize; 2]> = Mutex::new([0; 2]);

/// Qué hace cada atajo global.
#[derive(Clone, Copy)]
pub enum Global {
    /// Trae Colibrí al frente y va al agente que terminó.
    Jump = 0,
    /// Trae Colibrí al frente; si ya lo está, lo oculta (ir y volver con la misma tecla).
    Focus = 1,
}

/// '----' (el objeto del evento) y 'hkid' (su tipo: un `EventHotKeyId`).
const PARAM_DIRECT: u32 = 0x2D2D_2D2D;
const TYPE_HOTKEY_ID: u32 = 0x686B_6964;

/// Corre en el hilo principal, dentro del bucle de eventos de la app.
extern "C" fn on_hotkey(_next: *mut c_void, event: *mut c_void, _user: *mut c_void) -> i32 {
    let mut which = EventHotKeyId {
        signature: 0,
        id: 0,
    };
    unsafe {
        GetEventParameter(
            event,
            PARAM_DIRECT,
            TYPE_HOTKEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyId>(),
            std::ptr::null_mut(),
            (&raw mut which).cast(),
        );
    }
    let focus = which.id == Global::Focus as u32 + 1;
    if !focus {
        PRESSED.store(true, Ordering::Relaxed);
    }
    unsafe {
        // Traer la app al frente aquí mismo: en segundo plano la ventana puede no repintar.
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if !app.is_null() {
            let active: bool = msg_send![app, isActive];
            if focus && active {
                let nil: *mut AnyObject = std::ptr::null_mut();
                let _: () = msg_send![app, hide: nil];
            } else {
                // Una ventana minimizada no vuelve sola al activar la app.
                let windows: *mut AnyObject = msg_send![app, windows];
                let count: usize = if windows.is_null() {
                    0
                } else {
                    msg_send![windows, count]
                };
                for i in 0..count {
                    let window: *mut AnyObject = msg_send![windows, objectAtIndex: i];
                    let mini: bool = msg_send![window, isMiniaturized];
                    if mini {
                        let nil: *mut AnyObject = std::ptr::null_mut();
                        let _: () = msg_send![window, deminiaturize: nil];
                    }
                }
                let _: () = msg_send![app, activateIgnoringOtherApps: true];
            }
        }
    }
    crate::shell::wake_all();
    0
}

/// Registra `text` ("ctrl+alt+cmd+j") como ese atajo global, sustituyendo al anterior.
/// Vacío o no reconocido = sin atajo global. Devuelve false si no se pudo registrar.
/// Debe llamarse desde el hilo principal.
pub fn set(which: Global, text: &str) -> bool {
    let mut all = CURRENT.lock().unwrap();
    let current = &mut all[which as usize];
    unsafe {
        if *current != 0 {
            UnregisterEventHotKey(*current as *mut c_void);
            *current = 0;
        }
        let Some((code, modifiers)) = Shortcut::parse(text).and_then(|sc| carbon(&sc)) else {
            return text.trim().is_empty();
        };
        if HANDLER.set(()).is_ok() {
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
                id: which as u32 + 1,
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
        let sc = Shortcut::parse("ctrl+alt+cmd+space").unwrap();
        assert_eq!(carbon(&sc), Some((0x31, 0x0100 | 0x0800 | 0x1000)));
        assert_eq!(carbon(&Shortcut::parse("j").unwrap()), None);
        assert_eq!(carbon(&Shortcut::parse("shift+j").unwrap()), None);
    }
}
