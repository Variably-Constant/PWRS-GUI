//! Alpha for cutouts beyond one texel at a time. -FixAlphaBorder gives
//! each transparent texel the color of the nearest texel with alpha above
//! 0, so filtering and mip levels blend no dark or keyed color in at a
//! cutout's edge; the nearest texel is found exactly, in two passes:
//! Meijster, Roerdink and Hesselink's linear-time distance transform, one
//! pass down each column and one along each row. -KeepCoverage scales each
//! rendered mip level's alpha so the share of its texels an alpha test
//! passes stays level 0's.

use crate::device::CoreRef;
use crate::error::invalid;
use crate::tasks::Interrupt;
use crate::texture::format_name;

use pwrs::prelude::*;
use wgpu::util::DeviceExt;
use wgpu::TextureFormat as F;

/// How far -FixAlphaBorder carries color: into every transparent texel, or
/// into those no farther than `reach` texels from the texel it comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Border {
    pub(crate) reach: Option<u32>,
}

impl Border {
    /// What a note says the fix did.
    pub(crate) fn said(&self) -> String {
        match self.reach {
            None => "every transparent texel took the color of the nearest texel with alpha above 0, found on the GPU".to_string(),
            Some(1) => "each transparent texel within 1 texel of one with alpha above 0 took the color of the nearest, found on the GPU".to_string(),
            Some(n) => format!("each transparent texel within {n} texels of one with alpha above 0 took the color of the nearest, found on the GPU"),
        }
    }
}

/// -BorderReach: at least 1 texel, and only beside -FixAlphaBorder.
pub(crate) fn border_of(fix: bool, reach: Option<u32>) -> PsResult<Option<Border>> {
    match (fix, reach) {
        (false, None) => Ok(None),
        (false, Some(_)) => Err(invalid("WgpuBorder", "-BorderReach limits -FixAlphaBorder: give -FixAlphaBorder")),
        (true, Some(0)) => Err(invalid("WgpuBorder", "-BorderReach is at least 1 texel; omit it to fill every transparent texel")),
        (true, reach) => Ok(Some(Border { reach })),
    }
}

/// The most memory the row pass's stacks take at once: the rows are fixed
/// in bands whose stacks fit in this and in the device's largest storage
/// binding.
const STACK_BYTES: u64 = 64 << 20;

/// The passes' uniforms: the texture's width and height, the depth slice
/// of a 3D texture, the squared reach and 1 when the reach has no limit,
/// and the first row and the number of rows of the band the row pass
/// fixes.
fn uniforms(size: (u32, u32), slice: u32, border: Border, band: (u32, u32)) -> [u8; 32] {
    let (reach2, far) = match border.reach {
        Some(r) => (u64::from(r).pow(2).min(u64::from(u32::MAX)) as u32, 0),
        None => (0, 1),
    };
    let mut out = [0u8; 32];
    for (at, v) in [size.0, size.1, slice, reach2, far, band.0, band.1, 0].into_iter().enumerate() {
        out[at * 4..at * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// The column and row passes. Every value fits in i32 for any texture up
/// to 32768 texels a side: a squared distance is at most 2 * 32767^2.
const PASSES: &str = "
struct Border {
    width: u32,
    height: u32,
    slice: u32,
    reach2: u32,
    far: u32,
    first_row: u32,
    rows: u32,
    unused: u32,
}

@group(0) @binding(1) var nearest_out: texture_storage_2d<r32sint, write>;
@group(0) @binding(2) var<storage, read_write> stack: array<u32>;
@group(0) @binding(3) var filled: texture_storage_2d<STORAGE_FORMAT, write>;
@group(0) @binding(4) var<uniform> b: Border;
@group(0) @binding(5) var nearest: texture_2d<i32>;

fn distance2(a: vec2<i32>, c: vec2<i32>) -> u32 {
    let e = vec2<u32>(abs(a - c));
    return e.x * e.x + e.y * e.y;
}

// Down one column: each texel gets the row of the nearest texel with alpha
// above 0 in its column, the upper one where two are as near, or -1 where
// the column has none. At each such texel, the texels since the one above
// it take the nearer of the two.
@compute @workgroup_size(64) fn columns(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= b.width) {
        return;
    }
    let x = i32(id.x);
    let height = i32(b.height);
    var above = -1;
    for (var y = 0; y < height; y++) {
        if (texel(vec2<i32>(x, y)).a > 0.0) {
            for (var at = above + 1; at <= y; at++) {
                var row = y;
                if (above >= 0 && at - above <= y - at) {
                    row = above;
                }
                textureStore(nearest_out, vec2<i32>(x, at), vec4<i32>(row, 0, 0, 0));
            }
            above = y;
        }
    }
    for (var at = above + 1; at < height; at++) {
        textureStore(nearest_out, vec2<i32>(x, at), vec4<i32>(above, 0, 0, 0));
    }
}

// The squared distance from texel (x, y) to the nearest texel with alpha
// above 0 in column c, which has one.
fn cost(x: i32, c: i32, y: i32) -> i32 {
    let g = y - textureLoad(nearest, vec2<i32>(c, y), 0).x;
    return (x - c) * (x - c) + g * g;
}

// Meijster's Sep: the last texel of row y at which column i's nearest
// texel is at least as near as column c's, c right of i.
fn sep(i: i32, c: i32, y: i32) -> i32 {
    let gi = y - textureLoad(nearest, vec2<i32>(i, y), 0).x;
    let gc = y - textureLoad(nearest, vec2<i32>(c, y), 0).x;
    let n = c * c - i * i + gc * gc - gi * gi;
    let d = 2 * (c - i);
    var q = n / d;
    if (n % d != 0 && n < 0) {
        q -= 1;
    }
    return q;
}

// Along one row of the band: a stack keeps the columns whose nearest
// texels are nearest somewhere along the row, each with the texel it is
// nearest from, packed as the column in the low 16 bits and that texel in
// the high 16. Walking back along the row, each texel with alpha 0 within
// reach of its nearest texel with alpha above 0 takes that texel's color.
@compute @workgroup_size(64) fn rows(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= b.rows) {
        return;
    }
    let y = i32(b.first_row + id.x);
    let width = i32(b.width);
    let base = id.x * b.width;
    var top = -1;
    for (var c = 0; c < width; c++) {
        if (textureLoad(nearest, vec2<i32>(c, y), 0).x < 0) {
            continue;
        }
        loop {
            if (top < 0) {
                break;
            }
            let e = stack[base + u32(top)];
            let start = i32(e >> 16u);
            if (cost(start, i32(e & 0xffffu), y) <= cost(start, c, y)) {
                break;
            }
            top--;
        }
        if (top < 0) {
            top = 0;
            stack[base] = u32(c);
        } else {
            let start = 1 + sep(i32(stack[base + u32(top)] & 0xffffu), c, y);
            if (start < width) {
                top++;
                stack[base + u32(top)] = u32(c) | (u32(start) << 16u);
            }
        }
    }
    for (var x = width - 1; x >= 0; x--) {
        let at = vec2<i32>(x, y);
        let here = texel(at);
        var out = here;
        if (top >= 0) {
            let e = stack[base + u32(top)];
            let c = i32(e & 0xffffu);
            let seed = vec2<i32>(c, textureLoad(nearest, vec2<i32>(c, y), 0).x);
            if (here.a == 0.0 && (b.far == 1u || distance2(seed, at) <= b.reach2)) {
                out = vec4<f32>(texel(seed).rgb, here.a);
            }
            if (x == i32(e >> 16u)) {
                top--;
            }
        }
        textureStore(filled, at, out);
    }
}
";

/// How the passes read a texel of the texture being fixed: a 2D view of
/// one layer, or a 3D texture at the depth slice the uniforms name.
const FLAT: &str = "
@group(0) @binding(0) var color: texture_2d<f32>;

fn texel(at: vec2<i32>) -> vec4<f32> {
    return textureLoad(color, at, 0);
}
";

const VOLUME: &str = "
@group(0) @binding(0) var color: texture_3d<f32>;

fn texel(at: vec2<i32>) -> vec4<f32> {
    return textureLoad(color, vec3<i32>(at, i32(b.slice)), 0);
}
";

/// Fixes the border of mip level 0 of `texture`, an Rgba16Float or
/// Rgba32Float texture, in every layer or depth slice: each texel with
/// alpha 0 within `border`'s reach of a texel with alpha above 0 takes the
/// color of the nearest such texel, its alpha left at 0.
pub(crate) fn fix_border(core: &CoreRef, texture: &wgpu::Texture, border: Border, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    let (width, height) = (texture.width(), texture.height());
    let format = texture.format();
    let storage = match format {
        F::Rgba16Float => "rgba16float",
        F::Rgba32Float => "rgba32float",
        other => return Err(invalid("WgpuBorder", format!("the border is fixed in Rgba16Float or Rgba32Float, not {}", format_name(other)))),
    };
    let band = (core.device.limits().max_storage_buffer_binding_size.min(STACK_BYTES) / (u64::from(width) * 4)).clamp(1, u64::from(height)) as u32;
    let commands = core.scoped("WgpuBorder", "the border cannot be fixed", || {
        let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Wgpu border"),
            source: wgpu::ShaderSource::Wgsl(format!("{PASSES}{}", if volume { VOLUME } else { FLAT }).replace("STORAGE_FORMAT", storage).into()),
        });
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let dimension = if volume { wgpu::TextureViewDimension::D3 } else { wgpu::TextureViewDimension::D2 };
        let color_entry = entry(0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: dimension, multisampled: false });
        let uniform_entry = entry(4, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None });
        let written = |binding: u32, format: F| entry(binding, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 });
        let columns_layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("Wgpu border"), entries: &[color_entry, written(1, F::R32Sint), uniform_entry] });
        let rows_layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wgpu border"),
            entries: &[
                color_entry,
                entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
                written(3, format),
                uniform_entry,
                entry(5, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Sint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            ],
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, name: &str| {
            let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu border"), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
            core.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Wgpu border"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let (columns, rows) = (pipeline(&columns_layout, "columns"), pipeline(&rows_layout, "rows"));
        let make = |format: F, usage: wgpu::TextureUsages| {
            core.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Wgpu border"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let nearest = make(F::R32Sint, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING);
        let filled = make(format, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let nearest_view = nearest.create_view(&wgpu::TextureViewDescriptor::default());
        let filled_view = filled.create_view(&wgpu::TextureViewDescriptor::default());
        let stack = core.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Wgpu border"),
            size: u64::from(band) * u64::from(width) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniform = |plane: u32, band: (u32, u32)| {
            core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu border"), contents: &uniforms((width, height), plane, border, band), usage: wgpu::BufferUsages::UNIFORM })
        };
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu border") });
        // Each dispatch is a pass of its own, so the next one reads what it
        // wrote.
        let dispatch = |encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, group: &wgpu::BindGroup, groups: u32| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Wgpu border"), timestamp_writes: None });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
        };
        for plane in 0..texture.depth_or_array_layers() {
            let color = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("Wgpu border"),
                dimension: Some(dimension),
                base_mip_level: 0,
                mip_level_count: Some(1),
                base_array_layer: if volume { 0 } else { plane },
                array_layer_count: Some(1),
                ..Default::default()
            });
            let down = uniform(plane, (0, 0));
            let group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wgpu border"),
                layout: &columns_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&nearest_view) },
                    wgpu::BindGroupEntry { binding: 4, resource: down.as_entire_binding() },
                ],
            });
            dispatch(&mut encoder, &columns, &group, width.div_ceil(64));
            let mut first = 0;
            while first < height {
                let count = band.min(height - first);
                let along = uniform(plane, (first, count));
                let group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Wgpu border"),
                    layout: &rows_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color) },
                        wgpu::BindGroupEntry { binding: 2, resource: stack.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&filled_view) },
                        wgpu::BindGroupEntry { binding: 4, resource: along.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&nearest_view) },
                    ],
                });
                dispatch(&mut encoder, &rows, &group, count.div_ceil(64));
                first += count;
            }
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: &filled, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z: plane }, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
        }
        encoder.finish()
    })?;
    core.submit_and_wait(vec![commands], stop)
}

/// The bins -KeepCoverage counts alpha into, each 1/4096 wide, from 0 to 1.
const BINS: usize = 4096;

/// The most memory the counts take at once: the layers of an array are
/// counted in groups whose counts fit in this and in the device's largest
/// buffer.
const COUNTS_BYTES: u64 = 64 << 20;

/// The counting pass and the scaling pass. Each texel of a counted level
/// adds one to the bin of its alpha, clamped to 0 to 1, and also to the
/// count after the bins when that alpha is at or above the reference. The
/// scaling pass multiplies alpha by the level's scale, at most to 1,
/// keeping a texel at or above the level's edge at or above the reference
/// and one below it below the reference, as the counts took them, where
/// rounding the product would carry it across.
const COVERAGE: &str = "
struct Coverage {
    reference: f32,
    scale: f32,
    slice: u32,
    depth: u32,
    edge: f32,
    unused0: u32,
    unused1: u32,
    unused2: u32,
}

@group(0) @binding(1) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(2) var<uniform> c: Coverage;

@compute @workgroup_size(8, 8, 1) fn count(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = extent();
    if (id.x >= size.x || id.y >= size.y || id.z >= c.depth) {
        return;
    }
    let a = clamp(texel(vec2<i32>(id.xy), id.z).a, 0.0, 1.0);
    atomicAdd(&counts[min(u32(a * 4096.0), 4095u)], 1u);
    if (a >= c.reference) {
        atomicAdd(&counts[4096], 1u);
    }
}

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

@fragment fn scale(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let v = texel(vec2<i32>(position.xy), c.slice);
    var a = min(v.a * c.scale, 1.0);
    if (v.a >= c.edge) {
        a = max(a, c.reference);
    } else {
        a = min(a, bitcast<f32>(bitcast<u32>(c.reference) - 1u));
    }
    return vec4<f32>(v.rgb, a);
}
";

/// How the coverage passes read a level: one layer of a 2D level, or a 3D
/// level at a depth slice.
const COVERAGE_FLAT: &str = "
@group(0) @binding(0) var level: texture_2d<f32>;

fn extent() -> vec2<u32> {
    return textureDimensions(level);
}

fn texel(at: vec2<i32>, z: u32) -> vec4<f32> {
    return textureLoad(level, at, 0);
}
";

const COVERAGE_VOLUME: &str = "
@group(0) @binding(0) var level: texture_3d<f32>;

fn extent() -> vec2<u32> {
    return textureDimensions(level).xy;
}

fn texel(at: vec2<i32>, z: u32) -> vec4<f32> {
    return textureLoad(level, vec3<i32>(at, i32(z)), 0);
}
";

/// The alpha a level's texels must reach to pass once the level is
/// scaled, from its alpha counted into `bins`: the bin edge `t`, from
/// 1/4096 to 4095/4096, whose count of texels at or above it comes nearest
/// `share` of the level's texels, the edge nearest `reference` where
/// several come as near. The level's scale is `reference / t`.
pub(crate) fn coverage_edge(bins: &[u32], share: f64, reference: f32) -> f32 {
    let total: u64 = bins.iter().map(|&n| u64::from(n)).sum();
    let target = share * total as f64;
    let measure = |above: u64, t: f32| ((above as f64 - target).abs(), (t - reference).abs());
    let last = BINS - 1;
    let mut above = u64::from(bins[last]);
    let mut best_edge = last as f32 / BINS as f32;
    let mut best = measure(above, best_edge);
    for edge in (1..last).rev() {
        above += u64::from(bins[edge]);
        let t = edge as f32 / BINS as f32;
        let (miss, change) = measure(above, t);
        if miss < best.0 || (miss == best.0 && change < best.1) {
            best = (miss, change);
            best_edge = t;
        }
    }
    best_edge
}

/// Scales the alpha of mip levels `first` and on of `texture`, an
/// Rgba16Float or Rgba32Float texture with the TextureBinding,
/// RenderAttachment and CopySrc usages, so each keeps the share of texels
/// with alpha at or above `reference` that level 0 has: each layer of an
/// array or cube against its own level 0, and every depth slice of a 3D
/// level counted together. One compute pass a level counts its alpha into
/// 4096 bins, and level 0's texels at or above the reference exactly; a
/// render pass through a copy of each level multiplies its alpha by the
/// reference over the edge coverage_edge gives, at most to 1, so the
/// texels at or above the edge pass an alpha test at the reference and the
/// rest fail it. Returns the scales, layer by layer and level by level.
pub(crate) fn keep_coverage(core: &CoreRef, texture: &wgpu::Texture, first: u32, reference: f32, stop: Option<&dyn Interrupt>) -> PsResult<Vec<f32>> {
    let levels = texture.mip_level_count();
    if first >= levels {
        return Ok(Vec::new());
    }
    let format = texture.format();
    if format.components() < 4 {
        return Err(invalid("WgpuAlpha", format!("{} has no alpha channel, so -KeepCoverage would change nothing", format_name(format))));
    }
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    let layers = if volume { 1 } else { texture.depth_or_array_layers() };
    let counted: Vec<u32> = std::iter::once(0).chain(first..levels).collect();
    let limits = core.device.limits();
    let alignment = u64::from(limits.min_storage_buffer_offset_alignment);
    let counted_bytes = (BINS as u64 + 1) * 4;
    let region = counted_bytes.div_ceil(alignment) * alignment;
    let layer_bytes = region * counted.len() as u64;
    let group_layers = (COUNTS_BYTES.min(limits.max_buffer_size) / layer_bytes).clamp(1, u64::from(layers)) as u32;
    let dimension = if volume { wgpu::TextureViewDimension::D3 } else { wgpu::TextureViewDimension::D2 };
    let level_view = |level: u32, layer: u32| {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Wgpu coverage"),
            dimension: Some(dimension),
            base_mip_level: level,
            mip_level_count: Some(1),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..Default::default()
        })
    };
    let extent_of = |level: u32| {
        let size = texture.size().mip_level_size(level, texture.dimension());
        (size.width, size.height, if volume { size.depth_or_array_layers } else { 1 })
    };
    let uniform = |scale: f32, edge: f32, slice: u32, depth: u32| {
        let mut bytes = [0u8; 32];
        bytes[0..4].copy_from_slice(&reference.to_le_bytes());
        bytes[4..8].copy_from_slice(&scale.to_le_bytes());
        bytes[8..12].copy_from_slice(&slice.to_le_bytes());
        bytes[12..16].copy_from_slice(&depth.to_le_bytes());
        bytes[16..20].copy_from_slice(&edge.to_le_bytes());
        core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu coverage"), contents: &bytes, usage: wgpu::BufferUsages::UNIFORM })
    };
    let entry = |binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
    let read = |visibility: wgpu::ShaderStages| {
        entry(0, visibility, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: dimension, multisampled: false })
    };
    let constants = |visibility: wgpu::ShaderStages| entry(2, visibility, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None });
    let module = core.scoped("WgpuCoverage", "the coverage cannot be counted", || {
        core.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Wgpu coverage"),
            source: wgpu::ShaderSource::Wgsl(format!("{COVERAGE}{}", if volume { COVERAGE_VOLUME } else { COVERAGE_FLAT }).into()),
        })
    })?;
    let (count_layout, count_pipeline) = core.scoped("WgpuCoverage", "the coverage cannot be counted", || {
        let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wgpu coverage"),
            entries: &[
                read(wgpu::ShaderStages::COMPUTE),
                entry(1, wgpu::ShaderStages::COMPUTE, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
                constants(wgpu::ShaderStages::COMPUTE),
            ],
        });
        let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu coverage"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = core.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Wgpu coverage"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("count"),
            compilation_options: Default::default(),
            cache: None,
        });
        (layout, pipeline)
    })?;
    let region_words = (region / 4) as usize;
    let mut scaled = Vec::new();
    let mut start = 0;
    while start < layers {
        let group = group_layers.min(layers - start);
        let size = u64::from(group) * layer_bytes;
        let (counts, commands) = core.scoped("WgpuCoverage", "the coverage cannot be counted", || {
            let counts = core.device.create_buffer(&wgpu::BufferDescriptor { label: Some("Wgpu coverage"), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu coverage") });
            for layer in start..start + group {
                for (at, &level) in counted.iter().enumerate() {
                    let region_at = u64::from(layer - start) * layer_bytes + at as u64 * region;
                    let (width, height, depth) = extent_of(level);
                    let view = level_view(level, layer);
                    let values = uniform(1.0, reference, 0, depth);
                    let bind_group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Wgpu coverage"),
                        layout: &count_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &counts, offset: region_at, size: std::num::NonZeroU64::new(counted_bytes) }) },
                            wgpu::BindGroupEntry { binding: 2, resource: values.as_entire_binding() },
                        ],
                    });
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Wgpu coverage"), timestamp_writes: None });
                    pass.set_pipeline(&count_pipeline);
                    pass.set_bind_group(0, &bind_group, &[]);
                    pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), depth);
                }
            }
            (counts, encoder.finish())
        })?;
        core.submit_and_wait(vec![commands], stop)?;
        let bytes = crate::buffer::read_bytes(core, &counts, 0, size, stop)?;
        let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|&w| u32::from_le_bytes(w)).collect();
        for layer in 0..group as usize {
            let at = |index: usize| &words[(layer * counted.len() + index) * region_words..][..BINS + 1];
            let zero = at(0);
            let total: u64 = zero[..BINS].iter().map(|&n| u64::from(n)).sum();
            let share = f64::from(zero[BINS]) / total as f64;
            for (index, &level) in counted.iter().enumerate().skip(1) {
                let edge = coverage_edge(&at(index)[..BINS], share, reference);
                scaled.push((start + layer as u32, level, reference / edge, edge));
            }
        }
        start += group;
    }
    let (layout, pipeline) = core.scoped("WgpuCoverage", "the coverage cannot be kept", || {
        let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("Wgpu coverage"), entries: &[read(wgpu::ShaderStages::FRAGMENT), constants(wgpu::ShaderStages::FRAGMENT)] });
        let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu coverage"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = core.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wgpu coverage"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("scale"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        (layout, pipeline)
    })?;
    // A level is scaled through a copy of it, as a pass cannot read and
    // draw one texture.
    let scale_level = |encoder: &mut wgpu::CommandEncoder, layer: u32, level: u32, scale: f32, edge: f32| {
        let (width, height, depth) = extent_of(level);
        let copy = core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Wgpu coverage"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: depth },
            mip_level_count: 1,
            sample_count: 1,
            dimension: if volume { wgpu::TextureDimension::D3 } else { wgpu::TextureDimension::D2 },
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d { x: 0, y: 0, z: if volume { 0 } else { layer } }, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &copy, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width, height, depth_or_array_layers: depth },
        );
        let source = copy.create_view(&wgpu::TextureViewDescriptor { label: Some("Wgpu coverage"), dimension: Some(dimension), ..Default::default() });
        let target = level_view(level, layer);
        for slice in 0..depth {
            let values = uniform(scale, edge, slice, depth);
            let bind_group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wgpu coverage"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source) }, wgpu::BindGroupEntry { binding: 2, resource: values.as_entire_binding() }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Wgpu coverage"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: volume.then_some(slice),
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    };
    // One layer a submission, so only that layer's copies are held at once.
    for layer in 0..layers {
        let levels: Vec<(u32, f32, f32)> = scaled.iter().filter(|&&(l, _, scale, _)| l == layer && scale != 1.0).map(|&(_, level, scale, edge)| (level, scale, edge)).collect();
        if levels.is_empty() {
            continue;
        }
        let commands = core.scoped("WgpuCoverage", "the coverage cannot be kept", || {
            let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu coverage") });
            for &(level, scale, edge) in &levels {
                scale_level(&mut encoder, layer, level, scale, edge);
            }
            encoder.finish()
        })?;
        core.submit_and_wait(vec![commands], stop)?;
    }
    Ok(scaled.into_iter().map(|(_, _, scale, _)| scale).collect())
}
