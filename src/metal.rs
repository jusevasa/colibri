//! Pintor de egui sobre Metal (sin wgpu).
//!
//! Dos piezas:
//!
//! - [`Renderer`]: dispositivo, tubería, texturas y búferes. Dibuja las mallas de egui en
//!   cualquier textura y sabe leer el resultado; no necesita ventana (así se prueba).
//! - [`Painter`]: la `CAMetalLayer` de la ventana y el control de drawables. Su regla es
//!   **no esperar nunca un drawable**: lleva la cuenta de los que están en vuelo y, si no
//!   queda ninguno libre, salta el cuadro y avisa cuando se libere uno.
//!
//! El resultado busca ser idéntico al de egui-wgpu (del que se portan el sombreador, la
//! mezcla y el recorte): destino `BGRA8Unorm` en espacio gamma, texturas `RGBA8Unorm` sin
//! conversión sRGB, alfa premultiplicado y el mismo tramado.
//!
//! Además de mallas, dibuja la rejilla de la terminal (`grid::GridBatch`, que llega como
//! `Primitive::Callback`) con una segunda tubería de rectángulos instanciados: mismo
//! sombreador de fragmentos, misma mezcla, mismo atlas de fuentes y mismo recorte.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::grid::{GridBatch, Instance};
use block2::RcBlock;
use egui::epaint::{self, Primitive, Vertex};
use egui::{TextureFilter, TextureId, TextureOptions, TextureWrapMode};
use objc2::MainThreadMarker;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
use objc2_foundation::{NSSize, NSString};
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor,
    MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice,
    MTLDrawable, MTLIndexType, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat,
    MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResourceOptions, MTLSamplerAddressMode,
    MTLSamplerDescriptor, MTLSamplerMinMagFilter, MTLSamplerState, MTLScissorRect, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
    MTLVertexDescriptor, MTLVertexFormat,
};
use objc2_quartz_core::{CAAutoresizingMask, CAMetalDrawable, CAMetalLayer, CATransaction};

type Device = ProtocolObject<dyn MTLDevice>;
type Queue = ProtocolObject<dyn MTLCommandQueue>;
type Pipeline = ProtocolObject<dyn MTLRenderPipelineState>;
type Texture = ProtocolObject<dyn MTLTexture>;
type Sampler = ProtocolObject<dyn MTLSamplerState>;
type Buffer = ProtocolObject<dyn MTLBuffer>;
type CommandBuffer = ProtocolObject<dyn MTLCommandBuffer>;
type Drawable = ProtocolObject<dyn CAMetalDrawable>;

/// Formato del destino (ventana y captura): el que egui-wgpu elige en macOS, en espacio
/// gamma. Con él el sombreador escribe los colores tal cual, sin pasar por lineal.
const TARGET_FORMAT: MTLPixelFormat = MTLPixelFormat::BGRA8Unorm;

/// Lado máximo de una textura 2D en cualquier GPU con Metal de macOS.
const MAX_TEXTURE_SIDE: usize = 16384;

/// Búferes de vértices e índices que se guardan para reutilizar.
const POOL_MAX: usize = 4;

/// Si no queda drawable libre y hace este tiempo que no se obtiene ninguno, se da por
/// perdida la cuenta (un manejador que nunca llegó) y se vuelve a intentar.
const STALE: Duration = Duration::from_secs(2);

/// Cuentas para medir (las escribe `debug::count_frame` con `COLIBRI_FPS`): cuadros
/// presentados, saltados por falta de drawable y la espera más larga en `nextDrawable`.
pub mod stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    pub static PRESENTED: AtomicU64 = AtomicU64::new(0);
    pub static SKIPPED: AtomicU64 = AtomicU64::new(0);
    pub static MAX_WAIT_US: AtomicU64 = AtomicU64::new(0);

    /// Lo acumulado desde la última llamada: (presentados, saltados, espera máxima en µs).
    pub fn take() -> (u64, u64, u64) {
        (
            PRESENTED.swap(0, Ordering::Relaxed),
            SKIPPED.swap(0, Ordering::Relaxed),
            MAX_WAIT_US.swap(0, Ordering::Relaxed),
        )
    }
}

/// Cuántos drawables puede haber en vuelo antes de saltar cuadros. Por defecto uno menos
/// que los de la capa; `COLIBRI_DRAWABLES=<n>` lo cambia para medir.
fn drawable_limit(layer_max: usize) -> usize {
    static OVERRIDE: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    OVERRIDE
        .get_or_init(|| std::env::var("COLIBRI_DRAWABLES").ok()?.parse().ok())
        .unwrap_or_else(|| layer_max.saturating_sub(1))
        .max(1)
}

// El búfer de vértices se llena copiando `epaint::Vertex` tal cual.
const _: () = assert!(size_of::<Vertex>() == 20);
const _: () = assert!(size_of::<egui::Color32>() == 4);

/// Sombreador: port a Metal Shading Language de `egui.wgsl` de egui-wgpu (variante de
/// destino en gamma, con tramado y filtrado por hardware, que es lo que usa eframe).
const SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Locals {
    float2 screen_size;
};

struct VertexIn {
    float2 pos   [[attribute(0)]];
    float2 uv    [[attribute(1)]];
    float4 color [[attribute(2)]]; // gamma 0-1, premultiplicado
};

struct VertexOut {
    float4 position [[position]];
    float2 uv;
    float4 color;
};

vertex VertexOut vs_main(VertexIn in [[stage_in]], constant Locals &locals [[buffer(1)]]) {
    VertexOut out;
    out.uv = in.uv;
    out.color = in.color;
    out.position = float4(
        2.0 * in.pos.x / locals.screen_size.x - 1.0,
        1.0 - 2.0 * in.pos.y / locals.screen_size.y,
        0.0,
        1.0);
    return out;
}

// Jimenez 2014, "Next Generation Post-Processing in Call of Duty".
static float interleaved_gradient_noise(float2 n) {
    float f = 0.06711056 * n.x + 0.00583715 * n.y;
    return fract(52.9829189 * fract(f));
}

static float3 dither_interleaved(float3 rgb, float levels, float4 frag_coord) {
    float noise = interleaved_gradient_noise(frag_coord.xy);
    // Algo menos de medio nivel: los colores planos no se traman.
    noise = (noise - 0.5) * 0.95;
    return rgb + noise / (levels - 1.0);
}

// Rejilla: un rectángulo por instancia (`grid::Instance`), cuatro vértices en tira.
struct GridInstance {
    float2 min;
    float2 max;
    float2 uv_min; // en texeles del atlas
    float2 uv_max;
    uint color;    // RGBA8, premultiplicado
    uint pad;
};

struct GridLocals {
    float2 screen_size;
    float2 uv_scale; // 1 / tamaño del atlas, como `uv_normalizer` del teselador
};

vertex VertexOut vs_grid(
    uint vid [[vertex_id]],
    uint iid [[instance_id]],
    const device GridInstance *instances [[buffer(0)]],
    constant GridLocals &locals [[buffer(1)]]
) {
    GridInstance g = instances[iid];
    // Mismo orden que `Mesh::add_rect_with_uv`: arriba-izq, arriba-der, abajo-izq, abajo-der.
    // Se elige la esquina (sin interpolar) para que salga exactamente el mismo número.
    bool right = (vid & 1u) != 0u;
    bool bottom = (vid & 2u) != 0u;
    float2 pos = float2(right ? g.max.x : g.min.x, bottom ? g.max.y : g.min.y);
    float2 uv = float2(right ? g.uv_max.x : g.uv_min.x, bottom ? g.uv_max.y : g.uv_min.y);
    VertexOut out;
    out.uv = uv * locals.uv_scale;
    out.color = unpack_unorm4x8_to_float(g.color);
    out.position = float4(
        2.0 * pos.x / locals.screen_size.x - 1.0,
        1.0 - 2.0 * pos.y / locals.screen_size.y,
        0.0,
        1.0);
    return out;
}

fragment float4 fs_main(
    VertexOut in [[stage_in]],
    texture2d<float> tex [[texture(0)]],
    sampler smp [[sampler(0)]]
) {
    // Texturas «normales», sin conversión sRGB: todo se opera en gamma.
    float4 tex_gamma = tex.sample(smp, in.uv);
    float4 out_color = in.color * tex_gamma;
    out_color.rgb = dither_interleaved(out_color.rgb, 256.0, in.position);
    return out_color;
}
"#;

/// Qué pasó con el cuadro pedido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Se pintó y se presentó en la ventana.
    Presented,
    /// No había un drawable libre (o la capa no tiene tamaño): no se pintó nada en la
    /// ventana y **no se esperó**. Cuando vuelva a haber uno libre se llama a `wake`.
    Skipped,
}

/// Resultado de `Painter::paint`.
pub struct Painted {
    pub outcome: Outcome,
    /// La imagen del cuadro, si se pidió con `capture` (se obtiene aunque el cuadro se
    /// salte: sale de una textura aparte, no del drawable).
    pub capture: Option<egui::ColorImage>,
}

// ---------------------------------------------------------------------------------------
// Drawables en vuelo
// ---------------------------------------------------------------------------------------

/// Cuenta de drawables tomados de la capa y aún no mostrados. La comparten el hilo
/// principal y los manejadores de «presentado», que Metal llama desde otro hilo.
struct Flight {
    in_flight: AtomicUsize,
    /// Cambia cuando la cuenta se da por perdida: los billetes viejos ya no descuentan.
    epoch: AtomicUsize,
    /// Algún cuadro se saltó por falta de drawable y nadie ha avisado todavía.
    skipped: AtomicBool,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Flight {
    fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            in_flight: AtomicUsize::new(0),
            epoch: AtomicUsize::new(0),
            skipped: AtomicBool::new(false),
            wake,
        })
    }

    /// ¿Se puede pedir otro drawable sin riesgo de esperar? Si no, deja apuntado que se
    /// saltó un cuadro para que el próximo drawable que se libere despierte a la app.
    fn free(&self, limit: usize) -> bool {
        if self.in_flight.load(Ordering::SeqCst) < limit {
            return true;
        }
        // Primero la marca y después se vuelve a mirar: un manejador que descuente entre
        // medias o lo vemos aquí o él ve la marca y avisa. Así no se pierde el aviso.
        self.skipped.store(true, Ordering::SeqCst);
        if self.in_flight.load(Ordering::SeqCst) < limit {
            self.skipped.store(false, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// Apunta un drawable recién tomado. Se descuenta al soltar el billete.
    fn ticket(self: &Arc<Self>) -> Arc<Ticket> {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        Arc::new(Ticket {
            flight: self.clone(),
            epoch: self.epoch.load(Ordering::SeqCst),
            released: AtomicBool::new(false),
        })
    }

    /// Olvida los drawables en vuelo (sus manejadores no llegaron).
    fn reset(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.in_flight.store(0, Ordering::SeqCst);
    }
}

/// Un drawable en vuelo. Lo sueltan el manejador de «presentado» o, si el drawable se
/// destruye sin llegar a mostrarse (y con él el bloque), su `Drop`; solo cuenta una vez.
struct Ticket {
    flight: Arc<Flight>,
    epoch: usize,
    released: AtomicBool,
}

impl Ticket {
    fn release(&self) {
        if self.released.swap(true, Ordering::SeqCst) {
            return;
        }
        let f = &self.flight;
        if f.epoch.load(Ordering::SeqCst) == self.epoch {
            let _ = f
                .in_flight
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
        }
        if f.skipped.swap(false, Ordering::SeqCst) {
            (f.wake)();
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.release();
    }
}

// ---------------------------------------------------------------------------------------
// Búferes
// ---------------------------------------------------------------------------------------

/// Un par de búferes (vértices e índices) para un cuadro.
#[derive(Clone)]
struct Bufs {
    vtx: Retained<Buffer>,
    idx: Retained<Buffer>,
}

/// Búferes libres. Un par sale de aquí al pintar y vuelve desde el manejador de
/// «completado» del búfer de órdenes: mientras la GPU lo lee no está en la lista, así que
/// nunca se pisa un búfer en uso.
struct Pool(Mutex<Vec<Bufs>>);

// SAFETY: los `MTLBuffer` se pueden retener, soltar y pasar entre hilos; el acceso a su
// contenido lo ordena esta lista (solo los toca quien los sacó).
unsafe impl Send for Pool {}
unsafe impl Sync for Pool {}

impl Pool {
    fn take(&self, device: &Device, vtx: usize, idx: usize) -> Option<Bufs> {
        {
            let mut free = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(i) = free
                .iter()
                .position(|b| b.vtx.length() >= vtx && b.idx.length() >= idx)
            {
                return Some(free.swap_remove(i));
            }
            // Ninguno alcanza: se descarta uno para que los pequeños no se queden para siempre.
            free.pop();
        }
        let make = |bytes: usize, min: usize| {
            device.newBufferWithLength_options(
                bytes.max(min).next_power_of_two(),
                MTLResourceOptions::StorageModeShared,
            )
        };
        Some(Bufs {
            vtx: make(vtx, 64 * 1024)?,
            idx: make(idx, 16 * 1024)?,
        })
    }

    fn put(&self, bufs: Bufs) {
        let mut free = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if free.len() < POOL_MAX {
            free.push(bufs);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------------------

/// Una textura de egui con el muestreador de sus opciones.
struct Tex {
    texture: Retained<Texture>,
    sampler: Retained<Sampler>,
    size: [usize; 2],
}

/// Un cuadro ya codificado, a falta de `commit`.
struct Frame {
    cmd: Retained<CommandBuffer>,
    /// Copia en memoria compartida de la textura de captura (BGRA, filas de `4 * ancho`).
    readback: Option<Retained<Buffer>>,
    size: [u32; 2],
}

impl Frame {
    /// Espera a la GPU y devuelve la captura. Llamar después de `commit`.
    fn image(&self) -> Option<egui::ColorImage> {
        let buf = self.readback.as_ref()?;
        self.cmd.waitUntilCompleted();
        let [w, h] = [self.size[0] as usize, self.size[1] as usize];
        // SAFETY: el búfer es compartido, mide `4 * w * h` y la GPU ya terminó con él.
        let bytes =
            unsafe { std::slice::from_raw_parts(buf.contents().as_ptr() as *const u8, 4 * w * h) };
        // Como egui-wgpu: lo que hay en el destino, premultiplicado, sin deshacer el alfa.
        let pixels = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| egui::Color32::from_rgba_premultiplied(p[2], p[1], p[0], p[3]))
            .collect();
        Some(egui::ColorImage::new([w, h], pixels))
    }
}

/// Todo lo que hace falta para dibujar mallas de egui en una textura. Sin ventana.
struct Renderer {
    device: Retained<Device>,
    queue: Retained<Queue>,
    pipeline: Retained<Pipeline>,
    /// Rectángulos instanciados de la rejilla (`grid::GridBatch`).
    grid_pipeline: Retained<Pipeline>,
    textures: HashMap<TextureId, Tex>,
    samplers: HashMap<TextureOptions, Retained<Sampler>>,
    pool: Arc<Pool>,
    /// Textura de captura, del tamaño del último cuadro capturado.
    offscreen: Option<(Retained<Texture>, [u32; 2])>,
}

impl Renderer {
    fn new() -> Result<Self, String> {
        let device = MTLCreateSystemDefaultDevice().ok_or("no hay dispositivo Metal")?;
        let queue = device
            .newCommandQueue()
            .ok_or("no se pudo crear la cola de órdenes de Metal")?;

        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(SHADER), None)
            .map_err(|e| format!("sombreador: {}", e.localizedDescription()))?;
        let function = |name: &str| {
            library
                .newFunctionWithName(&NSString::from_str(name))
                .ok_or_else(|| format!("falta la función {name} del sombreador"))
        };

        // Disposición de `epaint::Vertex`: posición, uv y color (4 bytes normalizados).
        let layout = MTLVertexDescriptor::new();
        let formats = [
            (MTLVertexFormat::Float2, 0),
            (MTLVertexFormat::Float2, 8),
            (MTLVertexFormat::UChar4Normalized, 16),
        ];
        for (i, (format, offset)) in formats.into_iter().enumerate() {
            // SAFETY: índices dentro del rango que admite Metal.
            let attr = unsafe { layout.attributes().objectAtIndexedSubscript(i) };
            attr.setFormat(format);
            unsafe {
                attr.setOffset(offset);
                attr.setBufferIndex(0);
            }
        }
        unsafe {
            layout
                .layouts()
                .objectAtIndexedSubscript(0)
                .setStride(size_of::<Vertex>());
        }

        let make = |vertex: &str, layout: Option<&MTLVertexDescriptor>| {
            let desc = MTLRenderPipelineDescriptor::new();
            desc.setVertexFunction(Some(&*function(vertex)?));
            desc.setFragmentFunction(Some(&*function("fs_main")?));
            desc.setVertexDescriptor(layout);
            // Mezcla de egui-wgpu para alfa premultiplicado.
            let color = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
            color.setPixelFormat(TARGET_FORMAT);
            color.setBlendingEnabled(true);
            color.setRgbBlendOperation(MTLBlendOperation::Add);
            color.setSourceRGBBlendFactor(MTLBlendFactor::One);
            color.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
            color.setAlphaBlendOperation(MTLBlendOperation::Add);
            color.setSourceAlphaBlendFactor(MTLBlendFactor::OneMinusDestinationAlpha);
            color.setDestinationAlphaBlendFactor(MTLBlendFactor::One);
            device
                .newRenderPipelineStateWithDescriptor_error(&desc)
                .map_err(|e| format!("tubería {vertex}: {}", e.localizedDescription()))
        };
        let pipeline = make("vs_main", Some(&layout))?;
        // La rejilla lee sus instancias del búfer directamente, sin descriptor.
        let grid_pipeline = make("vs_grid", None)?;

        Ok(Self {
            device,
            queue,
            pipeline,
            grid_pipeline,
            textures: HashMap::new(),
            samplers: HashMap::new(),
            pool: Arc::new(Pool(Mutex::new(Vec::new()))),
            offscreen: None,
        })
    }

    fn sampler(&mut self, options: TextureOptions) -> Option<Retained<Sampler>> {
        if let Some(s) = self.samplers.get(&options) {
            return Some(s.clone());
        }
        let filter = |f| match f {
            TextureFilter::Nearest => MTLSamplerMinMagFilter::Nearest,
            TextureFilter::Linear => MTLSamplerMinMagFilter::Linear,
        };
        let wrap = match options.wrap_mode {
            TextureWrapMode::ClampToEdge => MTLSamplerAddressMode::ClampToEdge,
            TextureWrapMode::Repeat => MTLSamplerAddressMode::Repeat,
            TextureWrapMode::MirroredRepeat => MTLSamplerAddressMode::MirrorRepeat,
        };
        // Los mipmaps no se usan (como en egui-wgpu).
        let desc = MTLSamplerDescriptor::new();
        desc.setMagFilter(filter(options.magnification));
        desc.setMinFilter(filter(options.minification));
        desc.setSAddressMode(wrap);
        desc.setTAddressMode(wrap);
        let sampler = self.device.newSamplerStateWithDescriptor(&desc)?;
        self.samplers.insert(options, sampler.clone());
        Some(sampler)
    }

    /// Crea o actualiza las texturas que egui cambió. Va antes de dibujar.
    fn set_textures(&mut self, delta: &egui::TexturesDelta) {
        for (id, deltas) in &delta.set {
            for delta in deltas {
                self.set_texture(*id, delta);
            }
        }
    }

    /// Suelta las texturas que egui ya no usa. Va después de dibujar.
    fn free_textures(&mut self, delta: &egui::TexturesDelta) {
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    fn set_texture(&mut self, id: TextureId, delta: &epaint::ImageDelta) {
        let epaint::ImageData::Color(image) = &delta.image;
        let [w, h] = image.size;
        if w == 0 || h == 0 || image.pixels.len() != w * h {
            return;
        }
        let Some(sampler) = self.sampler(delta.options) else {
            return;
        };

        let origin = match delta.pos {
            // Un trozo de una textura que ya existe.
            Some([x, y]) => {
                let Some(tex) = self.textures.get_mut(&id) else {
                    return;
                };
                if x + w > tex.size[0] || y + h > tex.size[1] {
                    return;
                }
                tex.sampler = sampler;
                [x, y]
            }
            // La textura entera: se crea de nuevo, a su tamaño exacto.
            None => {
                // SAFETY: formato y tamaño válidos (w, h > 0).
                let desc = unsafe {
                    MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                        MTLPixelFormat::RGBA8Unorm,
                        w,
                        h,
                        false,
                    )
                };
                desc.setUsage(MTLTextureUsage::ShaderRead);
                let Some(texture) = self.device.newTextureWithDescriptor(&desc) else {
                    return;
                };
                self.textures.insert(
                    id,
                    Tex {
                        texture,
                        sampler,
                        size: [w, h],
                    },
                );
                [0, 0]
            }
        };

        let region = MTLRegion {
            origin: MTLOrigin {
                x: origin[0],
                y: origin[1],
                z: 0,
            },
            size: MTLSize {
                width: w,
                height: h,
                depth: 1,
            },
        };
        let Some(bytes) = NonNull::new(image.pixels.as_ptr() as *mut c_void) else {
            return;
        };
        // SAFETY: `Color32` son 4 bytes RGBA; hay `w * h` píxeles y la región cabe en la
        // textura (comprobado arriba). Metal copia los bytes antes de volver.
        unsafe {
            self.textures[&id]
                .texture
                .replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, bytes, 4 * w);
        }
    }

    /// Textura de captura del tamaño pedido (se conserva entre cuadros).
    fn offscreen(&mut self, size: [u32; 2]) -> Option<Retained<Texture>> {
        if let Some((tex, s)) = &self.offscreen
            && *s == size
        {
            return Some(tex.clone());
        }
        // SAFETY: formato y tamaño válidos (quien llama descarta el tamaño cero).
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                TARGET_FORMAT,
                size[0] as usize,
                size[1] as usize,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::RenderTarget);
        desc.setStorageMode(MTLStorageMode::Private);
        let tex = self.device.newTextureWithDescriptor(&desc)?;
        self.offscreen = Some((tex.clone(), size));
        Some(tex)
    }

    /// Codifica un cuadro: lo dibuja en `window` (la textura del drawable), en la textura
    /// de captura, o en ambas. Devuelve el búfer de órdenes sin enviar. `None` si no hay
    /// dónde dibujar o Metal no da recursos.
    fn encode(
        &mut self,
        size: [u32; 2],
        pixels_per_point: f32,
        clear: [f32; 4],
        primitives: &[egui::ClippedPrimitive],
        window: Option<&Texture>,
        capture: bool,
    ) -> Option<Frame> {
        if size[0] == 0 || size[1] == 0 || (window.is_none() && !capture) {
            return None;
        }
        let offscreen = if capture {
            Some(self.offscreen(size)?)
        } else {
            None
        };

        // Todas las mallas del cuadro, seguidas, en un par de búferes. Las instancias de la
        // rejilla van en el búfer de vértices, detrás de las mallas (alineadas a 16).
        let (mut vtx_bytes, mut idx_bytes) = (0, 0);
        let mut grid_bytes = 0;
        for p in primitives {
            match &p.primitive {
                Primitive::Mesh(m) => {
                    vtx_bytes += m.vertices.len() * size_of::<Vertex>();
                    idx_bytes += m.indices.len() * size_of::<u32>();
                }
                Primitive::Callback(c) => {
                    if let Some(g) = c.callback.downcast_ref::<GridBatch>() {
                        grid_bytes += g.instances.len() * size_of::<Instance>();
                    }
                }
            }
        }
        let grid_start = vtx_bytes.next_multiple_of(16);
        let bufs = self
            .pool
            .take(&self.device, grid_start + grid_bytes, idx_bytes)?;
        let mut draws = Vec::with_capacity(primitives.len());
        let (mut vtx_at, mut idx_at) = (0, 0);
        let mut grid_at = grid_start;
        for p in primitives {
            let m = match &p.primitive {
                Primitive::Mesh(m) => m,
                Primitive::Callback(c) => {
                    // Solo hay un tipo de `Callback` en esta app: la rejilla.
                    let Some(g) = c.callback.downcast_ref::<GridBatch>() else {
                        continue;
                    };
                    let n = g.instances.len() * size_of::<Instance>();
                    if n == 0 {
                        continue;
                    }
                    // SAFETY: el búfer mide al menos `grid_start + grid_bytes` y nadie más
                    // lo usa; `Instance` es `repr(C)` de datos planos.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            g.instances.as_ptr() as *const u8,
                            (bufs.vtx.contents().as_ptr() as *mut u8).add(grid_at),
                            n,
                        );
                    }
                    draws.push(Draw {
                        scissor: scissor(&p.clip_rect, pixels_per_point, size),
                        texture: TextureId::default(),
                        vtx_at: grid_at,
                        idx_at: 0,
                        indices: 0,
                        instances: g.instances.len(),
                    });
                    grid_at += n;
                    continue;
                }
            };
            let (vn, in_) = (
                m.vertices.len() * size_of::<Vertex>(),
                m.indices.len() * size_of::<u32>(),
            );
            // SAFETY: los búferes miden al menos el total calculado arriba y nadie más
            // los usa (salieron de la lista de libres); `Vertex` y `u32` son datos planos.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    m.vertices.as_ptr() as *const u8,
                    (bufs.vtx.contents().as_ptr() as *mut u8).add(vtx_at),
                    vn,
                );
                std::ptr::copy_nonoverlapping(
                    m.indices.as_ptr() as *const u8,
                    (bufs.idx.contents().as_ptr() as *mut u8).add(idx_at),
                    in_,
                );
            }
            draws.push(Draw {
                scissor: scissor(&p.clip_rect, pixels_per_point, size),
                texture: m.texture_id,
                vtx_at,
                idx_at,
                indices: m.indices.len(),
                instances: 0,
            });
            vtx_at += vn;
            idx_at += in_;
        }

        let cmd = self.queue.commandBuffer()?;
        for target in [offscreen.as_deref(), window].into_iter().flatten() {
            self.pass(&cmd, target, size, pixels_per_point, clear, &bufs, &draws)?;
        }

        let readback = match &offscreen {
            Some(tex) => Some(self.copy_out(&cmd, tex, size)?),
            None => None,
        };

        // Los búferes vuelven a la lista cuando la GPU termina con ellos.
        let pool = self.pool.clone();
        let done = RcBlock::new(move |_: NonNull<CommandBuffer>| pool.put(bufs.clone()));
        // SAFETY: bloque válido; Metal lo copia y lo llama una vez, desde su hilo. Lo que
        // captura (la lista y los búferes) se puede usar y soltar desde cualquier hilo.
        unsafe { cmd.addCompletedHandler(RcBlock::as_ptr(&done)) };

        Some(Frame {
            cmd,
            readback,
            size,
        })
    }

    /// Una pasada de dibujo sobre `target`.
    #[allow(clippy::too_many_arguments)]
    fn pass(
        &self,
        cmd: &CommandBuffer,
        target: &Texture,
        size: [u32; 2],
        pixels_per_point: f32,
        clear: [f32; 4],
        bufs: &Bufs,
        draws: &[Draw],
    ) -> Option<()> {
        let desc = MTLRenderPassDescriptor::new();
        // SAFETY: el índice 0 siempre existe.
        let att = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
        att.setTexture(Some(target));
        att.setLoadAction(MTLLoadAction::Clear);
        att.setStoreAction(MTLStoreAction::Store);
        att.setClearColor(MTLClearColor {
            red: clear[0] as f64,
            green: clear[1] as f64,
            blue: clear[2] as f64,
            alpha: clear[3] as f64,
        });
        let enc = cmd.renderCommandEncoderWithDescriptor(&desc)?;

        // Tamaño de la pantalla en puntos, para pasar de puntos a coordenadas de recorte.
        let screen = [
            size[0] as f32 / pixels_per_point,
            size[1] as f32 / pixels_per_point,
        ];
        // SAFETY: el búfer de vértices mide lo que dicen los desplazamientos de `draws`.
        unsafe { enc.setVertexBuffer_offset_atIndex(Some(&bufs.vtx), 0, 0) };
        // Qué tubería está puesta: `Some(true)` = la de la rejilla.
        let mut current: Option<bool> = None;
        for d in draws {
            let (Some(rect), Some(tex)) = (d.scissor, self.textures.get(&d.texture)) else {
                continue;
            };
            let grid = d.instances > 0;
            if current != Some(grid) {
                current = Some(grid);
                // Las constantes de cada tubería: pantalla (y, en la rejilla, 1 / tamaño
                // del atlas, para normalizar los texeles igual que el teselador).
                let locals = [
                    screen[0],
                    screen[1],
                    1.0 / tex.size[0] as f32,
                    1.0 / tex.size[1] as f32,
                ];
                let len = if grid { 16 } else { 8 };
                enc.setRenderPipelineState(if grid {
                    &self.grid_pipeline
                } else {
                    &self.pipeline
                });
                // SAFETY: Metal copia los bytes en el momento.
                unsafe { enc.setVertexBytes_length_atIndex(NonNull::from(&locals).cast(), len, 1) };
            }
            enc.setScissorRect(rect);
            // SAFETY: los desplazamientos y cuentas salen de lo copiado en `encode`: cada
            // malla (y sus índices) y cada lote de instancias cabe en su búfer.
            unsafe {
                enc.setVertexBufferOffset_atIndex(d.vtx_at, 0);
                enc.setFragmentTexture_atIndex(Some(&tex.texture), 0);
                enc.setFragmentSamplerState_atIndex(Some(&tex.sampler), 0);
                if grid {
                    enc.drawPrimitives_vertexStart_vertexCount_instanceCount(
                        MTLPrimitiveType::TriangleStrip,
                        0,
                        4,
                        d.instances,
                    );
                } else {
                    enc.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
                        MTLPrimitiveType::Triangle,
                        d.indices,
                        MTLIndexType::UInt32,
                        &bufs.idx,
                        d.idx_at,
                    );
                }
            }
        }
        enc.endEncoding();
        Some(())
    }

    /// Copia la textura de captura a un búfer que se puede leer desde la CPU.
    fn copy_out(
        &self,
        cmd: &CommandBuffer,
        tex: &Texture,
        size: [u32; 2],
    ) -> Option<Retained<Buffer>> {
        let [w, h] = [size[0] as usize, size[1] as usize];
        let buf = self
            .device
            .newBufferWithLength_options(4 * w * h, MTLResourceOptions::StorageModeShared)?;
        let blit = cmd.blitCommandEncoder()?;
        // SAFETY: la región es la textura entera y el búfer mide `4 * w * h`.
        unsafe {
            blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
                tex,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
                MTLSize {
                    width: w,
                    height: h,
                    depth: 1,
                },
                &buf,
                0,
                4 * w,
                4 * w * h,
            );
        }
        blit.endEncoding();
        Some(buf)
    }

    /// Dibuja un cuadro fuera de pantalla y lo devuelve como imagen. Espera a la GPU.
    #[cfg(test)]
    fn render_image(
        &mut self,
        size: [u32; 2],
        pixels_per_point: f32,
        clear: [f32; 4],
        primitives: &[egui::ClippedPrimitive],
    ) -> Option<egui::ColorImage> {
        autoreleasepool(|_| {
            let frame = self.encode(size, pixels_per_point, clear, primitives, None, true)?;
            frame.cmd.commit();
            frame.image()
        })
    }
}

/// Una malla (o un lote de instancias de la rejilla) ya copiada a los búferes del cuadro.
struct Draw {
    /// `None`: recorte vacío, no se dibuja.
    scissor: Option<MTLScissorRect>,
    texture: TextureId,
    vtx_at: usize,
    idx_at: usize,
    indices: usize,
    /// Mayor que cero: es la rejilla (`vtx_at` apunta a sus instancias, sin índices).
    instances: usize,
}

/// El recorte de egui en píxeles, redondeado y limitado al destino como en egui-wgpu.
fn scissor(clip: &egui::Rect, pixels_per_point: f32, target: [u32; 2]) -> Option<MTLScissorRect> {
    let px = |v: f32| (pixels_per_point * v).round() as u32;
    let min_x = px(clip.min.x).clamp(0, target[0]);
    let min_y = px(clip.min.y).clamp(0, target[1]);
    let max_x = px(clip.max.x).clamp(min_x, target[0]);
    let max_y = px(clip.max.y).clamp(min_y, target[1]);
    (max_x > min_x && max_y > min_y).then_some(MTLScissorRect {
        x: min_x as usize,
        y: min_y as usize,
        width: (max_x - min_x) as usize,
        height: (max_y - min_y) as usize,
    })
}

// ---------------------------------------------------------------------------------------
// Painter
// ---------------------------------------------------------------------------------------

/// Dibuja las mallas de egui en la `CAMetalLayer` de una ventana.
pub struct Painter {
    renderer: Renderer,
    /// Subcapa de la capa de la vista de winit (como hace wgpu), no la capa de la vista.
    layer: Retained<CAMetalLayer>,
    view: Retained<NSView>,
    flight: Arc<Flight>,
    /// Tamaño del drawable ya puesto en la capa.
    size: [u32; 2],
    /// Última vez que la capa dio un drawable (o que se negó a darlo).
    last_try: Instant,
    /// La capa presenta dentro de la transacción de Core Animation (redimensionado en vivo).
    transaction: bool,
}

impl Painter {
    /// Crea el dispositivo Metal y la capa de la ventana (transparente). Hilo principal.
    ///
    /// `wake` se llama, desde cualquier hilo, cuando un drawable vuelve a estar libre
    /// después de que algún cuadro se saltara por no haber ninguno: quien lo recibe debe
    /// pedir un repintado.
    pub fn new(
        window: &winit::window::Window,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        MainThreadMarker::new().ok_or("el pintor se crea en el hilo principal")?;
        let handle = window
            .window_handle()
            .map_err(|e| format!("la ventana no da su vista: {e}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("la ventana no es de AppKit".into());
        };
        // SAFETY: winit garantiza que el puntero es su `NSView`, viva mientras la ventana.
        let view = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
            .ok_or("la ventana no tiene vista")?;

        let renderer = Renderer::new()?;

        view.setWantsLayer(true);
        let root = view.layer().ok_or("la vista no tiene capa")?;

        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&renderer.device));
        layer.setPixelFormat(TARGET_FORMAT);
        layer.setFramebufferOnly(true);
        // Ventana transparente: el contenido es alfa premultiplicado sobre lo que haya
        // detrás (el desenfoque de `blur.rs` lo pone el servidor de ventanas).
        layer.setOpaque(false);
        // El mismo espacio de color que configura wgpu para este formato.
        // SAFETY: constante de CoreGraphics.
        let srgb = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        layer.setColorspace(srgb.as_deref());
        layer.setDisplaySyncEnabled(true);
        // Red de seguridad: si aun con la cuenta `nextDrawable` llegara a esperar,
        // devuelve nil al cabo de un segundo en vez de colgarse.
        layer.setAllowsNextDrawableTimeout(true);
        layer.setContentsScale(window.scale_factor());
        layer.setFrame(root.bounds());
        layer.setAutoresizingMask(
            CAAutoresizingMask::LayerWidthSizable | CAAutoresizingMask::LayerHeightSizable,
        );
        root.addSublayer(&layer);

        Ok(Self {
            renderer,
            layer,
            view,
            flight: Flight::new(wake),
            size: [0, 0],
            last_try: Instant::now(),
            transaction: false,
        })
    }

    /// Lado máximo de una textura (para `RawInput::max_texture_side`).
    pub fn max_texture_side(&self) -> usize {
        MAX_TEXTURE_SIDE
    }

    /// Pinta un cuadro. Nunca bloquea el hilo esperando un drawable.
    ///
    /// - `size_px`: tamaño de la ventana en píxeles físicos (la capa se ajusta sola).
    /// - `clear`: color de fondo, RGBA premultiplicado (la ventana es transparente).
    /// - `textures`: se aplica **siempre**, aunque el cuadro se salte.
    /// - `capture`: además devuelve la imagen del cuadro.
    pub fn paint(
        &mut self,
        size_px: [u32; 2],
        pixels_per_point: f32,
        clear: [f32; 4],
        primitives: &[egui::ClippedPrimitive],
        textures: &egui::TexturesDelta,
        capture: bool,
    ) -> Painted {
        self.renderer.set_textures(textures);
        let painted =
            autoreleasepool(|_| self.frame(size_px, pixels_per_point, clear, primitives, capture));
        self.renderer.free_textures(textures);
        painted
    }

    fn frame(
        &mut self,
        size_px: [u32; 2],
        pixels_per_point: f32,
        clear: [f32; 4],
        primitives: &[egui::ClippedPrimitive],
        capture: bool,
    ) -> Painted {
        let skipped = |capture| Painted {
            outcome: Outcome::Skipped,
            capture,
        };
        if size_px[0] == 0 || size_px[1] == 0 {
            return skipped(None);
        }
        self.fit(size_px);
        let live = self.view.inLiveResize();
        if live != self.transaction {
            self.transaction = live;
            self.layer.setPresentsWithTransaction(live);
        }

        let acquired = self.acquire();
        let window = acquired.as_ref().map(|(d, _)| d.texture());
        let Some(frame) = self.renderer.encode(
            size_px,
            pixels_per_point,
            clear,
            primitives,
            window.as_deref(),
            capture,
        ) else {
            // Sin drawable y sin captura, o Metal no dio recursos. Si había drawable, su
            // billete se suelta aquí y la cuenta queda bien.
            return skipped(None);
        };

        let Some((drawable, ticket)) = acquired else {
            frame.cmd.commit();
            return skipped(frame.image());
        };

        let on_screen = ticket.clone();
        let presented = RcBlock::new(move |_: NonNull<ProtocolObject<dyn MTLDrawable>>| {
            on_screen.release();
        });
        // SAFETY: bloque válido; Metal lo copia. Lo llama desde otro hilo cuando el
        // drawable llega a la pantalla; si el drawable se destruye antes, el bloque se
        // suelta y el billete se descuenta en su `Drop`. Solo captura un `Arc` de átomos.
        unsafe { drawable.addPresentedHandler(RcBlock::as_ptr(&presented)) };
        drop(ticket);

        if self.transaction {
            // Redimensionado en vivo: el cuadro entra en la misma transacción que el nuevo
            // tamaño de la ventana, así el contenido no se ve estirado. Esperar a que la
            // orden quede programada es breve (no es esperar a la GPU ni a la pantalla).
            frame.cmd.commit();
            frame.cmd.waitUntilScheduled();
            drawable.present();
        } else {
            frame
                .cmd
                .presentDrawable(ProtocolObject::from_ref(&*drawable));
            frame.cmd.commit();
        }
        stats::PRESENTED.fetch_add(1, Ordering::Relaxed);
        Painted {
            outcome: Outcome::Presented,
            capture: frame.image(),
        }
    }

    /// Ajusta la capa a la vista: marco, escala y tamaño del drawable.
    fn fit(&mut self, size_px: [u32; 2]) {
        let scale = match self.view.window() {
            Some(w) => w.backingScaleFactor(),
            None => self.layer.contentsScale(),
        };
        let bounds = self.view.layer().map(|root| root.bounds());
        let moved = bounds.is_some_and(|b| b != self.layer.frame());
        let scaled = scale > 0.0 && scale != self.layer.contentsScale();
        if moved || scaled {
            // Sin las animaciones implícitas de Core Animation (un cuarto de segundo).
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            if let Some(b) = bounds.filter(|_| moved) {
                self.layer.setFrame(b);
            }
            if scaled {
                self.layer.setContentsScale(scale);
            }
            CATransaction::commit();
        }
        if size_px != self.size {
            self.size = size_px;
            self.layer
                .setDrawableSize(NSSize::new(size_px[0] as f64, size_px[1] as f64));
        }
    }

    /// Pide un drawable solo si es seguro que la capa tiene uno libre.
    fn acquire(&mut self) -> Option<(Retained<Drawable>, Arc<Ticket>)> {
        // De los `maximumDrawableCount` de la capa, uno está en pantalla y no vuelve hasta
        // que otro lo sustituye; ese ya no cuenta como «en vuelo» (su manejador ya saltó).
        // Por eso el tope es uno menos: con más, `nextDrawable` esperaría.
        let limit = drawable_limit(self.layer.maximumDrawableCount());
        if !self.flight.free(limit) {
            if self.last_try.elapsed() < STALE {
                stats::SKIPPED.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            // Hace mucho que nada se libera: se supone que algún manejador no llegó y se
            // prueba otra vez. Si la capa de verdad no tiene drawable, `nextDrawable`
            // devuelve nil al segundo y no se reintenta hasta pasado otro plazo.
            self.flight.reset();
        }
        self.last_try = Instant::now();
        let next = self.layer.nextDrawable();
        let waited = self.last_try.elapsed().as_micros() as u64;
        stats::MAX_WAIT_US.fetch_max(waited, Ordering::Relaxed);
        if next.is_none() {
            stats::SKIPPED.fetch_add(1, Ordering::Relaxed);
        }
        match next {
            Some(drawable) => Some((drawable, self.flight.ticket())),
            None => {
                // Si queda alguno en vuelo, que avise al liberarse.
                if self.flight.in_flight.load(Ordering::SeqCst) > 0 {
                    self.flight.skipped.store(true, Ordering::SeqCst);
                }
                None
            }
        }
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        // La vista de winit puede seguir viva: que no se quede con una capa sin dueño.
        self.layer.removeFromSuperlayer();
    }
}

// ---------------------------------------------------------------------------------------
// Pruebas (GPU real, fuera de pantalla)
// ---------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Mesh, Rect, pos2};

    const PPP: f32 = 1.0;
    const SIZE: [u32; 2] = [16, 16];

    /// Renderer con la textura por defecto (blanca), como el atlas de egui en `WHITE_UV`.
    fn renderer() -> Renderer {
        let mut r = Renderer::new().expect("hace falta una GPU con Metal");
        set(
            &mut r,
            TextureId::default(),
            egui::ColorImage::filled([2, 2], Color32::WHITE),
            None,
        );
        r
    }

    fn set(r: &mut Renderer, id: TextureId, image: egui::ColorImage, pos: Option<[usize; 2]>) {
        let mut delta = egui::TexturesDelta::default();
        delta.push(
            id,
            epaint::ImageDelta {
                image: image.into(),
                options: TextureOptions::NEAREST,
                pos,
            },
        );
        r.set_textures(&delta);
        // egui exige vaciarlo antes de soltarlo (lo comprueba en depuración).
        delta.clear();
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    fn solid(area: Rect, clip: Rect, color: Color32) -> egui::ClippedPrimitive {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(area, color);
        egui::ClippedPrimitive {
            clip_rect: clip,
            primitive: Primitive::Mesh(mesh),
        }
    }

    fn textured(area: Rect, id: TextureId) -> egui::ClippedPrimitive {
        let mut mesh = Mesh::with_texture(id);
        mesh.add_rect_with_uv(area, rect(0.0, 0.0, 1.0, 1.0), Color32::WHITE);
        egui::ClippedPrimitive {
            clip_rect: rect(0.0, 0.0, 16.0, 16.0),
            primitive: Primitive::Mesh(mesh),
        }
    }

    fn at(image: &egui::ColorImage, x: usize, y: usize) -> [u8; 4] {
        image.pixels[y * image.size[0] + x].to_array()
    }

    /// Igual salvo un nivel (redondeo de la mezcla y tramado).
    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1)
    }

    #[test]
    fn metal_rectangulo_opaco() {
        let mut r = renderer();
        let all = rect(0.0, 0.0, 16.0, 16.0);
        let red = Color32::from_rgb(200, 30, 10);
        let image = r
            .render_image(
                SIZE,
                PPP,
                [0.0, 0.0, 1.0, 1.0],
                &[solid(rect(4.0, 4.0, 12.0, 12.0), all, red)],
            )
            .unwrap();
        assert_eq!(image.size, [16, 16]);
        for y in 0..16 {
            for x in 0..16 {
                let inside = (4..12).contains(&x) && (4..12).contains(&y);
                let want = if inside {
                    [200, 30, 10, 255]
                } else {
                    [0, 0, 255, 255]
                };
                assert_eq!(at(&image, x, y), want, "píxel {x},{y}");
            }
        }
    }

    #[test]
    fn metal_puntos_a_pixeles() {
        // Con 2 píxeles por punto, la malla en puntos ocupa el doble en píxeles.
        let mut r = renderer();
        let image = r
            .render_image(
                SIZE,
                2.0,
                [0.0; 4],
                &[solid(
                    rect(1.0, 1.0, 3.0, 3.0),
                    rect(0.0, 0.0, 8.0, 8.0),
                    Color32::WHITE,
                )],
            )
            .unwrap();
        assert_eq!(at(&image, 1, 1), [0, 0, 0, 0]);
        assert_eq!(at(&image, 2, 2), [255; 4]);
        assert_eq!(at(&image, 5, 5), [255; 4]);
        assert_eq!(at(&image, 6, 6), [0, 0, 0, 0]);
    }

    #[test]
    fn metal_recorte() {
        let mut r = renderer();
        let green = Color32::from_rgb(0, 255, 0);
        // La malla cubre todo; el recorte deja solo x en [2, 6) e y en [8, 16).
        let prims = [
            solid(rect(0.0, 0.0, 16.0, 16.0), rect(2.0, 8.0, 6.0, 40.0), green),
            // Recorte vacío: no dibuja nada (y no descoloca las mallas siguientes).
            solid(
                rect(0.0, 0.0, 16.0, 16.0),
                rect(20.0, 20.0, 30.0, 30.0),
                Color32::WHITE,
            ),
        ];
        let image = r.render_image(SIZE, PPP, [0.0; 4], &prims).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                let inside = (2..6).contains(&x) && y >= 8;
                let want = if inside { [0, 255, 0, 255] } else { [0; 4] };
                assert_eq!(at(&image, x, y), want, "píxel {x},{y}");
            }
        }
    }

    #[test]
    fn metal_textura_parcial() {
        let mut r = renderer();
        let id = TextureId::Managed(7);
        let (red, blue) = (Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 0, 255));
        set(&mut r, id, egui::ColorImage::filled([4, 4], red), None);
        // Un trozo de 2×1 en (1, 2).
        set(
            &mut r,
            id,
            egui::ColorImage::filled([2, 1], blue),
            Some([1, 2]),
        );
        // Un trozo que no cabe se ignora, sin tocar la textura.
        set(
            &mut r,
            id,
            egui::ColorImage::filled([2, 2], Color32::WHITE),
            Some([3, 3]),
        );
        // Cada texel ocupa 4×4 píxeles.
        let image = r
            .render_image(
                SIZE,
                PPP,
                [0.0; 4],
                &[textured(rect(0.0, 0.0, 16.0, 16.0), id)],
            )
            .unwrap();
        for ty in 0..4 {
            for tx in 0..4 {
                let want = if ty == 2 && (tx == 1 || tx == 2) {
                    [0, 0, 255, 255]
                } else {
                    [255, 0, 0, 255]
                };
                assert_eq!(at(&image, tx * 4 + 2, ty * 4 + 2), want, "texel {tx},{ty}");
            }
        }

        // Al liberarla deja de dibujarse (la malla se salta).
        let mut delta = egui::TexturesDelta::default();
        delta.free.insert(id);
        r.free_textures(&delta);
        delta.clear();
        let image = r
            .render_image(
                SIZE,
                PPP,
                [0.0; 4],
                &[textured(rect(0.0, 0.0, 16.0, 16.0), id)],
            )
            .unwrap();
        assert_eq!(at(&image, 8, 8), [0; 4]);
    }

    #[test]
    fn metal_mezcla_premultiplicada() {
        let mut r = renderer();
        let all = rect(0.0, 0.0, 16.0, 16.0);
        // Rojo al 50 %, ya premultiplicado.
        let half = Color32::from_rgba_premultiplied(128, 0, 0, 128);
        let prims = [solid(rect(0.0, 0.0, 8.0, 16.0), all, half)];

        // Sobre transparente queda tal cual: el fondo de la ventana se ve a través.
        let image = r.render_image(SIZE, PPP, [0.0; 4], &prims).unwrap();
        assert!(
            near(at(&image, 4, 8), [128, 0, 0, 128]),
            "{:?}",
            at(&image, 4, 8)
        );
        assert_eq!(at(&image, 12, 8), [0; 4]);

        // Sobre blanco opaco: color = src + dst·(1 − α); el alfa sigue en 255.
        let image = r.render_image(SIZE, PPP, [1.0; 4], &prims).unwrap();
        assert!(
            near(at(&image, 4, 8), [255, 127, 127, 255]),
            "{:?}",
            at(&image, 4, 8)
        );
        assert_eq!(at(&image, 12, 8), [255; 4]);

        // Dos capas al 50 % sobre transparente: α = 128 + 128·(1 − 128/255) ≈ 192.
        let two = [prims[0].clone(), prims[0].clone()];
        let image = r.render_image(SIZE, PPP, [0.0; 4], &two).unwrap();
        assert!(
            near(at(&image, 4, 8), [192, 0, 0, 192]),
            "{:?}",
            at(&image, 4, 8)
        );
    }

    #[test]
    fn metal_cuadros_seguidos_reutilizan_buferes() {
        // Varios cuadros con mallas distintas: cada uno lee sus propios datos.
        let mut r = renderer();
        let all = rect(0.0, 0.0, 16.0, 16.0);
        for i in 0..20u8 {
            let c = Color32::from_rgb(i * 10, 255 - i * 10, i);
            let image = r
                .render_image(SIZE, PPP, [0.0; 4], &[solid(all, all, c)])
                .unwrap();
            assert_eq!(at(&image, 8, 8), c.to_array());
        }
        assert!(r.pool.0.lock().unwrap().len() <= POOL_MAX);
    }

    #[test]
    fn metal_cuadro_real_de_egui() {
        // Un cuadro de verdad: atlas de fuentes (con sus trozos), muestreo lineal y texto.
        let mut r = Renderer::new().expect("hace falta una GPU con Metal");
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(rect(0.0, 0.0, 64.0, 32.0)),
            ..Default::default()
        };
        let mut lit = 0;
        for _ in 0..2 {
            let mut out = ctx.run_ui(input.clone(), |ui| {
                ui.painter().rect_filled(ui.max_rect(), 0.0, Color32::BLACK);
                ui.colored_label(Color32::WHITE, "Colibrí");
            });
            let prims = ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
            r.set_textures(&out.textures_delta);
            let image = r
                .render_image([64, 32], out.pixels_per_point, [0.0; 4], &prims)
                .unwrap();
            r.free_textures(&out.textures_delta);
            out.textures_delta.clear();
            lit = image.pixels.iter().filter(|p| p.r() > 200).count();
        }
        assert!(lit > 20, "el texto no se ve ({lit} píxeles claros)");
        assert!(lit < 64 * 32 / 2, "demasiado blanco ({lit} píxeles)");
    }

    #[test]
    fn metal_rejilla_igual_que_mallas() {
        // Los mismos fondos y letras, una vez como mallas de egui (`rect_filled` + `text`)
        // y otra como instancias de la rejilla: los píxeles tienen que ser los mismos.
        let mut r = Renderer::new().expect("hace falta una GPU con Metal");
        let ctx = egui::Context::default();
        let mut cache = crate::grid::GlyphCache::default();
        let font = egui::FontId::monospace(13.0);
        let bold = egui::FontId::monospace(14.0);
        for ppp in [2.0f32, 1.0] {
            let screen = rect(0.0, 0.0, 300.0, 60.0);
            let mut input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(ppp);
            let size = [(300.0 * ppp) as u32, (60.0 * ppp) as u32];
            let clip = rect(2.0, 1.0, 290.0, 58.0);
            let texts = [
                (
                    "echo hola; ls -la | grep x",
                    true,
                    false,
                    Color32::from_rgb(215, 220, 228),
                ),
                ("verde negrita", true, true, Color32::from_rgb(80, 220, 100)),
                ("ñ", false, false, Color32::from_rgb(230, 80, 80)),
                ("漢", false, false, Color32::WHITE),
                ("🙂", false, false, Color32::WHITE),
            ];
            for _ in 0..2 {
                let mut instances = Vec::new();
                let mut out = ctx.run_ui(input.clone(), |ui| {
                    let c = ui.ctx().clone();
                    let painter = ui.painter().with_clip_rect(clip);
                    cache.begin(&c, &font, &bold);
                    let bgs = [
                        (rect(10.3, 4.0, 120.7, 21.0), Color32::from_rgb(40, 60, 160)),
                        (
                            rect(120.7, 4.0, 200.0, 21.0),
                            Color32::from_rgb(160, 40, 60),
                        ),
                        (
                            rect(0.0, 40.0, 300.0, 57.0),
                            Color32::from_rgba_premultiplied(20, 30, 40, 200),
                        ),
                    ];
                    for (b, color) in bgs {
                        painter.rect_filled(b, 0.0, color);
                        let b = egui::emath::GuiRounding::round_to_pixels(b, c.pixels_per_point());
                        instances.push(crate::grid::Instance::solid(b.min, b.max, color));
                    }
                    let mut y = 4.3;
                    for (i, (text, ascii, is_bold, color)) in texts.into_iter().enumerate() {
                        let pos = pos2(10.3 + i as f32 * 41.7, y);
                        let f = if is_bold { bold.clone() } else { font.clone() };
                        painter.text(pos, egui::Align2::LEFT_TOP, text, f, color);
                        cache.push_text(&c, &mut instances, pos, text, ascii, is_bold, color);
                        y += 7.9;
                    }
                });
                let mut prims =
                    ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
                // El fondo de la ventana: se pinta en los dos igual (antes de lo demás).
                let back = solid(screen, screen, Color32::from_rgb(11, 14, 20));
                prims.insert(0, back.clone());
                r.set_textures(&out.textures_delta);
                let meshes = r
                    .render_image(size, out.pixels_per_point, [0.0; 4], &prims)
                    .unwrap();
                let batch = [
                    back,
                    egui::ClippedPrimitive {
                        clip_rect: clip,
                        primitive: Primitive::Callback(egui::PaintCallback {
                            rect: screen,
                            callback: Arc::new(GridBatch { instances }),
                        }),
                    },
                ];
                let grid = r
                    .render_image(size, out.pixels_per_point, [0.0; 4], &batch)
                    .unwrap();
                r.free_textures(&out.textures_delta);
                out.textures_delta.clear();

                let lit = meshes.pixels.iter().filter(|p| p.r() > 150).count();
                assert!(lit > 100, "no se ve nada ({lit})");
                let diff: Vec<_> = (0..meshes.pixels.len())
                    .filter(|&i| meshes.pixels[i] != grid.pixels[i])
                    .map(|i| (i % size[0] as usize, i / size[0] as usize))
                    .collect();
                assert!(
                    diff.is_empty(),
                    "ppp {ppp}: {} píxeles distintos, p. ej. {:?}",
                    diff.len(),
                    &diff[..diff.len().min(10)]
                );
            }
        }
    }

    #[test]
    fn metal_cuenta_de_drawables() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let flight = Flight::new(Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(flight.free(2));
        let a = flight.ticket();
        assert!(flight.free(2));
        let b = flight.ticket();
        // Lleno: se salta y queda pendiente un aviso.
        assert!(!flight.free(2));
        assert!(!flight.free(2));
        assert_eq!(wakes.load(Ordering::SeqCst), 0);

        // El manejador llega desde otro hilo: un solo aviso por episodio.
        let a2 = a.clone();
        std::thread::spawn(move || a2.release()).join().unwrap();
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert!(flight.free(2));
        // Soltar otra vez el mismo billete (su `Drop`) no descuenta dos veces.
        drop(a);
        assert_eq!(flight.in_flight.load(Ordering::SeqCst), 1);

        // Un drawable que se destruye sin mostrarse también se descuenta, sin avisar
        // porque nadie se saltó un cuadro desde el último aviso.
        drop(b);
        assert_eq!(flight.in_flight.load(Ordering::SeqCst), 0);
        assert_eq!(wakes.load(Ordering::SeqCst), 1);

        // Tras dar la cuenta por perdida, los billetes viejos no la dejan negativa.
        let c = flight.ticket();
        flight.reset();
        let d = flight.ticket();
        drop(c);
        assert_eq!(flight.in_flight.load(Ordering::SeqCst), 1);
        drop(d);
        assert_eq!(flight.in_flight.load(Ordering::SeqCst), 0);
    }
}
