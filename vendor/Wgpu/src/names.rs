//! Features, limits, backends and device types as PowerShell sees them.

use pwrs::prelude::*;

use crate::error::invalid;

/// A graphics API wgpu drives.
#[psenum(name = "Wgpu.Backend")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum Backend {
    /// Vulkan on Windows, Linux and FreeBSD, and on macOS through MoltenVK.
    #[default]
    Vulkan,
    /// Metal on macOS.
    Metal,
    /// Direct3D 12 on Windows.
    Dx12,
    /// OpenGL and OpenGL ES; on Windows only when named, never by default.
    Gl,
    /// A backend that runs nothing, for exercising a script without a GPU.
    Noop,
}

impl Backend {
    pub(crate) fn bits(self) -> wgpu::Backends {
        match self {
            Backend::Vulkan => wgpu::Backends::VULKAN,
            Backend::Metal => wgpu::Backends::METAL,
            Backend::Dx12 => wgpu::Backends::DX12,
            Backend::Gl => wgpu::Backends::GL,
            Backend::Noop => wgpu::Backends::NOOP,
        }
    }

    pub(crate) fn from_wgpu(backend: wgpu::Backend) -> Backend {
        match backend {
            wgpu::Backend::Vulkan => Backend::Vulkan,
            wgpu::Backend::Metal => Backend::Metal,
            wgpu::Backend::Dx12 => Backend::Dx12,
            wgpu::Backend::Gl => Backend::Gl,
            wgpu::Backend::Noop | wgpu::Backend::BrowserWebGpu => Backend::Noop,
        }
    }
}

/// The set of backends a list names; without a list, Vulkan and DX12 on
/// Windows and every native backend elsewhere. GL is left out on Windows
/// because wgpu makes a GL context current on whichever thread creates a GL
/// instance or lists its adapters, and NVIDIA's driver deadlocks the
/// process when such a thread exits while other threads create or destroy
/// devices.
pub(crate) fn backends(list: Option<&[Backend]>) -> wgpu::Backends {
    match list {
        Some(list) => list.iter().fold(wgpu::Backends::empty(), |all, b| all | b.bits()),
        None if cfg!(windows) => wgpu::Backends::VULKAN | wgpu::Backends::DX12,
        None => wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12 | wgpu::Backends::GL,
    }
}

/// The kind of adapter.
#[psenum(name = "Wgpu.DeviceType")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum DeviceType {
    /// A GPU of its own.
    DiscreteGpu,
    /// A GPU sharing the CPU's package and memory.
    IntegratedGpu,
    /// A GPU of a virtual machine.
    VirtualGpu,
    /// Software running on the CPU, such as WARP or lavapipe.
    Cpu,
    /// Anything else.
    #[default]
    Other,
}

impl DeviceType {
    pub(crate) fn from_wgpu(t: wgpu::DeviceType) -> DeviceType {
        match t {
            wgpu::DeviceType::DiscreteGpu => DeviceType::DiscreteGpu,
            wgpu::DeviceType::IntegratedGpu => DeviceType::IntegratedGpu,
            wgpu::DeviceType::VirtualGpu => DeviceType::VirtualGpu,
            wgpu::DeviceType::Cpu => DeviceType::Cpu,
            wgpu::DeviceType::Other => DeviceType::Other,
        }
    }
}

/// A `SCREAMING_SNAKE_CASE` flag name in PascalCase: `SHADER_F16` is
/// `ShaderF16`.
pub(crate) fn pascal(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase(),
                None => String::new(),
            }
        })
        .collect()
}

/// A name with case, `_`, `-` and spaces taken out, so `ShaderF16`,
/// `SHADER_F16` and `shader-f16` compare equal.
pub(crate) fn fold(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// The names of every feature in `features`, in PascalCase.
pub(crate) fn feature_names(features: wgpu::Features) -> Vec<String> {
    features.iter_names().map(|(name, _)| pascal(name)).collect()
}

/// The feature a name stands for, in PascalCase, `SCREAMING_SNAKE_CASE` or
/// WebGPU's kebab-case, ignoring case.
pub(crate) fn parse_feature(name: &str) -> PsResult<wgpu::Features> {
    let key = fold(name);
    wgpu::Features::all()
        .iter_names()
        .find(|(flag, feature)| fold(flag) == key || feature.as_str().is_some_and(|kebab| fold(kebab) == key))
        .map(|(_, feature)| feature)
        .ok_or_else(|| invalid("WgpuUnknownFeature", format!("wgpu has no feature named {name}")))
}

/// Every feature name wgpu knows, in PascalCase.
pub(crate) fn all_feature_names() -> Vec<String> {
    feature_names(wgpu::Features::all())
}

/// The names of the downlevel flags an adapter reports, in PascalCase.
pub(crate) fn downlevel_names(flags: wgpu::DownlevelFlags) -> Vec<String> {
    flags.iter_names().map(|(name, _)| pascal(name)).collect()
}

/// Every limit of an adapter or a device, as wgpu reports it. A device
/// opened by this module asks for every limit its adapter has, so a
/// device's limits are its adapter's.
#[psclass(name = "Wgpu.Limits")]
#[derive(Default, Clone)]
pub struct Limits {
    /// The largest width of a one-dimensional texture, in texels.
    pub max_texture_dimension_1d: u64,
    /// The largest width or height of a two-dimensional texture, in texels.
    pub max_texture_dimension_2d: u64,
    /// The largest width, height or depth of a three-dimensional texture, in texels.
    pub max_texture_dimension_3d: u64,
    /// The most layers a two-dimensional texture array holds.
    pub max_texture_array_layers: u64,
    /// The most bind groups a pipeline uses at once.
    pub max_bind_groups: u64,
    /// The most bind group and vertex buffer slots used at once, counting empty slots below the highest.
    pub max_bind_groups_plus_vertex_buffers: u64,
    /// The highest binding number a bind group takes, plus one.
    pub max_bindings_per_bind_group: u64,
    /// The most uniform buffer bindings with dynamic offsets in one pipeline.
    pub max_dynamic_uniform_buffers_per_pipeline_layout: u64,
    /// The most storage buffer bindings with dynamic offsets in one pipeline.
    pub max_dynamic_storage_buffers_per_pipeline_layout: u64,
    /// The most sampled textures one shader stage sees.
    pub max_sampled_textures_per_shader_stage: u64,
    /// The most samplers one shader stage sees.
    pub max_samplers_per_shader_stage: u64,
    /// The most storage buffers one shader stage sees.
    pub max_storage_buffers_per_shader_stage: u64,
    /// The most storage textures one shader stage sees.
    pub max_storage_textures_per_shader_stage: u64,
    /// The most uniform buffers one shader stage sees.
    pub max_uniform_buffers_per_shader_stage: u64,
    /// The most resources other than samplers that binding arrays hold for one shader stage; 0 without binding arrays.
    pub max_binding_array_elements_per_shader_stage: u64,
    /// The most acceleration structures that binding arrays hold for one shader stage; 0 without them.
    pub max_binding_array_acceleration_structure_elements_per_shader_stage: u64,
    /// The most samplers that binding arrays hold for one shader stage; 0 without binding arrays.
    pub max_binding_array_sampler_elements_per_shader_stage: u64,
    /// The largest uniform buffer binding, in bytes.
    pub max_uniform_buffer_binding_size: u64,
    /// The largest storage buffer binding, in bytes: the most of an array one shader binding sees.
    pub max_storage_buffer_binding_size: u64,
    /// The most vertex buffers a render pipeline reads.
    pub max_vertex_buffers: u64,
    /// The largest buffer, in bytes; a buffer smaller than this can still fail for want of memory.
    pub max_buffer_size: u64,
    /// The most vertex attributes across all of a render pipeline's vertex buffers.
    pub max_vertex_attributes: u64,
    /// The largest distance between consecutive vertices in a vertex buffer, in bytes.
    pub max_vertex_buffer_array_stride: u64,
    /// The most `@location` variables passed between shader stages, such as vertex outputs.
    pub max_inter_stage_shader_variables: u64,
    /// The alignment, in bytes, of a uniform buffer binding's offset.
    pub min_uniform_buffer_offset_alignment: u64,
    /// The alignment, in bytes, of a storage buffer binding's offset.
    pub min_storage_buffer_offset_alignment: u64,
    /// The most color targets a render pipeline draws into.
    pub max_color_attachments: u64,
    /// The most bytes one sample takes across all color targets.
    pub max_color_attachment_bytes_per_sample: u64,
    /// The most `var<workgroup>` memory a compute entry point uses, in bytes.
    pub max_compute_workgroup_storage_size: u64,
    /// The largest product of a compute entry point's `@workgroup_size` dimensions.
    pub max_compute_invocations_per_workgroup: u64,
    /// The largest X dimension of a compute entry point's `@workgroup_size`.
    pub max_compute_workgroup_size_x: u64,
    /// The largest Y dimension of a compute entry point's `@workgroup_size`.
    pub max_compute_workgroup_size_y: u64,
    /// The largest Z dimension of a compute entry point's `@workgroup_size`.
    pub max_compute_workgroup_size_z: u64,
    /// The most workgroups one dispatch runs in each dimension, the bound of `-Workgroups`.
    pub max_compute_workgroups_per_dimension: u64,
    /// The most immediate data a pipeline takes, in bytes; 0 without the `Immediates` feature.
    pub max_immediate_size: u64,
    /// The most bindings other than samplers alive at once; it bounds only the Direct3D 12 backend.
    pub max_non_sampler_bindings: u64,
    /// The largest product of the workgroup counts of a mesh draw through a task shader.
    pub max_task_workgroup_total_count: u64,
    /// The largest workgroup count in each dimension of a mesh draw through a task shader.
    pub max_task_workgroups_per_dimension: u64,
    /// The largest product of the workgroup counts of a mesh draw without a task shader, or of a task shader's output.
    pub max_mesh_workgroup_total_count: u64,
    /// The largest workgroup count in each dimension of a mesh draw without a task shader, or of a task shader's output.
    pub max_mesh_workgroups_per_dimension: u64,
    /// The most invocations in one task shader workgroup.
    pub max_task_invocations_per_workgroup: u64,
    /// The largest dimension of a task shader's workgroup size.
    pub max_task_invocations_per_dimension: u64,
    /// The most invocations in one mesh shader workgroup.
    pub max_mesh_invocations_per_workgroup: u64,
    /// The largest dimension of a mesh shader's workgroup size.
    pub max_mesh_invocations_per_dimension: u64,
    /// The largest payload a task shader passes to a mesh shader, in bytes.
    pub max_task_payload_size: u64,
    /// The most vertices a mesh shader outputs.
    pub max_mesh_output_vertices: u64,
    /// The most primitives a mesh shader outputs.
    pub max_mesh_output_primitives: u64,
    /// The most layers a mesh shader outputs.
    pub max_mesh_output_layers: u64,
    /// The most views a mesh shader renders in multiview rendering.
    pub max_mesh_multiview_view_count: u64,
    /// The most primitives in a bottom-level acceleration structure; 0 without ray queries.
    pub max_blas_primitive_count: u64,
    /// The most geometries in a bottom-level acceleration structure; 0 without ray queries.
    pub max_blas_geometry_count: u64,
    /// The most instances in a top-level acceleration structure; 0 without ray queries.
    pub max_tlas_instance_count: u64,
    /// The most acceleration structures one shader stage uses; 0 without ray queries.
    pub max_acceleration_structures_per_shader_stage: u64,
    /// The most storage, uniform and vertex buffers and acceleration structures one shader stage binds, together.
    pub max_buffers_and_acceleration_structures_per_shader_stage: u64,
    /// The most views in multiview rendering.
    pub max_multiview_view_count: u64,
    /// The most rays one trace call dispatches; 0 without ray tracing pipelines.
    pub max_ray_dispatch_count: u64,
    /// The deepest ray recursion a ray tracing pipeline may ask for; 0 without ray tracing pipelines.
    pub max_ray_recursion_depth: u64,
}

impl From<&wgpu::Limits> for Limits {
    fn from(l: &wgpu::Limits) -> Limits {
        Limits {
            max_texture_dimension_1d: l.max_texture_dimension_1d.into(),
            max_texture_dimension_2d: l.max_texture_dimension_2d.into(),
            max_texture_dimension_3d: l.max_texture_dimension_3d.into(),
            max_texture_array_layers: l.max_texture_array_layers.into(),
            max_bind_groups: l.max_bind_groups.into(),
            max_bind_groups_plus_vertex_buffers: l.max_bind_groups_plus_vertex_buffers.into(),
            max_bindings_per_bind_group: l.max_bindings_per_bind_group.into(),
            max_dynamic_uniform_buffers_per_pipeline_layout: l.max_dynamic_uniform_buffers_per_pipeline_layout.into(),
            max_dynamic_storage_buffers_per_pipeline_layout: l.max_dynamic_storage_buffers_per_pipeline_layout.into(),
            max_sampled_textures_per_shader_stage: l.max_sampled_textures_per_shader_stage.into(),
            max_samplers_per_shader_stage: l.max_samplers_per_shader_stage.into(),
            max_storage_buffers_per_shader_stage: l.max_storage_buffers_per_shader_stage.into(),
            max_storage_textures_per_shader_stage: l.max_storage_textures_per_shader_stage.into(),
            max_uniform_buffers_per_shader_stage: l.max_uniform_buffers_per_shader_stage.into(),
            max_binding_array_elements_per_shader_stage: l.max_binding_array_elements_per_shader_stage.into(),
            max_binding_array_acceleration_structure_elements_per_shader_stage: l.max_binding_array_acceleration_structure_elements_per_shader_stage.into(),
            max_binding_array_sampler_elements_per_shader_stage: l.max_binding_array_sampler_elements_per_shader_stage.into(),
            max_uniform_buffer_binding_size: l.max_uniform_buffer_binding_size,
            max_storage_buffer_binding_size: l.max_storage_buffer_binding_size,
            max_vertex_buffers: l.max_vertex_buffers.into(),
            max_buffer_size: l.max_buffer_size,
            max_vertex_attributes: l.max_vertex_attributes.into(),
            max_vertex_buffer_array_stride: l.max_vertex_buffer_array_stride.into(),
            max_inter_stage_shader_variables: l.max_inter_stage_shader_variables.into(),
            min_uniform_buffer_offset_alignment: l.min_uniform_buffer_offset_alignment.into(),
            min_storage_buffer_offset_alignment: l.min_storage_buffer_offset_alignment.into(),
            max_color_attachments: l.max_color_attachments.into(),
            max_color_attachment_bytes_per_sample: l.max_color_attachment_bytes_per_sample.into(),
            max_compute_workgroup_storage_size: l.max_compute_workgroup_storage_size.into(),
            max_compute_invocations_per_workgroup: l.max_compute_invocations_per_workgroup.into(),
            max_compute_workgroup_size_x: l.max_compute_workgroup_size_x.into(),
            max_compute_workgroup_size_y: l.max_compute_workgroup_size_y.into(),
            max_compute_workgroup_size_z: l.max_compute_workgroup_size_z.into(),
            max_compute_workgroups_per_dimension: l.max_compute_workgroups_per_dimension.into(),
            max_immediate_size: l.max_immediate_size.into(),
            max_non_sampler_bindings: l.max_non_sampler_bindings.into(),
            max_task_workgroup_total_count: l.max_task_workgroup_total_count.into(),
            max_task_workgroups_per_dimension: l.max_task_workgroups_per_dimension.into(),
            max_mesh_workgroup_total_count: l.max_mesh_workgroup_total_count.into(),
            max_mesh_workgroups_per_dimension: l.max_mesh_workgroups_per_dimension.into(),
            max_task_invocations_per_workgroup: l.max_task_invocations_per_workgroup.into(),
            max_task_invocations_per_dimension: l.max_task_invocations_per_dimension.into(),
            max_mesh_invocations_per_workgroup: l.max_mesh_invocations_per_workgroup.into(),
            max_mesh_invocations_per_dimension: l.max_mesh_invocations_per_dimension.into(),
            max_task_payload_size: l.max_task_payload_size.into(),
            max_mesh_output_vertices: l.max_mesh_output_vertices.into(),
            max_mesh_output_primitives: l.max_mesh_output_primitives.into(),
            max_mesh_output_layers: l.max_mesh_output_layers.into(),
            max_mesh_multiview_view_count: l.max_mesh_multiview_view_count.into(),
            max_blas_primitive_count: l.max_blas_primitive_count.into(),
            max_blas_geometry_count: l.max_blas_geometry_count.into(),
            max_tlas_instance_count: l.max_tlas_instance_count.into(),
            max_acceleration_structures_per_shader_stage: l.max_acceleration_structures_per_shader_stage.into(),
            max_buffers_and_acceleration_structures_per_shader_stage: l.max_buffers_and_acceleration_structures_per_shader_stage.into(),
            max_multiview_view_count: l.max_multiview_view_count.into(),
            max_ray_dispatch_count: l.max_ray_dispatch_count.into(),
            max_ray_recursion_depth: l.max_ray_recursion_depth.into(),
        }
    }
}
