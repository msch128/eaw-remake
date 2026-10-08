//! Texture loading from the game file system into Bevy [`Image`]s.
//!
//! DDS files with DXT1/3/5 data are uploaded as BC1/2/3 without decompression, keeping the mip
//! chain of the file. Everything else (TGA, uncompressed DDS, block-compressed textures whose
//! size is not a multiple of four) is decoded to RGBA8 and gets a CPU-generated mip chain.
//!
//! Textures are always uploaded as *linear* (non-sRGB) formats: the original engine shades in gamma
//! space, so the shaders work directly on the stored values and convert once at the very end.

use std::collections::HashMap;
use std::sync::Arc;

use alamo_fs::{FileSource, GameFs};
use alamo_model::image::{Rgba8, decode_texture};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Directory textures are looked up in.
const TEXTURE_DIR: &str = "Art/Textures";

fn sampler() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    })
}

/// Reads `name` (e.g. `W_Rock.tga`) from the texture directory. The extension is not trusted:
/// the same stem is tried as `.dds` and `.tga` too, as the original engine did.
pub fn read_texture_file(fs: &GameFs, name: &str) -> Option<Vec<u8>> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let stem = name.rsplit_once('.').map_or(name, |s| s.0);
    let mut stems = vec![stem.to_string()];
    // Some `_gc` (galactic) variants referenced by models were never shipped; the plain texture
    // is the closest match.
    if let Some(plain) = stem
        .strip_suffix("_gc")
        .or_else(|| stem.strip_suffix("_GC"))
    {
        stems.push(plain.to_string());
    }
    let mut candidates = vec![name.to_string()];
    for s in &stems {
        for ext in ["dds", "tga", "png"] {
            candidates.push(format!("{s}.{ext}"));
        }
    }
    candidates
        .iter()
        .find_map(|n| fs.read(&format!("{TEXTURE_DIR}/{n}")).ok())
}
/// Builds a Bevy image from the bytes of a DDS or TGA file.
pub fn image_from_bytes(data: &[u8]) -> Result<Image, String> {
    if data.starts_with(b"DDS ")
        && let Some(img) = dds_compressed(data)
    {
        return Ok(img);
    }
    let rgba = decode_texture(data).map_err(|e| e.to_string())?;
    Ok(rgba_with_mips(&rgba))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// Uploads DXT1/3/5 data directly; `None` if the file is not such a DDS.
fn dds_compressed(d: &[u8]) -> Option<Image> {
    let height = u32_at(d, 12)?;
    let width = u32_at(d, 16)?;
    let mips = u32_at(d, 28)?.max(1);
    let pf_flags = u32_at(d, 80)?;
    if pf_flags & 4 == 0 || width == 0 || height == 0 || width % 4 != 0 || height % 4 != 0 {
        return None;
    }
    let (format, block) = match d.get(84..88)? {
        b"DXT1" => (TextureFormat::Bc1RgbaUnorm, 8usize),
        b"DXT3" => (TextureFormat::Bc2RgbaUnorm, 16),
        b"DXT5" => (TextureFormat::Bc3RgbaUnorm, 16),
        _ => return None,
    };
    let body = d.get(128..)?;
    // Count the mips that are fully present.
    let (mut used, mut total) = (0u32, 0usize);
    for level in 0..mips {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        let size = (w.div_ceil(4) * h.div_ceil(4)) as usize * block;
        if total + size > body.len() {
            break;
        }
        total += size;
        used += 1;
    }
    if used == 0 {
        return None;
    }
    let mut img = Image::new_uninit(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.data = Some(body[..total].to_vec());
    img.texture_descriptor.mip_level_count = used;
    img.sampler = sampler();
    Some(img)
}

/// Uploads an RGBA8 image with a box-filtered mip chain.
pub fn rgba_with_mips(src: &Rgba8) -> Image {
    let (mut w, mut h) = (src.width, src.height);
    let mut data = src.pixels.clone();
    let mut level = src.pixels.clone();
    let mut levels = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                let (x0, y0) = ((x * 2).min(w - 1), (y * 2).min(h - 1));
                let (x1, y1) = ((x * 2 + 1).min(w - 1), (y * 2 + 1).min(h - 1));
                for c in 0..4 {
                    let sum: u32 = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
                        .iter()
                        .map(|&(px, py)| level[(py * w + px) * 4 + c] as u32)
                        .sum();
                    next[(y * nw + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    let mut img = Image::new_uninit(
        Extent3d {
            width: src.width as u32,
            height: src.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.data = Some(data);
    img.texture_descriptor.mip_level_count = levels;
    img.sampler = sampler();
    img
}

/// 1x1 image of a constant color (RGBA8, linear).
pub fn solid(rgba: [u8; 4]) -> Image {
    let mut img = Image::new_uninit(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.data = Some(rgba.to_vec());
    img.sampler = sampler();
    img
}

/// Texture cache shared by all models: one GPU image per texture name.
#[derive(Resource, Default)]
pub struct TextureCache {
    by_name: HashMap<String, Option<Handle<Image>>>,
    fallback: Option<Handle<Image>>,
    fallback_grey: Option<Handle<Image>>,
    /// Names that could not be loaded (for diagnostics).
    pub missing: Vec<String>,
}

impl TextureCache {
    /// Loads (or returns the cached) texture `name`. `None` if the file is missing or undecodable.
    pub fn get(
        &mut self,
        fs: &Arc<GameFs>,
        images: &mut Assets<Image>,
        name: &str,
    ) -> Option<Handle<Image>> {
        let key = name.to_ascii_lowercase();
        if let Some(h) = self.by_name.get(&key) {
            return h.clone();
        }
        let loaded = read_texture_file(fs, name)
            .ok_or_else(|| "file not found".to_string())
            .and_then(|d| image_from_bytes(&d))
            .map(|img| images.add(img));
        let handle = match loaded {
            Ok(h) => Some(h),
            Err(e) => {
                warn!("texture {name}: {e}");
                self.missing.push(name.to_string());
                None
            }
        };
        self.by_name.insert(key, handle.clone());
        handle
    }

    /// Registers a texture that was decoded off the main thread (`None` = missing or undecodable).
    pub fn insert_decoded(&mut self, name: &str, image: Option<Image>, images: &mut Assets<Image>) {
        let key = name.to_ascii_lowercase();
        if self.by_name.contains_key(&key) {
            return;
        }
        if image.is_none() {
            warn!("texture {name}: missing or undecodable, using the neutral fallback");
            self.missing.push(name.to_string());
        }
        self.by_name.insert(key, image.map(|i| images.add(i)));
    }
    /// Neutral placeholder for textures that cannot be loaded: black for additive (sky, glow) shaders, mid grey
    /// otherwise, never a loud colour (the miss is logged).
    pub fn fallback(&mut self, images: &mut Assets<Image>, black: bool) -> Handle<Image> {
        let slot = if black { &mut self.fallback } else { &mut self.fallback_grey };
        slot.get_or_insert_with(|| images.add(solid(if black { [0, 0, 0, 255] } else { [128, 128, 128, 255] }))).clone()
    }

    /// Number of distinct textures requested so far.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// True if no texture was requested yet.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_length() {
        let src = Rgba8 {
            width: 8,
            height: 4,
            pixels: vec![255; 8 * 4 * 4],
        };
        let img = rgba_with_mips(&src);
        assert_eq!(img.texture_descriptor.mip_level_count, 4);
        // 8x4 + 4x2 + 2x1 + 1x1 pixels
        assert_eq!(img.data.unwrap().len(), (32 + 8 + 2 + 1) * 4);
    }

    #[test]
    fn dds_dxt1_keeps_blocks() {
        let mut d = vec![0u8; 128];
        d[..4].copy_from_slice(b"DDS ");
        d[12..16].copy_from_slice(&8u32.to_le_bytes());
        d[16..20].copy_from_slice(&8u32.to_le_bytes());
        d[28..32].copy_from_slice(&2u32.to_le_bytes());
        d[80..84].copy_from_slice(&4u32.to_le_bytes());
        d[84..88].copy_from_slice(b"DXT1");
        d.extend(vec![0u8; 4 * 8 + 8]);
        let img = image_from_bytes(&d).unwrap();
        assert_eq!(img.texture_descriptor.format, TextureFormat::Bc1RgbaUnorm);
        assert_eq!(img.texture_descriptor.mip_level_count, 2);
        assert_eq!(img.data.unwrap().len(), 40);
    }
}
