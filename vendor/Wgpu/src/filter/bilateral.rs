//! The bilateral grid of Chen, Paris and Durand (2007), which the
//! smoothing and clarity sum through at a -GridSpacing above 0, and
//! without one where the grid counts fewer operations than the exact sum.
//! Each texel is placed in a 3D grid across, down and along its key, with
//! cells `spacing` standard deviations apart, by tents that split it
//! between the cells around it; the grid is blurred by a Gaussian on each
//! axis whose standard deviation, with the variance the tents of the
//! placing and of the reading add, makes the asked ones; and each texel
//! reads its value back by trilinear interpolation, the carried sum over
//! the weight. The grid is worked through in tiles, each with a halo of
//! the blur's reach and one cell more, and each over only the keys its own
//! texels reach, in slabs where those do not fit, so a wide range of keys
//! across the texture costs only the cells each tile's texels reach.

use super::{gaussian_weights, Bilateral, Params, SmoothBy, Work};
use crate::buffer;
use crate::error::invalid;
use crate::tasks::Interrupt;

use pwrs::prelude::*;
use wgpu::util::DeviceExt;
use wgpu::TextureFormat as F;

/// A cell's bytes in one copy of the grid: the carried sum and the weight.
const CELL_BYTES: u64 = 20;

/// The key channel standing for the whole texel, keyed by luminance.
const LUMINANCE: u32 = 4;

/// Tiles across and down when the keys split into slabs too: the widest
/// tried, in cells.
const WIDEST_SLAB_TILE: u32 = 64;

/// The bytes of the shader's Grid struct.
const SHARED_BYTES: u64 = 112;

/// Slabs one submission carries; each is waited for before the next is
/// recorded, so a texture of many tiles holds one submission's values and
/// bind group at a time. AMD's Vulkan driver on a Radeon iGPU reports a
/// submission of 256 slabs out of memory and takes 128; 32 keeps a margin
/// of four.
const SLABS_PER_SUBMISSION: usize = 32;

/// The cells along the keys the shader's 32-bit cell indices hold, less
/// the two the grid adds past the largest key.
const MOST_KEY_CELLS: f64 = i32::MAX as f64 - 2.0;

/// What each part of the two sums costs, fitted to the RTX 5070's times
/// over photos and HDR photos 171 to 2048 texels tall, in steps of the
/// exact sum's square window, 6.5 ps each there. The exact sum: a step;
/// for each tap of its circle, what the tap adds per radius squared as the
/// window outgrows the texture cache; and a call. The grid: a tap of its
/// blur at a cell, a texel the splat reads for a column of cells, a slab's
/// passes, a call, and the planning that runs before the grid's cost is
/// known, its extent and tiles passes and pipelines.
const LOOP_COST: f64 = 1.0;
const TAP_SPREAD_COST: f64 = 0.008;
const EXACT_CALL_COST: f64 = 2.56e7;
const BLUR_TAP_COST: f64 = 0.621;
const SPLAT_COST: f64 = 26.3;
const SLAB_COST: f64 = 3.21e7;
const GRID_CALL_COST: f64 = 2.30e7;
const PLAN_COST: f64 = 4.63e8;

const SHADER: &str = r"
struct Grid {
    width: u32,
    height: u32,
    edge: u32,
    reach: i32,
    ds: f32,
    dr: f32,
    kmin: f32,
    channel: u32,
    ox: i32,
    oy: i32,
    oz: i32,
    nx: u32,
    ny: u32,
    nz: u32,
    cx0: i32,
    cx1: i32,
    cy0: i32,
    cy1: i32,
    cz0: i32,
    cz1: i32,
    tx0: i32,
    ty0: i32,
    axis: u32,
    radius: u32,
    tile_x: u32,
    tile_y: u32,
    tiles_x: u32,
    unused: u32,
}

@group(0) @binding(0) var values: texture_2d<f32>;
@group(0) @binding(1) var keys: texture_2d<f32>;
@group(0) @binding(2) var<uniform> g: Grid;
@group(0) @binding(3) var<storage, read_write> payload_a: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> mass_a: array<f32>;
@group(0) @binding(5) var<storage, read_write> payload_b: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> mass_b: array<f32>;
@group(0) @binding(7) var<storage, read> weights: array<f32>;
@group(0) @binding(8) var<storage, read_write> ranges: array<atomic<u32>>;
@group(0) @binding(9) var previous: texture_2d<f32>;
@group(0) @binding(10) var written: texture_storage_2d<rgba32float, write>;

// The texel `i` of `n` reads past an edge: the edge texel, the other side,
// or the texture mirrored with the edge texel repeated.
fn edge_index(i: i32, n: i32) -> i32 {
    if (i >= 0 && i < n) {
        return i;
    }
    switch g.edge {
        case 1u: {
            return ((i % n) + n) % n;
        }
        case 2u: {
            let period = 2 * n;
            var m = ((i % period) + period) % period;
            if (m >= n) {
                m = period - 1 - m;
            }
            return m;
        }
        default: {
            return clamp(i, 0, n - 1);
        }
    }
}

fn texel_at(t: texture_2d<f32>, x: i32, y: i32) -> vec4<f32> {
    return textureLoad(t, vec2<i32>(edge_index(x, i32(g.width)), edge_index(y, i32(g.height))), 0);
}

// The key a texel is placed by: its luminance's, or its channel's.
fn key_of(k: vec4<f32>) -> f32 {
    if (g.channel == 4u) {
        return k.x;
    }
    return k[g.channel];
}

// What a texel adds to the grid: the whole texel weighed by alpha, or its
// channel weighed by alpha beside its alpha.
fn carried(v: vec4<f32>) -> vec4<f32> {
    if (g.channel == 4u) {
        return v;
    }
    return vec4<f32>(v[g.channel], v.a, 0.0, 0.0);
}

// A float as a u32 that orders as the float does, for atomic minimums
// and maximums.
fn ordered(v: f32) -> u32 {
    let bits = bitcast<u32>(v);
    return select(bits | 0x80000000u, ~bits, (bits & 0x80000000u) != 0u);
}

fn cell(i: u32, j: u32, l: u32) -> u32 {
    return (l * g.ny + j) * g.nx + i;
}

// A texel's place in the grid, in cells.
fn place(p: vec2<i32>, key: f32) -> vec3<f32> {
    let reach = f32(g.reach);
    return vec3<f32>((f32(p.x) + reach) / g.ds, (f32(p.y) + reach) / g.ds, (key - g.kmin) / g.dr);
}

// The smallest and largest key of each of the four key channels.
@compute @workgroup_size(8, 8)
fn extent(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= g.width || id.y >= g.height) {
        return;
    }
    let k = textureLoad(keys, vec2<i32>(id.xy), 0);
    for (var c = 0u; c < 4u; c++) {
        let o = ordered(k[c]);
        atomicMin(&ranges[2u * c], o);
        atomicMax(&ranges[2u * c + 1u], o);
    }
}

// The smallest and largest key of the texels in each tile's columns.
@compute @workgroup_size(8, 8)
fn tiles(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= g.width || id.y >= g.height) {
        return;
    }
    let p = vec2<i32>(id.xy);
    let key = key_of(textureLoad(keys, p, 0));
    let at = place(p, key);
    let tile = (u32(floor(at.y)) / g.tile_y) * g.tiles_x + u32(floor(at.x)) / g.tile_x;
    let o = ordered(key);
    atomicMin(&ranges[2u * tile], o);
    atomicMax(&ranges[2u * tile + 1u], o);
}

// One column of the tile's cells: every texel within a cell of its
// center, as far as the window reaches past the edges, adds what it
// carries, weighed by the tents across and down, to the two cells of the
// column on either side of its key, split by the tent along the key. A column
// belongs to one invocation, so no two write the same cell.
@compute @workgroup_size(8, 8)
fn splat(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= g.nx || id.y >= g.ny) {
        return;
    }
    let reach = f32(g.reach);
    let cx = f32(g.ox + i32(id.x)) * g.ds - reach;
    let cy = f32(g.oy + i32(id.y)) * g.ds - reach;
    let x0 = max(i32(ceil(cx - g.ds)), -g.reach);
    let x1 = min(i32(floor(cx + g.ds)), i32(g.width) - 1 + g.reach);
    let y0 = max(i32(ceil(cy - g.ds)), -g.reach);
    let y1 = min(i32(floor(cy + g.ds)), i32(g.height) - 1 + g.reach);
    for (var y = y0; y <= y1; y++) {
        let wy = max(0.0, 1.0 - abs(f32(y) - cy) / g.ds);
        for (var x = x0; x <= x1; x++) {
            let w = wy * max(0.0, 1.0 - abs(f32(x) - cx) / g.ds);
            let v = carried(texel_at(values, x, y));
            let z = (key_of(texel_at(keys, x, y)) - g.kmin) / g.dr;
            let below = floor(z);
            let up = z - below;
            let l = i32(below) - g.oz;
            if (l >= 0 && l < i32(g.nz)) {
                let at = cell(id.x, id.y, u32(l));
                payload_a[at] += (w * (1.0 - up)) * v;
                mass_a[at] += w * (1.0 - up);
            }
            if (l + 1 >= 0 && l + 1 < i32(g.nz)) {
                let at = cell(id.x, id.y, u32(l + 1));
                payload_a[at] += (w * up) * v;
                mass_a[at] += w * up;
            }
        }
    }
}

// The cell a blur reads `t` cells along its axis from cell n's, clamped
// into the tile, and whether it is in the tile.
fn along(n: u32, t: i32) -> vec4<i32> {
    var q = vec3<i32>(i32(n % g.nx), i32((n / g.nx) % g.ny), i32(n / (g.nx * g.ny)));
    q[g.axis] += t;
    let inside = q.x >= 0 && q.y >= 0 && q.z >= 0 && q.x < i32(g.nx) && q.y < i32(g.ny) && q.z < i32(g.nz);
    let kept = clamp(q, vec3<i32>(0), vec3<i32>(i32(g.nx) - 1, i32(g.ny) - 1, i32(g.nz) - 1));
    return vec4<i32>(i32(cell(u32(kept.x), u32(kept.y), u32(kept.z))), select(0, 1, inside), 0, 0);
}

// One axis of the Gaussian over the tile's cells, from the first copy into
// the second; a cell past the tile reads as empty.
@compute @workgroup_size(64)
fn forward(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let n = id.x + id.y * groups.x * 64u;
    if (n >= g.nx * g.ny * g.nz) {
        return;
    }
    let r = i32(g.radius);
    var p = vec4<f32>(0.0);
    var m = 0.0;
    for (var t = -r; t <= r; t++) {
        let q = along(n, t);
        let w = select(0.0, weights[u32(t + r)], q.y == 1);
        p += w * payload_a[u32(q.x)];
        m += w * mass_a[u32(q.x)];
    }
    payload_b[n] = p;
    mass_b[n] = m;
}

// One axis of the Gaussian, from the second copy into the first.
@compute @workgroup_size(64)
fn back(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let n = id.x + id.y * groups.x * 64u;
    if (n >= g.nx * g.ny * g.nz) {
        return;
    }
    let r = i32(g.radius);
    var p = vec4<f32>(0.0);
    var m = 0.0;
    for (var t = -r; t <= r; t++) {
        let q = along(n, t);
        let w = select(0.0, weights[u32(t + r)], q.y == 1);
        p += w * payload_b[u32(q.x)];
        m += w * mass_b[u32(q.x)];
    }
    payload_a[n] = p;
    mass_a[n] = m;
}

// The texels whose cells are in the tile's core: the blurred cells around
// each, interpolated across, down and along its key, the carried sum over
// the weight. By luminance, the texel weighed by alpha; by channel, the
// channel's color written into the results so far, or for alpha the
// texel, its colors weighed by it.
@compute @workgroup_size(8, 8)
fn slice(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(g.tx0 + i32(id.x), g.ty0 + i32(id.y));
    if (p.x >= i32(g.width) || p.y >= i32(g.height)) {
        return;
    }
    let at = place(p, key_of(textureLoad(keys, p, 0)));
    let base = vec3<i32>(floor(at));
    if (base.x < g.cx0 || base.x >= g.cx1 || base.y < g.cy0 || base.y >= g.cy1 || base.z < g.cz0 || base.z >= g.cz1) {
        return;
    }
    let f = at - vec3<f32>(base);
    let local = vec3<u32>(base - vec3<i32>(g.ox, g.oy, g.oz));
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var corner = 0u; corner < 8u; corner++) {
        let step = vec3<u32>(corner & 1u, (corner >> 1u) & 1u, (corner >> 2u) & 1u);
        let share = select(1.0 - f.x, f.x, step.x == 1u) * select(1.0 - f.y, f.y, step.y == 1u) * select(1.0 - f.z, f.z, step.z == 1u);
        let q = local + step;
        let n = cell(q.x, q.y, q.z);
        sum += share * payload_b[n];
        weight += share * mass_b[n];
    }
    let own = textureLoad(values, p, 0);
    let before = textureLoad(previous, p, 0);
    if (g.channel == 4u) {
        textureStore(written, p, select(own, sum / weight, weight > 0.0));
        return;
    }
    if (g.channel == 3u) {
        let a = select(own.a, sum.x / weight, weight > 0.0);
        textureStore(written, p, vec4<f32>(before.rgb * a, a));
        return;
    }
    var after = before;
    after[g.channel] = select(0.0, sum.x / sum.y, sum.y > 0.0);
    textureStore(written, p, after);
}
";

/// The grid's shared values, laid out as the shader's Grid struct.
#[derive(Clone, Copy, Default)]
struct Shared {
    width: u32,
    height: u32,
    edge: u32,
    reach: i32,
    ds: f32,
    dr: f32,
    kmin: f32,
    channel: u32,
    origin: [i32; 3],
    size: [u32; 3],
    core: [[i32; 2]; 3],
    first: [i32; 2],
    axis: u32,
    radius: u32,
    tile: [u32; 2],
    tiles_x: u32,
}

impl Shared {
    fn bytes(&self) -> Vec<u8> {
        let i = |v: i32| v as u32;
        let words = [
            self.width,
            self.height,
            self.edge,
            i(self.reach),
            self.ds.to_bits(),
            self.dr.to_bits(),
            self.kmin.to_bits(),
            self.channel,
            i(self.origin[0]),
            i(self.origin[1]),
            i(self.origin[2]),
            self.size[0],
            self.size[1],
            self.size[2],
            i(self.core[0][0]),
            i(self.core[0][1]),
            i(self.core[1][0]),
            i(self.core[1][1]),
            i(self.core[2][0]),
            i(self.core[2][1]),
            i(self.first[0]),
            i(self.first[1]),
            self.axis,
            self.radius,
            self.tile[0],
            self.tile[1],
            self.tiles_x,
            0,
        ];
        bytemuck::cast_slice(&words).to_vec()
    }
}

/// The layout and pipelines of the grid's passes.
struct Kit {
    layout: wgpu::BindGroupLayout,
    extent: wgpu::ComputePipeline,
    tiles: wgpu::ComputePipeline,
    splat: wgpu::ComputePipeline,
    forward: wgpu::ComputePipeline,
    back: wgpu::ComputePipeline,
    slice: wgpu::ComputePipeline,
}

impl Kit {
    fn new(device: &wgpu::Device) -> Kit {
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let sampled = |binding: u32| {
            entry(binding, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false })
        };
        let storage = |binding: u32, read_only: bool| entry(binding, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wgpu bilateral grid"),
            entries: &[
                sampled(0),
                sampled(1),
                entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(SHARED_BYTES) }),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, true),
                storage(8, false),
                sampled(9),
                entry(10, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: F::Rgba32Float, view_dimension: wgpu::TextureViewDimension::D2 }),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("Wgpu bilateral grid"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu bilateral grid"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Wgpu bilateral grid"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Kit { extent: compute("extent"), tiles: compute("tiles"), splat: compute("splat"), forward: compute("forward"), back: compute("back"), slice: compute("slice"), layout }
    }
}

/// The two copies of a tile's cells and the blur's weights.
struct Store {
    payload: [wgpu::Buffer; 2],
    mass: [wgpu::Buffer; 2],
    weights: wgpu::Buffer,
}

/// A float from the order-preserving u32 the shader keeps it as.
fn unordered(o: u32) -> f32 {
    if o & 0x8000_0000 != 0 { f32::from_bits(o & 0x7FFF_FFFF) } else { f32::from_bits(!o) }
}

/// Tiles of `cells` with a halo of `halo` cells each side whose two copies
/// fit `cap` cells: square tiles across and down with every key where
/// that fits, or else slabs of keys too under tiles as wide as leave a
/// slab of at least one core cell. The core of a tile in cells, or `None`
/// where not even one cell with its halo fits.
fn plan(cells: [u32; 3], halo: u32, cap: u64) -> Option<[u32; 3]> {
    let span = |core: u32, axis: usize| u64::from(core.saturating_add(2 * halo).min(cells[axis]));
    let whole = u64::from(cells[2]);
    let side = ((cap / whole.max(1)) as f64).sqrt() as u64;
    let t = side.saturating_sub(2 * u64::from(halo)).min(u64::from(cells[0].max(cells[1]))) as u32;
    if t >= 1 && span(t, 0) * span(t, 1) * whole <= cap {
        return Some([t.min(cells[0]), t.min(cells[1]), cells[2]]);
    }
    for t in (1..=WIDEST_SLAB_TILE).rev() {
        let across = span(t, 0) * span(t, 1);
        let fit = cap / across;
        if fit > 2 * u64::from(halo) {
            let tz = (fit - 2 * u64::from(halo)).min(whole) as u32;
            return Some([t.min(cells[0]), t.min(cells[1]), tz]);
        }
    }
    None
}

/// The range a core of `core` cells reads with its halo, in a grid of
/// `cells` along that axis.
fn halo_range(core: [u32; 2], halo: u32, cells: u32) -> [u32; 2] {
    [core[0].saturating_sub(halo), core[1].saturating_add(halo).min(cells)]
}

/// What the exact sum costs a texture of `width` by `height` texels with
/// a window of `radius`, in the units the grid's costs count.
pub(super) fn exact_cost(width: u32, height: u32, radius: u32) -> f64 {
    let r = i64::from(radius);
    let taps: i64 = (-r..=r).map(|dy| 2 * ((r * r - dy * dy) as f64).sqrt().floor() as i64 + 1).sum();
    let side = (2 * r + 1) as f64;
    f64::from(width) * f64::from(height) * (side * side * LOOP_COST + taps as f64 * (r * r) as f64 * TAP_SPREAD_COST) + EXACT_CALL_COST
}

/// One slab of a tile: its core of cells across, down and along the keys,
/// its cells with the halo, and the texels its slice reads.
struct Slab {
    core: [[u32; 2]; 3],
    extent: [[u32; 2]; 3],
    size: [u32; 3],
    count: u64,
    texels: [[u32; 2]; 2],
}

impl Slab {
    /// The slab of `core` in a grid of `cells`, with a halo of `halo` cells
    /// each side, its cells `ds` texels apart and `reach` cells past the
    /// texture's edges, over a texture of `width` by `height` texels.
    fn new(core: [[u32; 2]; 3], halo: u32, cells: [u32; 3], ds: f32, reach: u32, width: u32, height: u32) -> Slab {
        let extent = [0, 1, 2].map(|axis| halo_range(core[axis], halo, cells[axis]));
        let size = extent.map(|e| e[1] - e[0]);
        let to_texel = |c: u32| f64::from(c) * f64::from(ds) - f64::from(reach);
        let texels = |range: [u32; 2], n: u32| {
            let first = (to_texel(range[0]).floor() - 1.0).max(0.0) as u32;
            let last = ((to_texel(range[1]).ceil() + 1.0).max(0.0) as u32).min(n);
            [first.min(n), last]
        };
        let count = size.iter().map(|&n| u64::from(n)).product();
        Slab { core, extent, size, count, texels: [texels(core[0], width), texels(core[1], height)] }
    }

    /// The Grid values of this slab, for passes along no axis in particular.
    fn shared(&self, base: &Shared, radius: u32) -> Shared {
        let corner = |e: [u32; 2]| e[0] as i32;
        let core = |c: [u32; 2]| [c[0] as i32, c[1] as i32];
        Shared {
            origin: self.extent.map(corner),
            size: self.size,
            core: self.core.map(core),
            first: [self.texels[0][0] as i32, self.texels[1][0] as i32],
            radius,
            ..*base
        }
    }

    /// What the slab's passes cost, in the units exact_cost counts: the
    /// three blurs over its cells, the splat of the texels each column
    /// reads, and the passes themselves.
    fn cost(&self, radius: u32, ds: f32) -> f64 {
        let columns = f64::from(self.size[0]) * f64::from(self.size[1]);
        let span = (2.0 * f64::from(ds)).max(1.0);
        self.count as f64 * f64::from(3 * (2 * radius + 1)) * BLUR_TAP_COST + columns * span * span * SPLAT_COST + SLAB_COST
    }
}

/// One key channel's grid planned: the values its passes share and its
/// slabs.
struct Run {
    shared: Shared,
    slabs: Vec<Slab>,
}

/// Sums the bilateral filter `b` of work texture `from`, whose keys are in
/// work texture `keys`, through the bilateral grid of cells `spacing`
/// standard deviations apart, into work texture `to`, and returns true.
/// `params` carries the texture's size and the edge. Given `rival`, what
/// the exact sum costs, the grid is planned and costed first and runs only
/// where it costs less; otherwise false is returned with nothing written,
/// as it is for a grid too large to hold, which without a rival is refused.
#[allow(clippy::too_many_arguments)]
pub(super) fn grid(work: &Work, params: &Params, from: usize, keys: usize, to: usize, b: Bilateral, spacing: f64, rival: Option<f64>, stop: Option<&dyn Interrupt>) -> PsResult<bool> {
    let core = work.core;
    let (width, height) = (params.width, params.height);
    let ds = (spacing * b.sigma) as f32;
    let dr = (spacing * b.range) as f32;
    let reach = super::window_radius(b.sigma);
    let spread = (1.0 / (spacing * spacing) - 1.0 / 3.0).max(0.0).sqrt();
    let weights = if spread > 0.0 { gaussian_weights(spread) } else { vec![1.0] };
    let radius = (weights.len() / 2) as u32;
    let halo = radius + 1;
    let channels: Vec<u32> = match b.by {
        SmoothBy::Luminance => vec![LUMINANCE],
        SmoothBy::Channels => vec![0, 1, 2, 3],
    };
    // Against a rival, the least the grid could cost, planning included:
    // each key channel's columns of cells over the texture, each with two
    // cells along the keys at the fewest, and a slab.
    if let Some(exact) = rival {
        let columns = (f64::from(width) / f64::from(ds)) * (f64::from(height) / f64::from(ds));
        let span = (2.0 * f64::from(ds)).max(1.0);
        let least = channels.len() as f64 * (columns * (2.0 * f64::from(3 * (2 * radius + 1)) * BLUR_TAP_COST + span * span * SPLAT_COST) + SLAB_COST) + GRID_CALL_COST + PLAN_COST;
        if exact < least {
            return Ok(false);
        }
    }
    let kit = core.scoped("WgpuFilter", "the texture cannot be filtered", || Kit::new(&core.device))?;
    let device_cap = core.limits.max_storage_buffer_binding_size.min(core.limits.max_buffer_size) / 16;
    let budget_cap = (work.grid_bytes / (2 * CELL_BYTES)).min(device_cap);
    let buffer = |size: u64, usage: wgpu::BufferUsages, contents: Option<&[u8]>| -> PsResult<wgpu::Buffer> {
        core.scoped("WgpuFilter", "the texture cannot be filtered", || match contents {
            Some(contents) => core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu bilateral grid"), contents, usage }),
            None => core.device.create_buffer(&wgpu::BufferDescriptor { label: Some("Wgpu bilateral grid"), size, usage, mapped_at_creation: false }),
        })
    };
    let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
    let views = &work.views;
    // A texture the results by channel pass between, beside `to`.
    let second = match b.by {
        SmoothBy::Luminance => None,
        SmoothBy::Channels => Some(core.scoped("WgpuFilter", "the texture cannot be filtered", || {
            core.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("Wgpu bilateral grid"),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: F::Rgba32Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        })?),
    };
    let base = Shared { width, height, edge: params.edge, reach: reach as i32, ds, dr, ..Shared::default() };
    // The bytes from one Grid value to the next in a uniform buffer, at the
    // device's alignment for a dynamic offset.
    let slot = SHARED_BYTES.next_multiple_of(u64::from(core.limits.min_uniform_buffer_offset_alignment));
    // A uniform buffer of `values`, each in a slot of its own.
    let uniforms = |values: &[Shared]| -> PsResult<wgpu::Buffer> {
        let mut bytes = vec![0u8; values.len() * slot as usize];
        for (i, value) in values.iter().enumerate() {
            let at = i * slot as usize;
            bytes[at..at + SHARED_BYTES as usize].copy_from_slice(&value.bytes());
        }
        buffer(0, wgpu::BufferUsages::UNIFORM, Some(&bytes))
    };
    let group = |uniform: &wgpu::Buffer, store: &Store, ranges: &wgpu::Buffer, previous: &wgpu::TextureView, written: &wgpu::TextureView| -> PsResult<wgpu::BindGroup> {
        core.scoped("WgpuFilter", "the texture cannot be filtered", || {
            core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wgpu bilateral grid"),
                layout: &kit.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[from]) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[keys]) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: uniform, offset: 0, size: wgpu::BufferSize::new(SHARED_BYTES) }) },
                    wgpu::BindGroupEntry { binding: 3, resource: store.payload[0].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: store.mass[0].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: store.payload[1].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: store.mass[1].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: store.weights.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: ranges.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(previous) },
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(written) },
                ],
            })
        })
    };
    // A pass of `pipeline` reading the Grid value in slot `at` of the bind
    // group's uniform buffer.
    let pass = |encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, group: &wgpu::BindGroup, at: u64, groups: [u32; 2]| {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Wgpu bilateral grid"), timestamp_writes: None });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[(at * slot) as u32]);
        pass.dispatch_workgroups(groups[0].max(1), groups[1].max(1), 1);
    };
    let most_groups = core.limits.max_compute_workgroups_per_dimension;
    let flat_groups = |cells: u64| -> [u32; 2] {
        let groups = cells.div_ceil(64);
        let across = groups.min(u64::from(most_groups));
        [across as u32, groups.div_ceil(across.max(1)) as u32]
    };
    // A placeholder store for the passes that touch no cells.
    let empty = Store { payload: [buffer(16, storage, None)?, buffer(16, storage, None)?], mass: [buffer(4, storage, None)?, buffer(4, storage, None)?], weights: buffer(4, wgpu::BufferUsages::STORAGE, Some(bytemuck::cast_slice(&[1.0f32])))? };
    // The smallest and largest key of each key channel.
    let start: Vec<u32> = (0..8).map(|i| if i % 2 == 0 { u32::MAX } else { 0 }).collect();
    let ranges = buffer(0, storage, Some(bytemuck::cast_slice(&start)))?;
    let extent = uniforms(&[base])?;
    let bound = group(&extent, &empty, &ranges, &views[from], &views[to])?;
    work.submit(stop, |encoder| pass(encoder, &kit.extent, &bound, 0, [width.div_ceil(8), height.div_ceil(8)]))?;
    let extremes: Vec<u32> = bytemuck::pod_collect_to_vec(&buffer::read_bytes(core, &ranges, 0, 32, stop)?);
    // Each key channel's grid planned and costed before any runs.
    let mut runs = Vec::with_capacity(channels.len());
    let mut cost = GRID_CALL_COST;
    for &channel in &channels {
        let index = if channel == LUMINANCE { 0 } else { channel as usize };
        let (kmin, kmax) = (unordered(extremes[2 * index]), unordered(extremes[2 * index + 1]));
        let along = (f64::from(kmax) - f64::from(kmin)) / f64::from(dr);
        if !along.is_finite() || along >= MOST_KEY_CELLS {
            if rival.is_some() {
                return Ok(false);
            }
            return Err(if along.is_finite() {
                invalid(
                    "WgpuGridSpacing",
                    format!(
                        "{} {} at -GridSpacing {spacing} is too fine for the bilateral grid over values from {lowest} to {highest}: it would need more cells along the values than the shader's 32-bit cell indices reach; a wider range or spacing works, or -GridSpacing 0 for the exact sum",
                        b.range_parameter,
                        b.range,
                        lowest = kmin,
                        highest = kmax
                    ),
                )
            } else {
                invalid(
                    "WgpuGridSpacing",
                    format!("the bilateral grid places values from {lowest} to {highest}, and cannot place one that is not finite; -GridSpacing 0 sums exactly", lowest = kmin, highest = kmax),
                )
            });
        }
        let reach_cells = |n: u32| ((f64::from(n) - 1.0 + 2.0 * f64::from(reach)) / f64::from(ds)).floor() as u32 + 2;
        let cells = [reach_cells(width), reach_cells(height), along.floor() as u32 + 2];
        let tile = match plan(cells, halo, budget_cap).or_else(|| plan(cells, halo, device_cap)) {
            Some(tile) => tile,
            None if rival.is_some() => return Ok(false),
            None => {
                return Err(invalid(
                    "WgpuGridSpacing",
                    format!("-GridSpacing {spacing} is too fine for {}: one tile of the bilateral grid with its halo needs more cells than its largest buffer holds; a wider spacing works, or 0 for the exact sum", core.info.name),
                ));
            }
        };
        let tiles_x = cells[0].div_ceil(tile[0]);
        let tiles_y = cells[1].div_ceil(tile[1]);
        let shared = Shared { kmin, channel, tile: [tile[0], tile[1]], tiles_x, ..base };
        // The keys each tile's texels reach.
        let count = tiles_x as usize * tiles_y as usize;
        let start: Vec<u32> = (0..2 * count).map(|i| if i % 2 == 0 { u32::MAX } else { 0 }).collect();
        let tile_ranges = buffer(0, storage, Some(bytemuck::cast_slice(&start)))?;
        let tiled = uniforms(&[shared])?;
        let bound = group(&tiled, &empty, &tile_ranges, &views[from], &views[to])?;
        work.submit(stop, |encoder| pass(encoder, &kit.tiles, &bound, 0, [width.div_ceil(8), height.div_ceil(8)]))?;
        let found: Vec<u32> = bytemuck::pod_collect_to_vec(&buffer::read_bytes(core, &tile_ranges, 0, (8 * count) as u64, stop)?);
        // A tile with a texel in its core is worked over the cells its
        // texels' keys reach and one more each side, as the GPU's division
        // may round a key into the next cell, in slabs of tile[2] cells.
        let cell_of = |o: u32| ((f64::from(unordered(o)) - f64::from(kmin)) / f64::from(dr)).floor().clamp(0.0, f64::from(cells[2] - 1)) as u32;
        let mut slabs = Vec::new();
        for by in 0..tiles_y {
            for bx in 0..tiles_x {
                let at = by as usize * tiles_x as usize + bx as usize;
                let (low, high) = (found[2 * at], found[2 * at + 1]);
                if low > high {
                    continue;
                }
                let core_x = [bx * tile[0], ((bx + 1) * tile[0]).min(cells[0])];
                let core_y = [by * tile[1], ((by + 1) * tile[1]).min(cells[1])];
                let last = (cell_of(high) + 2).min(cells[2]);
                let mut z = cell_of(low).saturating_sub(1);
                while z < last {
                    let end = z.saturating_add(tile[2]).min(last);
                    slabs.push(Slab::new([core_x, core_y, [z, end]], halo, cells, ds, reach, width, height));
                    z = end;
                }
            }
        }
        cost += slabs.iter().map(|slab| slab.cost(radius, ds)).sum::<f64>();
        runs.push(Run { shared, slabs });
    }
    if rival.is_some_and(|exact| exact < cost) {
        return Ok(false);
    }
    for (run, planned) in runs.iter().enumerate() {
        let most = planned.slabs.iter().map(|slab| slab.count).max().unwrap_or(1);
        let store = Store {
            payload: [buffer(most * 16, storage, None)?, buffer(most * 16, storage, None)?],
            mass: [buffer(most * 4, storage, None)?, buffer(most * 4, storage, None)?],
            weights: buffer(0, wgpu::BufferUsages::STORAGE, Some(bytemuck::cast_slice(&weights)))?,
        };
        // By channel, the results pass between `to` and the second texture,
        // ending in `to`.
        let (previous, written) = match &second {
            Some(other) if run % 2 == 0 => (&views[to], other),
            Some(other) => (other, &views[to]),
            None => (&views[from], &views[to]),
        };
        for batch in planned.slabs.chunks(SLABS_PER_SUBMISSION) {
            let values: Vec<Shared> = batch
                .iter()
                .flat_map(|slab| {
                    let at = slab.shared(&planned.shared, radius);
                    [at, Shared { axis: 0, ..at }, Shared { axis: 1, ..at }, Shared { axis: 2, ..at }]
                })
                .collect();
            let uniform = uniforms(&values)?;
            let bound = group(&uniform, &store, &ranges, previous, written)?;
            work.submit_and_wait(stop, |encoder| {
                for (i, slab) in batch.iter().enumerate() {
                    let at = 4 * i as u64;
                    encoder.clear_buffer(&store.payload[0], 0, Some(slab.count * 16));
                    encoder.clear_buffer(&store.mass[0], 0, Some(slab.count * 4));
                    pass(encoder, &kit.splat, &bound, at, [slab.size[0].div_ceil(8), slab.size[1].div_ceil(8)]);
                    pass(encoder, &kit.forward, &bound, at + 1, flat_groups(slab.count));
                    pass(encoder, &kit.back, &bound, at + 2, flat_groups(slab.count));
                    pass(encoder, &kit.forward, &bound, at + 3, flat_groups(slab.count));
                    let [[sx, ex], [sy, ey]] = slab.texels;
                    if ex > sx && ey > sy {
                        pass(encoder, &kit.slice, &bound, at, [(ex - sx).div_ceil(8), (ey - sy).div_ceil(8)]);
                    }
                }
            })?;
        }
    }
    Ok(true)
}
