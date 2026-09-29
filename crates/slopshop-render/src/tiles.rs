//! GPU caches of raster tiles.
//!
//! One 2D texture array per sample type holds fixed-size RGBA tiles in the source precision:
//! 8-bit (`Rgba8Unorm`), 16-bit integer (`Rgba16Uint`, exact), 16-bit float and 32-bit float.
//! Values are uploaded as stored (no transfer or color conversion: the shader does that), so
//! nothing is lost on the way to the GPU. Tiles are immutable, so a cached tile never needs
//! invalidation; the least recently used ones are evicted when space is needed. Only tiles
//! required by the current frame are uploaded.

use std::borrow::Cow;
use std::collections::HashMap;

use slopshop_core::color::{ChannelLayout, PixelFormat, SampleType, f32_to_f16};
use slopshop_core::raster::{ImageId, TILE_SIZE};

/// Identifies one tile of one pyramid level of one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TileKey {
    pub image: ImageId,
    pub level: u32,
    pub col: u32,
    pub row: u32,
}

/// GPU storage class of tiles; one cache per class. The index is the `format` code used by
/// composite.wgsl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GpuTileFormat {
    Unorm8,
    Uint16,
    Float16,
    Float32,
}

impl GpuTileFormat {
    pub const ALL: [Self; 4] = [Self::Unorm8, Self::Uint16, Self::Float16, Self::Float32];

    pub fn for_sample(sample: SampleType) -> Self {
        match sample {
            SampleType::U8 => Self::Unorm8,
            SampleType::U16 => Self::Uint16,
            SampleType::F16 => Self::Float16,
            SampleType::F32 => Self::Float32,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Unorm8 => wgpu::TextureFormat::Rgba8Unorm,
            Self::Uint16 => wgpu::TextureFormat::Rgba16Uint,
            Self::Float16 => wgpu::TextureFormat::Rgba16Float,
            Self::Float32 => wgpu::TextureFormat::Rgba32Float,
        }
    }

    pub fn texel_bytes(self) -> u32 {
        match self {
            Self::Unorm8 => 4,
            Self::Uint16 | Self::Float16 => 8,
            Self::Float32 => 16,
        }
    }

    pub fn tile_bytes(self) -> u64 {
        u64::from(TILE_SIZE) * u64::from(TILE_SIZE) * u64::from(self.texel_bytes())
    }

    pub fn binding_type(self) -> wgpu::TextureSampleType {
        match self {
            Self::Uint16 => wgpu::TextureSampleType::Uint,
            _ => wgpu::TextureSampleType::Float { filterable: false },
        }
    }
}

/// A stored tile as RGBA texels for its GPU format. RGBA tiles are uploaded as they are;
/// gray tiles are expanded (gray replicated, opaque alpha added): lossless.
pub(crate) fn gpu_texels<'a>(tile: &'a [u8], stored: PixelFormat) -> Cow<'a, [u8]> {
    let sample = stored.sample.bytes() as usize;
    let opaque: Vec<u8> = match stored.sample {
        SampleType::U8 => vec![255],
        SampleType::U16 => 65535u16.to_ne_bytes().to_vec(),
        SampleType::F16 => f32_to_f16(1.0).to_ne_bytes().to_vec(),
        SampleType::F32 => 1.0f32.to_ne_bytes().to_vec(),
    };
    match stored.layout {
        ChannelLayout::Rgba => Cow::Borrowed(tile),
        ChannelLayout::Gray | ChannelLayout::GrayAlpha => {
            let has_alpha = stored.layout.has_alpha();
            let in_px = if has_alpha { 2 * sample } else { sample };
            let mut out = Vec::with_capacity(tile.len() / in_px * 4 * sample);
            for px in tile.chunks_exact(in_px) {
                let gray = &px[..sample];
                out.extend_from_slice(gray);
                out.extend_from_slice(gray);
                out.extend_from_slice(gray);
                out.extend_from_slice(if has_alpha { &px[sample..] } else { &opaque });
            }
            Cow::Owned(out)
        }
        // Raster storage never keeps 3-channel tiles (RGB is stored as RGBA).
        ChannelLayout::Rgb => Cow::Borrowed(tile),
    }
}

#[derive(Debug)]
struct Slot {
    index: u32,
    last_used: u64,
}

#[derive(Debug)]
pub(crate) struct TileCache {
    format: GpuTileFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    slots: HashMap<TileKey, Slot>,
    free: Vec<u32>,
    frame: u64,
}

impl TileCache {
    pub fn new(device: &wgpu::Device, format: GpuTileFormat, capacity: u32) -> Self {
        let texture = create_array(device, format, capacity, "tile cache");
        let view = array_view(&texture);
        Self {
            format,
            texture,
            view,
            slots: HashMap::new(),
            free: (0..capacity).rev().collect(),
            frame: 0,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Start a frame: tiles used by earlier frames become candidates for eviction.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Slot holding `key` for this frame, uploading `texels` (called only on a miss; RGBA
    /// texels of this cache's format) if needed. `None` when every slot is already used by
    /// this frame.
    pub fn ensure<'a>(
        &mut self,
        queue: &wgpu::Queue,
        key: TileKey,
        texels: impl FnOnce() -> Cow<'a, [u8]>,
    ) -> Option<u32> {
        if let Some(slot) = self.slots.get_mut(&key) {
            slot.last_used = self.frame;
            return Some(slot.index);
        }
        let index = match self.free.pop() {
            Some(index) => index,
            None => self.evict()?,
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: index,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &texels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TILE_SIZE * self.format.texel_bytes()),
                rows_per_image: Some(TILE_SIZE),
            },
            wgpu::Extent3d {
                width: TILE_SIZE,
                height: TILE_SIZE,
                depth_or_array_layers: 1,
            },
        );
        self.slots.insert(
            key,
            Slot {
                index,
                last_used: self.frame,
            },
        );
        Some(index)
    }

    /// Free the least recently used slot not used by the current frame.
    fn evict(&mut self) -> Option<u32> {
        let (&key, _) = self
            .slots
            .iter()
            .filter(|(_, slot)| slot.last_used != self.frame)
            .min_by_key(|(_, slot)| slot.last_used)?;
        self.slots.remove(&key).map(|slot| slot.index)
    }
}

/// A one-layer placeholder bound when a frame has no raster layer of `format`.
pub(crate) fn placeholder_view(device: &wgpu::Device, format: GpuTileFormat) -> wgpu::TextureView {
    array_view(&create_array(device, format, 1, "tile placeholder"))
}

fn create_array(
    device: &wgpu::Device,
    format: GpuTileFormat,
    layers: u32,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: TILE_SIZE,
            height: TILE_SIZE,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: format.texture_format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("tile array"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}
