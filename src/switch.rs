//! Encendido de los hilos de fondo que trabajan cada cierto tiempo (`ports`, `procs`): solo
//! trabajan mientras su apartado se ve. Apagados duermen sin despertar; al encenderlos, o al
//! pedirles que miren ya (`poke`), despiertan al momento.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Un solo hilo espera en `wait`; los demás encienden, apagan o piden que mire ya.
#[derive(Default)]
pub struct Switch {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    on: bool,
    /// Se pidió que el hilo mire ya (al encenderlo o con `poke`).
    poked: bool,
}

impl Switch {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Enciende o apaga. Se puede llamar en cada pasada: solo despierta al hilo al pasar de
    /// apagado a encendido.
    pub fn set(&self, on: bool) {
        let mut state = self.state();
        if on && !state.on {
            state.poked = true;
            self.changed.notify_one();
        }
        state.on = on;
    }

    pub fn is_on(&self) -> bool {
        self.state().on
    }

    /// Que el hilo mire ya, sin esperar su plazo.
    pub fn poke(&self) {
        self.state().poked = true;
        self.changed.notify_one();
    }

    /// Desde el hilo: espera `period` si está encendido, o sin plazo hasta que lo enciendan.
    /// En los dos casos vuelve en cuanto se le pide (`poke`).
    pub fn wait(&self, period: Duration) {
        let state = self.state();
        let mut state = if state.on {
            self.changed
                .wait_timeout_while(state, period, |s| !s.poked)
                .unwrap_or_else(|e| e.into_inner())
                .0
        } else {
            self.changed
                .wait_while(state, |s| !s.poked)
                .unwrap_or_else(|e| e.into_inner())
        };
        state.poked = false;
    }
}

#[cfg(test)]
mod tests {
    use super::Switch;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// Un hilo como el de puertos: trabaja si está encendido y luego espera su plazo.
    fn hilo(sw: &Arc<Switch>) -> Arc<AtomicUsize> {
        let trabajos = Arc::new(AtomicUsize::new(0));
        let (sw, t) = (sw.clone(), trabajos.clone());
        std::thread::spawn(move || {
            loop {
                if sw.is_on() {
                    t.fetch_add(1, Ordering::SeqCst);
                }
                sw.wait(Duration::from_secs(30));
            }
        });
        trabajos
    }

    fn llega(plazo: Duration, listo: impl Fn() -> bool) -> bool {
        let fin = Instant::now() + plazo;
        while Instant::now() < fin {
            if listo() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        listo()
    }

    #[test]
    fn al_encender_o_pedirlo_trabaja_sin_esperar_su_plazo() {
        let sw = Arc::new(Switch::default());
        let trabajos = hilo(&sw);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(trabajos.load(Ordering::SeqCst), 0, "apagado no trabaja");

        let ms = Duration::from_millis;
        sw.set(true);
        assert!(
            llega(ms(500), || trabajos.load(Ordering::SeqCst) == 1),
            "al encender"
        );
        sw.poke();
        assert!(
            llega(ms(500), || trabajos.load(Ordering::SeqCst) == 2),
            "al pedirlo"
        );
        // Encenderlo otra vez sin haberlo apagado no es motivo para trabajar.
        sw.set(true);
        std::thread::sleep(ms(100));
        assert_eq!(trabajos.load(Ordering::SeqCst), 2);

        sw.set(false);
        sw.poke();
        std::thread::sleep(ms(100));
        assert_eq!(
            trabajos.load(Ordering::SeqCst),
            2,
            "apagado no trabaja aunque se pida"
        );
        sw.set(true);
        assert!(
            llega(ms(500), || trabajos.load(Ordering::SeqCst) == 3),
            "al volver a encender"
        );
    }
}
