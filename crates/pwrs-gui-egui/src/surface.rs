//! The GpuSurface widget: a texture of its own, as large as the widget in
//! physical pixels, that the session's draws fill each frame in a pass
//! egui-wgpu's paint callback records into the window's frame, and a blit
//! that shows it in egui's own pass. It draws on the UI thread with the
//! device egui draws with and takes no lock on egui-wgpu's renderer: what
//! it stores between frames lives in the callback's resources and in the
//! widget's state.

use crate::{Inputs, Node, camera, egui, flag, gpu_thread, opt_points, pointer, proto};
use arc_swap::ArcSwapOption;
use crossbeam_queue::SegQueue;
use eframe::egui_wgpu;
use proto::camera::CameraSpec;
use proto::{FromHelper, WindowId};
use pwrs_gui_gpu::{FrameHeader, SURFACE_FORMAT, SurfacePlan, wgpu};
use std::sync::Arc;
use std::time::Instant;

/// The color a surface shows until the session gives it draws: the
/// window's own background in `visuals`, as the bytes the blit shows.
fn unset(visuals: &egui::Visuals) -> [f64; 4] {
    let [r, g, b, _] = visuals.panel_fill.to_srgba_unmultiplied();
    [f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0, 1.0]
}

/// What a GpuSurface stores between frames.
#[derive(Default)]
pub(crate) struct Surface {
    /// What each frame draws; none until the session gives draws, and while
    /// the GPU layer plans them again on a new device.
    pub plan: Option<Arc<SurfacePlan>>,
    /// The clear color, draws and frame uniform's own fields the session
    /// gave, which the GPU layer plans again on a new device.
    pub given: Option<gpu_thread::Given>,
    /// When the surface was first laid out, which its frame uniform's time
    /// counts from, and when it was last.
    started: Option<Instant>,
    last: Option<Instant>,
    /// The texture and attachments the last frame drew into, which the
    /// paint callback makes at the surface's size; none before its first
    /// frame and once the device they were made on is gone.
    pub targets: Arc<ArcSwapOption<Targets>>,
    /// What went wrong recording the draws in a frame, which the window's
    /// next frame reports.
    problems: Arc<SegQueue<String>>,
    /// The last problem reported, so one that recurs each frame is
    /// reported once.
    reported: Option<String>,
    /// How many of its window's frames have laid the surface out. A save
    /// requested at a count is read once the count is two more, when the
    /// frame between has painted what the surface had at the request.
    pub frames: u64,
    /// The camera the session gave the surface, as the person has moved
    /// it, which fills the frame uniform's view and projection; none until
    /// given.
    pub camera: Option<CameraSpec>,
    /// The camera move a drag over the surface is making.
    pub held: Option<camera::Held>,
    /// The buttons pressed over the surface and the last move reported.
    pointer: pointer::Tracked,
    /// Where the last frame drew the surface, which a test event's
    /// fractions are measured on; none before its first frame.
    pub place: Option<pointer::Place>,
    /// Whether a press, not the keyboard, gave it the focus it has, so
    /// no ring shows.
    focus_from_press: bool,
}

/// The texture a surface's frames draw into, at its size in physical
/// pixels, the multisampled and depth attachments its plan takes, and what
/// the blit shows it through.
pub(crate) struct Targets {
    size: [u32; 2],
    samples: u32,
    depth: Option<wgpu::TextureFormat>,
    /// The texture each frame resolves into, which the blit shows and a
    /// save reads.
    pub texture: wgpu::Texture,
    view: wgpu::TextureView,
    multisampled: Option<wgpu::TextureView>,
    depth_view: Option<wgpu::TextureView>,
    /// The part of the texture the window shows, written each frame.
    window: wgpu::Buffer,
    shown: wgpu::BindGroup,
}

impl Targets {
    fn new(device: &wgpu::Device, blit: &Blit, size: [u32; 2], samples: u32, depth: Option<wgpu::TextureFormat>) -> Targets {
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let make = |label: &str, format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: extent, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
        };
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC;
        let texture = make("PWRS-GUI GpuSurface", SURFACE_FORMAT, 1, usage);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let multisampled = (samples > 1).then(|| make("PWRS-GUI GpuSurface multisampled", SURFACE_FORMAT, samples, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&wgpu::TextureViewDescriptor::default()));
        let depth_view = depth.map(|format| make("PWRS-GUI GpuSurface depth", format, samples, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&wgpu::TextureViewDescriptor::default()));
        let window = device.create_buffer(&wgpu::BufferDescriptor { label: Some("PWRS-GUI GpuSurface window"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let shown = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PWRS-GUI GpuSurface shown"),
            layout: &blit.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&blit.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &window, offset: 0, size: None }) },
            ],
        });
        Targets { size, samples, depth, texture, view, multisampled, depth_view, window, shown }
    }
}

/// Shows a surface's texture in egui's pass, the part of it the window
/// shows mapped onto the viewport egui gives the callback, texel for pixel.
/// A target in an sRGB format is given the texture's bytes as linear
/// values, so the target's encoding stores the same bytes.
const BLIT: &str = "
struct Shown { top_left: vec2<f32>, bottom_right: vec2<f32> }
@group(0) @binding(0) var texels: texture_2d<f32>;
@group(0) @binding(1) var nearest: sampler;
@group(0) @binding(2) var<uniform> shown: Shown;
struct Corner { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Corner {
    let at = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Corner;
    out.position = vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.0, 1.0);
    out.uv = shown.top_left + at * (shown.bottom_right - shown.top_left);
    return out;
}
@fragment fn fs_gamma(in: Corner) -> @location(0) vec4<f32> {
    return textureSample(texels, nearest, in.uv);
}
@fragment fn fs_linear(in: Corner) -> @location(0) vec4<f32> {
    let c = textureSample(texels, nearest, in.uv);
    let low = c.rgb / 12.92;
    let high = pow((c.rgb + 0.055) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(high, low, c.rgb <= vec3<f32>(0.04045)), c.a);
}
";

/// The pipeline the blit draws with for egui's target format, stored in
/// egui-wgpu's callback resources. egui's pass is of one sample and no
/// depth, as the renderer requests neither from eframe.
struct Blit {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
}

impl Blit {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Blit {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("PWRS-GUI GpuSurface blit"), source: wgpu::ShaderSource::Wgsl(BLIT.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PWRS-GUI GpuSurface blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { multisampled: false, sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2 },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("PWRS-GUI GpuSurface blit"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("PWRS-GUI GpuSurface blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(if format.is_srgb() { "fs_linear" } else { "fs_gamma" }),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("PWRS-GUI GpuSurface blit"), ..wgpu::SamplerDescriptor::default() });
        Blit { format, layout, sampler, pipeline }
    }
}

/// One frame of a surface: its plan drawn into its targets at `size`
/// physical pixels, with `header` in its frame uniform when a draw reads
/// it, and the part `window` names, its top-left and bottom-right corners
/// as texture coordinates, shown in egui's pass of target format `format`.
struct Frame {
    plan: Arc<SurfacePlan>,
    size: [u32; 2],
    header: FrameHeader,
    window: [f32; 4],
    format: wgpu::TextureFormat,
    targets: Arc<ArcSwapOption<Targets>>,
    problems: Arc<SegQueue<String>>,
}

impl egui_wgpu::CallbackTrait for Frame {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<Blit>().is_none_or(|b| b.format != self.format) {
            resources.insert(Blit::new(device, self.format));
        }
        let Some(blit) = resources.get::<Blit>() else {
            return Vec::new();
        };
        let targets = match self.targets.load_full() {
            Some(t) if t.size == self.size && t.samples == self.plan.samples && t.depth == self.plan.depth => t,
            _ => {
                let made = Arc::new(Targets::new(device, blit, self.size, self.plan.samples, self.plan.depth));
                self.targets.store(Some(Arc::clone(&made)));
                made
            }
        };
        let window: Vec<u8> = self.window.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_buffer(&targets.window, 0, &window);
        self.plan.write_frame(queue, &self.header);
        if let Err(f) = self.plan.compute(encoder) {
            self.problems.push(f.to_string());
        }
        let (view, resolve_target) = match &targets.multisampled {
            Some(multisampled) => (multisampled, Some(&targets.view)),
            None => (&targets.view, None),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PWRS-GUI GpuSurface"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(self.plan.clear), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: targets.depth_view.as_ref().map(|view| self.plan.depth_attachment(view)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Err(f) = self.plan.record(&mut pass) {
            self.problems.push(f.to_string());
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let (Some(blit), Some(targets)) = (resources.get::<Blit>(), self.targets.load_full()) else {
            return;
        };
        pass.set_pipeline(&blit.pipeline);
        pass.set_bind_group(0, &targets.shown, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The physical pixels from `low` to `high` points cover at
/// `pixels_per_point`, each edge rounded as egui-wgpu rounds a callback's
/// viewport, and the same clamped to `0..screen` pixels as egui-wgpu clamps
/// the viewport it gives the callback.
pub(crate) fn edges(low: f32, high: f32, pixels_per_point: f32, screen: f32) -> ((f32, f32), (f32, f32)) {
    let (from, to) = ((low * pixels_per_point).round(), (high * pixels_per_point).round());
    let shown_from = from.clamp(0.0, screen);
    ((from, to), (shown_from, to.clamp(shown_from, screen)))
}

/// Draws a GpuSurface: a box of its `width` and `height` in points, the
/// room across and the height the window leaves where absent, showing its
/// texture as the paint callback draws it each frame, the window's own
/// background until the session gives it draws. A problem recording
/// its draws is reported once. A surface whose passes or draws read its
/// frame uniform, or that has a pass run every frame, draws every frame,
/// its header's time counted from the frame that first laid it out. A surface given a camera senses drags, which
/// with the wheel move the camera its header's view and projection come
/// from. One whose node turns pointer events on raises them, and one that
/// takes keys takes the focus as [`crate::routed::focus_keys`] says or as
/// Set-GuiFocus gives it, a ring showing a focus the keyboard or a script
/// gave it.
pub(crate) fn draw_surface(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let (width, height) = (opt_points(node, "width")?, opt_points(node, "height")?);
    let room = ui.available_size();
    let size = egui::vec2(width.unwrap_or(room.x), height.unwrap_or(room.y));
    if !size.x.is_finite() || !size.y.is_finite() {
        return Err(String::from("a GpuSurface without a width and a height takes the room its place leaves, and inside a Scroll that room has no end"));
    }
    let wanted = pointer::Wanted::of(node)?;
    let keys = flag(node, "keys", false)?;
    let steered = inputs.surfaces.get(&node.id).is_some_and(|s| s.camera.is_some());
    let (rect, response) = ui.allocate_exact_size(size, if steered || keys { egui::Sense::drag() } else { egui::Sense::hover() });
    let format = inputs.surface_format.ok_or_else(|| String::from("egui has not given the format it draws into yet"))?;
    let take = inputs.take_focus(node.id);
    let surface = inputs.surfaces.entry(node.id).or_default();
    if take && keys {
        response.request_focus();
    }
    let ring = keys && crate::routed::focus_keys(ui, &response, &mut surface.focus_from_press);
    surface.frames += 1;
    let mut problem = None;
    while let Some(p) = surface.problems.pop() {
        problem = Some(p);
    }
    if let Some(p) = problem
        && surface.reported.as_ref() != Some(&p)
    {
        surface.reported = Some(p.clone());
        out.push(FromHelper::Error { window: Some(window), context: String::from("GPU"), text: format!("GpuSurface {}: {p}", node.name.as_deref().unwrap_or("")) });
    }
    let ctx = ui.ctx();
    let ppp = ctx.pixels_per_point();
    let screen = ctx.viewport_rect().size() * ppp;
    let ((left, right), (shown_left, shown_right)) = edges(rect.min.x, rect.max.x, ppp, screen.x.round());
    let ((top, bottom), (shown_top, shown_bottom)) = edges(rect.min.y, rect.max.y, ppp, screen.y.round());
    let (across, down) = (right - left, bottom - top);
    if across < 1.0 || down < 1.0 {
        return Ok(rect);
    }
    let place = pointer::Place { left, top, across, down, ppp };
    surface.place = Some(place);
    if wanted.any() {
        let drawn = pointer::Drawn { rect, place, over: response.contains_pointer() };
        surface.pointer.report(ctx, &drawn, wanted, (window, node.id), out);
    }
    let window_uv = [(shown_left - left) / across, (shown_top - top) / down, (shown_right - left) / across, (shown_bottom - top) / down];
    let plan = surface.plan.clone().unwrap_or_else(|| Arc::new(SurfacePlan::cleared(unset(ui.visuals()))));
    let now = Instant::now();
    let started = *surface.started.get_or_insert(now);
    let delta = surface.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32());
    surface.last = Some(now);
    let pointer = ctx.input(|i| i.pointer.hover_pos()).filter(|at| rect.contains(*at)).map_or([-1.0, -1.0], |at| [at.x * ppp - left, at.y * ppp - top]);
    let mut header = FrameHeader { time: now.duration_since(started).as_secs_f32(), delta, resolution: [across, down], pointer, dark: ui.visuals().dark_mode, scale: ppp, ..FrameHeader::default() };
    if let Some(camera) = surface.camera.as_mut() {
        if camera::steer(ui, &response, camera, &mut surface.held) {
            ctx.request_repaint();
        }
        header.view = camera.view();
        header.projection = camera.projection(f64::from(across) / f64::from(down));
    }
    if plan.animates() {
        ctx.request_repaint();
    }
    let frame = Frame { plan, size: [across as u32, down as u32], header, window: window_uv, format, targets: Arc::clone(&surface.targets), problems: Arc::clone(&surface.problems) };
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, frame));
    if ring {
        crate::routed::focus_ring(ui, rect);
    }
    Ok(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_wgpu::CallbackTrait;
    use pwrs_gui_gpu::{
        BlendMode, CompareFunction, CullMode, CustomBlend, DepthTest, DrawPipeline, Gpu, GpuOptions, Layer, Pipeline, PrimitiveTopology, StencilFace, StencilTest, SurfaceDraw, SurfaceSet,
    };

    #[test]
    fn a_surface_takes_the_pixels_egui_wgpu_gives_its_viewport_and_shows_the_part_on_screen() {
        assert_eq!(edges(10.0, 110.0, 1.5, 1000.0), ((15.0, 165.0), (15.0, 165.0)), "each edge rounded on its own");
        assert_eq!(edges(10.2, 20.6, 2.0, 1000.0), ((20.0, 41.0), (20.0, 41.0)));
        assert_eq!(edges(-30.0, 70.0, 1.0, 1000.0), ((-30.0, 70.0), (0.0, 70.0)), "a surface above the window shows its lower part");
        assert_eq!(edges(950.0, 1050.0, 1.0, 1000.0), ((950.0, 1050.0), (950.0, 1000.0)), "and one past its edge its upper part");
        assert_eq!(edges(1200.0, 1300.0, 1.0, 1000.0), ((1200.0, 1300.0), (1000.0, 1000.0)), "one wholly outside shows nothing");
    }

    /// A surface given no draws is cleared to its window's background,
    /// light or dark, as the bytes its texture stores.
    #[test]
    fn a_surface_without_draws_shows_the_windows_background() {
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let [r, g, b, _] = visuals.panel_fill.to_srgba_unmultiplied();
            let stored = unset(&visuals).map(|v| (v * 255.0).round() as u8);
            assert_eq!(stored, [r, g, b, 255], "the texture stores the panel's bytes, opaque");
        }
    }

    /// Fills a surface two pixels square with a color in each quarter: red
    /// and green in the top row, blue and a gray of 100 in each channel
    /// below them.
    const QUARTERS: &str = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.0, 1.0);
        }
        @fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
            var colors = array<vec4<f32>, 4>(vec4<f32>(1.0, 0.0, 0.0, 1.0), vec4<f32>(0.0, 1.0, 0.0, 1.0), vec4<f32>(0.0, 0.0, 1.0, 1.0), vec4<f32>(vec3<f32>(100.0 / 255.0), 1.0));
            return colors[u32(at.x) + 2u * u32(at.y)];
        }";

    /// The texels [`QUARTERS`] leaves in the surface's texture.
    const QUARTER_TEXELS: [u8; 16] = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 100, 100, 100, 255];

    /// `frame` prepared as egui-wgpu prepares a paint callback, then painted
    /// into a target of the frame's format `pixels` across and down, as
    /// egui's pass paints it, and read back.
    fn painted(layer: &Layer, frame: &Frame, resources: &mut egui_wgpu::CallbackResources, pixels: [u32; 2]) -> Vec<u8> {
        let gpu = layer.gpu();
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let target = gpu.create_texture(pixels[0], pixels[1], frame.format, Some(usage), "painted").expect("the target");
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: pixels, pixels_per_point: 1.0 };
        let mut encoder = gpu.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("painted") });
        let mut commands = frame.prepare(gpu.device(), gpu.queue(), &screen, &mut encoder, resources);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("painted"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            let whole = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(pixels[0] as f32, pixels[1] as f32));
            frame.paint(egui::PaintCallbackInfo { viewport: whole, clip_rect: whole, pixels_per_point: 1.0, screen_size_px: pixels }, &mut pass, resources);
        }
        commands.push(encoder.finish());
        gpu.queue().submit(commands);
        assert_eq!(layer.take_errors(), Vec::<String>::new(), "the frame raised no error");
        assert_eq!(frame.problems.pop(), None, "the plan recorded");
        gpu.read_texture(&target).expect("the pixels")
    }

    #[test]
    fn the_blit_shows_the_surface_texel_for_pixel_in_targets_that_encode_srgb_and_ones_that_do_not() {
        let layer = layer("PWRS-GUI blit test");
        let plan = Arc::new(layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, draws: vec![one_draw(QUARTERS)], passes: Vec::new(), uniform: Vec::new() }).expect("the plan"));
        for format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
            let mut resources = egui_wgpu::CallbackResources::default();
            let targets: Arc<ArcSwapOption<Targets>> = Arc::default();
            let frame = |window| Frame { plan: Arc::clone(&plan), size: [2, 2], header: FrameHeader::default(), window, format, targets: Arc::clone(&targets), problems: Arc::default() };
            // A target that encodes sRGB rounds the blit's linear values back
            // to bytes, each within one of the texel's.
            let tolerance = u8::from(format.is_srgb());
            let same = |got: &[u8], want: &[u8]| got.len() == want.len() && got.iter().zip(want).all(|(g, w)| g.abs_diff(*w) <= tolerance);
            let whole = painted(&layer, &frame([0.0, 0.0, 1.0, 1.0]), &mut resources, [2, 2]);
            assert!(same(&whole, &QUARTER_TEXELS), "{format:?}: the whole surface, texel for pixel: {whole:?}");
            let right = painted(&layer, &frame([0.5, 0.0, 1.0, 1.0]), &mut resources, [1, 2]);
            assert!(same(&right, &[0, 255, 0, 255, 100, 100, 100, 255]), "{format:?}: the right column alone, as a surface past the window's left edge shows: {right:?}");
        }
    }

    /// A GPU layer on a device of wgpu's choosing, labeled `label`.
    fn layer(label: &str) -> Layer {
        let gpu = Gpu::open(&GpuOptions::default()).expect("a device opens");
        Layer::adopt(gpu.adapter().clone(), gpu.device().clone(), gpu.queue().clone(), label).expect("the layer takes the device")
    }

    /// A pipeline built from `source` as New-GuiGpuRenderPipeline builds
    /// one given nothing else.
    fn plain(source: &str) -> Pipeline {
        Pipeline {
            source: source.to_string(),
            vertex_entry: None,
            fragment_entry: None,
            topology: PrimitiveTopology::TriangleList,
            cull: CullMode::None,
            blend: BlendMode::Replace,
            custom_blend: CustomBlend::default(),
            depth: false,
            depth_format: None,
            depth_test: DepthTest::default(),
            stencil: StencilTest::default(),
            sample_count: 1,
            constants: Vec::new(),
        }
    }

    /// One draw of three vertices by `pipeline`.
    fn drawing(pipeline: Pipeline) -> SurfaceDraw {
        SurfaceDraw { pipeline: DrawPipeline::Inline(pipeline), vertices: Vec::new(), index: None, vertex_count: Some(3), instance_count: 1, bindings: Vec::new(), blend_constant: None, stencil_reference: None }
    }

    /// One draw of three vertices by a pipeline built from `source` as
    /// New-GuiGpuRenderPipeline builds one given nothing else.
    fn one_draw(source: &str) -> SurfaceDraw {
        drawing(plain(source))
    }

    /// A red triangle over the whole surface at depth 0.5.
    const HALF_DEEP: &str = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.5, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }";

    /// A frame's pass clears the surface's depth attachment to the depth
    /// and stencil its plan gives: a triangle tested Greater draws against
    /// a depth of 0 and not against the 1 depth starts at, and one tested
    /// Equal to a reference of 1 draws on a stencil cleared to 1 and not on
    /// one left at 0.
    #[test]
    fn a_frame_clears_its_depth_attachment_to_the_depth_and_stencil_its_plan_gives() {
        let layer = layer("PWRS-GUI frame clear test");
        let shown = |pipeline: Pipeline, reference: Option<u32>, clear_depth: Option<f64>, clear_stencil: Option<u32>| {
            let draws = vec![SurfaceDraw { stencil_reference: reference, ..drawing(pipeline) }];
            let plan = Arc::new(layer.plan(SurfaceSet { clear: [0.0, 0.0, 1.0, 1.0], clear_depth, clear_stencil, draws, passes: Vec::new(), uniform: Vec::new() }).expect("the plan"));
            let frame = Frame { plan, size: [2, 2], header: FrameHeader::default(), window: [0.0, 0.0, 1.0, 1.0], format: wgpu::TextureFormat::Rgba8Unorm, targets: Arc::default(), problems: Arc::default() };
            let texels = painted(&layer, &frame, &mut egui_wgpu::CallbackResources::default(), [2, 2]);
            texels.as_chunks::<4>().0.to_vec()
        };
        let (red, blue) = ([255, 0, 0, 255], [0, 0, 255, 255]);
        let greater = Pipeline { depth: true, depth_test: DepthTest { compare: Some(CompareFunction::Greater), ..DepthTest::default() }, ..plain(HALF_DEEP) };
        assert_eq!(shown(greater.clone(), None, Some(0.0), None), vec![red; 4], "Greater than the 0 the frame cleared to");
        assert_eq!(shown(greater, None, None, None), vec![blue; 4], "nothing is greater than 1");
        let equal = StencilTest { front: StencilFace { compare: Some(CompareFunction::Equal), ..StencilFace::default() }, ..StencilTest::default() };
        let masked = Pipeline { depth_format: Some(wgpu::TextureFormat::Stencil8), stencil: equal, ..plain(HALF_DEEP) };
        assert_eq!(shown(masked.clone(), Some(1), None, Some(1)), vec![red; 4], "the stencil cleared to the reference");
        assert_eq!(shown(masked, Some(1), None, None), vec![blue; 4], "the stencil left at 0");
    }

    /// Covers a surface reading its frame uniform: red its width over 255,
    /// green the time, blue whether the pointer is outside, alpha the view's
    /// last element.
    const FRAMED: &str = "struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, view: mat4x4<f32>, projection: mat4x4<f32> }
        @group(1) @binding(0) var<uniform> frame: Frame;
        @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.0, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(frame.resolution.x / 255.0, frame.time, select(0.0, 1.0, frame.pointer.x < 0.0), frame.view[3][3]); }";

    #[test]
    fn a_frame_writes_its_header_into_the_frame_uniform_before_the_surface_draws() {
        let layer = layer("PWRS-GUI frame test");
        let plan = Arc::new(layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, draws: vec![one_draw(FRAMED)], passes: Vec::new(), uniform: Vec::new() }).expect("the plan"));
        assert!(plan.reads_frame());
        let header = FrameHeader { time: 100.0 / 255.0, resolution: [2.0, 2.0], ..FrameHeader::default() };
        let frame = Frame { plan, size: [2, 2], header, window: [0.0, 0.0, 1.0, 1.0], format: wgpu::TextureFormat::Rgba8Unorm, targets: Arc::default(), problems: Arc::default() };
        let shown = painted(&layer, &frame, &mut egui_wgpu::CallbackResources::default(), [2, 2]);
        assert!(shown.as_chunks::<4>().0.iter().all(|texel| *texel == [2, 100, 255, 255]), "the width, the time, the pointer outside and identity view: {shown:?}");
    }
}
