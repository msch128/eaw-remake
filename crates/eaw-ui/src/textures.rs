//! UI texture access: the `MT_CommandBar` mega texture (with its `.mtd` directory) plus loose
//! files, decoded on the CPU and uploaded as individual Bevy images.
//!
//! Nearly every small UI texture of the command bar (`i_button_*.tga`, `i_bar_*`, unit icons ...)
//! is not a file of its own: the engine packs them into `MT_CommandBar.tga` (4096x4096) and
//! describes the rectangles in `MT_CommandBar.mtd`. [`TextureLibrary`] resolves a texture name
//! (`I_BUTTON_PAD_REBEL.tga`) first through that directory, then as a loose file below
//! `Art/Textures` (`.dds`/`.tga` are interchangeable, like in the original engine).

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

/// Size of one `.mtd` record: 64 byte name, `x y w h` as `u32`, one flag byte.
const MTD_RECORD: usize = 81;

/// One entry of a mega texture directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MtdEntry {
    /// Left edge in pixels.
    pub x: u32,
    /// Top edge in pixels (the mega texture is stored top-down).
    pub y: u32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

/// Parsed mega texture directory (`.mtd`), keyed by upper-cased texture name.
#[derive(Debug, Default, Clone)]
pub struct MegaTextureDirectory {
    /// All entries by upper-case name (including extension, e.g. `I_BUTTON_BUILD.TGA`).
    pub entries: HashMap<String, MtdEntry>,
}

impl MegaTextureDirectory {
    /// Parses the binary `.mtd` format: `u32 count`, then `count` records of
    /// `char name[64]; u32 x, y, w, h; u8 flag`.
    pub fn parse(data: &[u8]) -> Self {
        let mut entries = HashMap::new();
        let count = data
            .get(0..4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) as usize;
        for i in 0..count {
            let Some(rec) = data.get(4 + i * MTD_RECORD..4 + (i + 1) * MTD_RECORD) else {
                break;
            };
            let end = rec[..64].iter().position(|&b| b == 0).unwrap_or(64);
            let name = String::from_utf8_lossy(&rec[..end]).to_uppercase();
            let u = |o: usize| u32::from_le_bytes([rec[o], rec[o + 1], rec[o + 2], rec[o + 3]]);
            entries.insert(
                name,
                MtdEntry {
                    x: u(64),
                    y: u(68),
                    w: u(72),
                    h: u(76),
                },
            );
        }
        Self { entries }
    }
}

/// CPU-side texture access without any Bevy dependency on the GPU: usable from tools.
pub struct TextureLibrary {
    fs: Arc<GameFs>,
    mtd: MegaTextureDirectory,
    mega: Option<Rgba8>,
}

/// Normalizes a texture name to `STEM` (upper case, no extension, no directory).
fn stem_key(name: &str) -> String {
    let name = name.trim().replace('\\', "/");
    let file = name.rsplit('/').next().unwrap_or(&name);
    let stem = file.rsplit_once('.').map_or(file, |s| s.0);
    stem.to_uppercase()
}

impl TextureLibrary {
    /// Opens the library: reads `MT_CommandBar.mtd` and decodes `MT_CommandBar.tga`
    /// (falling back to the compressed `.dds`). Missing mega textures only mean that every
    /// lookup falls through to loose files.
    pub fn new(fs: Arc<GameFs>) -> Self {
        let mtd = fs
            .read(&format!("{TEXTURE_DIR}/MT_CommandBar.mtd"))
            .map(|d| MegaTextureDirectory::parse(&d))
            .unwrap_or_default();
        let mut by_stem = HashMap::new();
        for (k, v) in mtd.entries {
            by_stem.insert(stem_key(&k), v);
        }
        let mtd = MegaTextureDirectory { entries: by_stem };
        let mega = ["MT_CommandBar.tga", "MT_CommandBarCompressed.dds"]
            .iter()
            .filter_map(|n| fs.read(&format!("{TEXTURE_DIR}/{n}")).ok())
            .find_map(|d| decode_texture(&d).ok());
        Self { fs, mtd, mega }
    }

    /// The game file system this library reads from.
    pub fn fs(&self) -> &Arc<GameFs> {
        &self.fs
    }

    /// Returns true if `name` is packed into the mega texture.
    pub fn in_mega_texture(&self, name: &str) -> bool {
        self.mtd.entries.contains_key(&stem_key(name))
    }

    /// Decodes a texture by name (mega texture first, then loose `.dds`/`.tga`).
    pub fn load(&self, name: &str) -> Option<Rgba8> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let (Some(e), Some(mega)) = (self.mtd.entries.get(&stem_key(name)), &self.mega) {
            return Some(crop(mega, e));
        }
        let stem = name
            .rsplit_once('.')
            .map_or(name, |s| s.0)
            .replace('\\', "/");
        [name.to_string(), format!("{stem}.dds"), format!("{stem}.tga")]
            .iter()
            .find_map(|n| self.fs.read(&format!("{TEXTURE_DIR}/{n}")).ok())
            .and_then(|d| decode_texture(&d).ok())
    }
}

fn crop(src: &Rgba8, e: &MtdEntry) -> Rgba8 {
    let (x0, y0) = (e.x as usize, e.y as usize);
    let w = (e.w as usize).min(src.width.saturating_sub(x0));
    let h = (e.h as usize).min(src.height.saturating_sub(y0));
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let o = ((y0 + y) * src.width + x0) * 4;
        pixels.extend_from_slice(&src.pixels[o..o + w * 4]);
    }
    Rgba8 {
        width: w,
        height: h,
        pixels,
    }
}

/// A texture ready for Bevy UI.
#[derive(Debug, Clone)]
pub struct UiTexture {
    /// GPU image.
    pub handle: Handle<Image>,
    /// Size in texture pixels.
    pub size: UVec2,
}

/// Cache of UI textures as Bevy images (resource).
#[derive(Resource)]
pub struct UiTextures {
    library: TextureLibrary,
    cache: HashMap<String, Option<UiTexture>>,
}

impl UiTextures {
    /// Creates the cache on top of a game file system.
    pub fn new(fs: Arc<GameFs>) -> Self {
        Self {
            library: TextureLibrary::new(fs),
            cache: HashMap::new(),
        }
    }

    /// The CPU-side library.
    pub fn library(&self) -> &TextureLibrary {
        &self.library
    }

    /// Loads (and caches) a texture by game name. Returns `None` if the texture does not exist.
    pub fn get(&mut self, images: &mut Assets<Image>, name: &str) -> Option<UiTexture> {
        let key = stem_key(name);
        if key.is_empty() {
            return None;
        }
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let tex = self
            .library
            .load(name)
            .map(|rgba| upload(images, rgba));
        self.cache.insert(key, tex.clone());
        tex
    }

    /// Registers an already decoded image under `key` (for generated textures such as faceplates).
    pub fn insert(&mut self, images: &mut Assets<Image>, key: &str, rgba: Rgba8) -> UiTexture {
        let tex = upload(images, rgba);
        self.cache.insert(stem_key(key), Some(tex.clone()));
        tex
    }
}

/// Uploads an RGBA image as a filtered, clamped sRGB UI texture.
pub fn upload(images: &mut Assets<Image>, rgba: Rgba8) -> UiTexture {
    let size = UVec2::new(rgba.width as u32, rgba.height as u32);
    let mut image = Image::new(
        Extent3d {
            width: size.x.max(1),
            height: size.y.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba.pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    UiTexture {
        handle: images.add(image),
        size,
    }
}
