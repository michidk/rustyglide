//! Executes `rustyglide-core` commands with wgpu.
//!
//! The game renders into two offscreen color targets (front and back) and one
//! depth buffer at its Glide resolution times an integer scale. Presenting
//! scales the front target into a window surface with the aspect ratio kept,
//! then runs an optional hook so a host can draw on top (an overlay).

use rustyglide_core::types::{BlendFactor, Buffer, ClampMode, CmpFnc, FilterMode, MipMapMode};
use rustyglide_core::{
    Command, DrawBatch, DrawVertex, PipelineKey, Rect, SamplerKey, ShaderState, TextureBinding, TextureId, flag,
};
use std::collections::HashMap;
use std::fmt;

pub use wgpu;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Uniform offsets must be multiples of the device's alignment (256 everywhere).
const STATE_STRIDE: u64 = 256;
const STATE_SIZE: u64 = std::mem::size_of::<ShaderState>() as u64;
const _: () = assert!(STATE_SIZE <= STATE_STRIDE);

#[derive(Debug)]
pub enum Error {
    Adapter(String),
    Device(String),
    Surface(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(e) => write!(f, "no suitable GPU adapter: {e}"),
            Self::Device(e) => write!(f, "GPU device creation failed: {e}"),
            Self::Surface(e) => write!(f, "window surface failed: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// What a present hook gets: the frame being presented, after the game image.
pub struct PresentFrame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub view: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    /// Surface size in pixels.
    pub size: (u32, u32),
    /// Where the game image was drawn: x, y, width, height.
    pub viewport: [f32; 4],
}

pub type PresentHook = Box<dyn FnMut(&mut PresentFrame<'_>)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Draw,
    Lfb,
    Clear,
}

struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    encodes_srgb: bool,
}

/// One recorded draw.
struct Op {
    target: usize,
    kind: Kind,
    key: PipelineKey,
    state_offset: u32,
    bind_group: wgpu::BindGroup,
    scissor: Rect,
    vertices: std::ops::Range<u32>,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: (u32, u32),
    scale: u32,
    color: [wgpu::Texture; 2],
    color_views: [wgpu::TextureView; 2],
    front: usize,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    module: wgpu::ShaderModule,
    texture_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<(Kind, PipelineKey), wgpu::RenderPipeline>,
    state_layout: wgpu::BindGroupLayout,
    state_buffer: wgpu::Buffer,
    state_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    textures: HashMap<TextureId, wgpu::TextureView>,
    samplers: HashMap<SamplerKey, wgpu::Sampler>,
    bind_groups: HashMap<[Option<TextureBinding>; 2], wgpu::BindGroup>,
    dummy_view: wgpu::TextureView,
    point_sampler: wgpu::Sampler,
    presenter: Option<Presenter>,
    /// The front buffer was drawn to since it was last presented.
    front_dirty: bool,
    /// Present at whole multiples of the frame size with nearest filtering.
    pub integer_scale: bool,
    /// A shader preset drawing the window image, and its frame counter.
    #[cfg(feature = "shaders")]
    shader: Option<(librashader::runtime::wgpu::FilterChain, usize)>,
}

/// A wgpu instance, adapter, device and queue.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// No driver debug output or validation unless asked for with `WGPU_DEBUG=1`
/// or `WGPU_VALIDATION=1`: debug builds otherwise flood the log with driver
/// performance notes.
pub fn instance_flags() -> wgpu::InstanceFlags {
    wgpu::InstanceFlags::empty().with_env()
}

/// Applies rustyglide's instance settings: [`instance_flags`], and every
/// backend when `WGPU_BACKEND` names none wgpu knows (such as `dx` for `dx12`),
/// instead of none at all.
pub fn configure(descriptor: &mut wgpu::InstanceDescriptor) {
    descriptor.flags = instance_flags();
    if descriptor.backends.is_empty() {
        log::error!(
            "WGPU_BACKEND={:?} names no backend; use vulkan, dx12, metal or gl. Trying all.",
            std::env::var("WGPU_BACKEND").unwrap_or_default()
        );
        descriptor.backends = wgpu::Backends::all();
    }
}

impl Gpu {
    /// Picks an adapter (compatible with `surface` when given) and opens a device.
    /// When the preferred adapter cannot open a device (broken drivers, a
    /// translation layer like Wine's Direct3D 12), the next compatible one is tried.
    pub fn new(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Self, Error> {
        let preferred = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
            apply_limit_buckets: false,
        }))
        .ok();
        let mut candidates: Vec<wgpu::Adapter> = preferred.into_iter().collect();
        for adapter in pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all())) {
            let info = adapter.get_info();
            let supported = surface.is_none_or(|s| adapter.is_surface_supported(s));
            if supported && !candidates.iter().any(|c| c.get_info() == info) {
                candidates.push(adapter);
            }
        }
        if candidates.is_empty() {
            return Err(Error::Adapter("no adapter can draw to this target".into()));
        }
        let mut last = String::new();
        for adapter in candidates {
            let info = adapter.get_info();
            let request = adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("rustyglide"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            });
            match pollster::block_on(request) {
                Ok((device, queue)) => {
                    log::info!("rustyglide: {} via {:?}", info.name, info.backend);
                    return Ok(Self { instance, adapter, device, queue });
                }
                Err(e) => {
                    log::warn!("rustyglide: {} via {:?} failed: {e}", info.name, info.backend);
                    last = e.to_string();
                }
            }
        }
        Err(Error::Device(last))
    }

    /// A device without a window, for tests and off-screen use.
    pub fn headless() -> Result<Self, Error> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        configure(&mut descriptor);
        Self::new(wgpu::Instance::new(descriptor), None)
    }
}

fn target_texture(device: &wgpu::Device, label: &str, size: (u32, u32), format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn blend_factor(f: BlendFactor, source: bool) -> wgpu::BlendFactor {
    use wgpu::BlendFactor as W;
    match f {
        BlendFactor::Zero => W::Zero,
        BlendFactor::SrcAlpha => W::SrcAlpha,
        BlendFactor::Color if source => W::Dst,
        BlendFactor::Color => W::Src,
        BlendFactor::DstAlpha => W::DstAlpha,
        BlendFactor::One => W::One,
        BlendFactor::OneMinusSrcAlpha => W::OneMinusSrcAlpha,
        BlendFactor::OneMinusColor if source => W::OneMinusDst,
        BlendFactor::OneMinusColor => W::OneMinusSrc,
        BlendFactor::OneMinusDstAlpha => W::OneMinusDstAlpha,
        BlendFactor::AlphaSaturateOrPrefog if source => W::SrcAlphaSaturated,
        // shortcut: pre-fog color as a destination factor uses the fogged
        // color; dual-source blending would be exact.
        BlendFactor::AlphaSaturateOrPrefog => W::Src,
    }
}

fn compare(f: CmpFnc) -> wgpu::CompareFunction {
    use wgpu::CompareFunction as C;
    match f {
        CmpFnc::Never => C::Never,
        CmpFnc::Less => C::Less,
        CmpFnc::Equal => C::Equal,
        CmpFnc::LEqual => C::LessEqual,
        CmpFnc::Greater => C::Greater,
        CmpFnc::NotEqual => C::NotEqual,
        CmpFnc::GEqual => C::GreaterEqual,
        CmpFnc::Always => C::Always,
    }
}

fn quad(rect: Rect, tex: Option<(f32, f32)>) -> [DrawVertex; 6] {
    let (x0, y0) = (rect.x as f32, rect.y as f32);
    let (x1, y1) = (x0 + rect.width as f32, y0 + rect.height as f32);
    let v = |x: f32, y: f32, u: f32, w: f32| DrawVertex {
        pos: [x, y],
        tex0: [u, w, 1.0],
        tex1: [0.0, 0.0, 1.0],
        oow: 1.0,
        ..Default::default()
    };
    let (u1, v1) = tex.unwrap_or((0.0, 0.0));
    [
        v(x0, y0, 0.0, 0.0),
        v(x1, y0, u1, 0.0),
        v(x1, y1, u1, v1),
        v(x0, y0, 0.0, 0.0),
        v(x1, y1, u1, v1),
        v(x0, y1, 0.0, v1),
    ]
}

impl Renderer {
    /// A renderer for a Glide screen of `size` pixels drawn at `scale` times
    /// that size. `surface` is the window to present to, if any.
    pub fn new(
        gpu: &Gpu,
        surface: Option<wgpu::Surface<'static>>,
        size: (u32, u32),
        scale: u32,
    ) -> Result<Self, Error> {
        let device = gpu.device.clone();
        let queue = gpu.queue.clone();
        let scale = scale.max(1);
        let scaled = (size.0 * scale, size.1 * scale);
        let color = [
            target_texture(&device, "front", scaled, COLOR_FORMAT),
            target_texture(&device, "back", scaled, COLOR_FORMAT),
        ];
        let color_views = [color[0].create_view(&Default::default()), color[1].create_view(&Default::default())];
        let depth = target_texture(&device, "depth", scaled, DEPTH_FORMAT);
        let depth_view = depth.create_view(&Default::default());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glide"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let state_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("state"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(STATE_SIZE),
                },
                count: None,
            }],
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("textures"),
            entries: &[texture_entry(0), sampler_entry(1), texture_entry(2), sampler_entry(3)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glide"),
            bind_group_layouts: &[Some(&state_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let state_buffer = Self::state_buffer(&device, 64);
        let state_group = Self::state_group(&device, &state_layout, &state_buffer);
        let vertex_buffer = Self::vertex_buffer(&device, 1 << 16);
        let dummy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("no texture"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            dummy.as_image_copy(),
            &[0; 4],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: None },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let point_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let presenter = surface.map(|s| Presenter::new(gpu, s, scaled)).transpose()?;
        let mut renderer = Self {
            dummy_view: dummy.create_view(&Default::default()),
            device,
            queue,
            size,
            scale,
            color,
            color_views,
            front: 0,
            depth,
            depth_view,
            module,
            texture_layout,
            pipeline_layout,
            pipelines: HashMap::new(),
            state_layout,
            state_buffer,
            state_group,
            vertex_buffer,
            textures: HashMap::new(),
            samplers: HashMap::new(),
            bind_groups: HashMap::new(),
            point_sampler,
            presenter,
            front_dirty: false,
            integer_scale: false,
            #[cfg(feature = "shaders")]
            shader: None,
        };
        renderer.clear_targets();
        Ok(renderer)
    }

    fn state_buffer(device: &wgpu::Device, states: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("states"),
            size: states * STATE_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn state_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("states"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(STATE_SIZE),
                }),
            }],
        })
    }

    fn vertex_buffer(device: &wgpu::Device, vertices: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertices"),
            size: vertices * std::mem::size_of::<DrawVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Glide screen size.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    fn clear_targets(&mut self) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for view in &self.color_views {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reset"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit([encoder.finish()]);
    }

    fn pipeline(&mut self, kind: Kind, key: PipelineKey) -> &wgpu::RenderPipeline {
        self.pipelines.entry((kind, key)).or_insert_with(|| {
            let attributes = wgpu::vertex_attr_array![
                0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Float32x4, 4 => Float32x3, 5 => Float32x3
            ];
            let blend = key.blend.map(|b| wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: blend_factor(b.rgb_src, true),
                    dst_factor: blend_factor(b.rgb_dst, false),
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: blend_factor(b.alpha_src, true),
                    dst_factor: blend_factor(b.alpha_dst, false),
                    operation: wgpu::BlendOperation::Add,
                },
            });
            let mut write_mask = wgpu::ColorWrites::empty();
            if key.write_rgb {
                write_mask |= wgpu::ColorWrites::COLOR;
            }
            if key.write_alpha {
                write_mask |= wgpu::ColorWrites::ALPHA;
            }
            let entry = match kind {
                Kind::Draw => "fs_main",
                Kind::Lfb => "fs_lfb",
                Kind::Clear => "fs_clear",
            };
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<DrawVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(key.depth_write),
                    depth_compare: Some(key.depth_compare.map_or(wgpu::CompareFunction::Always, compare)),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend, write_mask })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    fn sampler(&mut self, key: SamplerKey) -> wgpu::Sampler {
        self.samplers
            .entry(key)
            .or_insert_with(|| {
                let filter = |f: FilterMode| match f {
                    FilterMode::PointSampled => wgpu::FilterMode::Nearest,
                    FilterMode::Bilinear => wgpu::FilterMode::Linear,
                };
                let address = |c: ClampMode| match c {
                    ClampMode::Wrap => wgpu::AddressMode::Repeat,
                    ClampMode::Clamp => wgpu::AddressMode::ClampToEdge,
                };
                self.device.create_sampler(&wgpu::SamplerDescriptor {
                    label: None,
                    address_mode_u: address(key.clamp_s),
                    address_mode_v: address(key.clamp_t),
                    mag_filter: filter(key.mag),
                    min_filter: filter(key.min),
                    mipmap_filter: if key.lod_blend {
                        wgpu::MipmapFilterMode::Linear
                    } else {
                        wgpu::MipmapFilterMode::Nearest
                    },
                    lod_max_clamp: if key.mipmap == MipMapMode::Disable { 0.0 } else { 32.0 },
                    ..Default::default()
                })
            })
            .clone()
    }

    fn texture_group(&mut self, textures: [Option<TextureBinding>; 2]) -> wgpu::BindGroup {
        if let Some(group) = self.bind_groups.get(&textures) {
            return group.clone();
        }
        let resolve = |this: &mut Self, b: Option<TextureBinding>| match b {
            Some(b) => (
                this.textures.get(&b.texture).cloned().unwrap_or_else(|| this.dummy_view.clone()),
                this.sampler(b.sampler),
            ),
            None => (this.dummy_view.clone(), this.point_sampler.clone()),
        };
        let (v0, s0) = resolve(self, textures[0]);
        let (v1, s1) = resolve(self, textures[1]);
        let group = self.make_group(&v0, &s0, &v1, &s1);
        self.bind_groups.insert(textures, group.clone());
        group
    }

    fn make_group(
        &self,
        v0: &wgpu::TextureView,
        s0: &wgpu::Sampler,
        v1: &wgpu::TextureView,
        s1: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v0) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(s0) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(v1) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(s1) },
            ],
        })
    }

    fn upload(&self, label: &str, width: u32, height: u32, levels: &[(u32, u32, Vec<u8>)]) -> wgpu::Texture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: levels.len().max(1) as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, data)) in levels.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
                wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            );
        }
        texture
    }

    fn target_index(&self, buffer: Buffer) -> usize {
        if buffer == Buffer::Front { self.front } else { 1 - self.front }
    }

    /// Runs `commands`. Returns whether a buffer swap happened.
    pub fn execute(&mut self, commands: Vec<Command>) -> bool {
        let mut states: Vec<u8> = Vec::new();
        let mut last_state: Option<ShaderState> = None;
        let mut vertices: Vec<DrawVertex> = Vec::new();
        let mut ops: Vec<Op> = Vec::new();
        let mut swapped = false;
        let mut push_state = |state: &ShaderState, states: &mut Vec<u8>| -> u32 {
            if last_state.as_ref() != Some(state) {
                states.resize(states.len().next_multiple_of(STATE_STRIDE as usize), 0);
                states.extend_from_slice(bytemuck::bytes_of(state));
                last_state = Some(*state);
            }
            (states.len() - STATE_SIZE as usize) as u32
        };
        for command in commands {
            match command {
                Command::CreateTexture { id, levels } => {
                    let Some(&(w, h, _)) = levels.first() else { continue };
                    let texture = self.upload("glide texture", w, h, &levels);
                    self.textures.insert(id, texture.create_view(&Default::default()));
                }
                Command::DestroyTexture(id) => {
                    self.textures.remove(&id);
                    self.bind_groups.retain(|k, _| !k.iter().flatten().any(|b| b.texture == id));
                }
                Command::Draw(DrawBatch { buffer, pipeline, state, textures, scissor, vertices: v }) => {
                    let target = self.target_index(buffer);
                    let state_offset = push_state(&state, &mut states);
                    let start = vertices.len() as u32;
                    vertices.extend_from_slice(&v);
                    let bind_group = self.texture_group(textures);
                    ops.push(Op {
                        target,
                        kind: Kind::Draw,
                        key: pipeline,
                        state_offset,
                        bind_group,
                        scissor,
                        vertices: start..vertices.len() as u32,
                    });
                }
                Command::Clear { buffer, rect, color, depth, write_rgb, write_alpha, write_depth } => {
                    let target = self.target_index(buffer);
                    let mut state = ShaderState::zeroed_with_screen(self.size);
                    state.constant = color;
                    state.bias[3] = depth;
                    let state_offset = push_state(&state, &mut states);
                    let start = vertices.len() as u32;
                    vertices.extend_from_slice(&quad(rect, None));
                    let key = PipelineKey {
                        blend: None,
                        depth_compare: None,
                        depth_write: write_depth,
                        write_rgb,
                        write_alpha,
                    };
                    let bind_group = self.texture_group([None, None]);
                    ops.push(Op {
                        target,
                        kind: Kind::Clear,
                        key,
                        state_offset,
                        bind_group,
                        scissor: rect,
                        vertices: start..vertices.len() as u32,
                    });
                }
                Command::LfbWrite { buffer, rect, rgba, mask, depth, pipeline, scissor } => {
                    let target = self.target_index(buffer);
                    let (key, mut state) = match pipeline {
                        Some((key, state)) => (key, state),
                        None => {
                            let mut state = ShaderState::zeroed_with_screen(self.size);
                            state.flags[0] = flag::LFB | if depth.is_some() { flag::LFB_DEPTH } else { 0 };
                            let key = PipelineKey {
                                blend: None,
                                depth_compare: None,
                                depth_write: depth.is_some(),
                                write_rgb: true,
                                write_alpha: true,
                            };
                            (key, state)
                        }
                    };
                    state.screen = [self.size.0 as f32, self.size.1 as f32, 0.0, 0.0];
                    let state_offset = push_state(&state, &mut states);
                    let extra: Vec<u8> = mask
                        .iter()
                        .zip(depth.as_deref().unwrap_or(&[]).iter().copied().chain(std::iter::repeat(0)))
                        .flat_map(|(&m, d)| [m, (d >> 8) as u8, d as u8, 0])
                        .collect();
                    let color = self.upload("lfb", rect.width, rect.height, &[(rect.width, rect.height, rgba)]);
                    let extra = self.upload("lfb mask", rect.width, rect.height, &[(rect.width, rect.height, extra)]);
                    let bind_group = self.make_group(
                        &color.create_view(&Default::default()),
                        &self.point_sampler,
                        &extra.create_view(&Default::default()),
                        &self.point_sampler,
                    );
                    let start = vertices.len() as u32;
                    vertices.extend_from_slice(&quad(rect, Some((rect.width as f32, rect.height as f32))));
                    ops.push(Op {
                        target,
                        kind: Kind::Lfb,
                        key,
                        state_offset,
                        bind_group,
                        scissor,
                        vertices: start..vertices.len() as u32,
                    });
                }
                Command::Swap { .. } => {
                    self.front = 1 - self.front;
                    swapped = true;
                }
            }
        }
        if ops.iter().any(|op| op.target == self.front) {
            self.front_dirty = true;
        }
        self.encode(&ops, &states, &vertices);
        swapped
    }

    fn encode(&mut self, ops: &[Op], states: &[u8], vertices: &[DrawVertex]) {
        if ops.is_empty() {
            return;
        }
        let state_count = (states.len() as u64).div_ceil(STATE_STRIDE) + 1;
        if self.state_buffer.size() < state_count * STATE_STRIDE {
            self.state_buffer = Self::state_buffer(&self.device, state_count.next_power_of_two());
            self.state_group = Self::state_group(&self.device, &self.state_layout, &self.state_buffer);
        }
        let vertex_bytes: &[u8] = bytemuck::cast_slice(vertices);
        if self.vertex_buffer.size() < vertex_bytes.len() as u64 {
            let count = (vertices.len() as u64).next_power_of_two();
            self.vertex_buffer = Self::vertex_buffer(&self.device, count);
        }
        self.queue.write_buffer(&self.state_buffer, 0, states);
        self.queue.write_buffer(&self.vertex_buffer, 0, vertex_bytes);
        for op in ops {
            self.pipeline(op.kind, op.key);
        }
        let (tw, th) = (self.size.0 * self.scale, self.size.1 * self.scale);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut i = 0;
        while i < ops.len() {
            let target = ops[i].target;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glide"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.color_views[target],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            while i < ops.len() && ops[i].target == target {
                let op = &ops[i];
                i += 1;
                let s = self.scale;
                let x0 = (op.scissor.x * s).min(tw);
                let y0 = (op.scissor.y * s).min(th);
                let x1 = ((op.scissor.x + op.scissor.width) * s).min(tw);
                let y1 = ((op.scissor.y + op.scissor.height) * s).min(th);
                if x1 <= x0 || y1 <= y0 || op.vertices.is_empty() {
                    continue;
                }
                pass.set_pipeline(&self.pipelines[&(op.kind, op.key)]);
                pass.set_bind_group(0, &self.state_group, &[op.state_offset]);
                pass.set_bind_group(1, &op.bind_group, &[]);
                pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                pass.draw(op.vertices.clone(), 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    fn read_texture(&self, texture: &wgpu::Texture, aspect: wgpu::TextureAspect) -> Vec<u8> {
        let (w, h) = (texture.width(), texture.height());
        let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        if let Err(e) = self.device.poll(wgpu::PollType::wait_indefinitely()) {
            log::error!("rustyglide: read-back failed: {e}");
            return vec![0; (w * h * 4) as usize];
        }
        let Ok(data) = slice.get_mapped_range() else {
            log::error!("rustyglide: read-back buffer did not map");
            return vec![0; (w * h * 4) as usize];
        };
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            out.extend_from_slice(&data[(y * row) as usize..(y * row + w * 4) as usize]);
        }
        out
    }

    /// Samples a scaled target down to the Glide resolution, 4 bytes per pixel.
    fn downscale(&self, data: Vec<u8>) -> Vec<u8> {
        if self.scale == 1 {
            return data;
        }
        let (w, h) = self.size;
        let tw = (w * self.scale) as usize;
        let s = self.scale as usize;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let at = ((y * s + s / 2) * tw + x * s + s / 2) * 4;
                out.extend_from_slice(&data[at..at + 4]);
            }
        }
        out
    }

    /// RGBA8 pixels of a color buffer at the Glide resolution, rows top first.
    pub fn read_color(&self, buffer: Buffer) -> Vec<u8> {
        let texture = &self.color[self.target_index(buffer)];
        self.downscale(self.read_texture(texture, wgpu::TextureAspect::All))
    }

    /// Depth values (0..=1) at the Glide resolution, rows top first.
    pub fn read_depth(&self) -> Vec<f32> {
        let raw = self.downscale(self.read_texture(&self.depth, wgpu::TextureAspect::DepthOnly));
        bytemuck::cast_slice::<u8, f32>(&raw).to_vec()
    }

    /// Whether the front buffer changed since it was last presented.
    pub fn front_dirty(&self) -> bool {
        self.front_dirty
    }

    /// Resizes the window surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(p) = &mut self.presenter
            && width > 0
            && height > 0
        {
            p.config.width = width;
            p.config.height = height;
            p.surface.configure(&self.device, &p.config);
        }
    }

    /// Shows the front buffer in the window, then runs `hook`.
    /// Draws the window image with a RetroArch slang shader preset
    /// (`.slangp`), such as a CRT simulation, instead of plain scaling.
    #[cfg(feature = "shaders")]
    pub fn load_shader(&mut self, preset: &std::path::Path) -> Result<(), String> {
        let chain = librashader::runtime::wgpu::FilterChain::load_from_path(
            preset,
            librashader::presets::ShaderFeatures::NONE,
            &self.device,
            &self.queue,
            None,
        )
        .map_err(|e| e.to_string())?;
        self.shader = Some((chain, 0));
        Ok(())
    }

    pub fn present(&mut self, gamma: f32, hook: Option<&mut PresentHook>) {
        self.front_dirty = false;
        let Some(p) = &mut self.presenter else { return };
        let frame = match p.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                p.surface.configure(&self.device, &p.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        let params = [1.0 / gamma.max(0.01), if p.encodes_srgb { 1.0 } else { 0.0 }, 0.0, 0.0];
        self.queue.write_buffer(&p.params, 0, bytemuck::cast_slice(&params));
        let target = (self.size.0 * self.scale, self.size.1 * self.scale);
        let (viewport, integer) = present_viewport((p.config.width, p.config.height), target, self.integer_scale);
        let sampler = if integer { &self.point_sampler } else { &p.sampler };
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &p.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.color_views[self.front]),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: p.params.as_entire_binding() },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            #[cfg(feature = "shaders")]
            let plain = self.shader.is_none();
            #[cfg(not(feature = "shaders"))]
            let plain = true;
            if plain {
                pass.set_pipeline(&p.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.set_viewport(viewport[0], viewport[1], viewport[2].max(1.0), viewport[3].max(1.0), 0.0, 1.0);
                pass.draw(0..3, 0..1);
            }
        }
        #[cfg(feature = "shaders")]
        if let Some((chain, frame_count)) = &mut self.shader {
            use librashader::runtime::{Size, Viewport, wgpu::WgpuOutputView};
            let size = |w: f32, h: f32| Size { width: w.max(1.0) as u32, height: h.max(1.0) as u32 };
            let output = WgpuOutputView::new_from_raw(
                &view,
                size(p.config.width as f32, p.config.height as f32),
                p.config.format,
            );
            let area =
                Viewport { x: viewport[0], y: viewport[1], mvp: None, output, size: size(viewport[2], viewport[3]) };
            *frame_count += 1;
            if let Err(e) = chain.frame(&self.color[self.front], &area, &mut encoder, *frame_count, None) {
                log::error!("shader preset failed, presenting without it: {e}");
                self.shader = None;
            }
        }
        if let Some(hook) = hook {
            hook(&mut PresentFrame {
                device: &self.device,
                queue: &self.queue,
                encoder: &mut encoder,
                view: &view,
                format: p.config.format,
                size: (p.config.width, p.config.height),
                viewport,
            });
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

/// The window format that shows Glide's 8-bit frame unchanged. Any other
/// non-sRGB format may be linear (an HDR `Rgba16Float` swap chain is shown
/// as scRGB), which washes the picture out, so only plain 8-bit formats are
/// taken as they are; their sRGB twins work through the present shader.
fn surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba8UnormSrgb};
    let pick = |wanted: &[wgpu::TextureFormat]| formats.iter().copied().find(|f| wanted.contains(f));
    pick(&[Bgra8Unorm, Rgba8Unorm]).or_else(|| pick(&[Bgra8UnormSrgb, Rgba8UnormSrgb])).or(formats.first().copied())
}

impl Presenter {
    fn new(gpu: &Gpu, surface: wgpu::Surface<'static>, size: (u32, u32)) -> Result<Self, Error> {
        let device = &gpu.device;
        let caps = surface.get_capabilities(&gpu.adapter);
        let format =
            surface_format(&caps.formats).ok_or_else(|| Error::Surface("the surface supports no formats".into()))?;
        log::info!("rustyglide: presenting as {format:?}");
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(device, &config);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present"),
            source: wgpu::ShaderSource::Wgsl(include_str!("present.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_present"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_present"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("present params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self { surface, config, pipeline, layout, sampler, params, encodes_srgb: format.is_srgb() })
    }
}

trait StateExt {
    fn zeroed_with_screen(size: (u32, u32)) -> Self;
}

impl StateExt for ShaderState {
    fn zeroed_with_screen(size: (u32, u32)) -> Self {
        let mut s: ShaderState = bytemuck::Zeroable::zeroed();
        s.flags[1] = CmpFnc::Always as u32;
        s.screen = [size.0 as f32, size.1 as f32, 0.0, 0.0];
        s
    }
}

/// Where the frame of `frame` pixels lands in a surface of `surface` pixels:
/// `[x, y, width, height]`, and whether it is a whole multiple. With
/// `integer`, the largest whole multiple that fits; otherwise, or when even
/// 1x does not fit, the largest size with the frame's aspect ratio.
fn present_viewport(surface: (u32, u32), frame: (u32, u32), integer: bool) -> ([f32; 4], bool) {
    let (sw, sh) = (surface.0 as f32, surface.1 as f32);
    let k = (surface.0 / frame.0).min(surface.1 / frame.1);
    let (vw, vh, integer) = if integer && k >= 1 {
        ((frame.0 * k) as f32, (frame.1 * k) as f32, true)
    } else {
        let aspect = frame.0 as f32 / frame.1 as f32;
        let (vw, vh) = if sw / sh > aspect { (sh * aspect, sh) } else { (sw, sw / aspect) };
        (vw.round(), vh.round(), false)
    };
    ([((sw - vw) / 2.0).floor(), ((sh - vh) / 2.0).floor(), vw, vh], integer)
}

#[cfg(test)]
mod viewport_tests {
    use super::present_viewport;

    #[test]
    fn integer_scale_picks_the_largest_whole_multiple_and_centres_it() {
        assert_eq!(present_viewport((1920, 1080), (640, 480), true), ([320.0, 60.0, 1280.0, 960.0], true));
        assert_eq!(present_viewport((2560, 1440), (640, 480), true), ([320.0, 0.0, 1920.0, 1440.0], true));
        // Too small for 1x: fit instead.
        assert_eq!(present_viewport((320, 240), (640, 480), true), ([0.0, 0.0, 320.0, 240.0], false));
        assert_eq!(present_viewport((1920, 1080), (640, 480), false), ([240.0, 0.0, 1440.0, 1080.0], false));
    }
}

#[cfg(test)]
mod surface_format_tests {
    use super::surface_format;
    use wgpu::TextureFormat::*;

    #[test]
    fn prefers_plain_8_bit_over_linear_hdr_formats() {
        assert_eq!(surface_format(&[Rgba16Float, Rgb10a2Unorm, Bgra8UnormSrgb, Bgra8Unorm]), Some(Bgra8Unorm));
        assert_eq!(surface_format(&[Rgba16Float, Rgba8UnormSrgb]), Some(Rgba8UnormSrgb));
        assert_eq!(surface_format(&[Rgba16Float]), Some(Rgba16Float));
        assert_eq!(surface_format(&[]), None);
    }
}
