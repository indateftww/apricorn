//! The wgpu presenter — uploads the rasterized screens and draws them.
//!
//! The renderer architecture (PLAN.md Phase 3) is *CPU rasterize,
//! wgpu present*: `apricorn-gfx` produces two 256×192 RGBA8 buffers,
//! and this module does the smallest GPU job that shows them — one
//! `Rgba8Unorm` texture per screen, `Queue::write_texture` per frame,
//! and two quad draws of a trivial WGSL pipeline, one per LCD. No GPU
//! compositing, no shaders beyond the quad blit; the surface presents
//! with `Fifo` (vsync).
//!
//! Layout: the two screens are stacked and integer-scaled to the
//! largest 256×384 multiple that fits the window, centered with black
//! letterbox bars — the geometry the plan's "~256×384 logical" window
//! implies, and a faithful shape at any scale. Which engine's texture
//! is the *top* quad is decided by the caller (the frame's
//! `DisplaySelect`), never here.

use apricorn_gfx::ScreenBuffer;
use wgpu::util::DeviceExt;

/// The WGSL blit: one 6-vertex quad per draw, positioned in NDC by a
/// small uniform, sampling the screen texture directly — the quad's
/// corner (u, v) = (0, 0) is its top-left in NDC *and* row 0 of the
/// buffer (the rasterizer's rows run top-down; the pos/size geometry
/// already accounts for NDC Y running up, so no V flip here — flipping
/// it renders the screens upside down, which is exactly how this bug
/// first shipped).
const SHADER: &str = r#"
struct Params {
    /// The quad's top-left corner in NDC.
    pos: vec2<f32>,
    /// The quad's size in NDC (height negative — NDC Y runs up).
    size: vec2<f32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var screen_tex: texture_2d<f32>;

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VertexOut {
    // Two triangles: 0-1-2 and 0-2-3, corners in (u, v) space.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[index];
    var out: VertexOut;
    out.pos = vec4<f32>(params.pos + c * params.size, 0.0, 1.0);
    // No flip: (0, 0) is the quad's top-left in NDC and the buffer's
    // first row — the two top-lefts coincide.
    out.uv = c;
    return out;
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(screen_tex, screen_sampler, in.uv);
}
"#;

/// One LCD's GPU state: the screen texture, its quad uniform, and the
/// bind group pairing them.
struct Screen {
    texture: wgpu::Texture,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// The quad's NDC rectangle: the top-left corner plus a size whose Y
/// is negative (NDC Y runs up, window rows run down).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Quad {
    pos: [f32; 2],
    size: [f32; 2],
}

/// Maps a physical window position to the bottom LCD using the same integer
/// scale and letterbox origin as the presenter. Bars and the top LCD miss.
pub fn touch_for_window(
    width: u32,
    height: u32,
    x: f64,
    y: f64,
) -> Option<apricorn_core::input::Touch> {
    let scale = (width / 256).min(height / 384).max(1);
    let left = width.saturating_sub(256 * scale) / 2;
    let top = height.saturating_sub(384 * scale) / 2 + 192 * scale;
    if x < f64::from(left)
        || y < f64::from(top)
        || x >= f64::from(left + 256 * scale)
        || y >= f64::from(top + 192 * scale)
        || x >= f64::from(width)
        || y >= f64::from(height)
    {
        return None;
    }
    Some(apricorn_core::input::Touch {
        x: ((x - f64::from(left)) / f64::from(scale)) as u16,
        y: ((y - f64::from(top)) / f64::from(scale)) as u16,
    })
}

impl Quad {
    /// The stacked-screens layout for a window of `width`×`height`
    /// pixels: the largest integer scale fitting both screens, then
    /// the given screen's slice of the centered 256·s×384·s block.
    ///
    /// `screen_y` is the screen's row offset within the stacked pair
    /// (0 for the top LCD, 192 for the bottom). Unit-tested by the
    /// module's tests on hand-computed window sizes.
    fn for_window(width: u32, height: u32, screen_y: u32) -> Self {
        // Integer scale: the biggest s with 256·s ≤ width and 384·s ≤
        // height, always at least 1 (fractional fit still shows at
        // 1×, letterboxed with bars on all sides when needed).
        let scale = (width / ScreenBuffer::WIDTH as u32)
            .min(height / (2 * ScreenBuffer::HEIGHT as u32))
            .max(1);
        let stacked_w = ScreenBuffer::WIDTH as u32 * scale;
        let stacked_h = 2 * ScreenBuffer::HEIGHT as u32 * scale;
        // Saturating: a window smaller than the 1× pair centers with
        // a zero bar above/left (the quad is clipped by the swapchain,
        // not shifted negative).
        let origin_x = width.saturating_sub(stacked_w) / 2;
        let origin_y = height.saturating_sub(stacked_h) / 2;
        // Window pixels → NDC: x maps [0, width] to [-1, 1]; y runs the
        // opposite way (window row 0 is NDC +1), so the size's Y is
        // negated and the pos is the rect's *top* row.
        let rect = |x0: u32, y0: u32, w: u32, h: u32| Self {
            pos: [
                ((x0 as f64 / width as f64) * 2.0 - 1.0) as f32,
                (1.0 - (y0 as f64 / height as f64) * 2.0) as f32,
            ],
            size: [
                ((w as f64 / width as f64) * 2.0) as f32,
                -((h as f64 / height as f64) * 2.0) as f32,
            ],
        };
        rect(
            origin_x,
            origin_y + screen_y * scale,
            stacked_w,
            ScreenBuffer::HEIGHT as u32 * scale,
        )
    }
}

/// The window's GPU context: surface, device, pipeline, and one
/// [`Screen`] per LCD.
pub struct Presenter {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    screens: [Screen; 2],
}

impl Presenter {
    /// Initializes wgpu against `window` and creates the fixed-size
    /// screen textures (256×192, one per LCD).
    ///
    /// # Panics
    /// Panics when any wgpu call fails — a desktop shell with no GPU
    /// context cannot render, and there is nothing to fall back to.
    /// The error text names the failing step.
    pub fn new(window: std::sync::Arc<winit::window::Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window)
            .expect("the winit window becomes a wgpu surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("a GPU adapter is present on the desktop");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("the adapter opens a device and queue");

        let caps = surface.get_capabilities(&adapter);
        // Prefer a plain (non-sRGB) format: the rasterizer emits
        // display-ready sRGB bytes, so an sRGB target would re-encode
        // them and wash the colors out.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .unwrap_or_else(|| {
                caps.formats
                    .first()
                    .copied()
                    .expect("the surface has a format")
            });
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: Vec::new(),
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("screen-blit"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER)),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("screen-blit-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("screen-blit-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screen-blit-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("screen-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let screens = std::array::from_fn(|_| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("screen"),
                size: wgpu::Extent3d {
                    width: ScreenBuffer::WIDTH as u32,
                    height: ScreenBuffer::HEIGHT as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("screen-params"),
                contents: &[0; 16],
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("screen-bind-group"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            });
            Screen {
                texture,
                params,
                bind_group,
            }
        });

        Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            screens,
        }
    }

    /// Reconfigures the surface for a new window size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Uploads both screens and presents a frame: `top` and `bottom`
    /// are the LCDs' buffers (already mapped through the frame's
    /// `DisplaySelect` by the caller).
    pub fn present(&mut self, top: &ScreenBuffer, bottom: &ScreenBuffer) {
        let size = (self.config.width, self.config.height);
        for (screen, buffer) in self.screens.iter_mut().zip([top, bottom]) {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &screen.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                buffer.as_rgba().as_flattened(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((ScreenBuffer::WIDTH * 4) as u32),
                    rows_per_image: Some(ScreenBuffer::HEIGHT as u32),
                },
                wgpu::Extent3d {
                    width: ScreenBuffer::WIDTH as u32,
                    height: ScreenBuffer::HEIGHT as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
        for (screen, screen_y) in self
            .screens
            .iter_mut()
            .zip([0u32, ScreenBuffer::HEIGHT as u32])
        {
            let quad = Quad::for_window(size.0, size.1, screen_y);
            let params: [[u8; 4]; 4] = [
                quad.pos[0].to_ne_bytes(),
                quad.pos[1].to_ne_bytes(),
                quad.size[0].to_ne_bytes(),
                quad.size[1].to_ne_bytes(),
            ];
            self.queue
                .write_buffer(&screen.params, 0, params.as_flattened());
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            // Suboptimal still presents fine — the next resize
            // reconfigures for optimal again.
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // Occluded/minimized/outdated/lost: skip this present and
            // wait for the next; a resize reconfigures the surface.
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("screen-present"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("screen-pass"),
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
            pass.set_pipeline(&self.pipeline);
            for screen in &self.screens {
                pass.set_bind_group(0, &screen.bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stylus_mapping_matches_integer_letterboxing() {
        use super::touch_for_window;
        use apricorn_core::input::Touch;
        assert_eq!(
            touch_for_window(512, 768, 58.0, 562.0),
            Some(Touch { x: 29, y: 89 })
        );
        assert_eq!(touch_for_window(512, 768, 20.0, 383.0), None);
        assert_eq!(
            touch_for_window(900, 800, 194.0, 400.0),
            Some(Touch { x: 0, y: 0 })
        );
        assert_eq!(touch_for_window(900, 800, 193.0, 400.0), None);
        assert_eq!(touch_for_window(900, 800, 706.0, 400.0), None);
        assert_eq!(
            touch_for_window(100, 200, 99.0, 199.0),
            Some(Touch { x: 99, y: 7 })
        );
        assert_eq!(touch_for_window(100, 200, 100.0, 199.0), None);
    }
    use super::Quad;

    /// An exact 2× fit (512×768 window): both screens at scale 2, no
    /// bars. Each quad is 512×384 — the full window width and half
    /// its height — and the bottom quad's top edge sits exactly at
    /// the center.
    #[test]
    fn exact_two_x_fit_fills_the_window() {
        let top = Quad::for_window(512, 768, 0);
        assert_eq!(top.pos, [-1.0, 1.0]);
        assert_eq!(top.size, [2.0, -1.0]);
        let bottom = Quad::for_window(512, 768, 192);
        assert_eq!(bottom.pos, [-1.0, 0.0]);
        assert_eq!(bottom.size, [2.0, -1.0]);
    }

    /// A window wider and taller than 1× but under 2×: scale 1 with
    /// bars on all four sides, centered.
    #[test]
    fn odd_windows_letterbox_at_one_x() {
        // 300×500: stacked 256×384, bars x (300-256)/2 = 22, y
        // (500-384)/2 = 58. Computed with the same f64 arithmetic the
        // quad itself uses, so the comparison is exact.
        let top = Quad::for_window(300, 500, 0);
        assert_eq!(
            top.pos,
            [
                ((22.0 / 300.0) * 2.0 - 1.0) as f32,
                (1.0 - (58.0 / 500.0) * 2.0) as f32,
            ]
        );
        assert_eq!(
            top.size,
            [
                ((256.0 / 300.0) * 2.0) as f32,
                -((192.0 / 500.0) * 2.0) as f32
            ]
        );
    }

    /// A window smaller than even one screen pair: the scale clamps to
    /// 1 and the quads are clipped by the swapchain, not shifted.
    #[test]
    fn tiny_windows_clip_at_one_x() {
        let top = Quad::for_window(200, 300, 0);
        assert_eq!(top.pos[0], -1.0);
        assert_eq!(top.size[0], ((256.0 / 200.0) * 2.0) as f32); // wider than the window
    }

    /// A 3×2 odd window (900×800): the 2× pair (512×768) fits centered
    /// — scale 2 (900/256 = 3 but 800/384 = 2), so the top LCD is the
    /// lower half of the stacked pair.
    #[test]
    fn scale_is_limited_by_the_tighter_axis() {
        let top = Quad::for_window(900, 800, 0);
        // Bars: x (900-512)/2 = 194, y (800-768)/2 = 16.
        let expected_x = -1.0 + 2.0 * 194.0 / 900.0;
        let expected_y = 1.0 - 2.0 * 16.0 / 800.0;
        assert_eq!(top.pos, [expected_x, expected_y]);
        assert_eq!(top.size[0], 2.0 * 512.0 / 900.0);
        assert_eq!(top.size[1], -2.0 * 384.0 / 800.0);
        // The bottom LCD starts exactly 384·… — 384 pixels down: window
        // row 16 + 384 = 400, NDC y = 1 - 2·400/800 = 0.
        let bottom = Quad::for_window(900, 800, 192);
        assert_eq!(bottom.pos[1], 0.0);
    }
}
