//! Animaciones de la interfaz: un muelle con amortiguamiento crítico para lo que se desliza
//! (pestañas que cambian de hueco, que nacen o que cambian de ancho).
//!
//! Por qué un muelle y no una curva con duración fija: si el destino cambia a media
//! animación (arrastrar una pestaña por varios huecos seguidos), una curva vuelve a arrancar
//! desde parada, y se nota el tirón; el muelle conserva la velocidad que llevaba. Con
//! amortiguamiento crítico no se pasa del destino ni rebota. La solución es exacta (no se
//! integra paso a paso), así que da lo mismo si los cuadros llegan a 20, 60 o 120 por
//! segundo, o si alguno se retrasa: la posición depende solo del tiempo.

/// Rigidez (radianes por segundo). Con 30 recorre el 80 % del camino en 100 ms, el 95 % en
/// 160 ms (lo que tardaba el deslizamiento lineal de antes) y se para del todo a los
/// ~0,3 s en un salto del ancho de una pestaña.
const OMEGA: f32 = 30.0;

/// Por debajo de esto (en puntos y en puntos por segundo) el muelle se da por parado y se
/// coloca justo en su destino: medio píxel en una pantalla Retina.
const REST_OFFSET: f32 = 0.25;
const REST_SPEED: f32 = 10.0;

/// Al arrancar desde parado, el primer paso avanza como mucho esto. Lo que pasó desde la
/// última vez que se miró el muelle puede ser mucho (la interfaz no repinta en reposo), y
/// sin el tope el primer cuadro ya saldría a media animación. Medio cuadro a 60 Hz, como
/// hace egui con sus animaciones.
const FIRST_STEP: f32 = 1.0 / 120.0;

/// Un valor que va hacia su destino como un muelle con amortiguamiento crítico.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    value: f32,
    velocity: f32,
    target: f32,
    /// Reloj de egui (`InputState::time`) de la última vez que se avanzó.
    at: f64,
}

impl Spring {
    /// Parado en `value`.
    pub fn new(value: f32, now: f64) -> Self {
        Self {
            value,
            velocity: 0.0,
            target: value,
            at: now,
        }
    }

    /// Lo coloca en `value`, parado, sin animar: al redimensionar la ventana (si no, las
    /// pestañas irían por detrás del borde), con «Reducir movimiento» o mientras la pestaña
    /// sigue al puntero.
    pub fn snap(&mut self, value: f32, now: f64) {
        *self = Self::new(value, now);
    }

    /// Lo avanza hasta `now` camino de `target` y devuelve dónde está. Se puede llamar más
    /// de una vez en el mismo cuadro: con el mismo `now` no se mueve.
    pub fn update(&mut self, target: f32, now: f64) -> f32 {
        let mut dt = (now - self.at).max(0.0) as f32;
        if self.resting() && target != self.target {
            dt = dt.min(FIRST_STEP);
        }
        self.at = now;
        self.target = target;
        // Sin tiempo no hay movimiento (y así cambiar de destino no mueve ni un decimal).
        if self.resting() || dt == 0.0 {
            return self.value;
        }
        let (offset, velocity) = step(self.value - target, self.velocity, dt);
        if offset.abs() < REST_OFFSET && velocity.abs() < REST_SPEED {
            self.value = target;
            self.velocity = 0.0;
        } else {
            self.value = target + offset;
            self.velocity = velocity;
        }
        self.value
    }

    /// Aún no ha llegado: hay que pedir otro cuadro.
    pub fn moving(&self) -> bool {
        !self.resting()
    }

    fn resting(&self) -> bool {
        self.value == self.target && self.velocity == 0.0
    }
}

/// Distancia al destino y velocidad al cabo de `dt` segundos, partiendo de `offset` y
/// `velocity`. Solución cerrada de x'' = −2ωx' − ω²x: x(t) = (a + b·t)·e^(−ωt), con
/// a = x₀ y b = v₀ + ω·x₀.
fn step(offset: f32, velocity: f32, dt: f32) -> (f32, f32) {
    let a = offset;
    let b = velocity + OMEGA * offset;
    let decay = (-OMEGA * dt).exp();
    let x = (a + b * dt) * decay;
    let v = (b - OMEGA * (a + b * dt)) * decay;
    (x, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Avanza el muelle a `fps` cuadros por segundo hasta que se para; devuelve los valores.
    fn run(spring: &mut Spring, target: f32, start: f64, fps: f64) -> Vec<f32> {
        let mut out = Vec::new();
        let mut t = start;
        while out.len() < 1000 {
            t += 1.0 / fps;
            out.push(spring.update(target, t));
            if !spring.moving() {
                break;
            }
        }
        out
    }

    #[test]
    fn llega_sin_pasarse_y_se_para() {
        let mut s = Spring::new(0.0, 10.0);
        let values = run(&mut s, 230.0, 10.0, 60.0);
        assert_eq!(*values.last().unwrap(), 230.0);
        assert!(!s.moving());
        assert!(
            values.windows(2).all(|w| w[1] >= w[0]),
            "siempre hacia delante"
        );
        assert!(values.iter().all(|&v| v <= 230.0), "no se pasa: {values:?}");
        // Unos 0,3 s para el ancho entero de una pestaña.
        let secs = values.len() as f64 / 60.0;
        assert!((0.2..0.4).contains(&secs), "tardó {secs} s");
    }

    #[test]
    fn el_primer_cuadro_tras_el_reposo_no_salta() {
        // Parado desde hace 10 s: el primer cuadro solo avanza un poco.
        let mut s = Spring::new(0.0, 0.0);
        let first = s.update(100.0, 10.0);
        assert!(first > 0.0 && first < 5.0, "{first}");
        assert!(s.moving());
    }

    #[test]
    fn mismo_instante_no_se_mueve() {
        let mut s = Spring::new(0.0, 0.0);
        s.update(100.0, 0.0);
        let a = s.update(100.0, 0.05);
        let b = s.update(100.0, 0.05);
        assert_eq!(a, b);
    }

    #[test]
    fn da_igual_el_ritmo_de_cuadros() {
        // La solución es exacta: a 20, 60 o 120 cuadros por segundo pasa por el mismo sitio.
        let at = |fps: f64| {
            let mut s = Spring::new(0.0, 0.0);
            s.update(200.0, 0.0);
            let mut t = 0.0;
            while t < 0.1 - 1e-9 {
                t = (t + 1.0 / fps).min(0.1);
                s.update(200.0, t);
            }
            s.value
        };
        let (a, b, c) = (at(20.0), at(60.0), at(120.0));
        assert!((a - b).abs() < 0.01 && (b - c).abs() < 0.01, "{a} {b} {c}");
    }

    #[test]
    fn cambiar_de_destino_conserva_posicion_y_velocidad() {
        let mut s = Spring::new(0.0, 0.0);
        s.update(100.0, 0.0);
        s.update(100.0, 0.05);
        let (before, speed) = (s.value, s.velocity);
        assert!(speed > 0.0);
        // Nuevo destino en el mismo instante: ni salta ni se frena.
        assert_eq!(s.update(300.0, 0.05), before);
        assert_eq!(s.velocity, speed);
        // Y sigue hacia delante sin pararse antes de cambiar de rumbo.
        let after = s.update(300.0, 0.05 + 1.0 / 60.0);
        assert!(after > before);
    }

    #[test]
    fn snap_lo_deja_parado() {
        let mut s = Spring::new(0.0, 0.0);
        s.update(100.0, 0.0);
        s.update(100.0, 0.05);
        s.snap(40.0, 0.05);
        assert_eq!(s.value, 40.0);
        assert!(!s.moving());
        assert_eq!(s.update(40.0, 1.0), 40.0);
    }
}
