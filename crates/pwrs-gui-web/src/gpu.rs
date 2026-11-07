//! What the HTML renderer stores for its pages' GpuSurfaces: the copies of
//! the egui renderer's table objects the module sends, each with a version
//! that changes when the module sends it again, a buffer's or texture's
//! bytes loaded by pages from `pwrs://localhost/gpu/<kind>/<number>`; and a
//! surface's frame and camera as a page takes them, in JSON, with WebGPU's
//! names.

use crate::proto::camera::{CameraButton, CameraGesture, CameraMove, CameraSpec};
use crate::proto::gpu::{
    AddressMode, Blend, BlendFactor, BlendOperation, CompareFunction, CustomBlend, Cull, DepthFormat, IndexFormat, PageBinding, PageDraw, PagePass, PageResource, PageSurface, PageVertex, PassEvery,
    PipelineSpec, SamplerFilter, StencilOperation, TableObject, TextureFormat, Topology,
};
use arc_swap::ArcSwap;
use serde_json::{Value as Json, json};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use wry::http::header::{CONTENT_TYPE, HeaderValue};
use wry::http::{Response, StatusCode};

/// What a copy has besides its bytes: a buffer's size, a texture's size
/// and format, a sampler's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Info {
    Buffer { length: u64 },
    Texture { width: u32, height: u32, format: TextureFormat },
    Sampler { filter: SamplerFilter, address: AddressMode },
}

/// A copy of a table object: its bytes, none for a sampler, what else it
/// has, and the version the module last sent.
#[derive(Clone, Debug)]
pub(crate) struct Kept {
    pub(crate) version: u64,
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) info: Info,
}

/// The copies, by the name a page requests each by under `gpu/`. The
/// scheme's handler reads it without a lock on whichever thread the engine
/// calls it on; the loop's thread replaces it as copies come and go.
pub(crate) type Copies = ArcSwap<HashMap<String, Arc<Kept>>>;

/// The number the next copy's version takes, so a page never takes bytes it
/// fetched before for new ones.
static VERSIONS: AtomicU64 = AtomicU64::new(1);

/// The name a page loads `object`'s copy by under `gpu/`.
pub(crate) fn name_of(object: TableObject) -> String {
    match object {
        TableObject::Buffer(id) => format!("buffer/{id}"),
        TableObject::Texture(id) => format!("texture/{id}"),
        TableObject::Sampler(id) => format!("sampler/{id}"),
    }
}

fn replace(copies: &Copies, name: &str, kept: Kept) {
    let kept = Arc::new(kept);
    copies.rcu(|t| {
        let mut t = HashMap::clone(t);
        t.insert(name.to_string(), Arc::clone(&kept));
        t
    });
}

/// Stores `bytes` from byte `offset` of buffer `buffer`, which has `length`
/// bytes, over the stored copy of it, or over zeros when none of that
/// length is stored; returns the copy's new version.
pub(crate) fn keep_buffer(copies: &Copies, buffer: u64, length: u64, offset: u64, bytes: &[u8]) -> Result<u64, String> {
    let name = name_of(TableObject::Buffer(buffer));
    let size = usize::try_from(length).map_err(|e| format!("buffer {buffer}'s {length} bytes are more than this process can address: {e}"))?;
    let start = usize::try_from(offset).map_err(|e| format!("byte {offset} of buffer {buffer} is past what this process can address: {e}"))?;
    let end = start.checked_add(bytes.len()).filter(|end| *end <= size).ok_or_else(|| format!("{} bytes from byte {offset} are outside buffer {buffer}, which has {length} bytes", bytes.len()))?;
    let mut whole = match copies.load().get(&name) {
        Some(k) if k.info == (Info::Buffer { length }) => k.bytes.to_vec(),
        _ => vec![0; size],
    };
    whole[start..end].copy_from_slice(bytes);
    let version = VERSIONS.fetch_add(1, Ordering::Relaxed);
    replace(copies, &name, Kept { version, bytes: whole.into(), info: Info::Buffer { length } });
    Ok(version)
}

/// Stores every texel of texture `texture`; returns the copy's version.
pub(crate) fn keep_texture(copies: &Copies, texture: u64, width: u32, height: u32, format: TextureFormat, texels: Vec<u8>) -> u64 {
    let version = VERSIONS.fetch_add(1, Ordering::Relaxed);
    replace(copies, &name_of(TableObject::Texture(texture)), Kept { version, bytes: texels.into(), info: Info::Texture { width, height, format } });
    version
}

/// Stores sampler `sampler`'s settings; returns the copy's version.
pub(crate) fn keep_sampler(copies: &Copies, sampler: u64, filter: SamplerFilter, address: AddressMode) -> u64 {
    let version = VERSIONS.fetch_add(1, Ordering::Relaxed);
    replace(copies, &name_of(TableObject::Sampler(sampler)), Kept { version, bytes: Arc::from(Vec::new()), info: Info::Sampler { filter, address } });
    version
}

/// Lets the copy of `object` go.
pub(crate) fn forget(copies: &Copies, object: TableObject) {
    let name = name_of(object);
    copies.rcu(|t| {
        let mut t = HashMap::clone(t);
        t.remove(&name);
        t
    });
}

/// The bytes of the copy stored at `name` under `gpu/`, or not found.
pub(crate) fn serve(copies: &Copies, name: &str) -> Response<Cow<'static, [u8]>> {
    match copies.load().get(name) {
        Some(k) => {
            let mut response = Response::new(Cow::Owned(k.bytes.to_vec()));
            response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
            response
        }
        None => {
            let mut response = Response::new(Cow::Owned(format!("the HTML renderer has no copy of {name}").into_bytes()));
            *response.status_mut() = StatusCode::NOT_FOUND;
            response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
            response
        }
    }
}

/// The WebGPU name of a texture format.
fn format_name(format: TextureFormat) -> &'static str {
    match format {
        TextureFormat::R8Unorm => "r8unorm",
        TextureFormat::Rg8Unorm => "rg8unorm",
        TextureFormat::Rgba8Unorm => "rgba8unorm",
        TextureFormat::Rgba8UnormSrgb => "rgba8unorm-srgb",
        TextureFormat::Bgra8Unorm => "bgra8unorm",
        TextureFormat::Bgra8UnormSrgb => "bgra8unorm-srgb",
        TextureFormat::R16Float => "r16float",
        TextureFormat::Rg16Float => "rg16float",
        TextureFormat::Rgba16Float => "rgba16float",
        TextureFormat::R32Float => "r32float",
        TextureFormat::Rg32Float => "rg32float",
        TextureFormat::Rgba32Float => "rgba32float",
        TextureFormat::R32Uint => "r32uint",
        TextureFormat::R32Sint => "r32sint",
    }
}

/// The bytes one texel of `format` takes.
fn texel_bytes(format: TextureFormat) -> u32 {
    match format {
        TextureFormat::R8Unorm => 1,
        TextureFormat::Rg8Unorm | TextureFormat::R16Float => 2,
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb | TextureFormat::Rg16Float | TextureFormat::R32Float | TextureFormat::R32Uint | TextureFormat::R32Sint => 4,
        TextureFormat::Rgba16Float | TextureFormat::Rg32Float => 8,
        TextureFormat::Rgba32Float => 16,
    }
}

/// A resource as a page takes it: a copy's name and version, with what it
/// has, or an array's bytes; a copy not stored is named as missing.
fn resource_json(resource: &PageResource, copies: &HashMap<String, Arc<Kept>>) -> Json {
    let object = match resource {
        PageResource::Bytes(bytes) => return json!({ "bytes": bytes }),
        PageResource::Table(object) => *object,
    };
    let name = name_of(object);
    match copies.get(&name) {
        None => json!({ "missing": name }),
        Some(k) => match k.info {
            Info::Buffer { length } => json!({ "buffer": name, "version": k.version, "length": length }),
            Info::Texture { width, height, format } => json!({ "texture": name, "version": k.version, "width": width, "height": height, "format": format_name(format), "texelBytes": texel_bytes(format) }),
            Info::Sampler { filter, address } => json!({
                "sampler": name,
                "version": k.version,
                "filter": match filter { SamplerFilter::Nearest => "nearest", SamplerFilter::Linear => "linear" },
                "address": match address { AddressMode::ClampToEdge => "clamp-to-edge", AddressMode::Repeat => "repeat", AddressMode::MirrorRepeat => "mirror-repeat" },
            }),
        },
    }
}

fn factor_name(factor: BlendFactor) -> &'static str {
    match factor {
        BlendFactor::Zero => "zero",
        BlendFactor::One => "one",
        BlendFactor::Src => "src",
        BlendFactor::OneMinusSrc => "one-minus-src",
        BlendFactor::SrcAlpha => "src-alpha",
        BlendFactor::OneMinusSrcAlpha => "one-minus-src-alpha",
        BlendFactor::Dst => "dst",
        BlendFactor::OneMinusDst => "one-minus-dst",
        BlendFactor::DstAlpha => "dst-alpha",
        BlendFactor::OneMinusDstAlpha => "one-minus-dst-alpha",
        BlendFactor::SrcAlphaSaturated => "src-alpha-saturated",
        BlendFactor::Constant => "constant",
        BlendFactor::OneMinusConstant => "one-minus-constant",
        BlendFactor::Src1 => "src1",
        BlendFactor::OneMinusSrc1 => "one-minus-src1",
        BlendFactor::Src1Alpha => "src1-alpha",
        BlendFactor::OneMinusSrc1Alpha => "one-minus-src1-alpha",
    }
}

fn operation_name(operation: BlendOperation) -> &'static str {
    match operation {
        BlendOperation::Add => "add",
        BlendOperation::Subtract => "subtract",
        BlendOperation::ReverseSubtract => "reverse-subtract",
        BlendOperation::Min => "min",
        BlendOperation::Max => "max",
    }
}

/// A blend as WebGPU states it, the same the Wgpu module builds for each
/// mode; null for Replace.
fn blend_json(blend: Blend, custom: &CustomBlend) -> Json {
    let component = |s: BlendFactor, d: BlendFactor, o: BlendOperation| json!({ "srcFactor": factor_name(s), "dstFactor": factor_name(d), "operation": operation_name(o) });
    let both = |s: BlendFactor, d: BlendFactor| json!({ "color": component(s, d, BlendOperation::Add), "alpha": component(s, d, BlendOperation::Add) });
    match blend {
        Blend::Replace => Json::Null,
        Blend::Alpha => json!({
            "color": component(BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha, BlendOperation::Add),
            "alpha": component(BlendFactor::One, BlendFactor::OneMinusSrcAlpha, BlendOperation::Add),
        }),
        Blend::Premultiplied => both(BlendFactor::One, BlendFactor::OneMinusSrcAlpha),
        Blend::Additive => both(BlendFactor::One, BlendFactor::One),
        Blend::Constant => both(BlendFactor::Constant, BlendFactor::OneMinusConstant),
        Blend::Custom => {
            let source = custom.source.unwrap_or(BlendFactor::One);
            let destination = custom.destination.unwrap_or(BlendFactor::Zero);
            let operation = custom.operation.unwrap_or(BlendOperation::Add);
            json!({
                "color": component(source, destination, operation),
                "alpha": component(custom.alpha_source.unwrap_or(source), custom.alpha_destination.unwrap_or(destination), custom.alpha_operation.unwrap_or(operation)),
            })
        }
    }
}

/// The WebGPU name of a depth attachment's format.
fn depth_format_name(format: DepthFormat) -> &'static str {
    match format {
        DepthFormat::Stencil8 => "stencil8",
        DepthFormat::Depth16Unorm => "depth16unorm",
        DepthFormat::Depth24Plus => "depth24plus",
        DepthFormat::Depth24PlusStencil8 => "depth24plus-stencil8",
        DepthFormat::Depth32Float => "depth32float",
        DepthFormat::Depth32FloatStencil8 => "depth32float-stencil8",
    }
}

fn compare_name(compare: CompareFunction) -> &'static str {
    match compare {
        CompareFunction::Never => "never",
        CompareFunction::Less => "less",
        CompareFunction::Equal => "equal",
        CompareFunction::LessEqual => "less-equal",
        CompareFunction::Greater => "greater",
        CompareFunction::NotEqual => "not-equal",
        CompareFunction::GreaterEqual => "greater-equal",
        CompareFunction::Always => "always",
    }
}

fn stencil_operation_name(operation: StencilOperation) -> &'static str {
    match operation {
        StencilOperation::Keep => "keep",
        StencilOperation::Zero => "zero",
        StencilOperation::Replace => "replace",
        StencilOperation::Invert => "invert",
        StencilOperation::IncrementClamp => "increment-clamp",
        StencilOperation::DecrementClamp => "decrement-clamp",
        StencilOperation::IncrementWrap => "increment-wrap",
        StencilOperation::DecrementWrap => "decrement-wrap",
    }
}

/// A pipeline's depth attachment as a page takes it: its WebGPU format, its
/// name as the Wgpu module writes it, and whether it holds depth and
/// stencil; null for a pipeline without one.
fn depth_target_json(spec: &PipelineSpec) -> Json {
    match spec.depth_target() {
        Some(f) => json!({ "format": depth_format_name(f), "name": f.name(), "depth": f.has_depth(), "stencil": f.has_stencil() }),
        None => Json::Null,
    }
}

/// A pipeline's depth and stencil state as WebGPU states it, each part the
/// session left absent filled as the Wgpu module fills it, and the depth
/// test's parts only for a format that holds depth; null for a pipeline
/// without a depth attachment.
fn depth_stencil_json(spec: &PipelineSpec) -> Json {
    let Some(format) = spec.depth_target() else {
        return Json::Null;
    };
    let face = |(compare, fail, depth_fail, pass): (CompareFunction, StencilOperation, StencilOperation, StencilOperation)| {
        json!({ "compare": compare_name(compare), "failOp": stencil_operation_name(fail), "depthFailOp": stencil_operation_name(depth_fail), "passOp": stencil_operation_name(pass) })
    };
    let (read, write) = spec.stencil.masks();
    let test = &spec.depth_test;
    let mut state = json!({
        "format": depth_format_name(format),
        "stencilFront": face(spec.stencil.front_face()),
        "stencilBack": face(spec.stencil.back_face()),
        "stencilReadMask": read,
        "stencilWriteMask": write,
        "depthBias": test.bias.unwrap_or(0),
        "depthBiasSlopeScale": test.bias_slope_scale.unwrap_or(0.0),
        "depthBiasClamp": test.bias_clamp.unwrap_or(0.0),
    });
    if format.has_depth() {
        state["depthWriteEnabled"] = json!(test.write.unwrap_or(true));
        state["depthCompare"] = json!(compare_name(test.compare.unwrap_or(CompareFunction::Less)));
    }
    state
}

fn spec_json(spec: &PipelineSpec) -> Json {
    let constants: serde_json::Map<String, Json> = spec.constants.iter().map(|(name, value)| (name.clone(), json!(value))).collect();
    json!({
        "source": spec.source,
        "vertexEntry": spec.vertex_entry,
        "fragmentEntry": spec.fragment_entry,
        "topology": match spec.topology {
            Topology::PointList => "point-list",
            Topology::LineList => "line-list",
            Topology::LineStrip => "line-strip",
            Topology::TriangleList => "triangle-list",
            Topology::TriangleStrip => "triangle-strip",
        },
        "cull": match spec.cull { Cull::None => "none", Cull::Front => "front", Cull::Back => "back" },
        "blend": blend_json(spec.blend, &spec.custom_blend),
        "depthTarget": depth_target_json(spec),
        "depthStencil": depth_stencil_json(spec),
        "sampleCount": spec.sample_count,
        "constants": constants,
    })
}

fn vertex_json(v: &PageVertex, copies: &HashMap<String, Arc<Kept>>) -> Json {
    json!({ "location": v.location, "format": v.format, "stride": v.stride, "resource": resource_json(&v.resource, copies) })
}

fn binding_json(b: &PageBinding, copies: &HashMap<String, Arc<Kept>>) -> Json {
    json!({ "group": b.group, "binding": b.binding, "resource": resource_json(&b.resource, copies) })
}

fn draw_json(d: &PageDraw, copies: &HashMap<String, Arc<Kept>>) -> Json {
    json!({
        "spec": spec_json(&d.spec),
        "vertices": d.vertices.iter().map(|v| vertex_json(v, copies)).collect::<Vec<_>>(),
        "bindings": d.bindings.iter().map(|b| binding_json(b, copies)).collect::<Vec<_>>(),
        "frame": d.frame,
        "index": d.index.as_ref().map(|(r, f)| json!({ "resource": resource_json(r, copies), "format": match f { IndexFormat::Uint16 => "uint16", IndexFormat::Uint32 => "uint32" } })),
        "vertexCount": d.vertex_count,
        "instanceCount": d.instance_count,
        "blendConstant": d.blend_constant,
        "stencilReference": d.stencil_reference,
    })
}

fn pass_json(p: &PagePass, copies: &HashMap<String, Arc<Kept>>) -> Json {
    let constants: serde_json::Map<String, Json> = p.constants.iter().map(|(name, value)| (name.clone(), json!(value))).collect();
    json!({
        "source": p.source,
        "entry": p.entry,
        "constants": constants,
        "bindings": p.bindings.iter().map(|b| binding_json(b, copies)).collect::<Vec<_>>(),
        "frame": p.frame,
        "workgroups": p.workgroups,
        "indirect": p.indirect.map(|(buffer, offset)| json!({ "resource": resource_json(&PageResource::Table(TableObject::Buffer(buffer)), copies), "offset": offset })),
        "once": p.every == PassEvery::Once,
    })
}

/// `surface` as a page takes it, each copy it names with the version stored.
pub(crate) fn surface_json(surface: &PageSurface, copies: &Copies) -> Json {
    let kept = copies.load();
    json!({
        "clear": surface.clear,
        "clearDepth": surface.clear_depth,
        "clearStencil": surface.clear_stencil,
        "draws": surface.draws.iter().map(|d| draw_json(d, &kept)).collect::<Vec<_>>(),
        "passes": surface.passes.iter().map(|p| pass_json(p, &kept)).collect::<Vec<_>>(),
        "uniform": surface.uniform,
    })
}

/// A camera as a page takes it: its pose and projection, and its controls
/// as the gestures they bind, each with the move it makes, in the order
/// they are matched.
pub(crate) fn camera_json(spec: &CameraSpec) -> Json {
    let bindings: Vec<Json> = spec
        .controls
        .bindings()
        .iter()
        .map(|(gesture, made)| {
            let (kind, button, keys) = match gesture {
                CameraGesture::Drag { button, modifiers } => {
                    let named = match button {
                        CameraButton::Left => "Left",
                        CameraButton::Right => "Right",
                        CameraButton::Middle => "Middle",
                    };
                    ("drag", Some(named), modifiers)
                }
                CameraGesture::Wheel { modifiers } => ("wheel", None, modifiers),
            };
            let moves = match made {
                CameraMove::Orbit => "Orbit",
                CameraMove::Pan => "Pan",
                CameraMove::Zoom => "Zoom",
            };
            json!({ "gesture": kind, "button": button, "shift": keys.shift, "ctrl": keys.ctrl, "alt": keys.alt, "move": moves })
        })
        .collect();
    json!({
        "target": spec.target,
        "distance": spec.distance,
        "yaw": spec.yaw,
        "pitch": spec.pitch,
        "fieldOfView": spec.field_of_view,
        "near": spec.near,
        "far": spec.far,
        "bindings": bindings,
    })
}

/// The copies `surface` names that are not stored.
pub(crate) fn missing(surface: &PageSurface, copies: &Copies) -> Vec<String> {
    let kept = copies.load();
    let resources = surface
        .draws
        .iter()
        .flat_map(|d| d.vertices.iter().map(|v| &v.resource).chain(d.bindings.iter().map(|b| &b.resource)).chain(d.index.iter().map(|(r, _)| r)))
        .chain(surface.passes.iter().flat_map(|p| p.bindings.iter().map(|b| &b.resource)));
    let mut out: Vec<String> = resources
        .filter_map(|r| match r {
            PageResource::Table(object) => Some(name_of(*object)),
            PageResource::Bytes(_) => None,
        })
        .chain(surface.passes.iter().filter_map(|p| p.indirect.map(|(buffer, _)| name_of(TableObject::Buffer(buffer)))))
        .filter(|name| !kept.contains_key(name))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::gpu::{DepthTest, StencilFace, StencilTest};

    #[test]
    fn a_buffer_s_copy_takes_writes_over_what_it_kept_with_a_new_version_each_time() {
        let copies = Copies::default();
        let first = keep_buffer(&copies, 3, 8, 0, &[1, 2, 3, 4, 5, 6, 7, 8]).expect("whole");
        let second = keep_buffer(&copies, 3, 8, 4, &[9, 9]).expect("part");
        assert!(second > first, "each send a new version");
        assert_eq!(&*copies.load()["buffer/3"].bytes, &[1, 2, 3, 4, 9, 9, 7, 8]);
        assert!(keep_buffer(&copies, 3, 8, 7, &[1, 2]).is_err(), "bytes past the buffer's end are refused");
        keep_buffer(&copies, 3, 4, 0, &[5]).expect("a new length starts from zeros");
        assert_eq!(&*copies.load()["buffer/3"].bytes, &[5, 0, 0, 0]);
        forget(&copies, TableObject::Buffer(3));
        assert!(copies.load().is_empty());
        assert_eq!(serve(&copies, "buffer/3").status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_page_s_surface_names_each_copy_with_its_version_and_what_it_holds() {
        let copies = Copies::default();
        keep_buffer(&copies, 1, 4, 0, &[0, 0, 128, 63]).expect("stored");
        keep_texture(&copies, 2, 1, 1, TextureFormat::Rgba8UnormSrgb, vec![255, 0, 0, 255]);
        keep_sampler(&copies, 3, SamplerFilter::Linear, AddressMode::MirrorRepeat);
        let spec = PipelineSpec {
            source: String::from("wgsl"),
            vertex_entry: None,
            fragment_entry: None,
            topology: Topology::TriangleStrip,
            cull: Cull::Back,
            blend: Blend::Alpha,
            custom_blend: CustomBlend::default(),
            depth: true,
            depth_format: None,
            depth_test: DepthTest { compare: Some(CompareFunction::Greater), bias: Some(-2), ..DepthTest::default() },
            stencil: StencilTest::default(),
            sample_count: 4,
            constants: vec![(String::from("scale"), 2.0)],
        };
        let table = |object| PageResource::Table(object);
        let surface = PageSurface {
            clear: [0.0, 0.0, 0.0, 1.0],
            clear_depth: Some(0.0),
            clear_stencil: None,
            draws: vec![PageDraw {
                spec,
                vertices: vec![PageVertex { location: 0, format: String::from("float32x2"), stride: 8, resource: PageResource::Bytes(vec![0; 24]) }],
                bindings: vec![
                    PageBinding { group: 0, binding: 0, resource: table(TableObject::Buffer(1)) },
                    PageBinding { group: 0, binding: 1, resource: table(TableObject::Texture(2)) },
                    PageBinding { group: 0, binding: 2, resource: table(TableObject::Sampler(3)) },
                    PageBinding { group: 0, binding: 3, resource: table(TableObject::Texture(9)) },
                ],
                frame: true,
                index: None,
                vertex_count: 3,
                instance_count: 1,
                blend_constant: None,
                stencil_reference: None,
            }],
            passes: vec![PagePass {
                source: String::from("wgsl"),
                entry: String::from("main"),
                constants: Vec::new(),
                bindings: vec![PageBinding { group: 0, binding: 0, resource: table(TableObject::Buffer(1)) }],
                frame: false,
                workgroups: [0, 0, 0],
                indirect: Some((1, 4)),
                every: PassEvery::Once,
            }],
            uniform: Vec::new(),
        };
        let json = surface_json(&surface, &copies);
        assert_eq!((json["clearDepth"].as_f64(), json["clearStencil"].is_null()), (Some(0.0), true), "the clears as the session gave them, for the page to check");
        let draw = &json["draws"][0];
        assert_eq!(draw["spec"]["depthTarget"], json!({ "format": "depth32float", "name": "Depth32Float", "depth": true, "stencil": false }), "-Depth alone draws in Depth32Float");
        let state = &draw["spec"]["depthStencil"];
        assert_eq!((state["format"].as_str(), state["depthCompare"].as_str(), state["depthWriteEnabled"].as_bool()), (Some("depth32float"), Some("greater"), Some(true)));
        assert_eq!((state["depthBias"].as_i64(), state["depthBiasSlopeScale"].as_f64(), state["depthBiasClamp"].as_f64()), (Some(-2), Some(0.0), Some(0.0)));
        assert_eq!(state["stencilFront"], json!({ "compare": "always", "failOp": "keep", "depthFailOp": "keep", "passOp": "keep" }), "a stencil test left as the Wgpu module leaves it");
        assert_eq!(state["stencilReadMask"].as_u64(), Some(u64::from(u32::MAX)));
        assert!(draw["stencilReference"].is_null());
        assert_eq!(draw["spec"]["topology"], "triangle-strip");
        assert_eq!(draw["spec"]["blend"]["color"]["srcFactor"], "src-alpha");
        assert_eq!(draw["spec"]["blend"]["alpha"]["srcFactor"], "one");
        assert_eq!(draw["spec"]["constants"]["scale"], 2.0);
        assert_eq!(draw["frame"], true, "the draw says it reads the frame uniform, which the page binds");
        let pass = &json["passes"][0];
        assert_eq!((pass["frame"].as_bool(), pass["once"].as_bool()), (Some(false), Some(true)));
        assert_eq!(pass["indirect"]["resource"]["buffer"], "buffer/1", "an indirect dispatch's buffer is a copy like any other");
        assert_eq!((pass["indirect"]["resource"]["length"].as_u64(), pass["indirect"]["offset"].as_u64()), (Some(4), Some(4)));
        assert_eq!(draw["bindings"][0]["resource"]["length"], 4);
        assert_eq!(draw["bindings"][1]["resource"]["format"], "rgba8unorm-srgb");
        assert_eq!(draw["bindings"][2]["resource"]["address"], "mirror-repeat");
        assert_eq!(draw["bindings"][3]["resource"]["missing"], "texture/9");
        assert_eq!(draw["vertices"][0]["resource"]["bytes"].as_array().map(Vec::len), Some(24));
        assert_eq!(missing(&surface, &copies), vec![String::from("texture/9")]);
        assert_eq!(blend_json(Blend::Replace, &CustomBlend::default()), Json::Null);
        let custom = CustomBlend { destination: Some(BlendFactor::One), alpha_operation: Some(BlendOperation::Max), ..CustomBlend::default() };
        let state = blend_json(Blend::Custom, &custom);
        assert_eq!((state["color"]["srcFactor"].as_str(), state["color"]["dstFactor"].as_str(), state["alpha"]["operation"].as_str()), (Some("one"), Some("one"), Some("max")));
    }

    #[test]
    fn a_stencil_only_attachment_states_no_depth_test_and_each_back_face_part_left_out_is_the_front_s() {
        let stencil = StencilTest {
            front: StencilFace { compare: Some(CompareFunction::Equal), pass: Some(StencilOperation::Replace), ..StencilFace::default() },
            back: StencilFace { fail: Some(StencilOperation::Zero), ..StencilFace::default() },
            read_mask: Some(0x0f),
            write_mask: Some(0xf0),
        };
        let spec = PipelineSpec {
            source: String::from("wgsl"),
            vertex_entry: None,
            fragment_entry: None,
            topology: Topology::TriangleList,
            cull: Cull::None,
            blend: Blend::Replace,
            custom_blend: CustomBlend::default(),
            depth: false,
            depth_format: Some(DepthFormat::Stencil8),
            depth_test: DepthTest::default(),
            stencil,
            sample_count: 1,
            constants: Vec::new(),
        };
        let state = depth_stencil_json(&spec);
        assert!(state.get("depthCompare").is_none() && state.get("depthWriteEnabled").is_none(), "a format without depth states no depth test: {state}");
        assert_eq!(state["stencilBack"], json!({ "compare": "equal", "failOp": "zero", "depthFailOp": "keep", "passOp": "replace" }));
        assert_eq!((state["stencilReadMask"].as_u64(), state["stencilWriteMask"].as_u64()), (Some(15), Some(240)));
        assert_eq!(depth_target_json(&spec), json!({ "format": "stencil8", "name": "Stencil8", "depth": false, "stencil": true }));
        let none = PipelineSpec { depth_format: None, stencil: StencilTest::default(), ..spec };
        assert_eq!((depth_stencil_json(&none), depth_target_json(&none)), (Json::Null, Json::Null), "no depth attachment");
    }

    #[test]
    fn a_page_s_camera_carries_its_pose_and_the_gestures_its_controls_bind_in_order() {
        use crate::proto::camera::{CameraControls, CameraModifiers};
        let blender = CameraSpec { yaw: 30.0, controls: CameraControls::Blender, ..CameraSpec::default() };
        let json = camera_json(&blender);
        assert_eq!((json["distance"].as_f64(), json["yaw"].as_f64(), json["fieldOfView"].as_f64()), (Some(5.0), Some(30.0), Some(50.0)));
        let bindings = json["bindings"].as_array().expect("bindings");
        assert_eq!(bindings.len(), 4);
        assert_eq!((bindings[0]["gesture"].as_str(), bindings[0]["button"].as_str(), bindings[0]["move"].as_str()), (Some("drag"), Some("Middle"), Some("Orbit")));
        assert_eq!((bindings[1]["shift"].as_bool(), bindings[1]["move"].as_str()), (Some(true), Some("Pan")));
        assert_eq!((bindings[3]["gesture"].as_str(), bindings[3]["button"].is_null()), (Some("wheel"), true));
        let custom = CameraSpec { controls: CameraControls::Custom(vec![(CameraGesture::Wheel { modifiers: CameraModifiers::CTRL }, CameraMove::Orbit)]), ..CameraSpec::default() };
        let one = &camera_json(&custom)["bindings"][0];
        assert_eq!((one["gesture"].as_str(), one["ctrl"].as_bool(), one["move"].as_str()), (Some("wheel"), Some(true), Some("Orbit")));
        assert_eq!(camera_json(&CameraSpec { controls: CameraControls::Fixed, ..CameraSpec::default() })["bindings"].as_array().map(Vec::len), Some(0));
    }
}
