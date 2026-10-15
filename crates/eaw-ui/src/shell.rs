//! Shell models: the placement data of the interface.
//!
//! Every screen of the original UI (galactic command bar, tactical command bar, reinforcement
//! window, hero frame ...) is an `.alo` model made of flat quads. Hidden quads named like a
//! command bar component (`b_zoom`, `queue03`, `Text_Credits` ...) mark the rectangle of that
//! component, visible meshes (`*_Faceplate_ALT0/1/2`) are the frame art, textured from a
//! `I_GALACTIC_DASHBOARD_*.dds` sheet. All coordinates are in *design pixels* of the original
//! 1366x768 layout, x to the right, **y up**, origin at the bottom-left corner of the bar.

use std::collections::HashMap;

use alamo_fs::{FileSource, GameFs};
use alamo_model::Model;
use alamo_model::image::Rgba8;
use bevy::math::Vec2;

use crate::faction::UiFaction;

/// Reference screen height the original layouts were authored for.
pub const REFERENCE_HEIGHT: f32 = 768.0;

/// A named rectangle of a shell (centre and size in design pixels, y up).
#[derive(Debug, Clone, PartialEq)]
pub struct ShellItem {
    /// Mesh/bone name (component name, original capitalization).
    pub name: String,
    /// Centre of the rectangle.
    pub center: Vec2,
    /// Size of the rectangle.
    pub size: Vec2,
    /// Depth of the bone (higher is in front).
    pub z: f32,
}

/// A vertex of a frame art triangle.
#[derive(Debug, Clone, Copy)]
pub struct ArtVertex {
    /// Position in design pixels, y up.
    pub pos: Vec2,
    /// Texture coordinate (0,0 = top-left).
    pub uv: Vec2,
}

/// A visible, textured mesh of a shell (frame art).
#[derive(Debug, Clone)]
pub struct ShellArt {
    /// Mesh name, e.g. `Empire_Faceplate_ALT0`.
    pub name: String,
    /// Faction variant from the `_ALTn` name suffix (`None` if the art is shared).
    pub alt: Option<usize>,
    /// Base texture file, e.g. `I_galactic_dashboard_wide.dds`.
    pub texture: String,
    /// Triangles.
    pub triangles: Vec<[ArtVertex; 3]>,
    /// Depth (higher is in front).
    pub z: f32,
}

impl ShellArt {
    /// Bounding box `(min, max)` in design pixels.
    pub fn bounds(&self) -> (Vec2, Vec2) {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for v in self.triangles.iter().flatten() {
            lo = lo.min(v.pos);
            hi = hi.max(v.pos);
        }
        (lo, hi)
    }
}

/// Parsed shell model.
#[derive(Debug, Clone, Default)]
pub struct ShellLayout {
    /// Model file name.
    pub model: String,
    /// Component rectangles by lower-cased name.
    pub items: HashMap<String, ShellItem>,
    /// Item names in model order.
    pub order: Vec<String>,
    /// Frame art meshes.
    pub art: Vec<ShellArt>,
}

impl ShellLayout {
    /// Loads and parses `Art/Models/<model>`.
    pub fn load(fs: &GameFs, model: &str) -> Option<Self> {
        let data = fs.read(&format!("Art/Models/{model}")).ok()?;
        Self::from_model(model, &Model::parse(&data).ok()?)
    }

    /// Extracts the layout from a parsed model.
    pub fn from_model(name: &str, m: &Model) -> Option<Self> {
        let mut out = ShellLayout {
            model: name.to_string(),
            ..Default::default()
        };
        for (i, mesh) in m.meshes.iter().enumerate() {
            let t = m.world_transform(m.bone_of_object(i));
            let place = |p: [f32; 3]| {
                Vec2::new(
                    t[0][0] * p[0] + t[0][1] * p[1] + t[0][2] * p[2] + t[0][3],
                    t[1][0] * p[0] + t[1][1] * p[1] + t[1][2] * p[2] + t[1][3],
                )
            };
            let z = t[2][3];
            let textured = mesh
                .sub_meshes
                .iter()
                .find_map(|s| s.material.texture("BaseTexture").map(|n| (s, n)));
            if !mesh.hidden
                && let Some((sub, tex)) = textured
            {
                let triangles = sub
                    .indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .filter_map(|c| {
                        let v = |k: u16| {
                            sub.vertices.get(k as usize).map(|v| ArtVertex {
                                pos: place(v.position),
                                uv: Vec2::new(v.uv[0][0], v.uv[0][1]),
                            })
                        };
                        Some([v(c[0])?, v(c[1])?, v(c[2])?])
                    })
                    .collect();
                out.art.push(ShellArt {
                    name: mesh.name.clone(),
                    alt: alt_suffix(&mesh.name),
                    texture: tex.to_string(),
                    triangles,
                    z,
                });
                continue;
            }
            let (lo, hi) = (mesh.bbox_min, mesh.bbox_max);
            let c = place([
                (lo[0] + hi[0]) * 0.5,
                (lo[1] + hi[1]) * 0.5,
                (lo[2] + hi[2]) * 0.5,
            ]);
            let item = ShellItem {
                name: mesh.name.clone(),
                center: c,
                size: Vec2::new((hi[0] - lo[0]).abs(), (hi[1] - lo[1]).abs()),
                z,
            };
            out.order.push(item.name.clone());
            out.items.insert(item.name.to_lowercase(), item);
        }
        // `r_pop_text` / `r_pop_icon` of the reinforcement window are bones without a mesh (so no collision rectangle): the game places
        // them by their bone. Their sizes are estimates (the text runs to the window's right edge, the icon is about 22 px).
        for (i, bone) in m.bones.iter().enumerate() {
            let lname = bone.name.to_lowercase();
            let size = match lname.as_str() {
                "r_pop_text" => Vec2::new(70.0, 16.0),
                "r_pop_icon" => Vec2::new(22.0, 22.0),
                _ => continue,
            };
            if out.items.contains_key(&lname) {
                continue;
            }
            let t = m.world_transform(i);
            let item = ShellItem { name: bone.name.clone(), center: Vec2::new(t[0][3], t[1][3]), size, z: t[2][3] };
            out.order.push(item.name.clone());
            out.items.insert(lname, item);
        }
        Some(out)
    }

    /// Looks up a component rectangle by name (case-insensitive).
    pub fn item(&self, name: &str) -> Option<&ShellItem> {
        self.items.get(&name.to_lowercase())
    }

    /// The frame art that applies to `faction`: shared art plus the matching `_ALTn` variant.
    pub fn art_for(&self, faction: UiFaction) -> impl Iterator<Item = &ShellArt> {
        let alt = faction.mesh_alt();
        self.art
            .iter()
            .filter(move |a| a.alt.is_none_or(|n| n == alt))
    }

    /// Bounding box of the art for `faction` in design pixels.
    pub fn art_bounds(&self, faction: UiFaction) -> (Vec2, Vec2) {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for a in self.art_for(faction) {
            let (l, h) = a.bounds();
            lo = lo.min(l);
            hi = hi.max(h);
        }
        (lo, hi)
    }
}

fn alt_suffix(name: &str) -> Option<usize> {
    let (_, n) = name.to_lowercase().rsplit_once("_alt").map(|(a, b)| (a.to_string(), b.to_string()))?;
    n.parse().ok()
}

/// Bilinear texture sample with clamping; returns straight RGBA 0..255 as floats.
fn sample(tex: &Rgba8, u: f32, v: f32) -> [f32; 4] {
    let x = (u * tex.width as f32 - 0.5).clamp(0.0, tex.width as f32 - 1.0);
    let y = (v * tex.height as f32 - 0.5).clamp(0.0, tex.height as f32 - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(tex.width - 1), (y0 + 1).min(tex.height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |xx: usize, yy: usize| {
        let o = (yy * tex.width + xx) * 4;
        [
            tex.pixels[o] as f32,
            tex.pixels[o + 1] as f32,
            tex.pixels[o + 2] as f32,
            tex.pixels[o + 3] as f32,
        ]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut o = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        o[k] = top + (bot - top) * fy;
    }
    o
}

/// Rasterizes textured triangles into a new RGBA image.
///
/// `lo`/`hi` is the covered rectangle in design pixels, `oversample` the number of image pixels
/// per design pixel. The faceplates are bilinear-filtered CPU renders of the original meshes, so
/// the result is exactly the geometry of the game at any chosen resolution.
pub fn rasterize(
    triangles: &[[ArtVertex; 3]],
    tex: &Rgba8,
    lo: Vec2,
    hi: Vec2,
    oversample: f32,
) -> Rgba8 {
    let w = (((hi.x - lo.x) * oversample).ceil() as usize).max(1);
    let h = (((hi.y - lo.y) * oversample).ceil() as usize).max(1);
    let mut acc = vec![0.0f32; w * h * 4];
    let mut covered = vec![false; w * h];
    let to_px = |p: Vec2| Vec2::new((p.x - lo.x) * oversample, (hi.y - p.y) * oversample);
    for tri in triangles {
        let p = [to_px(tri[0].pos), to_px(tri[1].pos), to_px(tri[2].pos)];
        let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        if area.abs() < 1e-6 {
            continue;
        }
        let min = p[0].min(p[1]).min(p[2]);
        let max = p[0].max(p[1]).max(p[2]);
        let (x0, x1) = (min.x.floor().max(0.0) as usize, (max.x.ceil() as usize).min(w));
        let (y0, y1) = (min.y.floor().max(0.0) as usize, (max.y.ceil() as usize).min(h));
        for y in y0..y1 {
            for x in x0..x1 {
                let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = (p[1] - q).perp_dot(p[2] - q) / area;
                let w1 = (p[2] - q).perp_dot(p[0] - q) / area;
                let w2 = 1.0 - w0 - w1;
                const EPS: f32 = -1e-4;
                if w0 < EPS || w1 < EPS || w2 < EPS || covered[y * w + x] {
                    continue;
                }
                covered[y * w + x] = true;
                let uv = tri[0].uv * w0 + tri[1].uv * w1 + tri[2].uv * w2;
                let s = sample(tex, uv.x, uv.y);
                let o = (y * w + x) * 4;
                acc[o..o + 4].copy_from_slice(&s);
            }
        }
    }
    Rgba8 {
        width: w,
        height: h,
        pixels: acc.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn quad(lo: Vec2, hi: Vec2, uv_lo: Vec2, uv_hi: Vec2) -> Vec<[ArtVertex; 3]> {
        let v = |x: f32, y: f32, u: f32, w: f32| ArtVertex {
            pos: Vec2::new(x, y),
            uv: Vec2::new(u, w),
        };
        let (a, b, c, d) = (
            v(lo.x, hi.y, uv_lo.x, uv_lo.y),
            v(hi.x, hi.y, uv_hi.x, uv_lo.y),
            v(hi.x, lo.y, uv_hi.x, uv_hi.y),
            v(lo.x, lo.y, uv_lo.x, uv_hi.y),
        );
        vec![[a, b, c], [a, c, d]]
    }

    #[test]
    fn alt_suffix_parses() {
        assert_eq!(alt_suffix("Rebel_Faceplate_ALT1"), Some(1));
        assert_eq!(alt_suffix("b_Help_Droid_Empire_ALT0"), Some(0));
        assert_eq!(alt_suffix("Text_Planet"), None);
        assert_eq!(alt_suffix("thing_ALTx"), None);
    }

    #[test]
    fn rasterize_maps_uv_and_flips_y() {
        // 2x2 texture: red, green / blue, white.
        let tex = Rgba8 {
            width: 2,
            height: 2,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 0, 255, 255, 255, 255, 255, 255,
            ],
        };
        let tris = quad(Vec2::ZERO, Vec2::new(2.0, 2.0), Vec2::ZERO, Vec2::ONE);
        let img = rasterize(&tris, &tex, Vec2::ZERO, Vec2::new(2.0, 2.0), 1.0);
        assert_eq!((img.width, img.height), (2, 2));
        // Top-left of the image (y up in the mesh, uv (0,0) = top-left of the texture) is red.
        assert_eq!(&img.pixels[0..4], &[255, 0, 0, 255]);
        // Bottom-right is white.
        assert_eq!(&img.pixels[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn rasterize_oversample_scales_the_image() {
        let tex = Rgba8 { width: 1, height: 1, pixels: vec![10, 20, 30, 255] };
        let tris = quad(Vec2::new(5.0, 5.0), Vec2::new(8.0, 7.0), Vec2::ZERO, Vec2::ONE);
        let img = rasterize(&tris, &tex, Vec2::new(5.0, 5.0), Vec2::new(8.0, 7.0), 2.0);
        assert_eq!((img.width, img.height), (6, 4));
        assert!(img.pixels.chunks(4).all(|p| p == [10, 20, 30, 255]));
    }

    #[test]
    fn art_filtering_by_faction() {
        let art = |name: &str, alt| ShellArt {
            name: name.into(),
            alt,
            texture: String::new(),
            triangles: quad(Vec2::ZERO, Vec2::ONE, Vec2::ZERO, Vec2::ONE),
            z: 0.0,
        };
        let layout = ShellLayout {
            art: vec![art("a", Some(0)), art("b", Some(1)), art("c", None)],
            ..Default::default()
        };
        let names = |f| layout.art_for(f).map(|a| a.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(UiFaction::Empire), ["a", "c"]);
        assert_eq!(names(UiFaction::Rebel), ["b", "c"]);
        assert_eq!(names(UiFaction::Underworld), ["c"]);
    }
}

