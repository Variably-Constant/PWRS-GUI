//! Running compute shaders: one call from arrays to arrays, expressions
//! over arrays, and reductions.

use std::sync::Arc;
use std::time::Instant;

use pwrs::prelude::*;

use crate::binding::{self, Bound, Resource};
use crate::buffer::{self, Buffer};
use crate::device::{core_for, CoreRef, Device, Pending};
use crate::element::{self, ElementType};
use crate::error::{device_error, invalid};
use crate::pipeline::{self, ComputeInner, ComputePipeline};
use crate::shader::{ShaderLanguage, ShaderStage};
use crate::tasks::Interrupt;

/// How many workgroups a run dispatches.
#[derive(Clone, Default)]
pub(crate) enum Dispatch {
    /// Workgroups in x, y and z.
    Workgroups([u32; 3]),
    /// Invocations in x, y and z, divided by the workgroup size and rounded up.
    Invocations([u64; 3]),
    /// Counts read from a buffer on the GPU at this byte offset.
    Indirect(wgpu::Buffer, u64),
    /// One invocation per element of the first buffer bound, in x.
    #[default]
    Auto,
}

/// One run of a compute shader.
#[derive(Default)]
pub(crate) struct Request {
    /// WGSL or GLSL text, a Wgpu.Shader, or a Wgpu.ComputePipeline.
    pub(crate) shader: PsObject,
    pub(crate) resources: Option<PsObject>,
    pub(crate) entry: Option<String>,
    pub(crate) constants: Option<PsHashtable>,
    pub(crate) language: Option<ShaderLanguage>,
    pub(crate) dispatch: Dispatch,
    pub(crate) immediates: Option<PsObject>,
    pub(crate) timing: bool,
}

/// What a run did: the buffers it may have written that were made from
/// arrays, the workgroups it dispatched, and how long it took.
pub(crate) struct Outcome {
    pub(crate) written: Vec<(String, wgpu::Buffer, ElementType, u64)>,
    pub(crate) workgroups: [u32; 3],
    pub(crate) gpu_ns: Option<f64>,
    pub(crate) cpu_ms: f64,
    pub(crate) core: CoreRef,
}

fn pipeline_for(core: &CoreRef, request: &Request) -> PsResult<Arc<ComputeInner>> {
    if request.shader.type_name()? == "Wgpu.ComputePipeline" {
        return PsProxy::<ComputePipeline>::from_ps(&request.shader)?.with(|p| p.inner.clone());
    }
    let parsed = pipeline::parsed_of(&request.shader, request.language, Some(ShaderStage::Compute))?;
    pipeline::build(core, &parsed, request.entry.as_deref(), &pipeline::constants_of(request.constants.as_ref())?, None)
}

/// Runs a compute shader on `core` once and waits for it.
pub(crate) fn run(core: &CoreRef, request: &Request, stop: Option<&dyn Interrupt>) -> PsResult<Outcome> {
    submit(core, request)?.finish(stop)
}

/// Binds, records and submits one run of a compute shader on `core`,
/// without waiting for the GPU.
pub(crate) fn submit(core: &CoreRef, request: &Request) -> PsResult<Submitted> {
    let inner = pipeline_for(core, request)?;
    submit_pipeline(&inner, request)
}

fn ceil_div(n: u64, d: u64) -> u64 {
    n.div_ceil(d.max(1))
}

fn workgroups(inner: &ComputeInner, dispatch: &Dispatch, bound: &[Bound]) -> PsResult<Option<[u32; 3]>> {
    let limit = inner.core.limits.max_compute_workgroups_per_dimension as u64;
    let fit = |n: u64, axis: &str| -> PsResult<u32> {
        if n > limit {
            Err(invalid("WgpuTooManyWorkgroups", format!("{n} workgroups in {axis} is more than the device's {limit} per dimension; spread them over y and z")))
        } else {
            Ok(n as u32)
        }
    };
    let size = inner.workgroup_size;
    Ok(match dispatch {
        Dispatch::Workgroups(w) => Some(*w),
        Dispatch::Invocations(n) => Some([
            fit(ceil_div(n[0], size[0] as u64), "x")?,
            fit(ceil_div(n[1], size[1] as u64), "y")?,
            fit(ceil_div(n[2], size[2] as u64), "z")?,
        ]),
        Dispatch::Indirect(..) => None,
        Dispatch::Auto => {
            let first = bound.iter().find_map(|b| match &b.resource {
                Resource::Buffer { buffer, offset, size: range } => {
                    let bytes = match range {
                        Some(r) => r.get(),
                        None => buffer.size() - offset,
                    };
                    let stride = if b.binding.stride > 0 { b.binding.stride } else { b.binding.element_type.map_or(4, ElementType::size) };
                    Some(match b.uploaded {
                        Some((_, length)) if b.binding.stride == 0 || b.binding.stride == b.binding.element_type.map_or(0, ElementType::size) => length,
                        _ => bytes.saturating_sub(b.binding.size) / stride,
                    })
                }
                Resource::View(_) | Resource::Sampler(_) => None,
            });
            let Some(invocations) = first else {
                return Err(invalid("WgpuNoDispatchSize", "the shader binds no buffer to count invocations from; give -Workgroups or -Invocations"));
            };
            Some([fit(ceil_div(invocations, size[0] as u64), "x")?, 1, 1])
        }
    })
}

/// Runs a built pipeline once and waits for it.
pub(crate) fn run_pipeline(inner: &Arc<ComputeInner>, request: &Request, stop: Option<&dyn Interrupt>) -> PsResult<Outcome> {
    submit_pipeline(inner, request)?.finish(stop)
}

/// A run handed to the device's submission thread, which the GPU may
/// still be running.
pub(crate) struct Submitted {
    written: Vec<(String, wgpu::Buffer, ElementType, u64)>,
    workgroups: [u32; 3],
    queries: Option<(wgpu::QuerySet, wgpu::Buffer)>,
    started: Instant,
    pending: Pending,
    core: CoreRef,
}

impl Submitted {
    /// Waits for the run's submission and then for the run, and reads how
    /// long the GPU spent on it.
    pub(crate) fn finish(self, stop: Option<&dyn Interrupt>) -> PsResult<Outcome> {
        let core = &self.core;
        let index = self.pending.wait(core, stop)?;
        core.wait_for(&index, stop)?;
        let gpu_ns = match &self.queries {
            Some((_, resolve)) => {
                let bytes = buffer::read_bytes(core, resolve, 0, 16, stop)?;
                let ticks: Vec<u64> = bytemuck::pod_collect_to_vec(&bytes);
                Some(ticks[1].wrapping_sub(ticks[0]) as f64 * core.queue.get_timestamp_period() as f64)
            }
            None => None,
        };
        Ok(Outcome { written: self.written, workgroups: self.workgroups, gpu_ns, cpu_ms: self.started.elapsed().as_secs_f64() * 1000.0, core: self.core })
    }
}

/// Binds, records and submits one run of a built pipeline, without
/// waiting for the GPU.
pub(crate) fn submit_pipeline(inner: &Arc<ComputeInner>, request: &Request) -> PsResult<Submitted> {
    let started = Instant::now();
    let bound = binding::resolve_all(&inner.core, &inner.bindings, request.resources.as_ref())?;
    let immediates = match &request.immediates {
        Some(data) => Some(element::with_host_bytes(data, None, |_, b| Ok(b.to_vec()))?),
        None => None,
    };
    submit_bound(inner, bound, &request.dispatch, immediates, request.timing, started)
}

/// The bind groups, workgroup counts and immediate data of one dispatch of
/// a built pipeline over resources matched to its bindings: `counts` is
/// `None` for an indirect dispatch, and the immediate data is padded with
/// zeros to whole words.
pub(crate) struct Prepared {
    pub(crate) groups: Vec<(u32, wgpu::BindGroup)>,
    pub(crate) counts: Option<[u32; 3]>,
    pub(crate) immediates: Option<Vec<u8>>,
}

/// What one dispatch of `inner` over `bound` sets on a pass, refusing
/// immediate data longer than the entry point reads.
pub(crate) fn prepare(inner: &ComputeInner, bound: &[Bound], dispatch: &Dispatch, immediates: Option<Vec<u8>>) -> PsResult<Prepared> {
    let groups = binding::make_groups(&inner.core, &inner.groups, bound, &inner.label)?;
    let counts = workgroups(inner, dispatch, bound)?;
    let immediates = match immediates {
        Some(bytes) => {
            if bytes.len() as u32 > inner.immediate_size {
                return Err(invalid("WgpuImmediates", format!("{} bytes of immediate data were given and the entry point reads {}", bytes.len(), inner.immediate_size)));
            }
            let mut padded = bytes;
            padded.resize(padded.len().next_multiple_of(4), 0);
            Some(padded)
        }
        None => None,
    };
    Ok(Prepared { groups, counts, immediates })
}

/// Sets `inner`'s pipeline and `prepared`'s bind groups and immediate data
/// on `pass`, and dispatches.
pub(crate) fn record_dispatch(inner: &ComputeInner, pass: &mut wgpu::ComputePass<'_>, prepared: &Prepared, dispatch: &Dispatch) {
    pass.set_pipeline(&inner.pipeline);
    for (index, group) in &prepared.groups {
        pass.set_bind_group(*index, group, &[]);
    }
    if let Some(bytes) = &prepared.immediates {
        pass.set_immediates(0, bytes);
    }
    match (dispatch, prepared.counts) {
        (Dispatch::Indirect(buffer, offset), _) => pass.dispatch_workgroups_indirect(buffer, *offset),
        (_, Some([x, y, z])) => pass.dispatch_workgroups(x, y, z),
        (_, None) => pass.dispatch_workgroups(0, 0, 0),
    }
}

/// Records and submits one run of a built pipeline over resources already
/// matched to its bindings, without waiting for the GPU.
pub(crate) fn submit_bound(inner: &Arc<ComputeInner>, bound: Vec<Bound>, dispatch: &Dispatch, immediates: Option<Vec<u8>>, timing: bool, started: Instant) -> PsResult<Submitted> {
    let core = &inner.core;
    let prepared = prepare(inner, &bound, dispatch, immediates)?;
    let timing = timing && core.features.contains(wgpu::Features::TIMESTAMP_QUERY);
    let queries = if timing {
        Some(core.scoped("WgpuQuery", "the timestamp queries cannot be made", || {
            let set = core.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("Wgpu timing"), ty: wgpu::QueryType::Timestamp, count: 2 });
            let resolve = core.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Wgpu timing"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            (set, resolve)
        })?)
    } else {
        None
    };
    let commands = core.scoped("WgpuDispatch", "the dispatch is invalid", || {
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(&inner.label) });
        {
            let timestamp_writes = queries.as_ref().map(|(set, _)| wgpu::ComputePassTimestampWrites { query_set: set, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(&inner.label), timestamp_writes });
            record_dispatch(inner, &mut pass, &prepared, dispatch);
        }
        if let Some((set, resolve)) = &queries {
            encoder.resolve_query_set(set, 0..2, resolve, 0);
        }
        encoder.finish()
    })?;
    let pending = core.begin_submit("WgpuSubmit", "the submission is invalid", vec![commands])?;
    let written = bound
        .into_iter()
        .filter(|b| b.binding.writable())
        .filter_map(|b| match (b.resource, b.uploaded) {
            (Resource::Buffer { buffer, .. }, Some((ty, length))) => Some((b.binding.name.clone(), buffer, ty, length)),
            _ => None,
        })
        .collect();
    Ok(Submitted { written, workgroups: prepared.counts.unwrap_or([0, 0, 0]), queries, started, pending, core: core.clone() })
}

/// What Invoke-WgpuCompute -PassThru writes: every array the shader may
/// have written, by binding name, and how long the run took.
#[psclass(name = "Wgpu.ComputeResult", columns = ["Outputs", "Workgroups", "GpuTimeNs", "CpuTimeMs"])]
#[derive(Default, Clone)]
pub struct ComputeResult {
    /// A hashtable of each array the shader may have written, by binding name.
    pub outputs: PsObject,
    /// The workgroups dispatched in x, y and z; 0 for an indirect dispatch.
    pub workgroups: Vec<u32>,
    /// Nanoseconds the GPU spent in the pass, where the device has timestamp queries and -PassThru asked for them.
    pub gpu_time_ns: Option<f64>,
    /// Milliseconds the whole call took, binding and copying included.
    pub cpu_time_ms: f64,
}

/// Runs a compute shader once over arrays or buffers and writes the
/// arrays it may have written.
///
/// The shader is WGSL or GLSL text, a Wgpu.Shader, or a pipeline from
/// New-WgpuComputePipeline. -Binding gives its resources: an array in the
/// order the shader declares its bindings, a hashtable by binding name or
/// "group:binding", or for one binding the resource itself. An array of
/// numbers is copied to the GPU as the element type the shader declares,
/// and each array bound read_write comes back as an array of that type; a
/// Wgpu.Buffer stays on the GPU. Without -Workgroups or -Invocations the
/// run has one invocation per element of the first buffer.
///
/// # Examples
/// Invoke-WgpuCompute $double ([float[]](1..1024))
/// Invoke-WgpuCompute $add @{ a = $a; b = [float[]](1..10) } -Invocations 10
/// Invoke-WgpuCompute -Shader $pipeline -Binding $buf -Workgroups 64, 64
/// Invoke-WgpuCompute $wgsl $data -PassThru | Select-Object GpuTimeNs
#[cmdlet(verb = "Invoke", noun = "WgpuCompute", alias = ["Invoke-GpuCompute"], output = ["System.Array", "Wgpu.ComputeResult"])]
#[derive(Default)]
pub struct InvokeWgpuCompute {
    /// WGSL or GLSL text, a Wgpu.Shader, or a Wgpu.ComputePipeline.
    #[param(mandatory, position = 0, raw)]
    pub shader: Option<PsObject>,
    /// The resources for the shader's bindings.
    #[param(position = 1, raw, alias = ["Data", "Buffer", "Resource"])]
    pub binding: Option<PsObject>,
    /// Workgroups to dispatch in x, y and z (1 for a dimension left out).
    #[param]
    pub workgroups: Option<Vec<u32>>,
    /// Invocations in x, y and z, divided by the shader's workgroup size and rounded up.
    #[param]
    pub invocations: Option<Vec<u64>>,
    /// Read the workgroup counts from this buffer on the GPU (three uint).
    #[param]
    pub indirect: Option<PsProxy<Buffer>>,
    /// The byte offset of the counts in -Indirect.
    #[param]
    pub indirect_offset: Option<u64>,
    /// The entry point; the shader's only compute entry point when omitted.
    #[param]
    pub entry: Option<String>,
    /// Override constant values, by name or `@id`.
    #[param]
    pub constant: Option<PsHashtable>,
    /// Immediate data, as an array of numbers.
    #[param(raw)]
    pub immediate: Option<PsObject>,
    /// The language of shader text; inferred when omitted.
    #[param]
    pub language: Option<ShaderLanguage>,
    /// Write a Wgpu.ComputeResult with every output and the time taken, measured on the GPU where it can be.
    #[param]
    pub pass_thru: bool,
    /// The device to run on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl InvokeWgpuCompute {
    fn dispatch(&self) -> PsResult<Dispatch> {
        let three = |v: &[u64], what: &str| -> PsResult<[u64; 3]> {
            match v {
                [x] => Ok([*x, 1, 1]),
                [x, y] => Ok([*x, *y, 1]),
                [x, y, z] => Ok([*x, *y, *z]),
                _ => Err(invalid("WgpuDispatchSize", format!("{what} takes one to three counts, x, y and z"))),
            }
        };
        Ok(match (&self.workgroups, &self.invocations, &self.indirect) {
            (Some(w), None, None) => {
                let w: Vec<u64> = w.iter().map(|v| *v as u64).collect();
                let [x, y, z] = three(&w, "-Workgroups")?;
                Dispatch::Workgroups([x as u32, y as u32, z as u32])
            }
            (None, Some(n), None) => Dispatch::Invocations(three(n, "-Invocations")?),
            (None, None, Some(buffer)) => Dispatch::Indirect(buffer.with(|b| b.buffer.clone())?, self.indirect_offset.unwrap_or(0)),
            (None, None, None) => Dispatch::Auto,
            _ => return Err(invalid("WgpuDispatchSize", "give one of -Workgroups, -Invocations and -Indirect")),
        })
    }
}

impl Cmdlet for InvokeWgpuCompute {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shader) = &self.shader else {
            return Err(invalid("WgpuNoShader", "no shader was given"));
        };
        let core = core_for(&self.device, ps)?;
        let request = Request {
            shader: shader.clone(),
            resources: self.binding.clone(),
            entry: self.entry.clone(),
            constants: self.constant.as_ref().map(|t| PsHashtable(t.0.clone())),
            language: self.language,
            dispatch: self.dispatch()?,
            immediates: self.immediate.clone(),
            timing: self.pass_thru,
        };
        let outcome = run(&core, &request, Some(ps))?;
        if self.pass_thru {
            let outputs = PsHashtable::new()?;
            for (name, buffer, ty, length) in &outcome.written {
                let values = buffer::read_new(&outcome.core, buffer, 0, length * ty.size(), *ty, false, Some(ps))?;
                outputs.set(name, values)?;
            }
            return ps.write(ComputeResult { outputs: outputs.0, workgroups: outcome.workgroups.to_vec(), gpu_time_ns: outcome.gpu_ns, cpu_time_ms: outcome.cpu_ms });
        }
        for (_, buffer, ty, length) in &outcome.written {
            let values = buffer::read_new(&outcome.core, buffer, 0, length * ty.size(), *ty, false, Some(ps))?;
            ps.write_object(&values)?;
        }
        Ok(())
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The WGSL scalar for an element type a map or a reduction can use.
fn wgsl_scalar(ty: ElementType, what: &str) -> PsResult<&'static str> {
    ty.wgsl().ok_or_else(|| invalid("WgpuNoWgslType", format!("{what} is {ty:?}, which WGSL has no type for; convert it to Int32, UInt32 or Float32 first")))
}

fn enables(types: &[ElementType]) -> &'static str {
    if types.contains(&ElementType::Float16) { "enable f16;\n" } else { "" }
}

/// A literal WGSL reads as the value of a number.
fn literal(value: &PsObject) -> PsResult<String> {
    let text = match value.type_name()?.as_str() {
        "System.Single" | "System.Double" | "System.Decimal" => {
            let v = f64::from_ps(value)?;
            if v.is_finite() { format!("{v:?}") } else { return Err(invalid("WgpuNotFinite", format!("{v} has no WGSL literal"))) }
        }
        "System.Boolean" => {
            if bool::from_ps(value)? { "true".to_string() } else { "false".to_string() }
        }
        _ => format!("{}", i64::from_ps(value)?),
    };
    Ok(text)
}

/// Runs a WGSL expression over arrays on the GPU, one invocation per
/// element, and writes the results as one array.
///
/// The expression reads `x`, the element of -InputObject, `i`, its index
/// as u32, and each variable -Variable names: an array is read element by
/// element like `x`, a number is a constant. The result has -ElementType,
/// or the type of `x`.
///
/// # Examples
/// Invoke-WgpuMap 'x * x' -InputObject ([float[]](1..10))
/// 1..1000 | Invoke-WgpuMap 'sqrt(f32(x))'
/// Invoke-WgpuMap 'a * x + b' -InputObject $x -Variable @{ a = 2.5; b = $y }
/// Invoke-WgpuMap 'u32(x) % 7u' $x -ElementType UInt32
#[cmdlet(verb = "Invoke", noun = "WgpuMap", alias = ["Invoke-GpuMap"], output = ["System.Array", "Wgpu.Buffer"])]
#[derive(Default)]
pub struct InvokeWgpuMap {
    /// The WGSL expression, in terms of `x`, `i` and the variables.
    #[param(mandatory, position = 0)]
    pub expression: String,
    /// The array `x` runs over, or its elements piped in.
    #[param(position = 1, value_from_pipeline, raw)]
    pub input_object: Option<PsObject>,
    /// More inputs: an array is read element by element, a number is a constant.
    #[param]
    pub variable: Option<PsHashtable>,
    /// The type of the results; the type of `x` when omitted.
    #[param(alias = ["Type"])]
    pub element_type: Option<ElementType>,
    /// Write the results as a Wgpu.Buffer left on the GPU.
    #[param]
    pub as_buffer: bool,
    /// The device to run on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
    collected: Vec<PsObject>,
}

impl Cmdlet for InvokeWgpuMap {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(item) = &self.input_object {
            self.collected.push(item.clone());
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        let x = match self.collected.len() {
            0 => return Err(invalid("WgpuNoData", "give the array x runs over with -InputObject or the pipeline")),
            1 => self.collected[0].clone(),
            _ => {
                let mut bytes = Vec::new();
                let mut ty = None;
                for item in &self.collected {
                    element::with_host_bytes(item, ty, |t, b| {
                        ty = Some(t);
                        bytes.extend_from_slice(b);
                        Ok(())
                    })?;
                }
                element::to_clr_array(ty.unwrap_or(ElementType::Float32), &bytes)?
            }
        };
        let (x_type, x_buffer, n) = element::with_host_bytes(&x, None, |ty, bytes| {
            let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
            Ok((ty, buffer::create_raw(&core, bytes.len() as u64, usage, "x", Some(bytes))?, bytes.len() as u64 / ty.size()))
        })?;
        let out_type = self.element_type.unwrap_or(x_type);
        let mut declarations = String::new();
        let mut lets = String::new();
        let mut buffers = vec![("x".to_string(), x_buffer, n * x_type.size())];
        let mut types = vec![x_type, out_type];
        if let Some(table) = &self.variable {
            for name in table.keys()? {
                if !is_identifier(&name) || name == "x" || name == "i" {
                    return Err(invalid("WgpuVariableName", format!("{name} cannot name a variable; use a WGSL identifier other than x and i")));
                }
                let value = table.get(&name)?;
                let type_name = value.type_name()?;
                if type_name.ends_with("[]") || type_name.contains("List") {
                    let (ty, buffer, length) = element::with_host_bytes(&value, None, |ty, bytes| {
                        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
                        Ok((ty, buffer::create_raw(&core, bytes.len() as u64, usage, &name, Some(bytes))?, bytes.len() as u64 / ty.size()))
                    })?;
                    if length < n {
                        return Err(invalid("WgpuVariableLength", format!("{name} has {length} elements and x has {n}")));
                    }
                    let binding = buffers.len() + 1;
                    declarations.push_str(&format!("@group(0) @binding({binding}) var<storage, read> {name}_in: array<{}>;\n", wgsl_scalar(ty, &name)?));
                    lets.push_str(&format!("    let {name} = {name}_in[i];\n"));
                    types.push(ty);
                    buffers.push((name, buffer, length * ty.size()));
                } else {
                    declarations.push_str(&format!("const {name} = {};\n", literal(&value)?));
                }
            }
        }
        let shader = format!(
            "{enable}@group(0) @binding(0) var<storage, read> x_in: array<{xt}>;\n\
             @group(0) @binding(1) var<storage, read_write> out_values: array<{ot}>;\n\
             {declarations}\
             @compute @workgroup_size(256)\n\
             fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {{\n\
             \x20   let i = gid.x + gid.y * groups.x * 256u;\n\
             \x20   if (i >= arrayLength(&out_values)) {{ return; }}\n\
             \x20   let x = x_in[i];\n\
             {lets}\
             \x20   out_values[i] = {ot}({expression});\n\
             }}\n",
            enable = enables(&types),
            xt = wgsl_scalar(x_type, "x")?,
            ot = wgsl_scalar(out_type, "the result")?,
            expression = self.expression,
        );
        let out_buffer = buffer::create_raw(&core, n * out_type.size(), buffer::general_usage(), "Invoke-WgpuMap", None)?;
        let parsed = crate::shader::compile(crate::shader::Source::Wgsl(shader), "Invoke-WgpuMap".to_string())?;
        let inner = pipeline::build(&core, &parsed, Some("main"), &[], Some("Invoke-WgpuMap".to_string()))?;
        let mut bound = Vec::new();
        for (index, (name, buffer, bytes)) in buffers.iter().enumerate() {
            let binding_index = if index == 0 { 0 } else { index as u32 + 1 };
            if let Some(b) = inner.bindings.iter().find(|b| b.binding == binding_index) {
                bound.push(Bound { binding: b.clone(), resource: binding::exact(buffer.clone(), *bytes), uploaded: None });
            } else if index == 0 {
                return Err(invalid("WgpuMapUnused", format!("the expression does not read {name}")));
            }
        }
        if let Some(b) = inner.bindings.iter().find(|b| b.binding == 1) {
            bound.push(Bound { binding: b.clone(), resource: binding::exact(out_buffer.clone(), n * out_type.size()), uploaded: None });
        }
        let groups = binding::make_groups(&core, &inner.groups, &bound, &inner.label)?;
        let total = n.div_ceil(256);
        let limit = core.limits.max_compute_workgroups_per_dimension as u64;
        let (gx, gy) = if total <= limit { (total, 1) } else { (limit, total.div_ceil(limit)) };
        let commands = core.scoped("WgpuDispatch", "the expression cannot run", || {
            let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Invoke-WgpuMap") });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Invoke-WgpuMap"), timestamp_writes: None });
                pass.set_pipeline(&inner.pipeline);
                for (index, group) in &groups {
                    pass.set_bind_group(*index, group, &[]);
                }
                pass.dispatch_workgroups(gx as u32, gy as u32, 1);
            }
            encoder.finish()
        })?;
        core.submit_and_wait(vec![commands], Some(ps))?;
        if self.as_buffer {
            return ps.write(Buffer::wrap(&core, out_buffer, out_type, n, "Invoke-WgpuMap".to_string()));
        }
        let values = buffer::read_new(&core, &out_buffer, 0, n * out_type.size(), out_type, false, Some(ps))?;
        ps.write_object(&values)
    }
}

/// What Measure-WgpuArray writes.
#[psclass(name = "Wgpu.Measurement", columns = ["Count", "Sum", "Average", "Minimum", "Maximum"])]
#[derive(Default, Clone)]
pub struct Measurement {
    /// The number of elements.
    pub count: u64,
    /// Their sum: for integers, added in at least 64 bits and exact up to 2^53, past which a double skips integers; for floats, added per workgroup in the element's own precision.
    pub sum: f64,
    /// Their mean.
    pub average: f64,
    /// The smallest.
    pub minimum: f64,
    /// The largest.
    pub maximum: f64,
}

/// The reduction kernel for `ty`: each workgroup of 256 sums its strided
/// share and keeps its minimum and maximum, written as four 32-bit words
/// (sum as two words for integers, so it does not overflow) or three
/// floats of the accumulator type.
fn reduction_shader(ty: ElementType) -> PsResult<(String, ElementType, usize)> {
    let t = wgsl_scalar(ty, "the array")?;
    let steps: String = [128u32, 64, 32, 16, 8, 4, 2, 1].iter().map(|w| format!("    if (lid.x < {w}u) {{ combine(lid.x, lid.x + {w}u); }}\n    workgroupBarrier();\n")).collect();
    let head = format!(
        "{enable}@group(0) @binding(0) var<storage, read> data: array<{t}>;\n",
        enable = if ty == ElementType::Float16 { "enable f16;\n" } else { "" }
    );
    let body = |acc: &str, outputs: &str, setup: &str, add: &str| {
        format!(
            "var<workgroup> s_sum: array<{acc}, 256>;\n\
             var<workgroup> s_min: array<{acc}, 256>;\n\
             var<workgroup> s_max: array<{acc}, 256>;\n\
             {outputs}\n\
             fn combine(a: u32, b: u32) {{\n{add}\n    s_min[a] = min(s_min[a], s_min[b]);\n    s_max[a] = max(s_max[a], s_max[b]);\n}}\n\
             @compute @workgroup_size(256)\n\
             fn main(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {{\n\
             \x20   let n = arrayLength(&data);\n\
             \x20   let first = {acc}(data[0]);\n\
             \x20   var sum = {acc}(0);\n\
             \x20   var low = first;\n\
             \x20   var high = first;\n\
             {setup}\
             \x20   s_sum[lid.x] = sum;\n\
             \x20   s_min[lid.x] = low;\n\
             \x20   s_max[lid.x] = high;\n\
             \x20   workgroupBarrier();\n\
             {steps}\
             \x20   if (lid.x == 0u) {{ write_out(wid.x); }}\n\
             }}\n"
        )
    };
    let stride_loop = |accumulate: &str| {
        format!(
            "    var index = wid.x * 256u + lid.x;\n    loop {{\n        if (index >= n) {{ break; }}\n        let v = {accumulate};\n        low = min(low, v);\n        high = max(high, v);\n        index += groups.x * 256u;\n    }}\n"
        )
    };
    Ok(match ty {
        ElementType::Float16 | ElementType::Float32 | ElementType::Float64 => {
            let acc = if ty == ElementType::Float64 { "f64" } else { "f32" };
            let outputs = format!("@group(0) @binding(1) var<storage, read_write> partials: array<{acc}>;\nfn write_out(g: u32) {{ partials[g * 3u] = s_sum[0]; partials[g * 3u + 1u] = s_min[0]; partials[g * 3u + 2u] = s_max[0]; }}");
            let setup = stride_loop(&format!("{acc}(data[index]);\n        sum += v"));
            let shader = head + &body(acc, &outputs, &setup, "    s_sum[a] = s_sum[a] + s_sum[b];");
            (shader, if acc == "f64" { ElementType::Float64 } else { ElementType::Float32 }, 3)
        }
        ElementType::Int32 | ElementType::UInt32 => {
            let to_words = if ty == ElementType::Int32 { "vec2<u32>(bitcast<u32>(v), select(0u, 0xffffffffu, v < 0))" } else { "vec2<u32>(v, 0u)" };
            let outputs = "var<workgroup> s_lo: array<u32, 256>;\nvar<workgroup> s_hi: array<u32, 256>;\n\
                 @group(0) @binding(1) var<storage, read_write> partials: array<u32>;\n\
                 fn add64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> { let lo = a.x + b.x; return vec2<u32>(lo, a.y + b.y + select(0u, 1u, lo < a.x)); }\n\
                 fn write_out(g: u32) { partials[g * 4u] = s_lo[0]; partials[g * 4u + 1u] = s_hi[0]; partials[g * 4u + 2u] = bitcast<u32>(s_min[0]); partials[g * 4u + 3u] = bitcast<u32>(s_max[0]); }";
            let setup = format!(
                "    var wide = vec2<u32>(0u, 0u);\n    var index = wid.x * 256u + lid.x;\n    loop {{\n        if (index >= n) {{ break; }}\n        let v = data[index];\n        wide = add64(wide, {to_words});\n        low = min(low, v);\n        high = max(high, v);\n        index += groups.x * 256u;\n    }}\n    s_lo[lid.x] = wide.x;\n    s_hi[lid.x] = wide.y;\n"
            );
            let add = "    let sum = add64(vec2<u32>(s_lo[a], s_hi[a]), vec2<u32>(s_lo[b], s_hi[b]));\n    s_lo[a] = sum.x;\n    s_hi[a] = sum.y;";
            let shader = head + &body(t, outputs, &setup, add);
            (shader, ty, 4)
        }
        ElementType::Int64 | ElementType::UInt64 => {
            let outputs = format!("@group(0) @binding(1) var<storage, read_write> partials: array<{t}>;\nfn write_out(g: u32) {{ partials[g * 3u] = s_sum[0]; partials[g * 3u + 1u] = s_min[0]; partials[g * 3u + 2u] = s_max[0]; }}");
            let setup = stride_loop("data[index];\n        sum += v");
            let shader = head + &body(t, &outputs, &setup, "    s_sum[a] = s_sum[a] + s_sum[b];");
            (shader, ty, 3)
        }
        other => return Err(invalid("WgpuNoWgslType", format!("Measure-WgpuArray reduces Float16, Float32, Float64, Int32, UInt32, Int64 and UInt64; {other:?} has no WGSL type"))),
    })
}

/// Sums, averages, and finds the minimum and maximum of an array or a
/// buffer on the GPU, like Measure-Object over numbers.
///
/// Int32 and UInt32 elements are added in 64 bits, and Int64 and UInt64
/// elements in 64 bits within each workgroup, which a workgroup whose sum
/// leaves that range overflows. Every workgroup's sum is then added in
/// 128 bits, and Sum is a double, exact up to 2^53. Float sums add each
/// workgroup's elements in the element's own precision and the
/// workgroups' sums in double.
///
/// # Examples
/// Measure-WgpuArray ([float[]](1..1000000))
/// $buffer | Measure-WgpuArray
/// 1..100 | Measure-WgpuArray
#[cmdlet(verb = "Measure", noun = "WgpuArray", alias = ["Measure-GpuArray"], output = ["Wgpu.Measurement"])]
#[derive(Default)]
pub struct MeasureWgpuArray {
    /// The numbers: an array, a Wgpu.Buffer, or numbers piped in.
    #[param(mandatory, position = 0, value_from_pipeline, raw)]
    pub input_object: Option<PsObject>,
    /// The element type to measure the numbers as; their own type, or Float32, when omitted.
    #[param(alias = ["Type"])]
    pub element_type: Option<ElementType>,
    /// The device to run on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
    collected: Vec<PsObject>,
}

impl Cmdlet for MeasureWgpuArray {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(item) = &self.input_object {
            self.collected.push(item.clone());
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut core = core_for(&self.device, ps)?;
        let (ty, data, n) = if self.collected.len() == 1 && self.collected[0].type_name()? == "Wgpu.Buffer" {
            let (owner, raw, ty, length) = PsProxy::<Buffer>::from_ps(&self.collected[0])?.with(|b| (b.core.clone(), b.buffer.clone(), b.element_type, b.length))?;
            core = owner;
            (self.element_type.unwrap_or(ty), raw, length)
        } else {
            let mut bytes = Vec::new();
            let mut ty = self.element_type;
            for item in &self.collected {
                element::with_host_bytes(item, ty, |t, b| {
                    ty = Some(t);
                    bytes.extend_from_slice(b);
                    Ok(())
                })?;
            }
            let ty = ty.unwrap_or(ElementType::Float32);
            let raw = buffer::create_raw(&core, bytes.len() as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, "Measure-WgpuArray", Some(&bytes))?;
            (ty, raw, bytes.len() as u64 / ty.size())
        };
        if n == 0 {
            return Err(invalid("WgpuEmpty", "there are no numbers to measure"));
        }
        let (shader, partial_type, per_group) = reduction_shader(ty)?;
        let parsed = crate::shader::compile(crate::shader::Source::Wgsl(shader), "Measure-WgpuArray".to_string())?;
        let inner = pipeline::build(&core, &parsed, Some("main"), &[], Some("Measure-WgpuArray".to_string()))?;
        let groups_needed = n.div_ceil(256).min(core.limits.max_compute_workgroups_per_dimension as u64);
        let partials = buffer::create_raw(&core, groups_needed * per_group as u64 * partial_type.size(), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, "partials", None)?;
        let bound: Vec<Bound> = inner
            .bindings
            .iter()
            .map(|b| {
                let resource = if b.binding == 0 {
                    binding::exact(data.clone(), n * ty.size())
                } else {
                    binding::exact(partials.clone(), groups_needed * per_group as u64 * partial_type.size())
                };
                Bound { binding: b.clone(), resource, uploaded: None }
            })
            .collect();
        let groups = binding::make_groups(&core, &inner.groups, &bound, &inner.label)?;
        let commands = core.scoped("WgpuDispatch", "the reduction cannot run", || {
            let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Measure-WgpuArray") });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Measure-WgpuArray"), timestamp_writes: None });
                pass.set_pipeline(&inner.pipeline);
                for (index, group) in &groups {
                    pass.set_bind_group(*index, group, &[]);
                }
                pass.dispatch_workgroups(groups_needed as u32, 1, 1);
            }
            encoder.finish()
        })?;
        core.submit_and_wait(vec![commands], Some(ps))?;
        let bytes = buffer::read_bytes(&core, &partials, 0, groups_needed * per_group as u64 * partial_type.size(), Some(ps))?;
        let (sum, low, high) = finish(ty, partial_type, per_group, &bytes)?;
        ps.write(Measurement { count: n, sum, average: sum / n as f64, minimum: low, maximum: high })
    }
}

/// Combines the per-workgroup partials into (sum, minimum, maximum).
fn finish(ty: ElementType, partial_type: ElementType, per_group: usize, bytes: &[u8]) -> PsResult<(f64, f64, f64)> {
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    match (ty, partial_type) {
        (ElementType::Int32 | ElementType::UInt32, _) => {
            let words: Vec<u32> = bytemuck::pod_collect_to_vec(bytes);
            let mut sum: i128 = 0;
            for g in words.chunks_exact(per_group) {
                let wide = (g[1] as u64) << 32 | g[0] as u64;
                sum += if ty == ElementType::Int32 { wide as i64 as i128 } else { wide as i128 };
                let (lo, hi) = if ty == ElementType::Int32 { (g[2] as i32 as f64, g[3] as i32 as f64) } else { (g[2] as f64, g[3] as f64) };
                low = low.min(lo);
                high = high.max(hi);
            }
            Ok((sum as f64, low, high))
        }
        (_, ElementType::Float32) => {
            let values: Vec<f32> = bytemuck::pod_collect_to_vec(bytes);
            let mut sum = 0.0f64;
            for g in values.chunks_exact(per_group) {
                sum += g[0] as f64;
                low = low.min(g[1] as f64);
                high = high.max(g[2] as f64);
            }
            Ok((sum, low, high))
        }
        (_, ElementType::Float64) => {
            let values: Vec<f64> = bytemuck::pod_collect_to_vec(bytes);
            let mut sum = 0.0f64;
            for g in values.chunks_exact(per_group) {
                sum += g[0];
                low = low.min(g[1]);
                high = high.max(g[2]);
            }
            Ok((sum, low, high))
        }
        (_, ElementType::Int64) => {
            let values: Vec<i64> = bytemuck::pod_collect_to_vec(bytes);
            let mut sum: i128 = 0;
            for g in values.chunks_exact(per_group) {
                sum += g[0] as i128;
                low = low.min(g[1] as f64);
                high = high.max(g[2] as f64);
            }
            Ok((sum as f64, low, high))
        }
        (_, ElementType::UInt64) => {
            let values: Vec<u64> = bytemuck::pod_collect_to_vec(bytes);
            let mut sum: u128 = 0;
            for g in values.chunks_exact(per_group) {
                sum += g[0] as u128;
                low = low.min(g[1] as f64);
                high = high.max(g[2] as f64);
            }
            Ok((sum as f64, low, high))
        }
        (_, other) => Err(device_error("WgpuReduction", "the reduction's partials cannot be read", format!("{other:?}"))),
    }
}
