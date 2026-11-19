//! ⌘C / ⌘X / ⌘V leyendo el portapapeles de macOS directamente.
//!
//! egui solo sabe pegar texto o una imagen (y para saberlo la decodifica entera). Pero una
//! foto que llega del iPhone por Continuity, igual que un archivo copiado en Finder, no trae
//! ni texto ni imagen: trae la *ruta de un archivo*, y con egui el ⌘V se perdía. Aquí se pega
//! esa ruta como hacen Terminal o Warp (Claude Code la reconoce y adjunta la imagen); de una
//! imagen suelta solo se pregunta si la hay, nunca se lee.

use std::sync::Arc;

use eframe::egui::{self, Key};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;

enum Clip {
    Text(String),
    Image,
    Empty,
}

fn read() -> Clip {
    // SAFETY: mensajes a NSPasteboard con los tipos que documenta AppKit; se llama desde el
    // hilo principal (el de la interfaz).
    unsafe {
        let pb: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        if pb.is_null() {
            return Clip::Empty;
        }
        let files = file_paths(pb);
        if !files.is_empty() {
            return Clip::Text(files.join(" "));
        }
        let plain = NSString::from_str("public.utf8-plain-text");
        let text: Option<Retained<NSString>> = msg_send![pb, stringForType: &*plain];
        if let Some(text) = text.map(|t| t.to_string()).filter(|t| !t.is_empty()) {
            return Clip::Text(text);
        }
        // «public.image» abarca PNG, TIFF, JPEG, HEIC…
        let image = NSString::from_str("public.image");
        let types: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: &*image];
        let has_image: bool = msg_send![pb, canReadItemWithDataConformingToTypes: types];
        if has_image { Clip::Image } else { Clip::Empty }
    }
}

/// Rutas de los archivos del portapapeles, escapadas para el shell como al arrastrarlos a
/// Terminal. Van antes que el texto: Finder acompaña el archivo con su nombre a secas.
///
/// SAFETY: `pb` debe ser un NSPasteboard válido.
unsafe fn file_paths(pb: *mut AnyObject) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let items: *mut AnyObject = msg_send![pb, pasteboardItems];
        if items.is_null() {
            return out;
        }
        let file_url = NSString::from_str("public.file-url");
        let count: usize = msg_send![items, count];
        for i in 0..count {
            let item: *mut AnyObject = msg_send![items, objectAtIndex: i];
            let text: Option<Retained<NSString>> = msg_send![item, stringForType: &*file_url];
            let Some(text) = text else { continue };
            // Pasa por NSURL para resolver las referencias de archivo (file:///.file/id=…).
            let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: &*text];
            if url.is_null() {
                continue;
            }
            let path: Option<Retained<NSString>> = msg_send![url, path];
            if let Some(path) = path.map(|p| p.to_string()).filter(|p| !p.is_empty()) {
                out.push(escape(&path));
            }
        }
    }
    out
}

fn escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if !(c.is_alphanumeric() || "-_./@+:,%=~".contains(c)) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Cambia las pulsaciones de ⌘C, ⌘X y ⌘V por los eventos de portapapeles de egui (eframe
/// arranca con `clipboard_shortcuts: false`, así que llegan como teclas normales). Con una
/// imagen y sin texto sale `PasteImage` con una imagen vacía: solo importa el aviso.
pub fn translate(events: &mut Vec<egui::Event>) {
    let mut out = Vec::with_capacity(events.len());
    for ev in events.drain(..) {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = &ev
        else {
            out.push(ev);
            continue;
        };
        let cmd = modifiers.command_only();
        match key {
            Key::Copy => out.push(egui::Event::Copy),
            Key::C if cmd => out.push(egui::Event::Copy),
            Key::Cut => out.push(egui::Event::Cut),
            Key::X if cmd => out.push(egui::Event::Cut),
            Key::Paste | Key::V if *key == Key::Paste || cmd => match read() {
                Clip::Text(text) => out.push(egui::Event::Paste(text.replace("\r\n", "\n"))),
                Clip::Image => out.push(egui::Event::PasteImage(Arc::new(
                    egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
                ))),
                Clip::Empty => {}
            },
            _ => out.push(ev),
        }
    }
    *events = out;
}
