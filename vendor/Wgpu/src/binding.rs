//! Resources bound to a shader's bindings, and bind groups.

use std::num::NonZeroU64;
use std::sync::Arc;

use pwrs::prelude::*;

use crate::buffer::{self, Buffer, BufferSlice};
use crate::device::CoreRef;
use crate::element::{self, ElementType};
use crate::error::{invalid, not_found};
use crate::pipeline::{ComputeInner, ComputePipeline, Group};
use crate::shader::{BindingKind, Member, Reflected};

/// A resource ready to bind.
#[derive(Clone)]
pub(crate) enum Resource {
    Buffer { buffer: wgpu::Buffer, offset: u64, size: Option<NonZeroU64> },
    View(wgpu::TextureView),
    Sampler(wgpu::Sampler),
}

/// A buffer bound over its first `bytes`, rounded up to whole 4-byte
/// words, so `arrayLength` counts the elements it holds and none of the
/// padding a backend adds to a buffer with vertex usage.
pub(crate) fn exact(buffer: wgpu::Buffer, bytes: u64) -> Resource {
    Resource::Buffer { buffer, offset: 0, size: NonZeroU64::new(bytes.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)) }
}

/// A resource matched to its binding. `uploaded` is the element type and
/// length of a buffer made from an array for this call, which the call
/// may read back.
pub(crate) struct Bound {
    pub(crate) binding: Reflected,
    pub(crate) resource: Resource,
    pub(crate) uploaded: Option<(ElementType, u64)>,
}

fn is_dictionary(type_name: &str) -> bool {
    matches!(type_name, "System.Collections.Hashtable" | "System.Collections.Specialized.OrderedDictionary")
}

fn binding_list(bindings: &[Reflected]) -> String {
    bindings.iter().map(|r| format!("{} ({}:{})", r.name, r.group, r.binding)).collect::<Vec<_>>().join(", ")
}

/// Whether a hashtable key names a binding: a binding's variable name,
/// ignoring case, or anything with a colon, which WGSL names never hold.
fn names_a_binding(bindings: &[Reflected], key: &str) -> bool {
    key.contains(':') || bindings.iter().any(|r| r.name.eq_ignore_ascii_case(key))
}

/// The binding a key names: "group:binding", or its variable name,
/// ignoring case.
fn find<'a>(bindings: &'a [Reflected], key: &str) -> PsResult<&'a Reflected> {
    if let Some((g, b)) = key.split_once(':') {
        let number = |part: &str| part.trim().parse::<u32>().map_err(|e| invalid("WgpuBindingKey", format!("{key} is neither a binding name nor group:binding: {e}")));
        let (g, b) = (number(g)?, number(b)?);
        return bindings.iter().find(|r| r.group == g && r.binding == b).ok_or_else(|| not_found("WgpuNoBinding", format!("the shader uses no binding {g}:{b}")));
    }
    bindings
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(key))
        .ok_or_else(|| not_found("WgpuNoBinding", format!("the shader uses no binding named {key}; it uses {}", binding_list(bindings))))
}

fn every_binding_given(bindings: &[Reflected], given: &[(Reflected, PsObject)]) -> PsResult<()> {
    let missing: Vec<&Reflected> = bindings.iter().filter(|b| !given.iter().any(|(g, _)| g.group == b.group && g.binding == b.binding)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let names = missing.iter().map(|b| format!("{} ({}:{})", b.name, b.group, b.binding)).collect::<Vec<_>>().join(", ");
    Err(invalid("WgpuBindingMissing", format!("no resource was given for {names}")))
}

fn is_number(type_name: &str) -> bool {
    matches!(
        type_name,
        "System.Single" | "System.Double" | "System.Decimal" | "System.SByte" | "System.Byte" | "System.Int16" | "System.UInt16" | "System.Int32" | "System.UInt32" | "System.Int64" | "System.UInt64"
    )
}

/// Matches `resources` to `bindings`: an array of resources in binding
/// order; a hashtable keyed by binding name or "group:binding"; or, for a
/// shader with one binding, that binding's resource itself (a typed
/// array, an array of numbers, a buffer, a texture, or a hashtable of a
/// struct's members).
pub(crate) fn match_resources(bindings: &[Reflected], resources: &PsObject) -> PsResult<Vec<(Reflected, PsObject)>> {
    if resources.is_null() {
        every_binding_given(bindings, &[])?;
        return Ok(Vec::new());
    }
    let type_name = resources.type_name()?;
    if is_dictionary(&type_name) {
        let table = PsHashtable(resources.clone());
        let keys = table.keys()?;
        let one_struct = bindings.len() == 1 && !bindings[0].members.is_empty();
        if one_struct && !keys.iter().any(|k| names_a_binding(bindings, k)) {
            return Ok(vec![(bindings[0].clone(), resources.clone())]);
        }
        let mut out = Vec::with_capacity(keys.len());
        for key in &keys {
            out.push((find(bindings, key)?.clone(), table.get(key)?));
        }
        every_binding_given(bindings, &out)?;
        out.sort_by_key(|(b, _)| (b.group, b.binding));
        return Ok(out);
    }
    let is_list = type_name == "System.Object[]" || (type_name.starts_with("Wgpu.") && type_name.ends_with("[]"));
    let items = if is_list {
        let items = Vec::<PsObject>::from_ps(resources)?;
        let numbers = match items.first() {
            Some(first) => is_number(&first.type_name()?),
            None => false,
        };
        if numbers { vec![resources.clone()] } else { items }
    } else {
        vec![resources.clone()]
    };
    if items.len() != bindings.len() {
        return Err(invalid(
            "WgpuBindingCount",
            format!(
                "the shader uses {} bindings ({}) and {} resources were given; give one per binding in that order, or a hashtable by name",
                bindings.len(),
                binding_list(bindings),
                items.len()
            ),
        ));
    }
    Ok(bindings.iter().cloned().zip(items).collect())
}

/// The bytes of a struct buffer packed from a hashtable of its members'
/// values, laid out as the shader declares them. Members the table leaves
/// out are zero; a key naming no member is refused.
fn pack(binding: &Reflected, table: &PsHashtable) -> PsResult<Vec<u8>> {
    let mut out = vec![0u8; binding.size.max(4) as usize];
    for key in table.keys()? {
        let member = binding.members.iter().find(|m| m.name.eq_ignore_ascii_case(&key)).ok_or_else(|| {
            let names = binding.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ");
            invalid("WgpuNoMember", format!("{} has no member {key}; its members are {names}", binding.type_name))
        })?;
        let Some(element) = member.element else {
            return Err(invalid("WgpuMemberNotNumbers", format!("member {} of {} is not numbers; pack it by hand into a buffer", member.name, binding.type_name)));
        };
        let value = table.get(&key)?;
        element::with_host_bytes(&value, Some(element), |ty, bytes| place(&mut out, member, ty, bytes))?;
    }
    Ok(out)
}

/// Writes a member's numbers at its offset, column by column and element
/// by element. A member whose `count` is 0 is a runtime-sized array and
/// takes as many elements as it is given.
fn place(out: &mut Vec<u8>, member: &Member, ty: ElementType, bytes: &[u8]) -> PsResult<()> {
    let size = ty.size() as usize;
    let given = bytes.len() / size;
    let per_element = (member.rows * member.columns) as usize;
    let elements = if member.count == 0 {
        if !given.is_multiple_of(per_element) {
            return Err(invalid("WgpuMemberSize", format!("member {} takes numbers in groups of {per_element}; {given} were given", member.name)));
        }
        given / per_element
    } else {
        if given != member.numbers() as usize {
            return Err(invalid("WgpuMemberSize", format!("member {} takes {} numbers; {given} were given", member.name, member.numbers())));
        }
        member.count as usize
    };
    for a in 0..elements {
        for c in 0..member.columns as usize {
            for r in 0..member.rows as usize {
                let index = (a * member.columns as usize + c) * member.rows as usize + r;
                let at = member.offset as usize + a * member.stride as usize + c * member.column_stride as usize + r * size;
                if out.len() < at + size {
                    out.resize(at + size, 0);
                }
                out[at..at + size].copy_from_slice(&bytes[index * size..index * size + size]);
            }
        }
    }
    Ok(())
}

fn buffer_usage(kind: BindingKind) -> wgpu::BufferUsages {
    let copy = wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
    match kind {
        BindingKind::UniformBuffer => wgpu::BufferUsages::UNIFORM | copy,
        _ => wgpu::BufferUsages::STORAGE | copy,
    }
}

/// A buffer holding `bytes` for `binding`: at least the binding's fixed
/// size, and a multiple of 16 bytes for a uniform.
fn upload_bytes(core: &CoreRef, binding: &Reflected, bytes: &[u8]) -> PsResult<wgpu::Buffer> {
    let mut size = (bytes.len() as u64).max(binding.size);
    if binding.kind == BindingKind::UniformBuffer {
        size = size.next_multiple_of(16);
    }
    buffer::create_raw(core, size, buffer_usage(binding.kind), &binding.name, Some(bytes))
}

fn expected(kind: BindingKind) -> &'static str {
    match kind {
        BindingKind::Sampler | BindingKind::ComparisonSampler => "Wgpu.Sampler",
        BindingKind::Texture | BindingKind::StorageTexture | BindingKind::ExternalTexture => "Wgpu.Texture or Wgpu.TextureView",
        BindingKind::AccelerationStructure => "acceleration structure",
        BindingKind::UniformBuffer | BindingKind::StorageBuffer | BindingKind::ReadOnlyStorageBuffer => "Wgpu.Buffer, Wgpu.BufferSlice or an array",
    }
}

/// Turns one PowerShell value into the resource for `binding` on `core`.
pub(crate) fn resolve(core: &CoreRef, binding: &Reflected, value: &PsObject) -> PsResult<Bound> {
    if value.is_null() {
        return Err(invalid("WgpuBindingMissing", format!("no resource was given for {} ({}:{}): its value is $null", binding.name, binding.group, binding.binding)));
    }
    let type_name = value.type_name()?;
    let same_device = |other: &CoreRef| {
        if other.id == core.id {
            Ok(())
        } else {
            Err(invalid("WgpuOtherDevice", format!("the resource for {} lives on another device ({}); resources and pipelines must share one", binding.name, other.label)))
        }
    };
    let bound = |resource| Bound { binding: binding.clone(), resource, uploaded: None };
    match type_name.as_str() {
        "Wgpu.Buffer" => {
            let (buffer, owner, bytes) = PsProxy::<Buffer>::from_ps(value)?.with(|b| (b.buffer.clone(), b.core.clone(), b.length * b.element_type.size()))?;
            same_device(&owner)?;
            Ok(bound(exact(buffer, bytes)))
        }
        "Wgpu.BufferSlice" => {
            let (buffer, owner, offset, size) = PsProxy::<BufferSlice>::from_ps(value)?.with(|s| (s.buffer.clone(), s.core.clone(), s.offset, s.size))?;
            same_device(&owner)?;
            Ok(bound(Resource::Buffer { buffer, offset, size: NonZeroU64::new(size) }))
        }
        "Wgpu.Texture" | "Wgpu.TextureView" | "Wgpu.Sampler" => {
            let (resource, owner) = crate::texture::resolve(value, &type_name, binding)?;
            same_device(&owner)?;
            Ok(bound(resource))
        }
        name if is_dictionary(name) => {
            if binding.members.is_empty() {
                return Err(invalid("WgpuNotStruct", format!("{} is {}, not a struct, so it takes an array rather than a hashtable", binding.name, binding.type_name)));
            }
            let bytes = pack(binding, &PsHashtable(value.clone()))?;
            let buffer = upload_bytes(core, binding, &bytes)?;
            Ok(bound(Resource::Buffer { buffer, offset: 0, size: None }))
        }
        _ => {
            if !matches!(binding.kind, BindingKind::UniformBuffer | BindingKind::StorageBuffer | BindingKind::ReadOnlyStorageBuffer) {
                return Err(invalid("WgpuWrongResource", format!("{} is a {:?} binding; it takes a {}", binding.name, binding.kind, expected(binding.kind))));
            }
            element::with_host_bytes(value, binding.element_type, |ty, bytes| {
                if bytes.is_empty() {
                    return Err(invalid("WgpuEmpty", format!("the array for {} is empty", binding.name)));
                }
                let buffer = upload_bytes(core, binding, bytes)?;
                Ok(Bound {
                    binding: binding.clone(),
                    resource: Resource::Buffer { buffer, offset: 0, size: None },
                    uploaded: Some((ty, bytes.len() as u64 / ty.size())),
                })
            })
        }
    }
}

/// Resources Rust code gives for `bindings`, each keyed by its binding's
/// variable name, ignoring case, or by "group:binding". Every binding
/// takes exactly one.
pub(crate) fn bound_by_name(bindings: &[Reflected], given: Vec<(String, Resource)>) -> PsResult<Vec<Bound>> {
    let mut out: Vec<Bound> = Vec::with_capacity(given.len());
    for (key, resource) in given {
        let binding = find(bindings, &key)?;
        if out.iter().any(|b| b.binding.group == binding.group && b.binding.binding == binding.binding) {
            return Err(invalid("WgpuBindingTwice", format!("{key} names {} ({}:{}), which another resource already binds", binding.name, binding.group, binding.binding)));
        }
        out.push(Bound { binding: binding.clone(), resource, uploaded: None });
    }
    let missing: Vec<String> = bindings
        .iter()
        .filter(|b| !out.iter().any(|x| x.binding.group == b.group && x.binding.binding == b.binding))
        .map(|b| format!("{} ({}:{})", b.name, b.group, b.binding))
        .collect();
    if !missing.is_empty() {
        return Err(invalid("WgpuBindingMissing", format!("no resource was given for {}", missing.join(", "))));
    }
    out.sort_by_key(|b| (b.binding.group, b.binding.binding));
    Ok(out)
}

/// Resolves every resource for `bindings`.
pub(crate) fn resolve_all(core: &CoreRef, bindings: &[Reflected], resources: Option<&PsObject>) -> PsResult<Vec<Bound>> {
    let matched = match resources {
        Some(r) => match_resources(bindings, r)?,
        None => {
            every_binding_given(bindings, &[])?;
            Vec::new()
        }
    };
    matched.iter().map(|(b, v)| resolve(core, b, v)).collect()
}

/// The bind group for `group` from resolved resources.
pub(crate) fn make_group(core: &CoreRef, group: &Group, bound: &[Bound], label: &str) -> PsResult<wgpu::BindGroup> {
    let mut entries = Vec::with_capacity(group.bindings.len());
    for b in &group.bindings {
        let found = bound
            .iter()
            .find(|x| x.binding.group == b.group && x.binding.binding == b.binding)
            .ok_or_else(|| invalid("WgpuBindingMissing", format!("no resource was given for {} ({}:{})", b.name, b.group, b.binding)))?;
        let resource = match &found.resource {
            Resource::Buffer { buffer, offset, size } => wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer, offset: *offset, size: *size }),
            Resource::View(view) => wgpu::BindingResource::TextureView(view),
            Resource::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler),
        };
        entries.push(wgpu::BindGroupEntry { binding: b.binding, resource });
    }
    core.scoped("WgpuBindGroup", "the resources do not fit the shader's bindings", || {
        core.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &group.layout, entries: &entries })
    })
}

/// Every bind group of a pipeline from resolved resources.
pub(crate) fn make_groups(core: &CoreRef, groups: &[Group], bound: &[Bound], label: &str) -> PsResult<Vec<(u32, wgpu::BindGroup)>> {
    groups.iter().map(|g| Ok((g.index, make_group(core, g, bound, label)?))).collect()
}

/// A bind group: resources bound to one group of a pipeline's bindings,
/// set on a pass with SetBindGroup.
#[psclass(name = "Wgpu.BindGroup", mode = proxy, columns = ["Label", "Group", "Bindings"])]
pub struct BindGroup {
    /// The bind group's label.
    pub label: String,
    /// The group index it was made for.
    pub group: u32,
    /// The names of the bindings it fills, in binding order.
    pub bindings: Vec<String>,
    #[psfield(skip)]
    pub(crate) group_object: wgpu::BindGroup,
}

/// The bind group `group` of a pipeline's `groups`, from `resources`.
pub(crate) fn bind_group_from(core: &CoreRef, groups: &[Group], group: u32, resources: &PsObject, pipeline_label: &str) -> PsResult<BindGroup> {
    let target = groups
        .iter()
        .find(|g| g.index == group)
        .ok_or_else(|| not_found("WgpuNoGroup", format!("the pipeline has no bind group {group}; it has {}", groups.len())))?;
    let bound = resolve_all(core, &target.bindings, Some(resources))?;
    let label = format!("{pipeline_label} group {group}");
    let group_object = make_group(core, target, &bound, &label)?;
    Ok(BindGroup { label, group, bindings: target.bindings.iter().map(|b| b.name.clone()).collect(), group_object })
}

/// The bind group `group` of a compute pipeline, from `resources`.
pub(crate) fn bind_group_for(inner: &Arc<ComputeInner>, group: u32, resources: &PsObject) -> PsResult<BindGroup> {
    bind_group_from(&inner.core, &inner.groups, group, resources, &inner.label)
}

pub(crate) fn bind_group(pipeline: &PsProxy<ComputePipeline>, group: u32, resources: &PsObject) -> PsResult<BindGroup> {
    let inner = pipeline.with(|p| p.inner.clone())?;
    bind_group_for(&inner, group, resources)
}

/// Binds resources to one group of a compute pipeline's bindings.
///
/// The resources are an array in binding order, a hashtable by binding
/// name or "group:binding", or one resource for a group with one binding.
///
/// # Examples
/// $group = New-WgpuBindGroup -Pipeline $pipeline -Resource $a, $b
/// New-WgpuBindGroup $pipeline @{ input = $a; output = $b } -Group 1
#[cmdlet(verb = "New", noun = "WgpuBindGroup", alias = ["New-GpuBindGroup"], output = ["Wgpu.BindGroup"])]
#[derive(Default)]
pub struct NewWgpuBindGroup {
    /// The pipeline whose bindings the group fills.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub pipeline: Option<PsProxy<ComputePipeline>>,
    /// The resources.
    #[param(mandatory, position = 1, raw)]
    pub resource: Option<PsObject>,
    /// The group index; 0 when omitted.
    #[param]
    pub group: Option<u32>,
}

impl Cmdlet for NewWgpuBindGroup {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (Some(pipeline), Some(resource)) = (&self.pipeline, &self.resource) else {
            return Err(invalid("WgpuNoPipeline", "a pipeline and resources are both needed"));
        };
        ps.write(bind_group(pipeline, self.group.unwrap_or(0), resource)?)
    }
}
