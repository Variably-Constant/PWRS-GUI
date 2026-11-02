//! Textures and samplers in the layer's table, which a surface's draws and
//! passes, and compute runs, bind by name as the Wgpu module binds them: a
//! two-dimensional texture of a format and size, filled from the session's
//! texels, and a sampler of a filter and an address mode. Each records what
//! the session wrote, which fills it again on a new device.

use crate::{Failure, Layer, to_usize};
use wgpu_pwsh::{ElementType, Gpu, wgpu};

/// A texture's format, named as the Wgpu module names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    R8Unorm,
    Rg8Unorm,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    R16Float,
    Rg16Float,
    Rgba16Float,
    R32Float,
    Rg32Float,
    Rgba32Float,
    R32Uint,
    R32Sint,
}

impl TextureFormat {
    pub fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            TextureFormat::R8Unorm => wgpu::TextureFormat::R8Unorm,
            TextureFormat::Rg8Unorm => wgpu::TextureFormat::Rg8Unorm,
            TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
            TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            TextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
            TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
            TextureFormat::R16Float => wgpu::TextureFormat::R16Float,
            TextureFormat::Rg16Float => wgpu::TextureFormat::Rg16Float,
            TextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
            TextureFormat::R32Float => wgpu::TextureFormat::R32Float,
            TextureFormat::Rg32Float => wgpu::TextureFormat::Rg32Float,
            TextureFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
            TextureFormat::R32Uint => wgpu::TextureFormat::R32Uint,
            TextureFormat::R32Sint => wgpu::TextureFormat::R32Sint,
        }
    }

    /// The number type a texel's channels are written and read as.
    pub fn element(self) -> ElementType {
        match self {
            TextureFormat::R8Unorm | TextureFormat::Rg8Unorm | TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => ElementType::UInt8,
            TextureFormat::R16Float | TextureFormat::Rg16Float | TextureFormat::Rgba16Float => ElementType::Float16,
            TextureFormat::R32Float | TextureFormat::Rg32Float | TextureFormat::Rgba32Float => ElementType::Float32,
            TextureFormat::R32Uint => ElementType::UInt32,
            TextureFormat::R32Sint => ElementType::Int32,
        }
    }

    /// How many channels a texel has.
    pub fn channels(self) -> u32 {
        match self {
            TextureFormat::R8Unorm | TextureFormat::R16Float | TextureFormat::R32Float | TextureFormat::R32Uint | TextureFormat::R32Sint => 1,
            TextureFormat::Rg8Unorm | TextureFormat::Rg16Float | TextureFormat::Rg32Float => 2,
            _ => 4,
        }
    }

    /// The bytes one texel takes.
    pub fn texel_bytes(self) -> u64 {
        u64::from(self.channels()) * self.element().size()
    }
}

/// How a sampler picks between texels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    /// The nearest texel.
    Nearest,
    /// The texels around, weighted by distance.
    #[default]
    Linear,
}

/// What a sampler reads outside the texture's edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AddressMode {
    /// The edge's texels.
    #[default]
    ClampToEdge,
    /// The texture again.
    Repeat,
    /// The texture again, mirrored each time.
    MirrorRepeat,
}

/// A sampler's filter and address mode, which apply in every direction and
/// to magnifying and minifying alike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SamplerSpec {
    pub filter: Filter,
    pub address: AddressMode,
}

/// A texture of the table: its size, format and label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureInfo {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub label: String,
}

pub(crate) struct TextureEntry {
    pub(crate) texture: wgpu::Texture,
    info: TextureInfo,
    /// The texels the session wrote last, rows packed; none for a texture
    /// it has not written, which starts zeroed.
    kept: Option<Vec<u8>>,
}

pub(crate) struct SamplerEntry {
    pub(crate) sampler: wgpu::Sampler,
    spec: SamplerSpec,
    label: String,
}

/// Every usage `format` allows on `gpu`'s adapter, of copy, sampling,
/// storage and render attachment, as the Wgpu module's New-WgpuTexture
/// gives a texture when none is named.
fn usage_for(gpu: &Gpu, format: wgpu::TextureFormat) -> wgpu::TextureUsages {
    let wanted = wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT;
    gpu.adapter().get_texture_format_features(format).allowed_usages & wanted
}

fn make_texture(gpu: &Gpu, info: &TextureInfo, texels: Option<&[u8]>) -> Result<wgpu::Texture, Failure> {
    let format = info.format.wgpu();
    let texture = gpu.create_texture(info.width, info.height, format, Some(usage_for(gpu, format)), &info.label)?;
    if let Some(bytes) = texels {
        gpu.write_texture(&texture, bytes)?;
    }
    Ok(texture)
}

fn make_sampler(gpu: &Gpu, spec: SamplerSpec, label: &str) -> wgpu::Sampler {
    let filter = match spec.filter {
        Filter::Nearest => wgpu::FilterMode::Nearest,
        Filter::Linear => wgpu::FilterMode::Linear,
    };
    let mipmap = match spec.filter {
        Filter::Nearest => wgpu::MipmapFilterMode::Nearest,
        Filter::Linear => wgpu::MipmapFilterMode::Linear,
    };
    let address = match spec.address {
        AddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        AddressMode::Repeat => wgpu::AddressMode::Repeat,
        AddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
    };
    gpu.device().create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: address,
        address_mode_v: address,
        address_mode_w: address,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: mipmap,
        ..wgpu::SamplerDescriptor::default()
    })
}

fn no_texture(id: u64) -> Failure {
    Failure::new("GuiGpuNoTexture", format!("there is no GPU texture {id}"))
}

fn no_sampler(id: u64) -> Failure {
    Failure::new("GuiGpuNoSampler", format!("there is no GPU sampler {id}"))
}

/// Refuses `texels` for a texture of `info`'s size and format unless they
/// are its every texel, rows packed.
fn whole(info: &TextureInfo, texels: &[u8]) -> Result<(), Failure> {
    let wanted = u64::from(info.width) * u64::from(info.height) * info.format.texel_bytes();
    if texels.len() as u64 == wanted {
        return Ok(());
    }
    let text = format!(
        "a {} by {} texture of {:?} takes {} bytes, {} values of {:?}, and {} bytes were given",
        info.width,
        info.height,
        info.format,
        wanted,
        wanted / info.format.element().size(),
        info.format.element(),
        texels.len()
    );
    Err(Failure::new("GuiGpuTextureSize", text))
}

impl Layer {
    /// A `width` by `height` texture of `format`, labeled `label` or by its
    /// number, filled with `texels`, every texel with rows packed, or
    /// zeroed without them, and its number.
    pub fn new_texture(&mut self, width: u32, height: u32, format: TextureFormat, texels: Option<&[u8]>, label: Option<&str>) -> Result<u64, Failure> {
        if width == 0 || height == 0 {
            return Err(Failure::new("GuiGpuTextureSize", format!("a texture is at least 1 texel on a side, and {width} by {height} was requested")));
        }
        let id = self.next;
        let info = TextureInfo { width, height, format, label: label.map_or_else(|| format!("PWRS-GUI texture {id}"), str::to_string) };
        if let Some(bytes) = texels {
            whole(&info, bytes)?;
        }
        let texture = make_texture(&self.gpu, &info, texels)?;
        self.next += 1;
        self.textures.insert(id, TextureEntry { texture, info, kept: texels.map(<[u8]>::to_vec) });
        Ok(id)
    }

    /// Writes every texel of texture `id`, rows packed.
    pub fn write_texture(&mut self, id: u64, texels: &[u8]) -> Result<(), Failure> {
        let entry = self.textures.get_mut(&id).ok_or_else(|| no_texture(id))?;
        whole(&entry.info, texels)?;
        self.gpu.write_texture(&entry.texture, texels)?;
        entry.kept = Some(texels.to_vec());
        Ok(())
    }

    /// Every texel of texture `id`, rows packed, once the GPU has finished
    /// the work submitted before.
    pub fn read_texture(&self, id: u64) -> Result<Vec<u8>, Failure> {
        let entry = self.textures.get(&id).ok_or_else(|| no_texture(id))?;
        let texels = self.gpu.read_texture(&entry.texture)?;
        let wanted = to_usize(u64::from(entry.info.width) * u64::from(entry.info.height) * entry.info.format.texel_bytes())?;
        Ok(texels.into_iter().take(wanted).collect())
    }

    /// Texture `id`'s size, format and label.
    pub fn texture_info(&self, id: u64) -> Result<TextureInfo, Failure> {
        self.textures.get(&id).map(|e| e.info.clone()).ok_or_else(|| no_texture(id))
    }

    /// Removes texture `id` from the table. Its memory goes when nothing
    /// still uses it: at once, or once the surface plans made with it are
    /// let go.
    pub fn free_texture(&mut self, id: u64) -> Result<(), Failure> {
        self.textures.remove(&id).map(|_| ()).ok_or_else(|| no_texture(id))
    }

    /// A sampler of `spec`, labeled `label` or by its number, and its
    /// number.
    pub fn new_sampler(&mut self, spec: SamplerSpec, label: Option<&str>) -> Result<u64, Failure> {
        let id = self.next;
        let label = label.map_or_else(|| format!("PWRS-GUI sampler {id}"), str::to_string);
        let sampler = make_sampler(&self.gpu, spec, &label);
        self.next += 1;
        self.samplers.insert(id, SamplerEntry { sampler, spec, label });
        Ok(id)
    }

    /// Removes sampler `id` from the table.
    pub fn free_sampler(&mut self, id: u64) -> Result<(), Failure> {
        self.samplers.remove(&id).map(|_| ()).ok_or_else(|| no_sampler(id))
    }

    /// Texture `id` of the table, for a binding.
    pub(crate) fn table_texture(&self, id: u64) -> Result<&wgpu::Texture, Failure> {
        self.textures.get(&id).map(|e| &e.texture).ok_or_else(|| no_texture(id))
    }

    /// Sampler `id` of the table, for a binding.
    pub(crate) fn table_sampler(&self, id: u64) -> Result<&wgpu::Sampler, Failure> {
        self.samplers.get(&id).map(|e| &e.sampler).ok_or_else(|| no_sampler(id))
    }

    /// Makes the table's textures and samplers again on `gpu`, a new
    /// device, each texture filled with the texels the session wrote last.
    pub(crate) fn rebuild_textures(&mut self, gpu: &Gpu) -> Result<(), Failure> {
        for entry in self.textures.values_mut() {
            entry.texture = make_texture(gpu, &entry.info, entry.kept.as_deref())?;
        }
        for entry in self.samplers.values_mut() {
            entry.sampler = make_sampler(gpu, entry.spec, &entry.label);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GpuOptions;

    fn layer() -> Layer {
        let gpu = Gpu::open(&GpuOptions::default()).expect("a device opens");
        Layer::adopt(gpu.adapter().clone(), gpu.device().clone(), gpu.queue().clone(), "PWRS-GUI texture test").expect("the layer takes the device")
    }

    #[test]
    fn a_texture_keeps_the_texels_written_and_refuses_any_other_count() {
        let mut layer = layer();
        let texels: Vec<u8> = (0..16).collect();
        let id = layer.new_texture(2, 2, TextureFormat::Rgba8Unorm, Some(&texels), None).expect("made");
        assert_eq!(layer.read_texture(id).expect("read"), texels);
        assert_eq!(layer.texture_info(id).expect("info"), TextureInfo { width: 2, height: 2, format: TextureFormat::Rgba8Unorm, label: format!("PWRS-GUI texture {id}") });
        assert_eq!(layer.write_texture(id, &[0; 15]).expect_err("15 bytes for 16").id, "GuiGpuTextureSize");
        let zeroed = layer.new_texture(3, 1, TextureFormat::R32Float, None, Some("zeros")).expect("made");
        assert_eq!(layer.read_texture(zeroed).expect("read"), vec![0; 12], "a texture the session has not written is zeroed");
        let ones_twos_threes = [1.0f32, 2.0, 3.0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        layer.write_texture(zeroed, &ones_twos_threes).expect("written");
        assert_eq!(layer.read_texture(zeroed).expect("read"), ones_twos_threes);
        layer.free_texture(id).expect("freed");
        assert_eq!(layer.read_texture(id).expect_err("freed").id, "GuiGpuNoTexture");
        assert_eq!(layer.new_texture(0, 4, TextureFormat::R8Unorm, None, None).expect_err("no texels").id, "GuiGpuTextureSize");
        assert_eq!(TextureFormat::Rgba16Float.texel_bytes(), 8);
    }

    #[test]
    fn a_rebuilt_layer_keeps_its_textures_texels_and_its_samplers() {
        let mut layer = layer();
        let texels = vec![1u8, 2, 3, 4];
        let id = layer.new_texture(1, 1, TextureFormat::Rgba8Unorm, Some(&texels), None).expect("made");
        let sampler = layer.new_sampler(SamplerSpec { filter: Filter::Nearest, address: AddressMode::Repeat }, None).expect("made");
        let other = Gpu::open(&GpuOptions::default()).expect("a second device opens");
        layer.rebuild(other.adapter().clone(), other.device().clone(), other.queue().clone()).expect("rebuilds");
        assert_eq!(layer.read_texture(id).expect("read on the new device"), texels);
        assert!(layer.table_sampler(sampler).is_ok());
        layer.free_sampler(sampler).expect("freed");
        assert_eq!(layer.free_sampler(sampler).expect_err("freed twice").id, "GuiGpuNoSampler");
    }
}
