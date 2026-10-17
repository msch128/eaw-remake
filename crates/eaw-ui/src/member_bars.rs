//! Health bars and flags of single small craft (fighters, bombers, squadron members).
//!
//! The Remake draws one small health bar and one flag under every fighter, own and enemy, at every
//! zoom (the user's 1080p video `battle1s` b0703s, b0732s, b0850s, b0760s and the 3821x2047
//! screenshot `07_tactical_battle`). Ours show only on the craft of selected or hovered squadrons (the user's rule 2026-10-17, the host
//! decides: `battle-view` `tactical_ui::member_bars`); this module draws whatever [`MemberBars`] holds. The art is the game's own, from `MT_CommandBar`:
//!
//! * the bar is `I_BAR_CONTROL_WIDE` (a black track, alpha 0.31) with the `I_BAR_HEALTH_SMALL<level>`
//!   fill on it, `level` = health in tenths: flat colours, red 1-3, orange 4-5, yellow 6-7, yellow-green
//!   8-9, green 10, black below. The fill is as wide as the health fraction (b0866s: a bar at 80 % has
//!   the dark track right of the fill). A shield bar (`I_BAR_SHIELD_SMALL10`, cyan) sits above it;
//! * the flag is `I_GARRISON_FLAG` (12 x 14 px: white pole, grey pennant).
//!
//! Sizes measured on the references, in design pixels (1/768 of the screen height, the unit the rest
//! of this crate scales): the bar is 20.6 long and 1.2 thick (29 x 2 px in the 1080p video, 55 x 3 px at
//! 2047 px height: the 34 x 2 px small texture at 0.61), about as wide as the craft's bounding sphere
//! is across and centred on the craft; its top edge is 1 design px under the bottom of a TIE's
//! silhouette (the sphere's bottom: half a bar length below the craft's centre). The flag is the
//! texture at 0.92 of its size, its top at the bar's top and its left 1.3 design px inside the right end
//! of the bar.
//!
//! Hundreds of fighters fight in one battle, so none of this is UI nodes (a node tree per craft costs
//! layout and extraction every frame), and not one UI item per rectangle either (bevy_ui keeps a render
//! entity, a phase item and a sort entry for each, about 2 us a piece): the host fills [`MemberBars`],
//! [`build_member_quads`] turns it into rectangles of one small atlas (an opaque white strip that is
//! tinted into bars of any length and colour, plus the flag at the size it is drawn at), and
//! [`extract_member_quads`] hands them to the UI batcher as two runs of text glyphs, which bevy_ui
//! draws as one item each: no entities, no layout, one draw call, whatever the number of craft. The
//! runs sit just below the command bar, so the interface covers a craft that is behind it. This uses
//! the public extraction types of `bevy_ui_render` (`ExtractedUiNodes`, `ExtractedGlyph`); Bevy is
//! pinned to 0.19.1, look at them again when it moves.

use alamo_model::image::Rgba8;
use bevy::asset::RenderAssetUsages;
use bevy::color::LinearRgba;
use bevy::image::ImageSampler;
use bevy::math::{Affine2, Rect};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::sync_world::{MainEntity, TemporaryRenderEntity};
use bevy::render::{Extract, ExtractSchedule, RenderApp};
use bevy::ui::{ComputedStackIndex, ComputedUiTargetCamera};
use bevy::ui_render::{
    ExtractedGlyph, ExtractedUiItem, ExtractedUiNode, ExtractedUiNodes, RenderUiSystems,
    UiCameraMap,
};

use crate::tactical::TacticalOverlayRoot;
use crate::textures::UiTextures;
use crate::widgets::EawCommandBar;

/// Bar length as a share of the craft's drawn bounding sphere (diameter). Estimate from two measurements: the bar is 20.6 design px in
/// the Remake, whose small craft are drawn 24.6 design px across. Our craft are drawn at their model scale, a few pixels across at most, so
/// the bar is the craft's width only close up and [`MIN_BAR`] farther out.
pub const BAR_OF_SPHERE: f32 = 0.84;
/// Bar thickness in design px: the small bar texture's 2 px at the 0.61 its length is drawn at (29 x 2 px at 1080p, 55 x 3 px at 2047 px).
pub const BAR_THICKNESS: f32 = 1.21;
/// Shortest bar in design px, so far-away craft keep a readable bar (estimate).
pub const MIN_BAR: f32 = 8.0;
/// Flag size as a share of its texture (measured 0.94 in the screenshot, 0.89 in the video).
pub const FLAG_SCALE: f32 = 0.92;
/// How far the flag texture reaches left of the bar's right end, in design px (measured on the screenshot: 3.4 of 2.67 px per design px).
pub const FLAG_OVERLAP: f32 = 1.28;

/// Size of the opaque white strip of the atlas in pixels: no bar is longer or thicker than this (longer ones are cut).
const WHITE_STRIP: (u32, u32) = (512, 8);

/// The bar and flag of one craft.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberBar {
    /// Stable id of the craft (the order of overlapping bars).
    pub id: u32,
    /// Centre of the craft's drawn bounding sphere in logical window pixels (y down).
    pub pos: Vec2,
    /// Radius of that sphere on screen, in the same pixels.
    pub radius: f32,
    /// Hull 0..1.
    pub health: f32,
    /// Shield 0..1 for craft that have shields.
    pub shield: Option<f32>,
}

/// Every small craft that shows a bar and a flag this frame (resource, filled by the host each frame).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct MemberBars {
    /// Physical pixels per logical pixel of the render target (the camera's scaling factor); 0 counts as 1.
    pub scale_factor: f32,
    /// The craft, in drawing order.
    pub items: Vec<MemberBar>,
}

/// Colours and the flag of the bars, read from the game's textures.
#[derive(Debug, Clone)]
pub struct BarLook {
    /// `I_BAR_CONTROL_WIDE`: the track every bar sits on.
    pub track: LinearRgba,
    /// `I_BAR_HEALTH_SMALL0..10`.
    pub health: [LinearRgba; 11],
    /// `I_BAR_SHIELD_SMALL10`.
    pub shield: LinearRgba,
    /// `I_GARRISON_FLAG` at its own size, `None` when the game has no such texture.
    pub flag: Option<Rgba8>,
}

fn srgba(r: u8, g: u8, b: u8, a: u8) -> LinearRgba {
    Color::srgba_u8(r, g, b, a).to_linear()
}

impl Default for BarLook {
    /// The colours of the Remake's textures (read from `MT_CommandBar.tga`), for tools and tests without the game.
    fn default() -> Self {
        let level = |l: usize| match l {
            0 => srgba(0, 0, 0, 0x49),
            1..=3 => srgba(0xf6, 0, 0, 0xf6),
            4 | 5 => srgba(0xf6, 0x8a, 0, 0xf6),
            6 | 7 => srgba(0xf6, 0xe1, 0, 0xf6),
            8 | 9 => srgba(0xc3, 0xf6, 0, 0xf6),
            _ => srgba(0x3d, 0xec, 0x3d, 0xdd),
        };
        Self {
            track: srgba(0, 0, 0, 0x50),
            health: std::array::from_fn(level),
            shield: srgba(0x25, 0xb9, 0xee, 0xdd),
            flag: None,
        }
    }
}

impl BarLook {
    /// Reads the colours and the flag from the game's textures (the mega texture must be decoded).
    pub fn from_textures(textures: &UiTextures) -> Self {
        let mut look = Self::default();
        let library = textures.library();
        let first_pixel = |name: &str| {
            library
                .load(name)
                .filter(|t| t.pixels.len() >= 4)
                .map(|t| srgba(t.pixels[0], t.pixels[1], t.pixels[2], t.pixels[3]))
        };
        if let Some(c) = first_pixel("i_bar_control_wide.tga") {
            look.track = c;
        }
        if let Some(c) = first_pixel("i_bar_shield_small10.tga") {
            look.shield = c;
        }
        for (l, slot) in look.health.iter_mut().enumerate() {
            if let Some(c) = first_pixel(&format!("i_bar_health_small{l}.tga")) {
                *slot = c;
            }
        }
        look.flag = library
            .load("i_garrison_flag.tga")
            .filter(|t| t.width > 0 && t.height > 0 && t.pixels.len() >= t.width * t.height * 4);
        look
    }

    /// Fill colour of a bar at `health` (0..1): the texture of its level in tenths.
    pub fn health_color(&self, health: f32) -> LinearRgba {
        self.health[health_level(health)]
    }

    /// The flag as it is drawn, in physical pixels (`None` without the texture).
    pub fn flag_size(&self, ui_phys: f32) -> Option<UVec2> {
        self.flag.as_ref().map(|f| {
            UVec2::new(
                (f.width as f32 * FLAG_SCALE * ui_phys).round().max(1.0) as u32,
                (f.height as f32 * FLAG_SCALE * ui_phys).round().max(1.0) as u32,
            )
        })
    }
}

/// Health in tenths, 0..=10 (the level texture of a bar).
pub fn health_level(health: f32) -> usize {
    if health.is_finite() {
        (health.clamp(0.0, 1.0) * 10.0).round() as usize
    } else {
        0
    }
}

/// Where the parts of the atlas are, in atlas pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasLayout {
    /// Size of the atlas image.
    pub size: UVec2,
    /// The opaque white strip every bar is a piece of.
    pub white: Rect,
    /// The flag at the size it is drawn at.
    pub flag: Option<Rect>,
}

/// The atlas: an opaque white strip (tinted by the quad colour it becomes a bar of any colour, cut to any length up to its width) and
/// the flag resampled to `flag_px`, the size it is drawn at, so every quad shows its texels one to one.
pub fn build_atlas(flag: Option<&Rgba8>, flag_px: Option<UVec2>) -> (Image, AtlasLayout) {
    let (strip_w, strip_h) = WHITE_STRIP;
    let flag_px = flag.zip(flag_px).filter(|(_, px)| px.x > 0 && px.y > 0);
    let (w, h) = (
        strip_w.max(flag_px.map_or(0, |(_, px)| px.x)),
        strip_h + flag_px.map_or(0, |(_, px)| px.y),
    );
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..strip_h {
        data[(y * w * 4) as usize..((y * w + strip_w) * 4) as usize].fill(255);
    }
    let flag_rect = flag_px.map(|(src, px)| {
        let rect = Rect::new(0.0, strip_h as f32, px.x as f32, (strip_h + px.y) as f32);
        resample(src, px, &mut data, w, strip_h);
        rect
    });
    let mut image = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // One texel per screen pixel: no filtering, so a bar's edge is as sharp as the rectangle it is cut from.
    image.sampler = ImageSampler::nearest();
    let layout = AtlasLayout {
        size: UVec2::new(w, h),
        white: Rect::new(0.0, 0.0, strip_w as f32, strip_h as f32),
        flag: flag_rect,
    };
    (image, layout)
}

/// Bilinear resample of `src` to `px` into `data` (RGBA8, `stride` pixels wide) from row `top` on. Colours are weighted by alpha, so the
/// transparent black around the flag does not darken its edge.
fn resample(src: &Rgba8, px: UVec2, data: &mut [u8], stride: u32, top: u32) {
    let (sw, sh) = (src.width as i32, src.height as i32);
    let at = |x: i32, y: i32| {
        let o = (y.clamp(0, sh - 1) as usize * src.width + x.clamp(0, sw - 1) as usize) * 4;
        let a = f32::from(src.pixels[o + 3]) / 255.0;
        [
            f32::from(src.pixels[o]) * a,
            f32::from(src.pixels[o + 1]) * a,
            f32::from(src.pixels[o + 2]) * a,
            a,
        ]
    };
    for y in 0..px.y {
        let fy = ((y as f32 + 0.5) * sh as f32 / px.y as f32 - 0.5).max(0.0);
        let (y0, ty) = (fy.floor() as i32, fy.fract());
        for x in 0..px.x {
            let fx = ((x as f32 + 0.5) * sw as f32 / px.x as f32 - 0.5).max(0.0);
            let (x0, tx) = (fx.floor() as i32, fx.fract());
            let mut out = [0.0f32; 4];
            for (dx, dy, wgt) in [
                (0, 0, (1.0 - tx) * (1.0 - ty)),
                (1, 0, tx * (1.0 - ty)),
                (0, 1, (1.0 - tx) * ty),
                (1, 1, tx * ty),
            ] {
                let p = at(x0 + dx, y0 + dy);
                for (o, v) in out.iter_mut().zip(p) {
                    *o += v * wgt;
                }
            }
            let o = (((top + y) * stride + x) * 4) as usize;
            // `out` holds colours times alpha on the 0..255 scale and alpha on 0..1.
            let a = out[3];
            let straight = |c: f32| {
                if a > 1e-4 {
                    (c / a).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            };
            data[o..o + 4].copy_from_slice(&[
                straight(out[0]),
                straight(out[1]),
                straight(out[2]),
                (a * 255.0).round().clamp(0.0, 255.0) as u8,
            ]);
        }
    }
}

/// One rectangle in physical pixels, a piece of the atlas tinted with a colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberQuad {
    /// Centre on the target.
    pub centre: Vec2,
    /// The piece of the atlas it shows, in atlas pixels; its size is the rectangle's size.
    pub uv: Rect,
    /// Tint (white for the flag, which carries its own colours).
    pub color: LinearRgba,
}

impl MemberQuad {
    /// Width and height on the target.
    pub fn size(&self) -> Vec2 {
        self.uv.size()
    }

    /// Top left corner on the target.
    pub fn top_left(&self) -> Vec2 {
        self.centre - self.size() * 0.5
    }

    /// A piece of the white strip, `w` x `h` pixels, at `left`/`top`, in `color`.
    fn bar(left: f32, top: f32, w: f32, h: f32, color: LinearRgba, layout: &AtlasLayout) -> Self {
        let (w, h) = (w.min(layout.white.width()), h.min(layout.white.height()));
        Self {
            centre: Vec2::new(left + w * 0.5, top + h * 0.5),
            uv: Rect::new(
                layout.white.min.x,
                layout.white.min.y,
                layout.white.min.x + w,
                layout.white.min.y + h,
            ),
            color,
        }
    }

    fn flag(left: f32, top: f32, uv: Rect) -> Self {
        let size = uv.size();
        Self {
            centre: Vec2::new(left + size.x * 0.5, top + size.y * 0.5),
            uv,
            color: LinearRgba::WHITE,
        }
    }
}

/// The rectangles of `items`: the tracks and fills of the bars into `bars`, the flags into `flags` (drawn after, so a flag is never
/// covered by the bar of its neighbour). `ui_phys` is the physical pixels of one design pixel (UI scale times the target's scaling
/// factor); positions come in logical pixels and leave in physical ones. Everything is rounded to whole pixels, so a bar that moves does
/// not shimmer.
pub fn member_quads(
    items: &[MemberBar],
    scale_factor: f32,
    ui_phys: f32,
    look: &BarLook,
    layout: &AtlasLayout,
    bars: &mut Vec<MemberQuad>,
    flags: &mut Vec<MemberQuad>,
) {
    let sf = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let thick = (BAR_THICKNESS * ui_phys).round().max(1.0);
    let pitch = (thick * 2.0).max(thick + 1.0);
    let overlap = (FLAG_OVERLAP * ui_phys).round();
    bars.reserve(items.len() * 3);
    flags.reserve(items.len());
    for m in items {
        let c = m.pos * sf;
        let len = (BAR_OF_SPHERE * 2.0 * m.radius * sf)
            .max(MIN_BAR * ui_phys)
            .round();
        let left = (c.x - len * 0.5).round();
        let top = (c.y + len * 0.5).round();
        let mut y = top;
        let mut bar = |at_y: f32, frac: f32, color: LinearRgba| {
            bars.push(MemberQuad::bar(left, at_y, len, thick, look.track, layout));
            let fill = if frac.is_finite() {
                (len * frac.clamp(0.0, 1.0)).round()
            } else {
                0.0
            };
            if fill >= 1.0 {
                bars.push(MemberQuad::bar(left, at_y, fill, thick, color, layout));
            }
        };
        if let Some(shield) = m.shield {
            bar(y, shield, look.shield);
            y += pitch;
        }
        bar(y, m.health, look.health_color(m.health));
        if let Some(uv) = layout.flag {
            flags.push(MemberQuad::flag(left + len - overlap, top, uv));
        }
    }
}

/// The rectangles of this frame, the atlas they are cut from and what it was made for (resource).
#[derive(Resource, Debug, Default)]
pub struct MemberQuads {
    bars: Vec<MemberQuad>,
    flags: Vec<MemberQuad>,
    /// An `Option`: the default handle of an image is the engine's white pixel, which must never be overwritten.
    atlas: Option<Handle<Image>>,
    layout: Option<AtlasLayout>,
    /// The `ui_phys` the atlas was drawn for (bits): the flag in it has that size.
    atlas_for: Option<u32>,
    look: Option<BarLook>,
}

impl MemberQuads {
    /// The tracks and fills of this frame's bars.
    pub fn bars(&self) -> &[MemberQuad] {
        &self.bars
    }

    /// The flags of this frame.
    pub fn flags(&self) -> &[MemberQuad] {
        &self.flags
    }
}

/// Turns [`MemberBars`] into [`MemberQuads`] once per frame (after every system that fills the bars).
pub fn build_member_quads(
    bars: Res<MemberBars>,
    ui_scale: Res<UiScale>,
    textures: Res<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut out: ResMut<MemberQuads>,
) {
    let MemberQuads {
        bars: bar_quads,
        flags,
        atlas,
        layout,
        atlas_for,
        look,
    } = &mut *out;
    bar_quads.clear();
    flags.clear();
    if bars.items.is_empty() {
        return;
    }
    if look.is_none() {
        if !textures.library().is_ready() {
            return;
        }
        *look = Some(BarLook::from_textures(&textures));
    }
    let Some(look) = look.as_ref() else { return };
    let sf = if bars.scale_factor > 0.0 {
        bars.scale_factor
    } else {
        1.0
    };
    let ui_phys = ui_scale.0 * sf;
    // The flag in the atlas has the size it is drawn at: draw the atlas again when that changes (window size, UI scale).
    if *atlas_for != Some(ui_phys.to_bits()) {
        let (image, new_layout) = build_atlas(look.flag.as_ref(), look.flag_size(ui_phys));
        match atlas.as_ref().filter(|handle| images.contains(*handle)) {
            Some(handle) => {
                if let Some(mut slot) = images.get_mut(handle) {
                    *slot = image;
                }
            }
            None => *atlas = Some(images.add(image)),
        }
        *layout = Some(new_layout);
        *atlas_for = Some(ui_phys.to_bits());
    }
    let Some(layout) = layout.as_ref() else {
        return;
    };
    member_quads(&bars.items, sf, ui_phys, look, layout, bar_quads, flags);
}

/// Render world: appends the rectangles to the UI batcher as two runs of glyphs of the atlas (bars, then flags), just below the command
/// bar: the world is under the interface, so a craft behind the bar, the minimap or a panel is covered by it (without a command bar:
/// in the layer of the overlays). bevy_ui draws a run of glyphs as one item whatever its length.
#[allow(clippy::type_complexity)]
pub fn extract_member_quads(
    mut commands: Commands,
    mut extracted: ResMut<ExtractedUiNodes>,
    quads: Extract<Res<MemberQuads>>,
    root: Extract<
        Query<(Entity, &ComputedStackIndex, &ComputedUiTargetCamera), With<TacticalOverlayRoot>>,
    >,
    command_bars: Extract<Query<&ComputedStackIndex, With<EawCommandBar>>>,
    camera_map: Extract<UiCameraMap>,
) {
    if quads.bars.is_empty() && quads.flags.is_empty() {
        return;
    }
    let Some(image) = quads.atlas.as_ref().map(Handle::id) else {
        return;
    };
    let Ok((root, stack, target)) = root.single() else {
        return;
    };
    let Some(camera) = camera_map.get_mapper().map(target) else {
        return;
    };
    // The first node of the lowest bar's tree is the lowest of the interface that matters here.
    let base = command_bars
        .iter()
        .map(|s| s.0)
        .min()
        .map_or(stack.0 as f32 + 0.5, |lowest| lowest as f32 - 0.5);
    let root = MainEntity::from(root);
    for (layer, run) in [(0.0, &quads.bars), (0.002, &quads.flags)] {
        if run.is_empty() {
            continue;
        }
        let start = extracted.glyphs.len();
        extracted.glyphs.extend(run.iter().map(|q| ExtractedGlyph {
            color: q.color,
            translation: q.centre,
            rect: q.uv,
        }));
        let end = extracted.glyphs.len();
        extracted.uinodes.push(ExtractedUiNode {
            z_order: base + layer,
            image,
            clip: None,
            extracted_camera_entity: camera,
            item: ExtractedUiItem::Glyphs { range: start..end },
            main_entity: root,
            render_entity: commands.spawn(TemporaryRenderEntity).id(),
            transform: Affine2::IDENTITY,
        });
    }
}

/// Registers the resources, the builder and the render world extraction (called by the plugin).
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<MemberBars>()
        .init_resource::<MemberQuads>()
        .add_systems(PostUpdate, build_member_quads);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(
            ExtractSchedule,
            extract_member_quads.in_set(RenderUiSystems::ExtractImages),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flag texture like the game's: 12 x 14, a white pole two pixels wide and a grey pennant, transparent black elsewhere.
    fn flag_texture() -> Rgba8 {
        let mut pixels = vec![0u8; 12 * 14 * 4];
        for y in 1..13 {
            for x in 0..12 {
                let o = (y * 12 + x) * 4;
                let px = match x {
                    2 => Some([0xf6, 0xf6, 0xf6, 0xf6]),
                    3 => Some([0xa3, 0xa3, 0xa3, 0xf6]),
                    4..=8 if y < 8 => Some([0xd8, 0xd8, 0xd8, 0xf6]),
                    _ => None,
                };
                if let Some(px) = px {
                    pixels[o..o + 4].copy_from_slice(&px);
                }
            }
        }
        Rgba8 {
            width: 12,
            height: 14,
            pixels,
        }
    }

    /// Flag look at the 1080p scale (UI scale 1.40625): the flag is drawn 16 x 18 px.
    fn look() -> (BarLook, AtlasLayout) {
        let look = BarLook {
            flag: Some(flag_texture()),
            ..BarLook::default()
        };
        let (_, layout) = build_atlas(look.flag.as_ref(), look.flag_size(1.40625));
        (look, layout)
    }

    fn bar(id: u32, x: f32, health: f32) -> MemberBar {
        MemberBar {
            id,
            pos: Vec2::new(x, 100.0),
            radius: 17.28,
            health,
            shield: None,
        }
    }

    fn quads_of(
        items: &[MemberBar],
        scale_factor: f32,
        ui_phys: f32,
        look: &BarLook,
        layout: &AtlasLayout,
    ) -> (Vec<MemberQuad>, Vec<MemberQuad>) {
        let (mut bars, mut flags) = (Vec::new(), Vec::new());
        member_quads(
            items,
            scale_factor,
            ui_phys,
            look,
            layout,
            &mut bars,
            &mut flags,
        );
        (bars, flags)
    }

    fn quads(items: &[MemberBar]) -> (Vec<MemberQuad>, Vec<MemberQuad>) {
        let (look, layout) = look();
        quads_of(items, 1.0, 1.40625, &look, &layout)
    }

    #[test]
    fn every_craft_gets_its_own_bar_and_flag() {
        let items = [bar(1, 100.0, 1.0), bar(2, 200.0, 0.5), bar(3, 300.0, 0.2)];
        let (bars, flags) = quads(&items);
        assert_eq!(flags.len(), 3, "one flag per craft");
        assert_eq!(bars.len(), 6, "a track and a fill per craft");
        // Each bar is centred on its own craft, in the order given.
        for (i, m) in items.iter().enumerate() {
            let track = bars[i * 2];
            assert!(
                (track.centre.x - m.pos.x).abs() <= 1.0,
                "{} vs {}",
                track.centre.x,
                m.pos.x
            );
        }
    }

    #[test]
    fn the_fill_follows_the_health_of_its_craft() {
        let (look, layout) = look();
        let width = |health: f32| {
            let (bars, _) = quads_of(&[bar(1, 100.0, health)], 1.0, 1.40625, &look, &layout);
            // The track comes first, the fill (when there is one) after it.
            bars.get(1).map_or(0.0, |q| q.size().x)
        };
        let full = width(1.0);
        assert_eq!(full, 29.0, "20.6 design px at 1080p, the Remake's 29 px");
        assert_eq!(width(0.5), (full * 0.5).round());
        assert_eq!(width(0.8), (full * 0.8).round());
        assert!(width(0.2) < width(0.5) && width(0.5) < width(0.8));
        assert_eq!(width(0.0), 0.0, "an empty bar shows only the track");
    }

    #[test]
    fn the_fill_colour_is_the_level_texture_of_the_health() {
        let look = BarLook::default();
        let colour = |h: f32| look.health_color(h);
        assert_eq!(colour(1.0), look.health[10]);
        assert_eq!(colour(0.96), look.health[10]);
        assert_eq!(colour(0.85), look.health[9]);
        assert_eq!(colour(0.5), look.health[5]);
        assert_eq!(colour(0.04), look.health[0]);
        assert_ne!(colour(1.0), colour(0.5));
        assert_ne!(colour(0.5), colour(0.2));
        assert_eq!(health_level(f32::NAN), 0);
        // The quad carries that colour: a half-health craft is orange, a full one green.
        let (bars, _) = quads(&[bar(1, 100.0, 0.5), bar(2, 200.0, 1.0)]);
        assert_eq!(bars[1].color, look.health[5]);
        assert_eq!(bars[3].color, look.health[10]);
        assert_eq!(bars[0].color, look.track, "the track is the dark one");
    }

    #[test]
    fn bar_and_flag_sit_where_the_remake_puts_them() {
        // 1080p video: the bar is 29 x 2 px, the flag about 16 x 18 px; the flag hangs from the right end of the bar.
        let (bars, flags) = quads(&[bar(1, 500.0, 1.0)]);
        let (track, flag) = (bars[0], flags[0]);
        assert_eq!(track.size(), Vec2::new(29.0, 2.0));
        assert_eq!(flag.size(), Vec2::new(16.0, 18.0));
        let (track_right, track_top) = (track.top_left().x + track.size().x, track.top_left().y);
        assert_eq!(
            flag.top_left().y,
            track_top,
            "the flag texture starts at the top of the bar"
        );
        assert_eq!(
            track_right - flag.top_left().x,
            2.0,
            "and reaches 1.3 design px (2 px) inside its right end"
        );
        // The bar's top touches the bottom of the sphere of the craft (centre + half the bar length).
        assert_eq!(track_top, (100.0f32 + 29.0 / 2.0).round());
    }

    #[test]
    fn a_shield_bar_goes_above_the_hull_bar() {
        let mut m = bar(1, 100.0, 1.0);
        m.shield = Some(0.5);
        let (bars, flags) = quads(&[m]);
        // Shield track and fill, then hull track and fill.
        assert_eq!(bars.len(), 4);
        assert!(
            bars[0].top_left().y < bars[2].top_left().y,
            "shield first, hull below it"
        );
        assert_eq!(flags.len(), 1, "still one flag per craft");
    }

    #[test]
    fn sizes_scale_with_the_screen_and_stay_whole_pixels() {
        // 4K target (scale factor 1.5, UI scale 1.875 per logical px): the sphere is 1.5 times larger in physical pixels.
        let item = MemberBar {
            radius: 17.28 * 2.0,
            ..bar(1, 100.0, 0.7)
        };
        let look = BarLook {
            flag: Some(flag_texture()),
            ..BarLook::default()
        };
        let (_, layout) = build_atlas(look.flag.as_ref(), look.flag_size(2.8125));
        let (bars, flags) = quads_of(&[item], 1.5, 2.8125, &look, &layout);
        for q in bars.iter().chain(&flags) {
            let (corner, size) = (q.top_left(), q.size());
            assert_eq!(
                (
                    corner.x.fract(),
                    corner.y.fract(),
                    size.x.fract(),
                    size.y.fract()
                ),
                (0.0, 0.0, 0.0, 0.0),
                "{q:?}"
            );
        }
        assert_eq!(
            bars[0].size().y,
            3.0,
            "3 px at 2160 px height like the screenshot"
        );
    }

    #[test]
    fn far_craft_keep_a_readable_bar() {
        let tiny = MemberBar {
            radius: 1.0,
            ..bar(1, 100.0, 1.0)
        };
        let (bars, _) = quads(&[tiny]);
        assert_eq!(bars[0].size().x, (MIN_BAR * 1.40625f32).round());
    }

    #[test]
    fn a_bar_is_a_piece_of_the_white_strip_and_a_flag_the_flag() {
        let (look, layout) = look();
        let (image, built) = build_atlas(look.flag.as_ref(), look.flag_size(1.40625));
        assert_eq!(built, layout);
        let data = image.data.as_ref().expect("pixels");
        let px = |x: u32, y: u32| {
            let o = ((y * layout.size.x + x) * 4) as usize;
            [data[o], data[o + 1], data[o + 2], data[o + 3]]
        };
        // The strip is opaque white (so a quad's colour is the bar's colour) ...
        assert_eq!(px(0, 0), [255; 4]);
        assert_eq!(
            px(layout.white.max.x as u32 - 1, layout.white.max.y as u32 - 1),
            [255; 4]
        );
        // ... and the bars are cut from it, whatever their length.
        let (bars, flags) = quads(&[
            bar(1, 300.0, 1.0),
            MemberBar {
                radius: 30.0,
                ..bar(2, 700.0, 0.4)
            },
        ]);
        assert!(bars.iter().all(|q| q.uv.min == layout.white.min
            && q.uv.max.x <= layout.white.max.x
            && q.uv.max.y <= layout.white.max.y));
        assert_ne!(bars[0].size().x, bars[2].size().x);
        // The flag keeps its look at the new size: the texture's colours (pole 0xf6 and 0xa3, pennant 0xd8), transparent right of the
        // pennant's tip, no dark fringe.
        let flag = layout.flag.expect("flag rect");
        assert_eq!(flags[0].uv, flag);
        let (fx, fy) = (flag.min.x as u32, flag.min.y as u32);
        let near = |p: [u8; 4], grey: u8| {
            p[0].abs_diff(grey) <= 4
                && p[1].abs_diff(grey) <= 4
                && p[2].abs_diff(grey) <= 4
                && p[3] >= 0xf0
        };
        let pole = px(fx + 3, fy + 10);
        assert!(pole[0] >= 0xe0 && pole[3] >= 0xf0, "bright pole: {pole:?}");
        assert!(
            near(px(fx + 6, fy + 3), 0xd8),
            "pennant: {:?}",
            px(fx + 6, fy + 3)
        );
        assert_eq!(
            px(fx + flag.width() as u32 - 1, fy + flag.height() as u32 - 1)[3],
            0
        );
        let drawn: Vec<[u8; 4]> = (fx..fx + flag.width() as u32)
            .flat_map(|x| (fy..fy + flag.height() as u32).map(move |y| (x, y)))
            .map(|(x, y)| px(x, y))
            .filter(|p| p[3] > 0)
            .collect();
        assert!(!drawn.is_empty());
        assert!(
            drawn.iter().all(|p| p[0] >= 0x90),
            "an edge pixel went dark: {:?}",
            drawn.iter().filter(|p| p[0] < 0x90).collect::<Vec<_>>()
        );
    }

    #[test]
    fn without_the_flag_texture_there_are_bars_only() {
        let look = BarLook::default();
        let (_, layout) = build_atlas(None, look.flag_size(1.40625));
        assert_eq!(layout.flag, None);
        let (bars, flags) = quads_of(&[bar(1, 100.0, 1.0)], 1.0, 1.40625, &look, &layout);
        assert_eq!((bars.len(), flags.len()), (2, 0));
    }
}
