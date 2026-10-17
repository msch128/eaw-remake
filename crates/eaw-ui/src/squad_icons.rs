//! One small icon per squadron of fighters or bombers (own and enemy) in the space battle.
//!
//! The craft of a squadron are drawn at their model scale, a few pixels across at most zoomed out, so an unpicked squadron is marked by
//! the unit's own icon art (`Icon_Name`, for example `I_BUTTON_UNIT_EMPIRE_TIE_FIGHTER`) at its middle: small, on a faint plate, without
//! the frame, bars and flag of a picked unit's box ([`crate::ShipOverlay`], which a selected or hovered squadron keeps instead).
//!
//! The host fills [`SquadIcons`] each frame; [`squad_icon_nodes`] keeps one see-through UI node per icon in a layer of its own below the
//! command bar (the bar, the minimap and the panels cover an icon behind them, as they cover the craft the icon stands for), so the world
//! below still gets the clicks and the hover (the host picks the squadron by the icon's position).

use std::collections::HashMap;

use alamo_model::image::Rgba8;
use bevy::prelude::*;
use bevy::ui::FocusPolicy;

use crate::EawUiSettings;
use crate::textures::{UiTextures, upload};

/// Icon size in design pixels (1/768 of the screen height), 16 px at 1080p. Estimate: no frame of the Remake references shows an unpicked
/// squadron's icon; the user's 1080p video shows the white box of a selected fighter at about 26 px.
pub const SQUAD_ICON_DESIGN_PX: f32 = 11.4;

/// Size in pixels the icon art is prepared at (it fills the square icon node like the old icon box): twice the icon at 1080p, so that the sampler never skips texels (the art is
/// drawn 16 px high, a fourth of its own size or less, and a thin line would flicker in and out of the samples).
const ART_PX: usize = 32;

/// Draw order of the icon layer: below the command bar and the panels (they have none, which counts as 0), above the world.
const ICON_LAYER_Z: i32 = -10;

/// One squadron's icon.
#[derive(Debug, Clone, PartialEq)]
pub struct SquadIcon {
    /// Stable id of the squadron (keeps its node between frames).
    pub id: u64,
    /// Centre in logical window pixels (y down), before UI scaling; whole physical pixels keep the icon from shimmering.
    pub screen_pos: Vec2,
    /// Texture name of the unit icon (`I_BUTTON_UNIT_*.TGA`).
    pub icon: String,
    /// An enemy squadron: its plate is red.
    pub enemy: bool,
}

/// Every squadron that shows its icon this frame (resource, filled by the host each frame; an empty list removes the nodes).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct SquadIcons {
    /// The icons.
    pub items: Vec<SquadIcon>,
}

/// One icon node.
#[derive(Component, Debug)]
pub struct SquadIconNode {
    /// Squadron id from [`SquadIcon::id`].
    pub id: u64,
}

/// Full-screen layer that holds the icons.
#[derive(Component)]
pub struct SquadIconRoot;

/// Spawns the icon layer (Startup).
pub(crate) fn spawn_icon_layer(mut commands: Commands, settings: Res<EawUiSettings>) {
    let mut e = commands.spawn((
        SquadIconRoot,
        Name::new("EaW squadron icons"),
        Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        FocusPolicy::Pass,
        GlobalZIndex(ICON_LAYER_Z),
    ));
    if let Some(c) = settings.camera {
        e.insert(UiTargetCamera(c));
    }
}

/// Plate behind the icon art: a faint black one for own squadrons, a dark red one for enemy squadrons (the art itself stays as it is).
pub fn plate_color(enemy: bool) -> Color {
    if enemy { Color::srgba(0.45, 0.02, 0.02, 0.55) } else { Color::srgba(0.0, 0.0, 0.0, 0.40) }
}

/// Top left corner of an icon centred at `centre` (logical pixels, before UI scaling), as the `Val`s of its node: `scale` is the UI scale,
/// which multiplies the `Px` values of a node, so positions are divided by it.
fn corner(centre: Vec2, scale: f32) -> (Val, Val) {
    let s = scale.max(0.01);
    let half = SQUAD_ICON_DESIGN_PX * 0.5 * s;
    (Val::Px((centre.x - half) / s), Val::Px((centre.y - half) / s))
}

/// Length of the overlap of the interval `lo..hi` with the pixel `i` (`i..i + 1`).
fn overlap(lo: f32, hi: f32, i: usize) -> f32 {
    (hi.min(i as f32 + 1.0) - lo.max(i as f32)).max(0.0)
}

/// `src` averaged down to `w` x `h` pixels (every output pixel is the mean of the source area it covers, colours weighted by alpha, so the
/// transparent black round the art does not darken its edge).
pub fn area_downsample(src: &Rgba8, w: usize, h: usize) -> Rgba8 {
    let (w, h) = (w.max(1), h.max(1));
    let (sx, sy) = (src.width as f32 / w as f32, src.height as f32 / h as f32);
    let mut pixels = vec![0u8; w * h * 4];
    for oy in 0..h {
        let (y_lo, y_hi) = (oy as f32 * sy, (oy + 1) as f32 * sy);
        for ox in 0..w {
            let (x_lo, x_hi) = (ox as f32 * sx, (ox + 1) as f32 * sx);
            let (mut colour, mut alpha, mut area) = ([0.0f32; 3], 0.0f32, 0.0f32);
            for y in y_lo.floor() as usize..(y_hi.ceil() as usize).min(src.height) {
                let wy = overlap(y_lo, y_hi, y);
                for x in x_lo.floor() as usize..(x_hi.ceil() as usize).min(src.width) {
                    let weight = wy * overlap(x_lo, x_hi, x);
                    let o = (y * src.width + x) * 4;
                    let a = f32::from(src.pixels[o + 3]) / 255.0 * weight;
                    for (c, v) in colour.iter_mut().zip(&src.pixels[o..o + 3]) {
                        *c += f32::from(*v) * a;
                    }
                    alpha += a;
                    area += weight;
                }
            }
            let out = (oy * w + ox) * 4;
            if alpha > 1e-6 {
                for (k, c) in colour.iter().enumerate() {
                    pixels[out + k] = (c / alpha).round().clamp(0.0, 255.0) as u8;
                }
                pixels[out + 3] = (alpha / area.max(1e-6) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rgba8 { width: w, height: h, pixels }
}

/// The prepared art of an icon, loaded once per name: `None` while the mega texture still decodes (the icon stays hidden and asks again
/// next frame) or when the game has no such texture (cached, the icon never shows).
fn icon_art(cache: &mut HashMap<String, Option<Handle<Image>>>, textures: &UiTextures, images: &mut Assets<Image>, name: &str) -> Option<Handle<Image>> {
    if let Some(hit) = cache.get(name) {
        return hit.clone();
    }
    if !textures.library().is_ready() {
        return None;
    }
    let art = textures.library().load(name).filter(|t| t.width > 0 && t.height > 0).map(|t| {
        debug!("squadron icon {name}: {} x {} px, prepared at {ART_PX} x {ART_PX} px", t.width, t.height);
        upload(images, area_downsample(&t, ART_PX, ART_PX)).handle
    });
    cache.insert(name.to_string(), art.clone());
    art
}

/// Creates, moves and removes the icon nodes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn squad_icon_nodes(
    icons: Res<SquadIcons>,
    scale: Res<UiScale>,
    root: Query<Entity, With<SquadIconRoot>>,
    mut existing: Local<HashMap<u64, Entity>>,
    mut art: Local<HashMap<String, Option<Handle<Image>>>>,
    mut nodes: Query<(&mut Node, &mut ImageNode, &mut BackgroundColor), With<SquadIconNode>>,
    textures: Res<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
) {
    let Ok(root) = root.single() else { return };
    existing.retain(|id, e| {
        let keep = icons.items.iter().any(|i| i.id == *id);
        if !keep {
            commands.entity(*e).try_despawn();
        }
        keep
    });
    for icon in &icons.items {
        let (left, top) = corner(icon.screen_pos, scale.0);
        // The art is in the mega texture, which decodes in the background: an icon spawned before that gets it once it is there.
        let handle = icon_art(&mut art, &textures, &mut images, &icon.icon);
        let display = if handle.is_some() { Display::Flex } else { Display::None };
        if let Some((mut node, mut image, mut plate)) = existing.get(&icon.id).and_then(|&e| nodes.get_mut(e).ok()) {
            if node.left != left {
                node.left = left;
            }
            if node.top != top {
                node.top = top;
            }
            if node.display != display {
                node.display = display;
            }
            if let Some(handle) = handle
                && image.image != handle
            {
                image.image = handle;
            }
            let color = plate_color(icon.enemy);
            if plate.0 != color {
                plate.0 = color;
            }
            continue;
        }
        let id = commands
            .spawn((
                Name::new("squadron icon"),
                SquadIconNode { id: icon.id },
                Node {
                    position_type: PositionType::Absolute,
                    left,
                    top,
                    display,
                    width: Val::Px(SQUAD_ICON_DESIGN_PX),
                    height: Val::Px(SQUAD_ICON_DESIGN_PX),
                    ..default()
                },
                handle.map_or_else(ImageNode::default, ImageNode::new),
                BackgroundColor(plate_color(icon.enemy)),
                FocusPolicy::Pass,
                ChildOf(root),
            ))
            .id();
        existing.insert(icon.id, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_16_px_at_1080p_and_scales_with_the_screen() {
        // UiScale at 1920 x 1080 is 1.41 (`widgets::ui_scale`).
        assert!((SQUAD_ICON_DESIGN_PX * 1.4118 - 16.0).abs() < 0.2, "{}", SQUAD_ICON_DESIGN_PX * 1.4118);
        // At 4K the same share of the screen height: twice as big.
        assert!((SQUAD_ICON_DESIGN_PX * 2.8236 - 32.0).abs() < 0.4);
    }

    #[test]
    fn the_node_sits_centred_on_the_squadron_whatever_the_ui_scale() {
        for scale in [1.0, 1.4118, 2.8236] {
            let (left, top) = corner(Vec2::new(500.0, 300.0), scale);
            let (Val::Px(l), Val::Px(t)) = (left, top) else { panic!("pixel values") };
            // Node values are multiplied by the UI scale on screen; the icon is SQUAD_ICON_DESIGN_PX of them wide.
            let half = SQUAD_ICON_DESIGN_PX * 0.5 * scale;
            assert!((l * scale + half - 500.0).abs() < 1e-3 && (t * scale + half - 300.0).abs() < 1e-3, "scale {scale}: {l} {t}");
        }
    }

    #[test]
    fn enemy_squadrons_differ_by_their_plate_not_by_a_frame() {
        assert_ne!(plate_color(true), plate_color(false));
        assert!(plate_color(false).alpha() < 0.5 && plate_color(true).alpha() < 0.6, "faint plates: the art and the world stay readable");
    }

    /// A 4 x 4 picture: left half opaque white, right half transparent black.
    fn half_and_half() -> Rgba8 {
        let mut pixels = Vec::new();
        for _ in 0..4 {
            for x in 0..4 {
                pixels.extend_from_slice(if x < 2 { &[255, 255, 255, 255] } else { &[0, 0, 0, 0] });
            }
        }
        Rgba8 { width: 4, height: 4, pixels }
    }

    #[test]
    fn the_art_is_averaged_down_without_darkening_its_edge() {
        let small = area_downsample(&half_and_half(), 2, 2);
        assert_eq!((small.width, small.height), (2, 2));
        let px = |x: usize, y: usize| &small.pixels[(y * 2 + x) * 4..(y * 2 + x) * 4 + 4];
        assert_eq!(px(0, 0), [255, 255, 255, 255], "the opaque half stays white");
        assert_eq!(px(1, 1)[3], 0, "the transparent half stays clear");
        // Three source columns into two output columns: the middle column is shared, half of it in each; the white one keeps its colour.
        let odd = area_downsample(&half_and_half(), 3, 1);
        assert_eq!(odd.pixels[0..3], [255, 255, 255], "colour is weighted by alpha: never grey at the edge");
        assert!(odd.pixels[4 + 3] > 0 && odd.pixels[4 + 3] < 255, "the shared column is half covered: {}", odd.pixels[4 + 3]);
        // A flat picture stays flat, at any size.
        let flat = Rgba8 { width: 5, height: 3, pixels: [10, 20, 30, 200].repeat(15) };
        for (w, h) in [(1, 1), (2, 2), (7, 4)] {
            let out = area_downsample(&flat, w, h);
            assert!(out.pixels.chunks(4).all(|p| p == [10, 20, 30, 200]), "{w}x{h}: {:?}", &out.pixels[..4]);
        }
    }
}
