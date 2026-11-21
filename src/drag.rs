//! Cálculo puro para despegar una pestaña arrastrándola fuera de la barra: cuándo se
//! despega, en qué hueco cae al soltarla sobre otra ventana y el paso de coordenadas de
//! macOS a las de la ventana. Lo que pregunta al sistema está en `macos.rs`.

use egui::{Pos2, Rect};

/// La pestaña arrastrada se despega: el puntero salió de la ventana (o egui lo da por
/// perdido, que es lo mismo) o bajó de la barra más de media altura de ella. Dentro de ese
/// margen sigue siendo un arrastre para reordenar.
pub fn detaching(pointer: Option<Pos2>, bar: Rect, window: Rect) -> bool {
    match pointer {
        None => true,
        Some(p) => !window.contains(p) || p.y > bar.bottom() + bar.height() / 2.0,
    }
}

/// Hueco en que entra una pestaña soltada en `x` sobre una barra de `n` pestañas de ancho
/// `tab_w` que empiezan en `tabs_left`: el borde entre pestañas más cercano (0 = delante de
/// la primera, `n` = detrás de la última). Sin medidas, al final.
pub fn drop_slot(x: f32, tabs_left: f32, tab_w: f32, n: usize) -> usize {
    if tab_w <= 0.0 {
        return n;
    }
    ((x - tabs_left) / tab_w).round().clamp(0.0, n as f32) as usize
}

/// Cocoa mide desde abajo a la izquierda de la pantalla principal; winit y egui, desde
/// arriba. Sirve en los dos sentidos.
pub fn flip_y(y: f64, primary_height: f64) -> f64 {
    primary_height - y
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2};

    fn bar() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 38.0))
    }

    fn window() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0))
    }

    #[test]
    fn detaching_sin_puntero() {
        assert!(detaching(None, bar(), window()));
    }

    #[test]
    fn detaching_dentro_del_margen() {
        assert!(!detaching(Some(pos2(100.0, 20.0)), bar(), window()));
        assert!(!detaching(Some(pos2(100.0, 57.0)), bar(), window())); // 38 + 19
    }

    #[test]
    fn detaching_bajo_el_margen() {
        assert!(detaching(Some(pos2(100.0, 57.5)), bar(), window()));
    }

    #[test]
    fn detaching_fuera_de_la_ventana() {
        assert!(detaching(Some(pos2(-5.0, 20.0)), bar(), window()));
        assert!(detaching(Some(pos2(100.0, -3.0)), bar(), window()));
    }

    #[test]
    fn drop_slot_huecos() {
        assert_eq!(drop_slot(70.0, 70.0, 100.0, 3), 0);
        assert_eq!(drop_slot(130.0, 70.0, 100.0, 3), 1);
        assert_eq!(drop_slot(10_000.0, 70.0, 100.0, 3), 3);
        assert_eq!(drop_slot(-50.0, 70.0, 100.0, 3), 0);
        assert_eq!(drop_slot(130.0, 70.0, 0.0, 3), 3);
    }

    #[test]
    fn flip_y_ida_y_vuelta() {
        assert_eq!(flip_y(100.0, 1117.0), 1017.0);
        assert_eq!(flip_y(flip_y(42.5, 900.0), 900.0), 42.5);
    }
}
