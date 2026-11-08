//! Compute pipelines, with bind group layouts built from what the shader
//! declares.

use std::sync::Arc;

use pwrs::prelude::*;

use crate::binding::{self, BindGroup};
use crate::device::{core_for, CoreRef, Device};
use crate::error::invalid;
use crate::shader::{self, Parsed, Reflected, Shader, ShaderBinding, ShaderLanguage, ShaderStage};
use crate::tasks;

/// One bind group of a pipeline: its index, its layout, and the bindings
/// in it.
pub(crate) struct Group {
    pub(crate) index: u32,
    pub(crate) layout: wgpu::BindGroupLayout,
    pub(crate) bindings: Vec<Reflected>,
}

/// A compute pipeline and what it was built from.
pub(crate) struct ComputeInner {
    pub(crate) core: CoreRef,
    pub(crate) pipeline: wgpu::ComputePipeline,
    pub(crate) groups: Vec<Group>,
    pub(crate) bindings: Vec<Reflected>,
    pub(crate) workgroup_size: [u32; 3],
    pub(crate) immediate_size: u32,
    pub(crate) entry: String,
    pub(crate) label: String,
}

/// The bind group layouts for `bindings`, one per group index from 0 to
/// the highest the bindings use, empty where a group has no binding.
pub(crate) fn layouts(core: &CoreRef, bindings: &[Reflected], visibility: wgpu::ShaderStages, label: &str) -> PsResult<Vec<Group>> {
    let Some(highest) = bindings.iter().map(|b| b.group).max() else {
        return Ok(Vec::new());
    };
    let mut groups = Vec::new();
    for index in 0..=highest {
        let members: Vec<Reflected> = bindings.iter().filter(|b| b.group == index).cloned().collect();
        let entries: Vec<wgpu::BindGroupLayoutEntry> = members
            .iter()
            .map(|b| wgpu::BindGroupLayoutEntry { binding: b.binding, visibility, ty: b.ty, count: b.count })
            .collect();
        let layout = core.scoped("WgpuLayout", "the shader's bindings cannot be laid out", || {
            core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(&format!("{label} group {index}")), entries: &entries })
        })?;
        groups.push(Group { index, layout, bindings: members });
    }
    Ok(groups)
}

/// The override values a hashtable gives, by name or `@id`.
pub(crate) fn constants_of(table: Option<&PsHashtable>) -> PsResult<Vec<(String, f64)>> {
    let mut out = Vec::new();
    if let Some(table) = table {
        for key in table.keys()? {
            let value = f64::from_ps(&table.get(&key)?)?;
            out.push((key, value));
        }
    }
    Ok(out)
}

/// The index of compute entry point `entry` of `parsed`, or of its only
/// compute entry point.
pub(crate) fn compute_entry(parsed: &Parsed, entry: Option<&str>) -> PsResult<usize> {
    let index = shader::entry_index(&parsed.module, entry, naga::ShaderStage::Compute)?;
    let ep = &parsed.module.entry_points[index];
    if ep.stage != naga::ShaderStage::Compute {
        return Err(invalid("WgpuNotCompute", format!("{} is a {:?} entry point, not a compute one", ep.name, ep.stage)));
    }
    Ok(index)
}

/// Builds a compute pipeline for entry point `entry` of `parsed`, or its
/// only compute entry point.
pub(crate) fn build(core: &CoreRef, parsed: &Arc<Parsed>, entry: Option<&str>, constants: &[(String, f64)], label: Option<String>) -> PsResult<Arc<ComputeInner>> {
    let index = compute_entry(parsed, entry)?;
    let ep = &parsed.module.entry_points[index];
    let entry = ep.name.clone();
    let label = match label {
        Some(l) => l,
        None => format!("{} {entry}", parsed.label),
    };
    let bindings = shader::reflect(&parsed.module, &parsed.info, Some(index));
    let immediate_size = shader::immediate_size(&parsed.module, &parsed.info, index);
    let groups = layouts(core, &bindings, wgpu::ShaderStages::COMPUTE, &label)?;
    let module = shader::module_on(core, parsed)?;
    let constants: Vec<(&str, f64)> = constants.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let pipeline = core.scoped("WgpuPipeline", "the compute pipeline cannot be built", || {
        let group_layouts: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(|g| Some(&g.layout)).collect();
        let layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(&label), bind_group_layouts: &group_layouts, immediate_size });
        core.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(&label),
            layout: Some(&layout),
            module: &module,
            entry_point: Some(&entry),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, zero_initialize_workgroup_memory: true },
            cache: None,
        })
    })?;
    Ok(Arc::new(ComputeInner {
        core: core.clone(),
        pipeline,
        groups,
        bindings,
        workgroup_size: ep.workgroup_size,
        immediate_size,
        entry,
        label,
    }))
}

/// The shader a value names: WGSL or GLSL text, or a Wgpu.Shader.
pub(crate) fn parsed_of(shader: &PsObject, language: Option<ShaderLanguage>, stage: Option<ShaderStage>) -> PsResult<Arc<Parsed>> {
    if shader.type_name()? == "Wgpu.Shader" {
        return PsProxy::<Shader>::from_ps(shader)?.with(|s| s.parsed.clone());
    }
    let text = String::from_ps(shader)?;
    shader::compile(shader::source_from_text(text, language, stage, Vec::new())?, "Wgpu shader".to_string())
}

/// A compute pipeline from a shader value on `core`.
pub(crate) fn create(core: &CoreRef, shader: &PsObject, entry: Option<&str>, constants: Option<&PsHashtable>, label: Option<String>) -> PsResult<ComputePipeline> {
    let parsed = parsed_of(shader, None, None)?;
    let inner = build(core, &parsed, entry, &constants_of(constants)?, label)?;
    Ok(ComputePipeline::from_inner(inner))
}

/// A compute pipeline, built for one device: a shader's entry point with
/// its bind group layouts and override values fixed.
#[psclass(name = "Wgpu.ComputePipeline", mode = proxy, columns = ["Label", "EntryPoint", "WorkgroupSize", "Bindings"])]
pub struct ComputePipeline {
    /// The pipeline's label.
    pub label: String,
    /// The entry point it runs.
    pub entry_point: String,
    /// Invocations per workgroup in x, y and z.
    pub workgroup_size: Vec<u32>,
    /// The bindings the entry point uses, in the order resources bind to them.
    pub bindings: Vec<ShaderBinding>,
    /// Bytes of immediate data the entry point reads.
    pub immediate_size: u32,
    /// The adapter of the device it was built on.
    pub adapter: String,
    #[psfield(skip)]
    pub(crate) inner: Arc<ComputeInner>,
}

impl ComputePipeline {
    pub(crate) fn from_inner(inner: Arc<ComputeInner>) -> ComputePipeline {
        ComputePipeline {
            label: inner.label.clone(),
            entry_point: inner.entry.clone(),
            workgroup_size: inner.workgroup_size.to_vec(),
            bindings: inner.bindings.iter().map(Reflected::describe).collect(),
            immediate_size: inner.immediate_size,
            adapter: inner.core.info.name.clone(),
            inner,
        }
    }
}

#[psmethods]
impl ComputePipeline {
    /// Binds `resources` and runs the pipeline over `x` by `y` by `z`
    /// workgroups (`y` and `z` 1 when omitted), waiting for it to finish.
    pub fn dispatch(&self, resources: PsObject, x: u32, y: Option<u32>, z: Option<u32>) -> PsResult<()> {
        let request = crate::compute::Request {
            resources: Some(resources),
            dispatch: crate::compute::Dispatch::Workgroups([x, y.unwrap_or(1), z.unwrap_or(1)]),
            ..Default::default()
        };
        crate::compute::run_pipeline(&self.inner, &request, None).map(|_| ())
    }

    /// Dispatch, returning at once with a task that ends when the GPU has
    /// finished the run. `resources` bind now, on the calling thread.
    pub fn dispatch_async(&self, resources: PsObject, x: u32, y: Option<u32>, z: Option<u32>, task: PsTask<()>) -> PsResult<()> {
        let request = crate::compute::Request {
            resources: Some(resources),
            dispatch: crate::compute::Dispatch::Workgroups([x, y.unwrap_or(1), z.unwrap_or(1)]),
            ..Default::default()
        };
        let submitted = crate::compute::submit_pipeline(&self.inner, &request)?;
        tasks::spawn(task, move |stop| submitted.finish(Some(stop)).map(|_| ()), Ok)
    }

    /// The bind group `group` of this pipeline, binding `resources`: an
    /// array in binding order, or a hashtable by binding name or
    /// "group:binding".
    pub fn bind_group(&self, group: u32, resources: PsObject) -> PsResult<BindGroup> {
        binding::bind_group_for(&self.inner, group, &resources)
    }
}

/// Builds a compute pipeline from a shader for one device.
///
/// Its bind group layouts come from the bindings the entry point uses, so
/// resources bind in the order the shader declares them, or by name.
/// Building it once and running it many times saves compiling the shader
/// on every run.
///
/// # Examples
/// $pipeline = New-WgpuComputePipeline -Shader $wgsl
/// New-WgpuComputePipeline -Shader $shader -Entry sum -Constant @{ scale = 2.0 }
#[cmdlet(verb = "New", noun = "WgpuComputePipeline", alias = ["New-GpuComputePipeline"], output = ["Wgpu.ComputePipeline"])]
#[derive(Default)]
pub struct NewWgpuComputePipeline {
    /// WGSL or GLSL text, or a Wgpu.Shader.
    #[param(mandatory, position = 0, value_from_pipeline, raw)]
    pub shader: Option<PsObject>,
    /// The entry point; the shader's only compute entry point when omitted.
    #[param]
    pub entry: Option<String>,
    /// Override constant values, by name or `@id`.
    #[param]
    pub constant: Option<PsHashtable>,
    /// The language of shader text; inferred when omitted.
    #[param]
    pub language: Option<ShaderLanguage>,
    /// The pipeline's label.
    #[param]
    pub label: Option<String>,
    /// The device to build it on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for NewWgpuComputePipeline {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shader) = &self.shader else {
            return Err(invalid("WgpuNoShader", "no shader was given"));
        };
        let core = core_for(&self.device, ps)?;
        let parsed = parsed_of(shader, self.language, Some(ShaderStage::Compute))?;
        let inner = build(&core, &parsed, self.entry.as_deref(), &constants_of(self.constant.as_ref())?, self.label.clone())?;
        ps.write(ComputePipeline::from_inner(inner))
    }
}
