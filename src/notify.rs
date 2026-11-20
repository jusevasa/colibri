//! Notificaciones nativas de macOS (UNUserNotificationCenter). Al hacer clic en una,
//! Colibrí pasa al frente y va a la pestaña que terminó.
//!
//! Solo funcionan cuando Colibrí corre como Colibri.app (macOS exige un bundle firmado).
//! Si se ejecuta el binario suelto, se usa `osascript` como respaldo, sin clic.

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static BUNDLED: AtomicBool = AtomicBool::new(false);
static COUNTER: AtomicU64 = AtomicU64::new(0);
static CLICKED: Mutex<Vec<u64>> = Mutex::new(Vec::new());

const DEFAULT_ACTION: &str = "com.apple.UNNotificationDefaultActionIdentifier";

define_class!(
    // SAFETY: NSObject no impone requisitos de subclase y Delegate no implementa Drop.
    #[unsafe(super(NSObject))]
    #[name = "ColibriNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        // Mostrar el aviso aunque Colibrí esté al frente (estás en otra pestaña).
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        // Clic en el aviso: recordar a qué pestaña ir.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &DynBlock<dyn Fn()>,
        ) {
            if response.actionIdentifier().to_string() == DEFAULT_ACTION {
                let id = response.notification().request().identifier().to_string();
                // Identificador: "tab-<sesión>-<n>".
                if let Some(session) = id.split('-').nth(1).and_then(|s| s.parse::<u64>().ok()) {
                    CLICKED.lock().unwrap().push(session);
                    crate::shell::wake_all();
                }
            }
            handler.call(());
        }
    }
);

pub fn init() {
    // Sin bundle, UNUserNotificationCenter lanza una excepción: no tocarlo.
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return;
    }
    BUNDLED.store(true, Ordering::Relaxed);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // El centro guarda el delegado como referencia débil: debe vivir toda la ejecución.
    std::mem::forget(delegate);
    let done = RcBlock::new(|granted: Bool, error: *mut NSError| {
        if !granted.as_bool() {
            // SAFETY: el sistema entrega un NSError válido o nulo.
            let why = unsafe { error.as_ref() }
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_default();
            eprintln!(
                "colibri: notificaciones no autorizadas: {why} (Ajustes del Sistema › Notificaciones › Colibrí)"
            );
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &done,
    );
}

pub fn send(title: &str, body: &str, session: u64) {
    if !BUNDLED.load(Ordering::Relaxed) {
        fallback(title, body);
        return;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let id = NSString::from_str(&format!("tab-{session}-{n}"));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, None);
}

/// Sesiones cuyos avisos se pulsaron desde la última llamada.
pub fn take_clicked() -> Vec<u64> {
    std::mem::take(&mut *CLICKED.lock().unwrap())
}

fn fallback(title: &str, body: &str) {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"Colibrí\" subtitle \"{}\"",
        esc(body),
        esc(title)
    );
    if let Ok(mut child) = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
