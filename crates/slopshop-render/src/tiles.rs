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
    let has_alpha = match stored.layout {
        ChannelLayout::Gray => false,
        ChannelLayout::GrayAlpha => true,
        // Raster storage never keeps 3-channel tiles (RGB is stored as RGBA).
        ChannelLayout::Rgba | ChannelLayout::Rgb => return Cow::Borrowed(tile),
    };
    Cow::Owned(match stored.sample {
        SampleType::U8 => expand_gray(tile, has_alpha, [255]),
        SampleType::U16 => expand_gray(tile, has_alpha, 65535u16.to_ne_bytes()),
        SampleType::F16 => expand_gray(tile, has_alpha, f32_to_f16(1.0).to_ne_bytes()),
        SampleType::F32 => expand_gray(tile, has_alpha, 1.0f32.to_ne_bytes()),
    })
}

/// Gray texels of `S` bytes per sample (with alpha after each when `has_alpha`) as RGBA: gray
/// replicated, alpha kept or `opaque`. A fixed sample size lets every copy compile to a move:
/// this runs for every gray tile uploaded.
fn expand_gray<const S: usize>(tile: &[u8], has_alpha: bool, opaque: [u8; S]) -> Vec<u8> {
    let (samples, _) = tile.as_chunks::<S>();
    let pixels = if has_alpha {
        samples.len() / 2
    } else {
        samples.len()
    };
    let mut out = vec![0; pixels * 4 * S];
    let (channels, _) = out.as_chunks_mut::<S>();
    let (texels, _) = channels.as_chunks_mut::<4>();
    if has_alpha {
        for (texel, &[gray, alpha]) in texels.iter_mut().zip(samples.as_chunks::<2>().0) {
            *texel = [gray, gray, gray, alpha];
        }
    } else {
        for (texel, &gray) in texels.iter_mut().zip(samples) {
            *texel = [gray, gray, gray, opaque];
        }
    }
    out
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
    /// Tiles uploaded since the cache was created (misses).
    uploads: u64,
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
            uploads: 0,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Tiles uploaded since the cache was created (misses), for frame statistics.
    pub fn uploads(&self) -> u64 {
        self.uploads
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
        self.uploads += 1;
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

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::color::{AlphaMode, ColorSpace};

    fn format(layout: ChannelLayout, sample: SampleType) -> PixelFormat {
        PixelFormat {
            layout,
            sample,
            color_space: ColorSpace::SRGB,
            alpha: AlphaMode::Straight,
        }
    }

    #[test]
    fn gray_texels_are_expanded_exactly() {
        let gray8 = format(ChannelLayout::Gray, SampleType::U8);
        assert_eq!(&*gpu_texels(&[7, 9], gray8), &[7, 7, 7, 255, 9, 9, 9, 255]);
        let gray_alpha8 = format(ChannelLayout::GrayAlpha, SampleType::U8);
        assert_eq!(&*gpu_texels(&[7, 3], gray_alpha8), &[7, 7, 7, 3]);
        let gray16 = format(ChannelLayout::Gray, SampleType::U16);
        let one = 0x1234u16.to_ne_bytes();
        let expanded: Vec<u8> = [one, one, one, 65535u16.to_ne_bytes()].concat();
        assert_eq!(&*gpu_texels(&one, gray16), &expanded[..]);
        let gray32 = format(ChannelLayout::GrayAlpha, SampleType::F32);
        let (v, a) = (0.25f32.to_ne_bytes(), 0.5f32.to_ne_bytes());
        let expanded: Vec<u8> = [v, v, v, a].concat();
        assert_eq!(&*gpu_texels(&[v, a].concat(), gray32), &expanded[..]);
        // RGBA tiles are uploaded as they are.
        let rgba = format(ChannelLayout::Rgba, SampleType::U8);
        assert!(matches!(gpu_texels(&[1, 2, 3, 4], rgba), Cow::Borrowed(_)));
    }
}
