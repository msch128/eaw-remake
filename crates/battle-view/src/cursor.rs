//! The mouse cursor of a battle: the game's own pointers, set as the window's cursor.
//!
//! `Data/XML/MousePointers.xml` lists every pointer of the original game: its first texture, the pixel of the picture that is the pointer's
//! position (`Hot_X`, `Hot_Y`) and the animation delay; the later frames are the textures with the next numbers (`ATTACKPOINTER_00` ..
//! `_07`, loose files below `Art/Textures`). The system's arrow stays for everything that is not special. While own units are selected and
//! the pointer is on something to attack (an enemy ship, structure or hardpoint) the pointer itself is the red crosshair and nothing else:
//! no button picture, no frame around it (the order cursor of `a7e667e` drew the command bar's attack button at the pointer). A command
//! button that arms an order shows that order's pointer under the names the data gives them: "attack only" (the crosshair over an enemy,
//! the greyed one elsewhere), attack move (red arrow), move (green arrow), targeted ability (burst, or the crossed one where it cannot be used).
//!
//! The picture is the window's [`CursorIcon`], so it is the OS pointer itself: exactly at the mouse, no frame behind, and no interface node
//! that could block the world under it (the flicker of `7e43c14`). An offscreen run has no window to set; the choice is made all the same
//! and kept in [`ShownCursor`], which the tests and `SP_CURSOR_LOG=1` read.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

use std::collections::HashMap;

use alamo_model::image::Rgba8;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};
use eaw_ui::textures::UiTextures;

use crate::control::{self, ArmedKind, ArmedOrder, Drag, Hover, Pointer, Selection, UiBlock};
use crate::ships::RenderedShips;
use crate::{BattleRuntime, BattleViewSettings};

/// The `MousePointers.xml` entries a battle shows. All of them are loaded together, once (when the first battle's session exists).
pub mod names {
    /// Own units selected, pointer on an enemy.
    pub const ATTACK: &str = "POINTER_ATTACK";
    /// The attack button armed, pointer on an enemy.
    pub const ATTACK_ONLY_TARGET: &str = "POINTER_ATTACK_ONLY_MODE_UNIT_TARGETED";
    /// The attack button armed, nothing to attack under the pointer (the greyed crosshair).
    pub const ATTACK_ONLY_NO_TARGET: &str = "POINTER_ATTACK_ONLY_MODE_NO_UNIT_TARGETED";
    /// The attack-move button armed.
    pub const ATTACK_MOVE: &str = "POINTER_ATTACK_MOVE";
    /// The move button armed.
    pub const MOVE_ONLY: &str = "POINTER_MOVE_ONLY_MODE_PASSABLE_TARGETED";
    /// A targeted ability armed, the position is valid.
    pub const ABILITY: &str = "POINTER_TARGET_SPECIAL_ABILITY_TO_SPACE_POSITION";
    /// A targeted ability armed, the sim would refuse the position.
    pub const ABILITY_INVALID: &str = "POINTER_TARGET_SPECIAL_ABILITY_TO_SPACE_POSITION_INVALID";
    /// All of the above.
    pub const ALL: [&str; 7] = [ATTACK, ATTACK_ONLY_TARGET, ATTACK_ONLY_NO_TARGET, ATTACK_MOVE, MOVE_ONLY, ABILITY, ABILITY_INVALID];
}

/// Seconds one `Anim_Frame_Delay` step lasts. Estimate: the data counts engine frames, not seconds, and the engine's rate is not in
/// the data; 60 per second gives the crosshair's 8 pictures (delay 6 in the Remake) a 0.8 s pulse.
pub const DELAY_STEP_SECONDS: f32 = 1.0 / 60.0;

/// Most pictures one pointer can have (the data has 8 at most).
const MAX_FRAMES: usize = 32;

/// One `MousePointer` entry of `MousePointers.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerDef {
    /// First picture as written in the data (`ATTACKPOINTER_00.TGA`; the files are `.dds`, the engine takes either).
    pub texture: String,
    /// Pixel of the picture that is the pointer's position (`Hot_X`, `Hot_Y`).
    pub hotspot: (u32, u32),
    /// `Anim_Frame_Delay`: engine frames each picture stays.
    pub frame_delay: u32,
}

/// `MousePointers.xml` by entry name (upper case, `POINTER_ATTACK`).
#[derive(Debug, Clone, Default)]
pub struct PointerTable(HashMap<String, PointerDef>);

impl PointerTable {
    /// Parses the file; entries without a texture are skipped.
    pub fn parse(bytes: &[u8]) -> Self {
        let mut warnings = Vec::new();
        let Some(root) = alamo_data::xml::parse(bytes, "MousePointers.xml", &mut warnings) else { return Self::default() };
        let number = |e: &alamo_data::Element, tag: &str| e.child_text(tag).and_then(|v| v.parse::<u32>().ok());
        let entries = root
            .children_named("mousepointer")
            .filter_map(|e| {
                let name = e.attr("name")?.trim().to_ascii_uppercase();
                let texture = e.child_text("base_texture").filter(|t| !t.is_empty())?.to_owned();
                let hotspot = (number(e, "hot_x").unwrap_or(0), number(e, "hot_y").unwrap_or(0));
                Some((name, PointerDef { texture, hotspot, frame_delay: number(e, "anim_frame_delay").unwrap_or(0) }))
            })
            .collect();
        Self(entries)
    }

    /// Reads and parses `Xml/MousePointers.xml` (mod over game).
    pub fn load(fs: &dyn alamo_fs::FileSource) -> Self {
        fs.read("Xml/MousePointers.xml").map(|b| Self::parse(&b)).unwrap_or_default()
    }

    /// The entry called `name`.
    pub fn get(&self, name: &str) -> Option<&PointerDef> {
        self.0.get(name)
    }
}

/// Name of picture number `k` of the pointer whose first picture is `first`: the same name with the number counted up
/// (`ATTACKPOINTER_00.TGA`, `ATTACKPOINTER_01.TGA`, ...). A name without a trailing number is one picture (`None` for `k > 0`).
fn frame_name(first: &str, k: usize) -> Option<String> {
    let (stem, ext) = first.rsplit_once('.').unwrap_or((first, ""));
    let digits = stem.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return (k == 0).then(|| first.to_owned());
    }
    let (prefix, number) = stem.split_at(stem.len() - digits);
    let n = number.parse::<usize>().ok()? + k;
    Some(if ext.is_empty() { format!("{prefix}{n:0digits$}") } else { format!("{prefix}{n:0digits$}.{ext}") })
}

/// The pictures of one pointer as images the window can take as its cursor.
#[derive(Debug, Clone)]
pub struct PointerImages {
    /// The animation, in order.
    pub frames: Vec<Handle<Image>>,
    /// Pixel of the pictures that is the pointer's position.
    pub hotspot: (u16, u16),
    /// Seconds each picture stays.
    pub frame_seconds: f32,
}

impl PointerImages {
    /// Index of the picture shown `t` seconds after the pointer came up (the animation loops).
    pub fn frame_at(&self, t: f32) -> usize {
        match self.frames.len() {
            0 | 1 => 0,
            n => (t.max(0.0) / self.frame_seconds) as usize % n,
        }
    }

    /// The window cursor for the picture shown `t` seconds after the pointer came up.
    pub fn icon_at(&self, t: f32) -> CursorIcon {
        let handle = self.frames[self.frame_at(t)].clone();
        CursorIcon::Custom(CustomCursor::Image(CustomCursorImage { handle, hotspot: self.hotspot, ..default() }))
    }
}

/// The pointers of the battles ([`names::ALL`]), loaded once. Pointers whose textures are missing are left out and the system arrow stays
/// for them.
#[derive(Resource, Debug, Default, Clone)]
pub struct CursorArt(HashMap<&'static str, PointerImages>);

impl CursorArt {
    /// Loads the pictures of every pointer of [`names::ALL`] named in `table` through `load` (texture name to pixels) into `images`. The
    /// images keep their pixels in the main world: the window cursor is made from them there, they are never drawn.
    pub fn build(table: &PointerTable, load: impl Fn(&str) -> Option<Rgba8>, images: &mut Assets<Image>) -> Self {
        let mut out = HashMap::new();
        // Pointers that start from the same texture share its pictures (the attack pointer and the armed attack button's).
        let mut loaded: HashMap<String, Option<(Vec<Handle<Image>>, (usize, usize))>> = HashMap::new();
        for name in names::ALL {
            let Some(def) = table.get(name) else {
                warn!("cursor: MousePointers.xml has no {name}");
                continue;
            };
            let pictures = loaded.entry(def.texture.to_ascii_uppercase()).or_insert_with(|| {
                let frames: Vec<Rgba8> = (0..MAX_FRAMES).map_while(|k| frame_name(&def.texture, k)).map_while(|n| load(&n)).collect();
                let size = frames.first().map(|f| (f.width, f.height))?;
                Some((frames.into_iter().map(|f| images.add(cursor_image(f))).collect(), size))
            });
            let Some((frames, size)) = pictures.clone() else {
                warn!("cursor: no picture {} for {name}", def.texture);
                continue;
            };
            let clamp = |h: u32, size: usize| h.min(size.saturating_sub(1) as u32) as u16;
            let hotspot = (clamp(def.hotspot.0, size.0), clamp(def.hotspot.1, size.1));
            let frame_seconds = def.frame_delay.max(1) as f32 * DELAY_STEP_SECONDS;
            out.insert(name, PointerImages { frames, hotspot, frame_seconds });
        }
        Self(out)
    }

    /// The pictures of the pointer `name`.
    pub fn get(&self, name: &str) -> Option<&PointerImages> {
        self.0.get(name)
    }
}

fn cursor_image(rgba: Rgba8) -> Image {
    Image::new(
        Extent3d { width: rgba.width.max(1) as u32, height: rgba.height.max(1) as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba.pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD,
    )
}

/// The pointer the battle shows: the `MousePointers.xml` entry, `None` for the system's arrow, and when it came up.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct ShownCursor {
    /// Entry of [`names`] on show.
    pub name: Option<&'static str>,
    /// `Time<Real>` seconds when it came up (the animation starts there).
    since: f32,
}

/// Whether the targeted ability under the pointer can be used (`None`: no ability armed or no position yet). Written by
/// `tactical_ui::ability_at`, which decides it for the circle and the reason label it draws.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct AbilityAim(pub Option<bool>);

/// `SP_CURSOR_LOG=1`: one log line for every change of the pointer.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct CursorLog(pub bool);

/// Registers the systems and resources (called by `TacticalUiPlugin`).
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<ShownCursor>().init_resource::<AbilityAim>().insert_resource(CursorLog(std::env::var_os("SP_CURSOR_LOG").is_some()));
    app.add_systems(
        Update,
        (
            load_art.run_if(resource_exists::<crate::BattleSession>.and_then(not(resource_exists::<CursorArt>))),
            battle_cursor
                .after(control::selection)
                .after(control::orders)
                .after(crate::tactical_ui::ability_at)
                .run_if(resource_exists::<BattleRuntime>),
            restore_cursor.run_if(resource_removed::<BattleRuntime>.or_eager(resource_removed::<crate::BattleSession>)),
        ),
    );
}

/// What the pointer shows: the entry of [`names`], or `None` for the system's arrow. A free pointer (over the world, no box being dragged)
/// is the crosshair while own units are selected and an enemy is under it; an armed order has its own pointer.
fn wanted(armed: Option<ArmedKind>, own_selected: bool, over_enemy: bool, ability_ok: bool) -> Option<&'static str> {
    match armed {
        Some(ArmedKind::Attack) => Some(if over_enemy { names::ATTACK_ONLY_TARGET } else { names::ATTACK_ONLY_NO_TARGET }),
        Some(ArmedKind::AttackMove) => Some(names::ATTACK_MOVE),
        Some(ArmedKind::Move) => Some(names::MOVE_ONLY),
        Some(ArmedKind::AbilityAt) => Some(if ability_ok { names::ABILITY } else { names::ABILITY_INVALID }),
        // The reinforcement ghost is the cue there.
        Some(ArmedKind::Reinforce) => None,
        None => (own_selected && over_enemy).then_some(names::ATTACK),
    }
}

/// Loads the pointers once the battle's session exists (while the loading screen is up).
fn load_art(mut commands: Commands, textures: Option<Res<UiTextures>>, mut images: ResMut<Assets<Image>>) {
    let Some(textures) = textures else { return };
    let library = textures.library();
    let table = PointerTable::load(library.fs().as_ref());
    let art = CursorArt::build(&table, |name| library.load(name), &mut images);
    info!(
        "cursor: {} of {} pointers loaded from MousePointers.xml ({})",
        art.0.len(),
        names::ALL.len(),
        names::ALL.iter().filter_map(|n| art.get(n).map(|p| format!("{n} {} frame(s) hot {:?} {:.3} s", p.frames.len(), p.hotspot, p.frame_seconds))).collect::<Vec<_>>().join(", ")
    );
    commands.insert_resource(art);
}

/// Chooses the pointer from what is under the mouse and sets it as the window's cursor, only when it changes (every other frame of
/// the same pointer leaves the window alone, so there is no resetting of the OS cursor).
fn battle_cursor(
    mut commands: Commands,
    armed: Res<ArmedOrder>,
    hover: Res<Hover>,
    sel: Res<Selection>,
    pointer: Res<Pointer>,
    block: Res<UiBlock>,
    drag: Res<Drag>,
    aim: Res<AbilityAim>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    ships: Res<RenderedShips>,
    art: Option<Res<CursorArt>>,
    time: Res<Time<Real>>,
    log: Res<CursorLog>,
    mut shown: ResMut<ShownCursor>,
    window: Query<(Entity, Option<&CursorIcon>), With<PrimaryWindow>>,
) {
    let player = settings.player_side;
    // Over the interface, outside the window and while a box is dragged (it must not turn into a crosshair when it passes an enemy) it is the arrow.
    let free = pointer.cursor.is_some_and(|c| !block.0 && !drag.box_dragging(c, settings.ui_scale));
    let want = if free {
        let own_selected = !control::selected_mobile(&rt, &ships, &sel, player).is_empty();
        let over_enemy = hover.hardpoint.is_some() || hover.ship.is_some_and(|id| ships.0.get(&id).is_some_and(|s| s.side != player && s.visible));
        wanted(armed.0, own_selected, over_enemy, aim.0 != Some(false))
    } else {
        None
    };
    let now = time.elapsed_secs();
    if shown.name != want {
        if log.0 {
            info!("cursor: {} -> {} at {:?}", shown.name.unwrap_or("system arrow"), want.unwrap_or("system arrow"), pointer.cursor);
        }
        *shown = ShownCursor { name: want, since: now };
    }
    let icon = want.and_then(|n| art.as_deref()?.get(n)).map_or_else(CursorIcon::default, |p| p.icon_at(now - shown.since));
    let Ok((window, current)) = window.single() else { return };
    // A window that never had a cursor set shows the system's arrow already.
    if *current.unwrap_or(&CursorIcon::default()) != icon {
        commands.entity(window).insert(icon);
    }
}

/// The battle is over: the system arrow is back. The pictures stay for the next battle: the window keeps every cursor it made from them
/// (it caches by image), so new images each battle would pile up OS cursors.
fn restore_cursor(mut commands: Commands, window: Query<Entity, With<PrimaryWindow>>, mut shown: ResMut<ShownCursor>) {
    for e in &window {
        commands.entity(e).insert(CursorIcon::default());
    }
    *shown = ShownCursor::default();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use battle_sim::{Battle, BattleConfig, BattleSetup, Catalog, HpDef, HpKind, ProjDef, ShipDef, SideSetup, TargetClass, UnitSetup};
    use bevy::camera::{CameraProjection, PerspectiveProjection, RenderTargetInfo};
    use bevy::input::ButtonInput;
    use bevy::time::TimeUpdateStrategy;

    use super::*;
    use crate::camera::BattleCamera;
    use crate::control::{ControlGroups, FocusCamera, HardpointOrderIssued, OrderIssued, OrderKind, OrderMarkers, RecentOrders};
    use crate::ships::ShipRender;

    /// `MousePointers.xml` of the tests: the seven pointers of a battle, with the layout of the real file (tabs, mixed case, comments).
    const TABLE: &str = r#"<?xml version="1.0"?>
<MousePointers>
	<!-- comment -->
	<MousePointer Name="POINTER_ATTACK">
		<Base_Texture> ATTACKPOINTER_00.TGA </Base_Texture>
		<Hot_X> 16 </Hot_X>
		<Hot_Y> 15 </Hot_Y>
		<Anim_Frame_Delay> 6 </Anim_Frame_Delay>
	</MousePointer>
	<MousePointer Name="POINTER_ATTACK_ONLY_MODE_UNIT_TARGETED"><Base_Texture>ATTACKPOINTER_00.TGA</Base_Texture><Hot_X>16</Hot_X><Hot_Y>16</Hot_Y><Anim_Frame_Delay>6</Anim_Frame_Delay></MousePointer>
	<MousePointer Name="POINTER_ATTACK_ONLY_MODE_NO_UNIT_TARGETED"><Base_Texture>ATTACK_OUT_OF_RANGE_POINTER_00.TGA</Base_Texture><Hot_X>16</Hot_X><Hot_Y>16</Hot_Y><Anim_Frame_Delay>6</Anim_Frame_Delay></MousePointer>
	<MousePointer Name="POINTER_ATTACK_MOVE"><Base_Texture>MOVE_ATTACK_POINTER_00.TGA</Base_Texture><Hot_X>0</Hot_X><Hot_Y>0</Hot_Y><Anim_Frame_Delay>6</Anim_Frame_Delay></MousePointer>
	<mousepointer name="pointer_move_only_mode_passable_targeted"><base_texture>MOVETOPOINTER_00.TGA</base_texture><hot_x>0</hot_x><hot_y>0</hot_y><anim_frame_delay>6</anim_frame_delay></mousepointer>
	<MousePointer Name="POINTER_TARGET_SPECIAL_ABILITY_TO_SPACE_POSITION"><Base_Texture>special_abilities_pointer_00.tga</Base_Texture><Hot_X>16</Hot_X><Hot_Y>16</Hot_Y><Anim_Frame_Delay>6</Anim_Frame_Delay></MousePointer>
	<MousePointer Name="POINTER_TARGET_SPECIAL_ABILITY_TO_SPACE_POSITION_INVALID"><Base_Texture>SPECIAL_ABILITIES_INVALID_POINTER_00.tga</Base_Texture><Hot_X>16</Hot_X><Hot_Y>16</Hot_Y></MousePointer>
	<MousePointer Name="POINTER_BROKEN"><Hot_X>1</Hot_X></MousePointer>
</MousePointers>"#;

    /// A 32 x 32 picture of one shade.
    fn picture(shade: u8) -> Rgba8 {
        Rgba8 { width: 32, height: 32, pixels: vec![shade; 32 * 32 * 4] }
    }

    /// The picture files of the tests: `attack_frames` for the crosshair (`ATTACKPOINTER_00` ...), the `_00` of every other pointer.
    fn picture_loader(attack_frames: usize) -> impl Fn(&str) -> Option<Rgba8> {
        move |name| {
            let upper = name.to_ascii_uppercase();
            let number: usize = upper.strip_suffix(".TGA")?.rsplit('_').next()?.parse().ok()?;
            let limit = if upper.starts_with("ATTACKPOINTER_") { attack_frames } else { 1 };
            (number < limit).then(|| picture(number as u8))
        }
    }

    fn art(images: &mut Assets<Image>, attack_frames: usize) -> CursorArt {
        CursorArt::build(&PointerTable::parse(TABLE.as_bytes()), picture_loader(attack_frames), images)
    }

    #[test]
    fn the_table_gives_texture_hotspot_and_delay_of_every_entry() {
        let table = PointerTable::parse(TABLE.as_bytes());
        assert_eq!(table.get("POINTER_ATTACK"), Some(&PointerDef { texture: "ATTACKPOINTER_00.TGA".into(), hotspot: (16, 15), frame_delay: 6 }));
        // Tags and attributes in any case, no spaces around the values, no delay: 0 (the engine's "none").
        assert_eq!(table.get("POINTER_MOVE_ONLY_MODE_PASSABLE_TARGETED").map(|d| d.texture.as_str()), Some("MOVETOPOINTER_00.TGA"));
        assert_eq!(table.get("POINTER_TARGET_SPECIAL_ABILITY_TO_SPACE_POSITION_INVALID").map(|d| d.frame_delay), Some(0));
        // An entry without a texture is no pointer.
        assert_eq!(table.get("POINTER_BROKEN"), None);
        assert_eq!(table.get("POINTER_NOT_THERE"), None);
        assert!(PointerTable::parse(b"not xml at all").get("POINTER_ATTACK").is_none());
    }

    #[test]
    fn frames_are_the_texture_with_the_following_numbers() {
        assert_eq!(frame_name("ATTACKPOINTER_00.TGA", 0).as_deref(), Some("ATTACKPOINTER_00.TGA"));
        assert_eq!(frame_name("ATTACKPOINTER_00.TGA", 7).as_deref(), Some("ATTACKPOINTER_07.TGA"));
        assert_eq!(frame_name("MOVE_ATTACK_POINTER_00.TGA", 12).as_deref(), Some("MOVE_ATTACK_POINTER_12.TGA"));
        assert_eq!(frame_name("special_abilities_pointer_00.tga", 3).as_deref(), Some("special_abilities_pointer_03.tga"));
        // No number in the name: one picture.
        assert_eq!(frame_name("cantpointer.tga", 0).as_deref(), Some("cantpointer.tga"));
        assert_eq!(frame_name("cantpointer.tga", 1), None);
    }

    #[test]
    fn the_art_takes_pictures_until_the_first_missing_one_and_keeps_the_pixels_for_the_window() {
        let mut images = Assets::<Image>::default();
        let art = art(&mut images, 8);
        let attack = art.get(names::ATTACK).expect("crosshair");
        assert_eq!(attack.frames.len(), 8);
        // 16 and 15 inside a 32 x 32 picture stay; the delay of 6 engine frames is 0.1 s here.
        assert_eq!(attack.hotspot, (16, 15));
        assert!((attack.frame_seconds - 6.0 * DELAY_STEP_SECONDS).abs() < 1e-6);
        assert_eq!(art.get(names::MOVE_ONLY).map(|p| p.frames.len()), Some(1));
        // The armed attack button's pointer starts from the same texture: the same pictures, not a second set.
        assert_eq!(art.get(names::ATTACK_ONLY_TARGET).map(|p| &p.frames), Some(&attack.frames));
        assert_eq!(images.len(), 8 + 5, "8 for the crosshair and one for each of the 5 other pointers, nothing twice");
        // The window reads RGBA8 pixels from the main world copy.
        let first = images.get(&attack.frames[0]).expect("image");
        assert_eq!(first.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb);
        assert!(first.data.as_ref().is_some_and(|d| d.len() == 32 * 32 * 4));
        // A pointer without a delay still animates (one engine step, not a division by zero); a hotspot outside the picture is pulled in.
        let table = PointerTable::parse(br#"<P><MousePointer Name="POINTER_ATTACK"><Base_Texture>A_00.TGA</Base_Texture><Hot_X>99</Hot_X><Hot_Y>99</Hot_Y></MousePointer></P>"#);
        let small = CursorArt::build(&table, |_| Some(Rgba8 { width: 8, height: 8, pixels: vec![0; 8 * 8 * 4] }), &mut images);
        assert_eq!(small.get(names::ATTACK).map(|p| (p.hotspot, p.frame_seconds)), Some(((7, 7), DELAY_STEP_SECONDS)));
        // Textures that are not there: the pointer is left out and the system arrow stays.
        let none = CursorArt::build(&table, |_| None, &mut images);
        assert!(none.get(names::ATTACK).is_none());
    }

    #[test]
    fn the_animation_loops_through_the_pictures_at_their_delay() {
        let mut images = Assets::<Image>::default();
        let art = art(&mut images, 8);
        let attack = art.get(names::ATTACK).expect("crosshair");
        let step = attack.frame_seconds;
        assert_eq!(attack.frame_at(0.0), 0);
        assert_eq!(attack.frame_at(step * 0.99), 0);
        assert_eq!(attack.frame_at(step * 1.01), 1);
        assert_eq!(attack.frame_at(step * 7.5), 7);
        assert_eq!(attack.frame_at(step * 8.01), 0, "after the last picture the first comes again");
        assert_eq!(attack.frame_at(-1.0), 0, "a clock that went back shows the first picture");
        assert_eq!(art.get(names::MOVE_ONLY).map(|p| p.frame_at(123.0)), Some(0), "one picture stays");
    }

    #[test]
    fn the_pointer_is_the_crosshair_over_enemies_and_the_armed_orders_have_their_own() {
        use ArmedKind::*;
        // Nothing armed: the system's arrow, except the crosshair with own units selected and an enemy under the pointer.
        assert_eq!(wanted(None, true, true, true), Some(names::ATTACK));
        assert_eq!(wanted(None, true, false, true), None, "open space keeps the arrow: moves are right clicks with the normal pointer");
        assert_eq!(wanted(None, false, true, true), None, "nothing selected, nothing to order");
        // The attack button armed: crosshair over an enemy, the greyed one elsewhere.
        assert_eq!(wanted(Some(Attack), true, true, true), Some(names::ATTACK_ONLY_TARGET));
        assert_eq!(wanted(Some(Attack), true, false, true), Some(names::ATTACK_ONLY_NO_TARGET));
        // The other buttons: red arrow, green arrow, the ability's burst (crossed where the sim would refuse the position).
        assert_eq!(wanted(Some(AttackMove), true, true, true), Some(names::ATTACK_MOVE));
        assert_eq!(wanted(Some(Move), true, true, true), Some(names::MOVE_ONLY));
        assert_eq!(wanted(Some(AbilityAt), true, false, true), Some(names::ABILITY));
        assert_eq!(wanted(Some(AbilityAt), true, false, false), Some(names::ABILITY_INVALID));
        // The reinforcement ghost is the cue while a reinforcement is placed.
        assert_eq!(wanted(Some(Reinforce), true, true, true), None);
    }

    /// Whether the window's [`CursorIcon`] was written (inserted or changed) in each frame: what the winit side turns into an OS cursor change.
    #[derive(Resource, Default)]
    struct CursorWrites(Vec<bool>);

    fn count_writes(cursor: Query<Ref<CursorIcon>, With<PrimaryWindow>>, mut writes: ResMut<CursorWrites>) {
        writes.0.push(cursor.single().is_ok_and(|c| c.is_changed()));
    }

    /// The orders the player gave, in order.
    #[derive(Resource, Default)]
    struct Orders(Vec<OrderKind>);

    fn collect_orders(mut issued: MessageReader<OrderIssued>, mut orders: ResMut<Orders>) {
        orders.0.extend(issued.read().map(|o| o.kind));
    }

    /// The real hover (`control::selection`), the real right-click orders (`control::orders`) and the cursor system over a camera that looks
    /// straight down: an own frigate (id 1, selected) on the left, an enemy cruiser (id 2, hull radius 300) in the middle with a weapon
    /// hardpoint out on a wing, and a primary window that exists only as data. One update is 1/60 s of `Time<Real>`.
    struct Scene {
        app: App,
        window: Entity,
        /// The cruiser's centre and pick radius on screen (px).
        cruiser: (Vec2, f32),
        /// The wing hardpoint on screen (px).
        hardpoint: Vec2,
    }

    impl Scene {
        fn new(attack_frames: usize) -> Self {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / 60.0)))
                .init_resource::<Assets<Image>>()
                .init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<Selection>()
                .init_resource::<ControlGroups>()
                .init_resource::<Hover>()
                .init_resource::<Drag>()
                .init_resource::<UiBlock>()
                .init_resource::<ArmedOrder>()
                .init_resource::<Pointer>()
                .init_resource::<RenderedShips>()
                .init_resource::<OrderMarkers>()
                .init_resource::<RecentOrders>()
                .init_resource::<ShownCursor>()
                .init_resource::<AbilityAim>()
                .init_resource::<CursorLog>()
                .init_resource::<CursorWrites>()
                .init_resource::<Orders>()
                .insert_resource(BattleViewSettings::default())
                .add_message::<FocusCamera>()
                .add_message::<OrderIssued>()
                .add_message::<HardpointOrderIssued>();
            let frigate = ShipDef { name: "frigate".into(), class: TargetClass::Frigate, hull: 1000.0, radius: 30.0, population: 1, ..ShipDef::default() };
            let turret = HpDef { name: "turret".into(), kind: HpKind::Weapon, targetable: true, destroyable: true, health: 100.0, ..HpDef::default() };
            let cruiser = ShipDef { name: "cruiser".into(), class: TargetClass::Capital, hull: 1.0e6, radius: 300.0, population: 1, hardpoints: vec![turret], ..ShipDef::default() };
            let catalog = Arc::new(Catalog::from_defs(vec![frigate, cruiser], vec![ProjDef { name: "laser".into(), ..ProjDef::default() }]));
            let side = |n: &str| SideSetup { name: n.into(), units: vec![UnitSetup { ship: catalog.ship_id(n).unwrap(), count: 1, tag: 1 }], structures: vec![], ai: false };
            let sim = crate::sim::SimHandle::spawn(Battle::new(BattleSetup { sides: vec![side("frigate"), side("cruiser")], catalog: catalog.clone(), config: BattleConfig::default() }));
            app.insert_resource(BattleRuntime {
                sim,
                catalog: catalog.clone(),
                visuals: Arc::new(crate::visuals::VisualCatalog { ships: vec![crate::visuals::ShipVisual::default(); 2], ..Default::default() }),
                dt: 1.0 / 30.0,
                clock: 0.0,
                cur: None,
                next: None,
                alpha: 0.0,
                fresh_events: vec![],
                entities: Default::default(),
                dying: Default::default(),
                request: None,
                player_faction: None,
                start_alive: vec![],
                retreat_countdown: 0.0,
                unit_cap: 0,
                no_spawn_radius: 0.0,
                reinforce_clearance: 0.0,
                arena_half: 10_000.0,
                fog_of_war: false,
                speed: 1.0,
                framed: false,
                reported: false,
                minimap_planets: vec![],
                minimap_props: vec![],
                minimap_tint: Color::BLACK,
            });

            // 1920 x 1080 straight down from 3000 units; screen right is +x, screen down is +z.
            let mut camera = Camera::default();
            camera.computed.target_info = Some(RenderTargetInfo { physical_size: UVec2::new(1920, 1080), scale_factor: 1.0 });
            camera.computed.clip_from_view = PerspectiveProjection { fov: 60f32.to_radians(), aspect_ratio: 1920.0 / 1080.0, ..default() }.get_clip_from_view();
            let eye = GlobalTransform::from(Transform::from_xyz(0.0, 3000.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z));
            app.world_mut().spawn((camera, BattleCamera::default(), eye));
            let window = app.world_mut().spawn((Window::default(), PrimaryWindow)).id();

            let ship = |id: u32, def: u32, side: u8, pos: Vec3, radius: f32, hardpoint: Option<Vec3>| ShipRender {
                id,
                entity: Entity::PLACEHOLDER,
                pos,
                yaw: 0.0,
                side,
                def,
                radius,
                drawn_radius: radius,
                hull: 1.0,
                shield: 0.0,
                squad: u32::MAX,
                group: 0,
                visible: true,
                order: battle_sim::Order::Idle,
                path: Vec::new(),
                hp_alive: hardpoint.iter().map(|_| true).collect(),
                hp_health: hardpoint.iter().map(|_| 1.0).collect(),
                hp_pos: hardpoint.into_iter().map(Some).collect(),
                can_retreat: true,
            };
            let (cruiser_pos, wing) = (Vec3::new(500.0, 0.0, 0.0), Vec3::new(500.0, 0.0, -700.0));
            let mut ships = app.world_mut().resource_mut::<RenderedShips>();
            ships.0.insert(1, ship(1, 0, 0, Vec3::new(-1500.0, 0.0, 600.0), 30.0, None));
            ships.0.insert(2, ship(2, 1, 1, cruiser_pos, 300.0, Some(wing)));
            app.world_mut().resource_mut::<Selection>().ships = vec![1];
            let built = art(&mut app.world_mut().resource_mut::<Assets<Image>>(), attack_frames);
            app.insert_resource(built);
            app.add_systems(Update, (control::selection, control::orders, battle_cursor, collect_orders).chain()).add_systems(Last, count_writes);

            // Where the cruiser and its wing turret are on screen, as the pick sees them.
            let mut q = app.world_mut().query_filtered::<(&Camera, &GlobalTransform), With<BattleCamera>>();
            let (cam, gt) = q.single(app.world()).unwrap();
            let centre = cam.world_to_viewport(gt, cruiser_pos).unwrap();
            let edge = cam.world_to_viewport(gt, cruiser_pos + gt.right() * 300.0).unwrap();
            let hardpoint = cam.world_to_viewport(gt, wing).unwrap();
            Self { app, window, cruiser: (centre, (edge - centre).length()), hardpoint }
        }

        /// One frame with the pointer at `at`: (pointer shown, the window's cursor is a custom picture).
        fn frame(&mut self, at: Vec2) -> (Option<&'static str>, bool) {
            self.app.world_mut().resource_mut::<Pointer>().cursor = Some(at);
            self.app.update();
            let custom = matches!(self.app.world().get::<CursorIcon>(self.window), Some(CursorIcon::Custom(_)));
            (self.app.world().resource::<ShownCursor>().name, custom)
        }
    }

    #[test]
    fn the_crosshair_comes_and_goes_exactly_where_the_pointer_enters_and_leaves_a_target_or_a_hardpoint() {
        let mut scene = Scene::new(1);
        let ((centre, radius), wing) = (scene.cruiser, scene.hardpoint);
        assert!(radius > 60.0 && (wing - centre).length() > radius + 24.0 + 40.0, "the wing turret is clear of the hull's pick circle: {radius} {wing}");
        // 120 frames: 60 across the hull along its middle (160 px each side), then 60 across the wing turret above it.
        let path: Vec<Vec2> = (0..120)
            .map(|f| {
                let k = (f % 60) as f32 / 59.0;
                let (from, to) = if f < 60 { (centre - Vec2::new(160.0, 0.0), centre + Vec2::new(160.0, 0.0)) } else { (wing - Vec2::new(80.0, 0.0), wing + Vec2::new(80.0, 0.0)) };
                from.lerp(to, k)
            })
            .collect();
        // The two hot areas: the hull's circle (its edge counts) and 24 px around the turret (its edge does not).
        let hull = |p: Vec2| (p - centre).length() <= radius;
        let turret = |p: Vec2| (p - wing).length() < 24.0;
        assert!(path.iter().all(|&p| ((p - centre).length() - radius).abs() > 0.05 && ((p - wing).length() - 24.0).abs() > 0.05), "no frame sits on a boundary");
        let seen: Vec<(Option<&str>, bool)> = path.iter().map(|&p| scene.frame(p)).collect();
        let expected: Vec<bool> = path.iter().map(|&p| hull(p) || turret(p)).collect();
        let shown: Vec<bool> = seen.iter().map(|s| s.0 == Some(names::ATTACK)).collect();
        assert_eq!(shown, expected, "the crosshair is on exactly the frames the pointer is over the hull or the turret");
        assert!(seen.iter().all(|s| s.0.is_none_or(|n| n == names::ATTACK)), "nothing else is ever shown");
        // In and out of the hull, in and out of the turret: four changes, no flicker in between.
        assert_eq!(shown.windows(2).filter(|w| w[0] != w[1]).count(), 4);
        assert!(expected.iter().filter(|&&e| e).count() > 20, "both passes spend frames on a target");
        // The window's own cursor follows the same frames, and is written on the changes only: the OS cursor is never reset in between.
        let custom: Vec<bool> = seen.iter().map(|s| s.1).collect();
        assert_eq!(custom, expected);
        let writes = &scene.app.world().resource::<CursorWrites>().0;
        let changes: Vec<bool> = std::iter::once(expected[0]).chain(expected.windows(2).map(|w| w[0] != w[1])).collect();
        assert_eq!(writes, &changes, "the window cursor is written exactly at the changes");
    }

    #[test]
    fn right_click_orders_work_with_the_crosshair_up_and_without_it() {
        let mut scene = Scene::new(1);
        let (centre, wing) = (scene.cruiser.0, scene.hardpoint);
        let click = |scene: &mut Scene, at: Vec2| {
            scene.app.world_mut().resource_mut::<Pointer>().right_pressed = true;
            let shown = scene.frame(at).0;
            scene.app.world_mut().resource_mut::<Pointer>().right_pressed = false;
            scene.frame(at);
            shown
        };
        // On the hull and on the turret: the crosshair is up and the click is an attack; on open ground: the arrow and a move.
        assert_eq!(click(&mut scene, centre), Some(names::ATTACK));
        assert_eq!(click(&mut scene, wing), Some(names::ATTACK));
        assert_eq!(click(&mut scene, centre + Vec2::new(400.0, 300.0)), None);
        assert_eq!(scene.app.world().resource::<Orders>().0, [OrderKind::Attack, OrderKind::Attack, OrderKind::Move]);
    }

    #[test]
    fn the_arrow_stays_over_the_interface_outside_the_window_and_during_a_box_drag() {
        let mut scene = Scene::new(1);
        let centre = scene.cruiser.0;
        assert_eq!(scene.frame(centre).0, Some(names::ATTACK));
        // The interface under the pointer: the arrow, and the window's cursor goes back to the system's.
        scene.app.world_mut().insert_resource(UiBlock(true, None));
        assert_eq!(scene.frame(centre), (None, false));
        scene.app.world_mut().insert_resource(UiBlock(false, None));
        assert_eq!(scene.frame(centre).0, Some(names::ATTACK));
        // Outside the window there is no pointer position.
        scene.app.world_mut().resource_mut::<Pointer>().cursor = None;
        scene.app.update();
        assert_eq!(scene.app.world().resource::<ShownCursor>().name, None);
        // A box drag that passes the enemy keeps the arrow.
        let start = centre + Vec2::new(300.0, 100.0);
        {
            let mut pointer = scene.app.world_mut().resource_mut::<Pointer>();
            (pointer.cursor, pointer.left_pressed, pointer.left_down) = (Some(start), true, true);
        }
        scene.app.update();
        scene.app.world_mut().resource_mut::<Pointer>().left_pressed = false;
        assert_eq!(scene.frame(centre).0, None, "dragging a box over the cruiser: no crosshair");
        scene.app.world_mut().resource_mut::<Pointer>().left_down = false;
        assert_eq!(scene.frame(centre).0, Some(names::ATTACK), "the box is done");
    }

    #[test]
    fn the_arrow_is_back_when_the_battle_ends() {
        use bevy::ecs::system::RunSystemOnce;
        let mut scene = Scene::new(1);
        let centre = scene.cruiser.0;
        assert_eq!(scene.frame(centre), (Some(names::ATTACK), true));
        scene.app.world_mut().run_system_once(restore_cursor).expect("restore");
        assert_eq!(scene.app.world().get::<CursorIcon>(scene.window), Some(&CursorIcon::default()));
        assert_eq!(scene.app.world().resource::<ShownCursor>().name, None);
        assert!(scene.app.world().get_resource::<CursorArt>().is_some(), "the pictures stay for the next battle");
    }

    #[test]
    fn an_armed_order_shows_its_pointer_and_a_right_click_puts_the_arrow_back() {
        let mut scene = Scene::new(1);
        let centre = scene.cruiser.0;
        let open = centre + Vec2::new(400.0, 300.0);
        scene.app.world_mut().resource_mut::<ArmedOrder>().0 = Some(ArmedKind::Attack);
        assert_eq!(scene.frame(centre).0, Some(names::ATTACK_ONLY_TARGET));
        assert_eq!(scene.frame(open).0, Some(names::ATTACK_ONLY_NO_TARGET));
        scene.app.world_mut().resource_mut::<ArmedOrder>().0 = Some(ArmedKind::Move);
        assert_eq!(scene.frame(open).0, Some(names::MOVE_ONLY));
        scene.app.world_mut().resource_mut::<ArmedOrder>().0 = Some(ArmedKind::AttackMove);
        assert_eq!(scene.frame(centre).0, Some(names::ATTACK_MOVE));
        // The right click cancels the armed order (`control::orders`): the pointer is the plain arrow again over open ground.
        scene.app.world_mut().resource_mut::<Pointer>().right_pressed = true;
        scene.frame(open);
        scene.app.world_mut().resource_mut::<Pointer>().right_pressed = false;
        assert_eq!(scene.frame(open).0, None);
        assert!(scene.app.world().resource::<Orders>().0.is_empty(), "cancelling is not an order");
    }

    #[test]
    fn the_crosshair_animation_runs_in_order_and_writes_the_window_only_when_the_picture_changes() {
        let mut scene = Scene::new(8);
        let centre = scene.cruiser.0;
        let attack = scene.app.world().resource::<CursorArt>().get(names::ATTACK).cloned().expect("crosshair");
        let mut indices = Vec::new();
        for _ in 0..120 {
            scene.frame(centre);
            let Some(CursorIcon::Custom(CustomCursor::Image(c))) = scene.app.world().get::<CursorIcon>(scene.window) else { panic!("custom cursor") };
            indices.push(attack.frames.iter().position(|h| *h == c.handle).expect("one of the crosshair's pictures"));
            assert_eq!(c.hotspot, (16, 15), "the data's hotspot");
        }
        // 120 frames of 1/60 s at 6 engine frames a picture: 20 steps, the 8 pictures in order and around again.
        let steps: Vec<usize> = indices.windows(2).filter(|w| w[0] != w[1]).map(|w| w[1]).collect();
        assert!((19..=21).contains(&steps.len()), "{steps:?}");
        assert!(steps.iter().enumerate().all(|(k, &i)| i == (k + 1) % 8), "{steps:?}");
        let writes = &scene.app.world().resource::<CursorWrites>().0;
        assert_eq!(writes.iter().filter(|&&w| w).count(), steps.len() + 1, "one write for the crosshair coming up, one for each next picture");
    }

    /// The real file and textures of the Remake (skipped without the game): the data's own numbers for the crosshair and the arrows.
    #[test]
    fn the_remake_data_gives_the_crosshair_its_eight_pictures_and_hotspot() {
        let Some(install) = alamo_fs::GameInstall::detect().filter(|i| i.mod_data("1770851727").is_dir()) else {
            eprintln!("skipped: the Remake is not installed");
            return;
        };
        let fs = Arc::new(alamo_fs::GameFs::for_mod(&install, Some("1770851727")).expect("game files"));
        let table = PointerTable::load(fs.as_ref());
        for name in names::ALL {
            assert!(table.get(name).is_some(), "MousePointers.xml has {name}");
        }
        assert_eq!(table.get(names::ATTACK).map(|d| (d.texture.as_str(), d.hotspot, d.frame_delay)), Some(("ATTACKPOINTER_00.TGA", (16, 16), 6)));
        assert_eq!(table.get(names::ATTACK_MOVE).map(|d| (d.hotspot, d.frame_delay)), Some(((0, 0), 6)));
        let library = eaw_ui::textures::TextureLibrary::new(fs);
        let mut images = Assets::<Image>::default();
        let art = CursorArt::build(&table, |n| library.load(n), &mut images);
        for name in names::ALL {
            assert!(art.get(name).is_some(), "pictures of {name}");
        }
        let attack = art.get(names::ATTACK).unwrap();
        assert_eq!((attack.frames.len(), attack.hotspot), (8, (16, 16)));
        assert!((attack.frame_seconds - 0.1).abs() < 1e-6);
        assert_eq!(images.get(&attack.frames[0]).map(|i| (i.width(), i.height())), Some((32, 32)));
        assert_eq!(art.get(names::ATTACK_ONLY_NO_TARGET).map(|p| p.frames.len()), Some(1), "the greyed crosshair has one picture");
        assert_eq!(art.get(names::ATTACK_MOVE).map(|p| p.frames.len()), Some(8));
        assert_eq!(art.get(names::MOVE_ONLY).map(|p| p.frames.len()), Some(8));
        assert_eq!(images.len(), 34, "crosshair 8, greyed 1, red arrow 8, green arrow 8, burst 8, crossed 1: the attack button's pointer shares the crosshair's pictures");
    }
}
