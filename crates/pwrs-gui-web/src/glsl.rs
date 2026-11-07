//! A GpuSurface draw's WGSL as a page with WebGL 2 alone runs it: its
//! vertex and fragment entry points each written as GLSL ES 3.00 by naga,
//! as wgpu's GL backend has them written, the output's Y flipped and its
//! depth widened to -1 to 1 so the page draws upside down into a texture
//! it shows flipped, with the names the page binds each uniform block and
//! sampler under, by the group and binding the WGSL gives them.

use naga::back::glsl::{Options, PipelineOptions, ReflectionInfo, Version, Writer, WriterFlags};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{AddressSpace, Handle, ImageClass, Module, ShaderStage, TypeInner};
use serde_json::{Value as Json, json};

/// Why a draw cannot be written for WebGL 2.
#[derive(Debug, PartialEq)]
pub(crate) enum Unwritable {
    /// It uses what WebGL 2 has no place for, named.
    NeedsWebGpu(String),
    /// naga could not read or write it, for the reason given.
    Refused(String),
}

/// The entry point of `stage` named `name` in `module`, or its only one of
/// that stage when `name` is `None`: its place and its name.
fn entry(module: &Module, stage: ShaderStage, name: Option<&str>) -> Result<(usize, String), Unwritable> {
    let of_stage: Vec<(usize, &naga::EntryPoint)> = module.entry_points.iter().enumerate().filter(|(_, e)| e.stage == stage).collect();
    let found = match name {
        Some(n) => of_stage.iter().find(|(_, e)| e.name == n),
        None if of_stage.len() == 1 => of_stage.first(),
        None => None,
    };
    found.map(|(at, e)| (*at, e.name.clone())).ok_or_else(|| {
        let names = of_stage.iter().map(|(_, e)| e.name.as_str()).collect::<Vec<_>>().join(", ");
        Unwritable::Refused(match name {
            Some(n) => format!("the shader has no {stage:?} entry point {n}; it has {names}"),
            None => format!("the shader has {} {stage:?} entry points, {names}, and the draw names none", of_stage.len()),
        })
    })
}

/// The first global variable the entry point at `at` uses that WebGL 2 has
/// no place for, a storage buffer or a storage texture, said with its
/// name, group and binding.
fn needs_webgpu(module: &Module, info: &naga::valid::ModuleInfo, at: usize) -> Option<String> {
    let uses = info.get_entry_point(at);
    module.global_variables.iter().filter(|(h, _)| !uses[*h].is_empty()).find_map(|(_, v)| {
        let kind = match (&v.space, &module.types[v.ty].inner) {
            (AddressSpace::Storage { .. }, _) => "storage buffer",
            (AddressSpace::Handle, TypeInner::Image { class: ImageClass::Storage { .. }, .. }) => "storage texture",
            _ => return None,
        };
        let name = match &v.name {
            Some(n) => n.as_str(),
            None => "a binding",
        };
        let place = v.binding.as_ref().map_or_else(String::new, |b| format!(" at {}:{}", b.group, b.binding));
        Some(format!("the shader uses {name}{place} as a {kind}, which WebGL 2 has none of"))
    })
}

/// One stage written as GLSL with what it binds: each uniform block's name
/// and each sampler's, with the group and binding of the texture it
/// samples and of its sampler, which a texel load has none of.
fn stage_json(module: &Module, glsl: String, reflection: &ReflectionInfo) -> Json {
    let at = |h: Handle<naga::GlobalVariable>| module.global_variables[h].binding.as_ref().map(|b| json!([b.group, b.binding]));
    let blocks: Vec<Json> = reflection.uniforms.iter().filter_map(|(h, name)| at(*h).map(|place| json!({ "name": name, "at": place }))).collect();
    let samplers: Vec<Json> = reflection.texture_mapping.iter().map(|(name, m)| json!({ "name": name, "texture": at(m.texture), "sampler": m.sampler.and_then(at) })).collect();
    json!({ "glsl": glsl, "blocks": blocks, "samplers": samplers })
}

/// The entry point `name` of `stage` written as GLSL ES 3.00 for WebGL,
/// its override constants set from `constants`.
fn write_stage(module: &Module, info: &naga::valid::ModuleInfo, stage: ShaderStage, name: &str, constants: &naga::back::PipelineConstants) -> Result<Json, Unwritable> {
    let (module, info) = naga::back::pipeline_constants::process_overrides(module, info, Some((stage, name)), constants).map_err(|e| Unwritable::Refused(format!("the {stage:?} stage's constants could not be set: {e}")))?;
    let options = Options {
        version: Version::Embedded { version: 300, is_webgl: true },
        writer_flags: WriterFlags::ADJUST_COORDINATE_SPACE | WriterFlags::FORCE_POINT_SIZE,
        binding_map: Default::default(),
        zero_initialize_workgroup_memory: false,
    };
    let pipeline = PipelineOptions { shader_stage: stage, entry_point: name.to_string(), multiview: None };
    let mut glsl = String::new();
    let mut writer = Writer::new(&mut glsl, &module, &info, &options, &pipeline, naga::proc::BoundsCheckPolicies::default()).map_err(|e| Unwritable::Refused(format!("the {stage:?} stage cannot be written as GLSL ES 3.00: {e}")))?;
    let reflection = writer.write().map_err(|e| Unwritable::Refused(format!("the {stage:?} stage cannot be written as GLSL ES 3.00: {e}")))?;
    Ok(stage_json(&module, glsl, &reflection))
}

/// The draw whose WGSL `source` has vertex entry point `vertex` and
/// fragment entry point `fragment` (each the shader's only one of its stage
/// when `None`), with override `constants` by name or `@id`, as a page with
/// WebGL 2 alone runs it: `{ vertex, fragment }`, each a stage as
/// [`stage_json`] gives it.
pub(crate) fn translate(source: &str, vertex: Option<&str>, fragment: Option<&str>, constants: &[(String, f64)]) -> Result<Json, Unwritable> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| Unwritable::Refused(e.emit_to_string(source)))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::all()).validate(&module).map_err(|e| Unwritable::Refused(e.emit_to_string(source)))?;
    let (vertex_at, vertex_name) = entry(&module, ShaderStage::Vertex, vertex)?;
    let (fragment_at, fragment_name) = entry(&module, ShaderStage::Fragment, fragment)?;
    if let Some(what) = needs_webgpu(&module, &info, vertex_at).or_else(|| needs_webgpu(&module, &info, fragment_at)) {
        return Err(Unwritable::NeedsWebGpu(what));
    }
    let constants: naga::back::PipelineConstants = constants.iter().cloned().collect();
    Ok(json!({
        "vertex": write_stage(&module, &info, ShaderStage::Vertex, &vertex_name, &constants)?,
        "fragment": write_stage(&module, &info, ShaderStage::Fragment, &fragment_name, &constants)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINTED: &str = "struct Tint { color: vec4<f32> }
        @group(0) @binding(0) var<uniform> tint: Tint;
        @group(0) @binding(1) var picture: texture_2d<f32>;
        @group(0) @binding(2) var look: sampler;
        override scale: f32 = 1.0;
        @vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position * scale, 0.0, 1.0); }
        @fragment fn fs() -> @location(0) vec4<f32> { return tint.color * textureSample(picture, look, vec2<f32>(0.5)); }";

    #[test]
    fn a_draw_is_written_for_webgl_with_its_uniform_blocks_and_samplers_by_binding() {
        let written = translate(TINTED, None, None, &[(String::from("scale"), 2.0)]).expect("written");
        let vertex = written["vertex"]["glsl"].as_str().expect("vertex GLSL");
        assert!(vertex.starts_with("#version 300 es"), "{vertex}");
        assert!(vertex.contains("gl_PointSize"), "points take a size of 1, which WebGL needs written");
        assert!(vertex.contains("2.0"), "the override is set: {vertex}");
        let fragment = &written["fragment"];
        let blocks = fragment["blocks"].as_array().expect("blocks");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["at"], json!([0, 0]));
        let samplers = fragment["samplers"].as_array().expect("samplers");
        assert_eq!(samplers.len(), 1);
        assert_eq!((samplers[0]["texture"].clone(), samplers[0]["sampler"].clone()), (json!([0, 1]), json!([0, 2])));
        let name = samplers[0]["name"].as_str().expect("a name");
        assert!(fragment["glsl"].as_str().is_some_and(|g| g.contains(name)), "the sampler's name is the one the GLSL declares");
    }

    #[test]
    fn storage_webgl_has_no_place_for_is_refused_and_named() {
        let stored = "@group(0) @binding(3) var<storage, read> tint: array<f32, 4>;
            @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
            @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(tint[0]); }";
        match translate(stored, None, None, &[]) {
            Err(Unwritable::NeedsWebGpu(what)) => assert!(what.contains("tint at 0:3 as a storage buffer"), "{what}"),
            other => panic!("a storage buffer needs WebGPU: {other:?}"),
        }
        assert!(matches!(translate("@vertex fn vs( {", None, None, &[]), Err(Unwritable::Refused(_))));
        assert!(matches!(translate(TINTED, Some("missing"), None, &[]), Err(Unwritable::Refused(text)) if text.contains("no Vertex entry point missing")));
    }
}
