//! Widgets: spawning the command bars and keeping them in sync with [`CommandBarData`].
//!
//! Layout is in *design pixels* of the original 1366x768 interface; [`crate::EawUiSettings`]
//! turns that into real pixels through Bevy's [`UiScale`], so the bars scale with the screen
//! height exactly like in the game and stay anchored to the bottom-left corner.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::ui::{ComputedNode, FocusPolicy, RelativeCursorPosition, UiGlobalTransform};
use bevy::window::PrimaryWindow;

use crate::EawUiAssets;
use crate::EawUiSettings;
use crate::components::{CommandBarComponent, CommandBarDb, ComponentKind};
use crate::data::{CommandBarData, PadRemap, EawButtonClicked, EawButtonState, ForcedTooltip, TooltipKeepOut, TooltipSuppress, WorldTooltip};
use crate::faction::UiFaction;
use crate::panels::{
    MinimapKind, MinimapSurface, PanelBarName, PanelData, PanelSlotName, PanelText, is_panel_slot,
};
use crate::shell::{ShellLayout, rasterize};
use crate::textures::{UiTexture, UiTextures};

/// Which command bar to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CommandBarMode {
    /// Galactic conquest map (`i_galactic_controls_new.alo`).
    #[default]
    Galactic,
    /// Space battle (`i_tactical_controls_new.alo`).
    SpaceTactical,
}

impl CommandBarMode {
    /// Shell model of this bar.
    pub fn model(self) -> &'static str {
        match self {
            Self::Galactic => "I_Galactic_Controls_New.ALO",
            Self::SpaceTactical => "I_Tactical_Controls_New.ALO",
        }
    }
}

/// Root of a spawned command bar.
#[derive(Component, Debug, Clone, Copy)]
pub struct EawCommandBar {
    /// Which bar.
    pub mode: CommandBarMode,
    /// Faction skin.
    pub faction: UiFaction,
}

/// A clickable component. `name` is the lower-cased component name.
#[derive(Component, Debug, Clone)]
pub struct EawButton {
    /// Lower-cased component name.
    pub name: String,
    /// Tooltip text ids (`Tooltip_Text`).
    pub tooltips: Vec<String>,
    /// Literal tooltip text that overrides `tooltips` (set for data driven slots).
    pub tooltip_literal: Option<String>,
    /// Icon shown in the tooltip header (texture name; data driven slots).
    pub tooltip_icon: Option<String>,
    /// Tooltip of the small upkeep/income marker of a build pad (shown while the cursor is over the marker).
    pub marker_tooltip: Option<String>,
    /// Keeps its pressed look after a click.
    pub toggle: bool,
    /// Can be dragged onto other draggable buttons ([`crate::panels::EawSlotDropped`]).
    pub drag: bool,
    /// Whether the mouse was pressed on it (click fires on release).
    armed: bool,
}

#[derive(Component, Default)]
pub struct ButtonLayers {
    icon: Option<Entity>,
    over: Option<Entity>,
    selected: Option<Entity>,
    disabled: Option<Entity>,
}

/// Place of an ability button in the shell: its top left corner (design px), and the `<Scale>` of its component, at which
/// [`ability_columns`] draws the whole button while it moves the button above its card.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct AbilityHome {
    corner: Vec2,
    scale: f32,
}

/// What a slot kind is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Queue,
    Pad,
    Select,
    Ability,
    Border,
    /// Filled by name from [`PanelData`].
    Panel,
}

/// A component whose content comes from a list in [`CommandBarData`].
#[derive(Component, Debug, Clone, Copy)]
pub struct Slot {
    kind: SlotKind,
    index: usize,
}

/// Space production lane (queue slots 0..5, left).
const LANE_SPACE: Color = Color::srgb(0.25, 0.55, 1.0);
/// Ground production lane (queue slots 5..10, right).
const LANE_GROUND: Color = Color::srgb(1.0, 0.6, 0.15);

/// Which part of a slot a child entity renders.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotPart {
    Icon,
    Progress,
    Label,
    /// Credit icon of an upkeep marker (red: costs credits per week, green: earns them).
    UpkeepIcon,
    /// The "+" next to the upkeep icon.
    UpkeepPlus,
    /// Roman level numeral (starbase levels).
    Level,
    /// Coloured strip above a production queue slot: blue for the space lane, orange for the ground lane.
    LaneMark,
    /// Small hero badge on the round portrait of a hero in a fleet box.
    HeroBadge,
}

/// Text label of a button (`b_retreat` countdown, `b_reinforcement` points); holds the button name.
#[derive(Component, Debug, Clone)]
pub struct ButtonLabel(pub String);

/// A discrete bar of the selection (`s_health_NN`, `s_shield_NN`).
#[derive(Component, Debug, Clone, Copy)]
pub struct StatusBar {
    index: usize,
    shield: bool,
    max_level: u32,
}

#[derive(Component)]
pub struct BarLayer {
    fill: bool,
}

#[derive(Component, Debug, Clone)]
pub struct BarTextures {
    frame: Vec<Handle<Image>>,
    fill: Vec<Handle<Image>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Credits,
    Income,
    PlanetName,
    Day,
    MonthIncome,
    TechLevel,
    Pop,
    Allegiance,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct BoundField(Field);

#[derive(Component)]
pub struct Tooltip;

fn rgba(c: [u8; 4]) -> Color {
    Color::srgba_u8(c[0], c[1], c[2], c[3])
}

fn field_for(name: &str) -> Option<Field> {
    Some(match name {
        "text_credits" | "text_credits_tactical" => Field::Credits,
        "text_planetary_income" => Field::Income,
        "text_planet" | "text_planet_tactical" => Field::PlanetName,
        "text_day_counter" => Field::Day,
        "text_day_credits" => Field::MonthIncome,
        "text_tech_level" | "text_tactical_tech" => Field::TechLevel,
        "text_pop_cap" | "text_planetary_pop" => Field::Pop,
        "text_allegiance_a" | "text_allegiance_tactical" => Field::Allegiance,
        _ => return None,
    })
}

/// Unit and hero slots of the planet-view fleet boxes (`q<box>_<row><col>`, not the pathfinder plate `q<box>_0000`): drawn without the tile frame.
pub(crate) fn is_fleet_box_unit(lname: &str) -> bool {
    let b = lname.as_bytes();
    b.len() == 7 && b[0] == b'q' && b[1].is_ascii_digit() && b[2] == b'_' && b[3..].iter().all(u8::is_ascii_digit) && &lname[3..] != "0000"
}

/// Builds `<prefix>NN` slot descriptors from component names.
fn slot_for(name: &str) -> Option<Slot> {
    let num = |p: &str| name.strip_prefix(p).and_then(|n| n.parse::<usize>().ok());
    if let Some(i) = num("tqueue").or_else(|| num("queue")) {
        return Some(Slot { kind: SlotKind::Queue, index: i });
    }
    if let Some(i) = num("b_create_") {
        return Some(Slot { kind: SlotKind::Pad, index: i });
    }
    if let Some(i) = num("s_select_") {
        return Some(Slot { kind: SlotKind::Select, index: i });
    }
    if let Some(i) = num("special_border_") {
        return Some(Slot { kind: SlotKind::Border, index: i });
    }
    if let Some(i) = num("special_button_") {
        return Some(Slot { kind: SlotKind::Ability, index: i });
    }
    None
}

/// Parameters of [`spawn_command_bar`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandBarSpec {
    /// Galactic or tactical.
    pub mode: CommandBarMode,
    /// Faction skin.
    pub faction: UiFaction,
}

/// Where a shell panel sits on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellAnchor {
    /// Bottom-left corner (command bars).
    #[default]
    BottomLeft,
    /// Bottom-right corner.
    BottomRight,
    /// Top-left corner.
    TopLeft,
    /// Top-right corner.
    TopRight,
    /// Top edge, horizontally centred (battle banner).
    TopCenter,
    /// Screen centre (fleet organizer, planet screen, dialogs).
    Center,
}

/// A generic shell panel: any `Art/Models/I_*.ALO` interface model.
#[derive(Debug, Clone)]
pub struct ShellSpec {
    /// Model file name, e.g. `I_Galactic_Organize_New.ALO`.
    pub model: String,
    /// Faction skin.
    pub faction: UiFaction,
    /// Screen anchor.
    pub anchor: ShellAnchor,
    /// Offset from the anchor in design pixels (x right, y up).
    pub offset: Vec2,
    /// Use this `_ALTn` art variant instead of the faction's (hero frames use a fixed one).
    pub art_alt: Option<usize>,
}

impl ShellSpec {
    /// A panel with default anchor and offset.
    pub fn new(model: &str, faction: UiFaction) -> Self {
        Self {
            model: model.to_string(),
            faction,
            anchor: ShellAnchor::Center,
            offset: Vec2::ZERO,
            art_alt: None,
        }
    }
}

/// Marks the root of a spawned shell panel.
#[derive(Component, Debug, Clone)]
pub struct EawPanel {
    /// Model the panel was built from.
    pub model: String,
}

/// Where the frame art of a shell panel is opaque, as a grid of [`OPAQUE_CELL`] design px cells over the panel root. The root is the
/// bounding box of all its art: the galactic faceplate spans the whole bar width and the height of the droid column, so most of that box
/// is open map above the build bar. Hosts test the cursor and screen rectangles against this instead of the box.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct ShellOpaque {
    /// Design size of the root.
    size: Vec2,
    cols: usize,
    rows: usize,
    cells: Vec<bool>,
}

/// Cell size of [`ShellOpaque`] in design px (about 5 screen px on a 1080p screen).
const OPAQUE_CELL: f32 = 4.0;
/// Alpha (0..255) from which a pixel of the frame art counts as interface; fainter pixels are soft edges (estimate; on the galactic bar
/// at 1080p the mask then covers the droid column, the info panel, the minimap block and the build strip, nothing above the strip).
const OPAQUE_ALPHA: u8 = 64;

impl ShellOpaque {
    fn new(size: Vec2) -> Self {
        let (cols, rows) = ((size.x / OPAQUE_CELL).ceil().max(1.0) as usize, (size.y / OPAQUE_CELL).ceil().max(1.0) as usize);
        Self { size, cols, rows, cells: vec![false; cols * rows] }
    }

    /// Marks the cells under the opaque pixels of an art image whose top-left corner sits at `top_left` (root design px, y down), with
    /// `oversample` image px per design px.
    fn add(&mut self, img: &alamo_model::image::Rgba8, top_left: Vec2, oversample: f32) {
        for y in 0..img.height {
            for x in 0..img.width {
                if img.pixels[(y * img.width + x) * 4 + 3] < OPAQUE_ALPHA {
                    continue;
                }
                let p = top_left + (Vec2::new(x as f32, y as f32) + 0.5) / oversample.max(1e-3);
                if let Some(i) = self.cell(p) {
                    self.cells[i] = true;
                }
            }
        }
    }

    fn cell(&self, p: Vec2) -> Option<usize> {
        let (c, r) = ((p.x / OPAQUE_CELL).floor(), (p.y / OPAQUE_CELL).floor());
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows).then(|| r as usize * self.cols + c as usize)
    }

    /// The root's design px of a point in physical (render target) px.
    fn local(&self, node: &ComputedNode, tf: &UiGlobalTransform, p: Vec2) -> Vec2 {
        let size = node.size().max(Vec2::splat(1e-3));
        (p - (tf.translation - size * 0.5)) * self.size / size
    }

    /// Whether the art is opaque under `p` (physical px, like `ComputedNode::contains_point`).
    pub fn hit(&self, node: &ComputedNode, tf: &UiGlobalTransform, p: Vec2) -> bool {
        self.cell(self.local(node, tf, p)).is_some_and(|i| self.cells[i])
    }

    /// The opaque art as rectangles `(lo, hi)` in physical px: the cells of each row merged into runs, equal runs of neighbouring rows
    /// merged into one rectangle (about a dozen for the galactic bar at 1080p).
    pub fn rects(&self, node: &ComputedNode, tf: &UiGlobalTransform) -> Vec<(Vec2, Vec2)> {
        let size = node.size().max(Vec2::splat(1e-3));
        let origin = tf.translation - size * 0.5;
        let k = size / self.size.max(Vec2::splat(1e-3)) * OPAQUE_CELL;
        // Open rectangles: (first col, end col, first row).
        let mut open: Vec<(usize, usize, usize)> = Vec::new();
        let mut out = Vec::new();
        let mut close = |(c0, c1, r0): (usize, usize, usize), r1: usize| {
            out.push((origin + Vec2::new(c0 as f32, r0 as f32) * k, origin + Vec2::new(c1 as f32, r1 as f32) * k));
        };
        for r in 0..=self.rows {
            let mut runs = Vec::new();
            if r < self.rows {
                let mut c = 0;
                while c < self.cols {
                    if self.cells[r * self.cols + c] {
                        let start = c;
                        while c < self.cols && self.cells[r * self.cols + c] {
                            c += 1;
                        }
                        runs.push((start, c));
                    } else {
                        c += 1;
                    }
                }
            }
            let mut next = Vec::new();
            for rect in open.drain(..) {
                match runs.iter().position(|&(a, b)| a == rect.0 && b == rect.1) {
                    Some(i) => {
                        runs.remove(i);
                        next.push(rect);
                    }
                    None => close(rect, r),
                }
            }
            next.extend(runs.into_iter().map(|(a, b)| (a, b, r)));
            open = next;
        }
        out
    }

    /// Whether any opaque art overlaps the rectangle `lo..hi` (physical px).
    pub fn overlaps(&self, node: &ComputedNode, tf: &UiGlobalTransform, lo: Vec2, hi: Vec2) -> bool {
        let (a, b) = (self.local(node, tf, lo), self.local(node, tf, hi));
        let c0 = (a.x / OPAQUE_CELL).floor().max(0.0) as usize;
        let r0 = (a.y / OPAQUE_CELL).floor().max(0.0) as usize;
        let c1 = ((b.x / OPAQUE_CELL).ceil().max(0.0) as usize).min(self.cols);
        let r1 = ((b.y / OPAQUE_CELL).ceil().max(0.0) as usize).min(self.rows);
        (r0..r1).any(|r| (c0..c1).any(|c| self.cells[r * self.cols + c]))
    }
}

struct Builder<'a> {
    world: &'a mut World,
    textures: UiTextures,
    db: CommandBarDb,
    font: Option<String>,
    root: Entity,
    root_h: f32,
    faction: UiFaction,
}

/// Spawns a command bar and returns its root entity (bottom-left anchored; despawn to remove).
/// Needs the textures to be loaded ([`crate::UiTexturesLoaded`]); returns `None` before that.
/// Prefer [`crate::EawUiCommandsExt::spawn_eaw_command_bar`], which waits for it.
pub fn spawn_command_bar(world: &mut World, spec: CommandBarSpec) -> Option<Entity> {
    let root = spawn_shell(
        world,
        &ShellSpec {
            model: spec.mode.model().to_string(),
            faction: spec.faction,
            anchor: ShellAnchor::BottomLeft,
            offset: Vec2::ZERO,
            art_alt: None,
        },
    )?;
    world.entity_mut(root).insert(EawCommandBar {
        mode: spec.mode,
        faction: spec.faction,
    });
    Some(root)
}

/// Builds any shell model as a UI panel: rasterized frame art plus every component of
/// `CommandBarComponents.xml` placed by the model's bones. Returns the root entity.
pub fn spawn_shell(world: &mut World, spec: &ShellSpec) -> Option<Entity> {
    let (fs, font, oversample, camera) = {
        let a = world.get_resource::<EawUiAssets>()?;
        let s = world.get_resource::<EawUiSettings>()?;
        (a.fs.clone(), s.font_family.clone(), s.faceplate_oversample, s.camera)
    };
    let layout = ShellLayout::load(&fs, &spec.model)?;
    if !world.get_resource::<UiTextures>()?.library().is_ready() {
        warn!("shell spawned before the UI textures finished loading");
        return None;
    }
    let textures = world.remove_resource::<UiTextures>()?;
    let db = world.remove_resource::<CommandBarDb>()?;
    let alt = spec.art_alt.unwrap_or_else(|| spec.faction.mesh_alt());
    let arts: Vec<_> = layout
        .art
        .iter()
        .filter(|a| a.alt.is_none_or(|n| n == alt))
        .filter(|a| !is_ground_component(&a.name.to_lowercase()))
        .collect();
    let (lo, hi) = bounds_of(arts.iter().map(|a| a.bounds()));
    let (width, height) = ((hi.x - lo.x).max(1.0), (hi.y - lo.y).max(1.0));

    let mut node = Node {
        position_type: PositionType::Absolute,
        width: Val::Px(width),
        height: Val::Px(height),
        ..default()
    };
    let (ox, oy) = (spec.offset.x, spec.offset.y);
    match spec.anchor {
        ShellAnchor::BottomLeft => {
            // Bar coordinates are absolute in the shell space (origin = bar corner).
            node.left = Val::Px(ox + lo.x);
            node.bottom = Val::Px(oy + lo.y);
        }
        ShellAnchor::BottomRight => {
            node.right = Val::Px(-ox);
            node.bottom = Val::Px(oy);
        }
        ShellAnchor::TopLeft => {
            node.left = Val::Px(ox);
            node.top = Val::Px(-oy);
        }
        ShellAnchor::TopRight => {
            node.right = Val::Px(-ox);
            node.top = Val::Px(-oy);
        }
        ShellAnchor::TopCenter => {
            node.left = Val::Percent(50.0);
            node.top = Val::Px(-oy);
            node.margin.left = Val::Px(-width * 0.5 + ox);
        }
        ShellAnchor::Center => {
            node.left = Val::Percent(50.0);
            node.top = Val::Percent(50.0);
            node.margin = UiRect {
                left: Val::Px(-width * 0.5 + ox),
                top: Val::Px(-height * 0.5 - oy),
                ..default()
            };
        }
    }
    let root = world
        .spawn((
            Name::new(format!("EaW panel {}", spec.model)),
            EawPanel {
                model: spec.model.clone(),
            },
            node,
            FocusPolicy::Pass,
        ))
        .id();
    if let Some(c) = camera {
        world.entity_mut(root).insert(UiTargetCamera(c));
    }
    let mut b = Builder {
        world,
        textures,
        db,
        font,
        root,
        root_h: height,
        faction: spec.faction,
    };

    // Frame art (rasterized meshes), back to front.
    let mut arts = arts;
    arts.sort_by(|a, c| a.z.total_cmp(&c.z));
    let mut decoded = HashMap::new();
    let mut opaque = ShellOpaque::new(Vec2::new(width, height));
    for art in arts {
        let tex = decoded
            .entry(art.texture.to_lowercase())
            .or_insert_with(|| b.textures.library().load(&art.texture));
        let Some(tex) = tex else { continue };
        let (alo, ahi) = art.bounds();
        let img = rasterize(&art.triangles, tex, alo, ahi, oversample);
        // `image_at` puts the image's top-left corner here (root space, y down).
        opaque.add(&img, Vec2::new(alo.x - lo.x, hi.y - ahi.y), oversample);
        let handle = b.upload_raw(&art.name, img);
        b.image_at(root, &handle, (alo + ahi) * 0.5 - lo, ahi - alo, Color::WHITE, Some(&art.name));
    }
    b.world.entity_mut(root).insert(opaque);

    // Components in draw order (z, then base layer). Components far outside the art belong to
    // other panels (e.g. the ground filters below the galactic bar).
    let mut items: Vec<_> = layout.order.iter().filter_map(|n| layout.item(n)).collect();
    // The droid's rectangle overlaps the pause / fast-forward buttons: it is built first so that they stay on top of it
    // (the press goes to the topmost button only).
    let z = |name: &str, z: f32| if name.to_lowercase().starts_with("b_droid_help") { f32::MIN } else { z };
    items.sort_by(|a, c| {
        let la = b.db.get(&a.name).map_or(0, |d| d.base_layer);
        let lc = b.db.get(&c.name).map_or(0, |d| d.base_layer);
        z(&a.name, a.z).total_cmp(&z(&c.name, c.z)).then(la.cmp(&lc))
    });
    for item in items {
        if is_ground_component(&item.name.to_lowercase()) {
            continue;
        }
        if !within(item.center, lo - Vec2::splat(MARGIN), hi + Vec2::splat(MARGIN)) {
            continue;
        }
        // The month bar's bone has no component and the data has no segment art: draw the segments.
        if item.name.eq_ignore_ascii_case("fight_bar_g") {
            b.spawn_segment_bar(item.center - lo, item.size);
            continue;
        }
        let Some(def) = b.db.get(&item.name).cloned() else {
            continue;
        };
        b.spawn_component(&def, item.center - lo, item.size);
    }

    let Builder {
        world, textures, db, ..
    } = b;
    world.insert_resource(textures);
    world.insert_resource(db);
    Some(root)
}

/// Ground (land) interface parts are out of scope and never spawned.
pub fn is_ground_component(lname: &str) -> bool {
    ["land_fleet_", "ground_forces", "drage_drop", "unit_pad", "build_pad", "land_art", "b_filters_walker", "b_filters_infantry", "b_filters_tracked", "b_filters_structure"]
        .iter()
        .any(|p| lname.starts_with(p))
}

/// Components further than this (design px) outside the frame art are not part of the panel.
const MARGIN: f32 = 40.0;

fn within(p: Vec2, lo: Vec2, hi: Vec2) -> bool {
    p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y
}

/// Union of `(min, max)` rectangles; an empty iterator gives `(0, 0)..(1, 1)`.
pub fn bounds_of(rects: impl Iterator<Item = (Vec2, Vec2)>) -> (Vec2, Vec2) {
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for (l, h) in rects {
        lo = lo.min(l);
        hi = hi.max(h);
    }
    if lo.x > hi.x { (Vec2::ZERO, Vec2::ONE) } else { (lo, hi) }
}
impl Builder<'_> {
    /// The credit symbol as a white mask: the original tints it with the text colour (the file itself is yellow).
    fn credit_icon(&mut self) -> Option<UiTexture> {
        let mut raw = self.textures.library().load("i_icon_credit.tga")?;
        // The symbol is drawn in the colour channels (yellow on black); it becomes the alpha of a white image.
        for px in raw.pixels.as_chunks_mut::<4>().0 {
            let lum = px[0].max(px[1]).max(px[2]);
            px[3] = (u16::from(lum) * u16::from(px[3]) / 255) as u8;
            px[..3].fill(255);
        }
        let size = UVec2::new(raw.width as u32, raw.height as u32);
        let handle = self.upload_raw("credit_icon_white", raw);
        Some(UiTexture { handle, size })
    }

    fn hide(&mut self, e: Entity) {
        if let Some(mut n) = self.world.entity_mut(e).get_mut::<Node>() {
            n.display = Display::None;
        }
    }

    fn tex(&mut self, name: &str) -> Option<UiTexture> {
        let mut images = self.world.get_resource_mut::<Assets<Image>>()?;
        self.textures.get(&mut images, name)
    }

    fn upload_raw(&mut self, key: &str, img: alamo_model::image::Rgba8) -> Handle<Image> {
        let mut images = self.world.resource_mut::<Assets<Image>>();
        self.textures.insert(&mut images, key, img).handle
    }

    /// Spawns an image child of `parent`, centred at `center` (design px, y up, relative to the
    /// bar origin when `parent` is the root).
    fn image_at(
        &mut self,
        parent: Entity,
        image: &Handle<Image>,
        center: Vec2,
        size: Vec2,
        tint: Color,
        name: Option<&str>,
    ) -> Entity {
        let mut e = self.world.spawn((
            ImageNode::new(image.clone()).with_color(tint),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(center.x - size.x * 0.5),
                top: Val::Px(self.root_h - center.y - size.y * 0.5),
                width: Val::Px(size.x),
                height: Val::Px(size.y),
                ..default()
            },
            FocusPolicy::Pass,
            ChildOf(parent),
        ));
        if let Some(n) = name {
            e.insert(Name::new(n.to_string()));
        }
        e.id()
    }

    /// Image centred inside a component rectangle (`rect_size`), offset in design px (y up).
    fn layer(
        &mut self,
        parent: Entity,
        rect_size: Vec2,
        tex: &UiTexture,
        size: Vec2,
        offset: Vec2,
        tint: Color,
    ) -> Entity {
        self.world
            .spawn((
                ImageNode::new(tex.handle.clone()).with_color(tint),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px((rect_size.x - size.x) * 0.5 + offset.x),
                    top: Val::Px((rect_size.y - size.y) * 0.5 - offset.y),
                    width: Val::Px(size.x),
                    height: Val::Px(size.y),
                    ..default()
                },
                FocusPolicy::Pass,
                ChildOf(parent),
            ))
            .id()
    }

    fn text_layer(
        &mut self,
        parent: Entity,
        rect_size: Vec2,
        def: &CommandBarComponent,
        text: &str,
        offset: Vec2,
        color: Color,
    ) -> Entity {
        let justify = if def.right_justified {
            JustifyContent::FlexEnd
        } else if def.left_justified {
            JustifyContent::FlexStart
        } else {
            JustifyContent::Center
        };
        let font = match &self.font {
            Some(f) => TextFont {
                font: FontSource::Family(f.as_str().into()),
                font_size: FontSize::Px(def.font_size * 1.45),
                ..default()
            },
            None => TextFont::from_font_size(def.font_size * 1.45),
        };
        let holder = self
            .world
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(if def.right_justified { offset.x.min(0.0) } else { offset.x }),
                    top: Val::Px(-offset.y),
                    width: Val::Px(rect_size.x),
                    height: Val::Px(rect_size.y),
                    padding: if def.right_justified {
                        UiRect::right(Val::Px(7.0))
                    } else if def.left_justified {
                        UiRect::left(Val::Px(4.0))
                    } else {
                        UiRect::ZERO
                    },
                    justify_content: justify,
                    align_items: AlignItems::Center,
                    ..default()
                },
                FocusPolicy::Pass,
                ChildOf(parent),
            ))
            .id();
        self.world
            .spawn((
                Text::new(text),
                font,
                TextColor(color),
                TextLayout::no_wrap().with_justify(if def.right_justified {
                    Justify::Right
                } else if def.left_justified {
                    Justify::Left
                } else {
                    Justify::Center
                }),
                TextShadow {
                    offset: Vec2::splat(1.0),
                    color: Color::srgba(0.0, 0.0, 0.0, 0.85),
                },
                FocusPolicy::Pass,
                ChildOf(holder),
            ))
            .id()
    }

    fn spawn_component(&mut self, def: &CommandBarComponent, center: Vec2, size: Vec2) {
        let lname = def.name.to_lowercase();
        // The help droid button is only the question mark at the bottom of its tall rectangle (R2 sits above it).
        let (center, size) = match (lname.as_str(), def.blank.first().and_then(|n| self.tex(n))) {
            ("b_droid_help", Some(t)) => {
                let sz = (t.size.as_vec2() * def.scale).min(size);
                (Vec2::new(center.x - (size.x - sz.x) * 0.5, center.y - (size.y - sz.y) * 0.5), sz)
            }
            _ => (center, size),
        };
        let fi = self.faction.list_index();

        // Discrete bars.
        if def.kind == Some(ComponentKind::Bar) {
            let parsed = lname
                .strip_prefix("s_health_")
                .map(|n| (n, false))
                .or_else(|| lname.strip_prefix("s_shield_").map(|n| (n, true)))
                .and_then(|(n, s)| n.parse::<usize>().ok().map(|i| (i, s)));
            // Other bars (hero health, ...) are driven by name through `PanelData::bars`.
            let (index, shield) = parsed.unwrap_or((0, false));
            let bar_w = if parsed.is_some() { 34.0 } else { size.x.max(8.0) };
            let bar_h = if parsed.is_some() { 2.0 } else { size.y.clamp(2.0, 6.0) };
            let frame = def
                .bar_texture
                .iter()
                .filter_map(|n| self.tex(n).map(|t| t.handle))
                .collect();
            let fill = def
                .bar_overlay
                .iter()
                .filter_map(|n| self.tex(n).map(|t| t.handle))
                .collect();
            let bar = self
                .world
                .spawn((
                    Name::new(def.name.clone()),
                    StatusBar {
                        index,
                        shield,
                        max_level: def.max_bar_level.max(1),
                    },
                    BarTextures { frame, fill },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(center.x - bar_w * 0.5),
                        top: Val::Px(self.root_h - center.y - bar_h * 0.5),
                        width: Val::Px(bar_w),
                        height: Val::Px(bar_h),
                        display: Display::None,
                        ..default()
                    },
                    ChildOf(self.root),
                ))
                .id();
            if parsed.is_none() {
                self.world.entity_mut(bar).insert(PanelBarName(lname.clone()));
            }
            for fill in [false, true] {
                self.world.spawn((
                    BarLayer { fill },
                    ImageNode::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    ChildOf(bar),
                ));
            }
            return;
        }

        // Minimap click surface (the faceplate art provides the frame).
        if matches!(lname.as_str(), "radar_galactic" | "radar" | "radar_galactic_adv_map") {
            self.world.spawn((
                Name::new(def.name.clone()),
                MinimapSurface {
                    kind: if lname == "radar" {
                        MinimapKind::Tactical
                    } else {
                        MinimapKind::Galactic
                    },
                },
                Button,
                RelativeCursorPosition::default(),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(center.x - size.x * 0.5),
                    top: Val::Px(self.root_h - center.y - size.y * 0.5),
                    width: Val::Px(size.x),
                    height: Val::Px(size.y),
                    overflow: Overflow::clip(),
                    ..default()
                },
                ChildOf(self.root),
            ));
            return;
        }

        // Plain text fields.
        if let Some(field) = field_for(&lname) {
            let holder = self
                .world
                .spawn((
                    Name::new(def.name.clone()),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(center.x - size.x * 0.5),
                        top: Val::Px(self.root_h - center.y - size.y * 0.5),
                        width: Val::Px(size.x),
                        height: Val::Px(size.y),
                        ..default()
                    },
                    FocusPolicy::Pass,
                    ChildOf(self.root),
                ))
                .id();
            if field == Field::Allegiance {
                let list = def.icon_alt.clone();
                let tex = CommandBarComponent::pick(&list, fi).and_then(|n| self.tex(n));
                if let Some(t) = tex {
                    let sz = t.size.as_vec2() * def.scale;
                    let l = self.layer(holder, size, &t, sz, def.icon_offset, Color::WHITE);
                    self.world.entity_mut(l).insert(BoundField(field));
                }
                return;
            }
            let color = def.text_color.map_or(Color::WHITE, rgba);
            let t = self.text_layer(holder, size, def, "", def.text_offset, color);
            self.world.entity_mut(t).insert(BoundField(field));
            if field == Field::Pop && let Some(mut font) = self.world.entity_mut(t).get_mut::<TextFont>() && let FontSize::Px(px) = font.font_size {
                // The component asks for 8 pt, the original draws it like the other read-outs.
                font.font_size = FontSize::Px(px * 0.8);
            }
            if matches!(field, Field::Credits | Field::Income | Field::MonthIncome)
                && let Some(tx) = self.credit_icon()
            {
                let sz = tx.size.as_vec2() * 0.6;
                let icon = self.layer(holder, size, &tx, sz, Vec2::new(-size.x * 0.5 + sz.x * 0.5 + 2.0, 0.0), PALE);
                if field == Field::Income {
                    self.world.entity_mut(icon).insert(BoundField(field));
                }
            }
            if let Some(icon) = def.icon.first().cloned()
                && let Some(tx) = self.tex(&icon)
            {
                let sz = tx.size.as_vec2() * def.scale;
                self.layer(holder, size, &tx, sz, def.icon_offset, Color::WHITE);
            }
            return;
        }

        if !matches!(
            def.kind,
            Some(ComponentKind::Button | ComponentKind::TextButton)
        ) {
            return;
        }
        // Components that are only hit areas (no texture at all) are not drawn.
        let has_art = [
            &def.blank,
            &def.icon,
            &def.icon_alt,
            &def.mouse_over,
            &def.selected,
            &def.disabled,
        ]
        .iter()
        .any(|l| !l.is_empty());
        let slot = slot_for(&lname)
            .or_else(|| is_panel_slot(&lname).then_some(Slot { kind: SlotKind::Panel, index: 0 }));
        if !has_art && slot.is_none() {
            // Text components of shell panels (names, counters) are driven by `PanelData::texts`.
            if def.kind == Some(ComponentKind::TextButton) && def.font_name.is_some() {
                let holder = self
                    .world
                    .spawn((
                        Name::new(def.name.clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(center.x - size.x * 0.5),
                            top: Val::Px(self.root_h - center.y - size.y * 0.5),
                            width: Val::Px(size.x),
                            height: Val::Px(size.y),
                            ..default()
                        },
                        FocusPolicy::Pass,
                        ChildOf(self.root),
                    ))
                    .id();
                let color = def.text_color.map_or(Color::WHITE, rgba);
                let t = self.text_layer(holder, size, def, "", def.text_offset, color);
                self.world.entity_mut(t).insert(PanelText(lname.clone()));
            }
            return;
        }
        let alt_icons = CommandBarComponent::pick(&def.icon_alt, usize::from(lname == "b_zoom")).is_some();

        let button = self
            .world
            .spawn((
                Name::new(def.name.clone()),
                EawButton {
                    name: lname.clone(),
                    tooltips: def.tooltips.clone(),
                    tooltip_literal: None,
                    tooltip_icon: None,
                    marker_tooltip: None,
                    toggle: def.toggle,
                    drag: def.drag_and_drop,
                    armed: false,
                },
                EawButtonState {
                    disabled: def.start_disabled,
                    ..default()
                },
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(center.x - size.x * 0.5),
                    top: Val::Px(self.root_h - center.y - size.y * 0.5),
                    width: Val::Px(size.x),
                    height: Val::Px(size.y),
                    // Tab textures are tall sheets of two tiles; the offsets pick one, the rest must not show.
                    overflow: if def.tab { Overflow::clip() } else { Overflow::visible() },
                    ..default()
                },
                ChildOf(self.root),
            ))
            .id();
        if let Some(s) = slot {
            self.world.entity_mut(button).insert(s);
            if s.kind == SlotKind::Panel {
                self.world.entity_mut(button).insert(PanelSlotName(lname.clone()));
            }
            if s.kind == SlotKind::Ability {
                self.world.entity_mut(button).insert(AbilityHome {
                    corner: Vec2::new(center.x - size.x * 0.5, self.root_h - center.y - size.y * 0.5),
                    scale: if def.scale > 0.0 { def.scale } else { 1.0 },
                });
            }
            // Slots stay invisible until data fills them.
            self.world.entity_mut(button).insert(Visibility::Hidden);
            self.hide(button);
        }

        let mut layers = ButtonLayers::default();
        let mut icon_px = 40.0f32;
        let tint = def.color.map_or(Color::WHITE, rgba);
        // Texture size x Scale, but never much larger than the component rectangle (some panels
        // reuse components with a Scale tuned for a different layout).
        let scaled = |t: &UiTexture| {
            let s = t.size.as_vec2() * def.scale;
            let cap = size.max_element().max(8.0) * 1.5;
            s * (cap / s.max_element().max(1.0)).min(1.0)
        };
        // Ability buttons draw every layer at its natural size; `ability_columns` scales the whole button by the `<Scale>`.
        let ability = slot.is_some_and(|s| s.kind == SlotKind::Ability);
        let layer_size = |t: &UiTexture| if ability { t.size.as_vec2() } else { scaled(t) };
        if let Some(t) = CommandBarComponent::pick(&def.blank, fi).and_then(|n| self.tex(n)) {
            let mut sz = scaled(&t);
            icon_px = (sz.min_element() * 0.84).max(20.0);
            if ability {
                // The ability border is drawn at its natural size and the icon fills it (the shell rectangle is 24x25, the textures
                // are 26x26); the whole button then shows at the component's Scale 0.8, as the Remake draws it (video b0700: icons
                // 23-26 px at 1080p next to a 77 px card pitch).
                sz = t.size.as_vec2();
                icon_px = sz.min_element() * 0.88;
            }
            // Tactical selection cards are flat in the Remake (unit icon over its bars): no pad frame.
            if !slot.is_some_and(|s| s.kind == SlotKind::Select) && !is_fleet_box_unit(&lname) {
                self.layer(button, size, &t, sz, Vec2::ZERO, tint);
            }
        }
        let icon_tex = CommandBarComponent::pick(&def.icon, fi)
            .or_else(|| {
                alt_icons
                    .then(|| CommandBarComponent::pick(&def.icon_alt, usize::from(lname == "b_zoom")))
                    .flatten()
            })
            .and_then(|n| self.tex(n));
        // The selection and ability slots have dynamic icons driven by SlotPart::Icon, so skip static icon layer.
        if let Some(t) = icon_tex.as_ref().filter(|_| !slot.is_some_and(|s| matches!(s.kind, SlotKind::Select | SlotKind::Ability))) {
            let sz = scaled(t);
            layers.icon = Some(self.layer(button, size, t, sz, def.icon_offset, tint));
        }
        if let Some(t) = CommandBarComponent::pick(&def.disabled, fi).and_then(|n| self.tex(n)) {
            let sz = layer_size(&t);
            let e = self.layer(button, size, &t, sz, def.icon_offset, tint);
            self.hide(e);
            layers.disabled = Some(e);
        }
        if let Some(t) = CommandBarComponent::pick(&def.selected, fi).and_then(|n| self.tex(n)) {
            let sz = layer_size(&t);
            let e = self.layer(button, size, &t, sz, def.icon_offset, tint);
            self.hide(e);
            layers.selected = Some(e);
        }
        if let Some(t) = CommandBarComponent::pick(&def.mouse_over, fi).and_then(|n| self.tex(n)) {
            let sz = layer_size(&t);
            let e = self.layer(button, size, &t, sz, def.mouse_over_offset, tint);
            self.hide(e);
            layers.over = Some(e);
        }
        self.world.entity_mut(button).insert(layers);
        if lname.starts_with("choice_button_") {
            // Labels of the battle banner buttons come from `PanelData::texts`.
            let color = def.text_color.map_or(Color::WHITE, rgba);
            let l = self.text_layer(button, size, def, "", Vec2::ZERO, color);
            // The original's banner labels are large (about 60% of the button height).
            if let Some(mut font) = self.world.entity_mut(l).get_mut::<TextFont>()
                && let FontSize::Px(px) = font.font_size
            {
                font.font_size = FontSize::Px(px * 1.55);
            }
            self.world.entity_mut(l).insert(PanelText(lname.clone()));
        }
        if matches!(lname.as_str(), "b_retreat" | "b_reinforcement") {
            let off = Vec2::new(0.0, -size.y * 0.5 - 4.0);
            let l = self.text_layer(button, size, def, "", off, Color::srgb(0.9, 0.95, 1.0));
            self.world.entity_mut(l).insert(ButtonLabel(lname.clone()));
        }

        if let Some(s) = slot {
            // Slot content: unit icon, build progress, count/price label.
            let icon = self
                .world
                .spawn((
                    SlotPart::Icon,
                    ImageNode::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(size.x * 0.5 - icon_px * 0.5),
                        top: Val::Px(size.y * 0.5 - icon_px * 0.5),
                        width: Val::Px(icon_px),
                        height: Val::Px(icon_px),
                        display: Display::None,
                        ..default()
                    },
                    FocusPolicy::Pass,
                    ChildOf(button),
                ))
                .id();
            let _ = icon;
            if s.kind == SlotKind::Panel && is_fleet_box_unit(&lname) && let Some(t) = self.tex("i_icon_hero_you.tga") {
                let badge = icon_px * 0.45;
                self.world.spawn((
                    SlotPart::HeroBadge,
                    ImageNode::new(t.handle.clone()),
                    Node { position_type: PositionType::Absolute, left: Val::Px(size.x * 0.5 - icon_px * 0.6), top: Val::Px(size.y * 0.5 + icon_px * 0.1), width: Val::Px(badge), height: Val::Px(badge), display: Display::None, ..default() },
                    FocusPolicy::Pass,
                    ChildOf(button),
                ));
            }
            if s.kind == SlotKind::Queue {
                self.world.spawn((
                    SlotPart::LaneMark,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(size.x * 0.5 - icon_px * 0.5),
                        top: Val::Px(size.y * 0.5 - icon_px * 0.5 - 4.0),
                        width: Val::Px(icon_px),
                        height: Val::Px(3.0),
                        ..default()
                    },
                    BackgroundColor(LANE_SPACE),
                    FocusPolicy::Pass,
                    ChildOf(button),
                ));
            }
            self.world.spawn((
                SlotPart::Progress,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(size.x * 0.5 - icon_px * 0.5),
                    bottom: Val::Px(size.y * 0.5 - icon_px * 0.5),
                    width: Val::Px(icon_px),
                    height: Val::Px(0.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.5, 0.9, 0.45)),
                FocusPolicy::Pass,
                ChildOf(button),
            ));
            let label_off = if def.text_offset2 != Vec2::ZERO {
                def.text_offset2
            } else if def.text_offset == Vec2::ZERO {
                Vec2::new(0.0, -size.y * 0.5 - 6.0)
            } else {
                def.text_offset
            };
            // Prices and counts are white in the original whatever the component says, and larger than the default.
            let l = self.text_layer(button, size, def, "", label_off, Color::srgb(0.93, 0.95, 0.98));
            if let Some(mut font) = self.world.entity_mut(l).get_mut::<TextFont>()
                && let FontSize::Px(px) = font.font_size
            {
                font.font_size = FontSize::Px(px * 1.25);
            }
            self.world.entity_mut(l).insert(SlotPart::Label);
            if s.kind == SlotKind::Pad {
                // Upkeep marker (top left) and level numeral (top right) of build pads.
                if let Some(t) = self.credit_icon() {
                    let e = self.world.spawn((
                        SlotPart::UpkeepIcon,
                        ImageNode::new(t.handle.clone()),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(7.0),
                            top: Val::Px(-2.0),
                            width: Val::Px(13.0),
                            height: Val::Px(13.0),
                            display: Display::None,
                            ..default()
                        },
                        FocusPolicy::Pass,
                        ChildOf(button),
                    ));
                    let _ = e;
                }
                let p = self.text_layer(button, size, def, "", Vec2::new(-size.x * 0.5 + 3.0, size.y * 0.5 - 8.0), Color::srgb(1.0, 0.25, 0.2));
                self.world.entity_mut(p).insert(SlotPart::UpkeepPlus);
                let l = self.text_layer(button, size, def, "", Vec2::new(size.x * 0.40, size.y * 0.5 - 8.0), Color::srgb(0.9, 0.93, 0.98));
                self.world.entity_mut(l).insert(SlotPart::Level);
            }
            if s.kind == SlotKind::Ability {
                // The animated outline the original runs over an ability button while the ability is on (`Icon_Alternate_Texture_Name`
                // sa_ami_outline_00..12 at `Anim_FPS`, looped): without it a toggled-on Hunt or S-foil lock looked exactly like an off one
                // (the `Selected_Texture_Name` frame is all but the normal border).
                let fps = def.raw.get("anim_fps").and_then(|v| v.trim().parse::<f32>().ok()).filter(|f| *f > 0.0).unwrap_or(8.0);
                let frames: Vec<UiTexture> = def.icon_alt.iter().filter_map(|n| self.tex(n)).collect();
                if let Some(first) = frames.first() {
                    let e = self.layer(button, size, first, first.size.as_vec2(), Vec2::ZERO, Color::WHITE);
                    self.hide(e);
                    self.world.entity_mut(e).insert(ActiveOutline { frames: frames.iter().map(|t| t.handle.clone()).collect(), fps });
                }
            }
            let _ = s;
        }
    }
}

/// Animated outline of an ability button, shown while its ability is on (see [`active_outlines`]).
#[derive(Component, Debug, Clone)]
pub(crate) struct ActiveOutline {
    frames: Vec<Handle<Image>>,
    fps: f32,
}

/// Shows the animated outline over every ability button whose ability is on (toggled on or running) and steps its frames.
pub(crate) fn active_outlines(time: Res<Time>, data: Res<CommandBarData>, slots: Query<(&Slot, &Children)>, mut outlines: Query<(&ActiveOutline, &mut Node, &mut ImageNode)>) {
    let t = time.elapsed_secs();
    for (slot, children) in &slots {
        if slot.kind != SlotKind::Ability {
            continue;
        }
        let on = data.abilities.get(slot.index).is_some_and(|a| a.active && !a.icon.is_empty());
        for c in children.iter() {
            let Ok((o, mut node, mut img)) = outlines.get_mut(c) else { continue };
            let want = if on { Display::Flex } else { Display::None };
            if node.display != want {
                node.display = want;
            }
            if on && !o.frames.is_empty() {
                let frame = &o.frames[(t * o.fps) as usize % o.frames.len()];
                if img.image != *frame {
                    img.image = frame.clone();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------------------------

/// Fires [`EawButtonClicked`] on release and handles toggle buttons.
pub(crate) fn button_clicks(
    keys: Res<ButtonInput<KeyCode>>,
    mut q: Query<(Entity, &mut EawButton, &Interaction, &mut EawButtonState)>,
    mut out: MessageWriter<EawButtonClicked>,
    mut locked: MessageWriter<crate::data::EawLockedClicked>,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    for (entity, mut b, i, mut st) in &mut q {
        if st.disabled {
            // A press on a greyed-out button is reported once so that the application can say why.
            if *i == Interaction::Pressed && !b.armed {
                locked.write(crate::data::EawLockedClicked { name: b.name.clone() });
            }
            b.armed = *i == Interaction::Pressed;
            continue;
        }
        match *i {
            Interaction::Pressed => b.armed = true,
            Interaction::Hovered if b.armed => {
                b.armed = false;
                if b.toggle {
                    st.selected = !st.selected;
                }
                out.write(EawButtonClicked {
                    name: b.name.clone(),
                    entity,
                    shift,
                });
            }
            _ => b.armed = false,
        }
    }
}

/// Swaps the state layers of buttons.
#[allow(clippy::type_complexity)]
pub(crate) fn button_visuals(
    buttons: Query<
        (&ButtonLayers, &Interaction, &EawButtonState, &EawButton),
        Or<(Changed<Interaction>, Changed<EawButtonState>)>,
    >,
    mut nodes: Query<&mut Node>,
    mut images: Query<&mut ImageNode>,
) {
    for (l, i, st, b) in &buttons {
        let hover = matches!(i, Interaction::Hovered | Interaction::Pressed) || st.force_hover;
        let pressed = matches!(i, Interaction::Pressed) || st.selected;
        let set = |nodes: &mut Query<&mut Node>, e: Option<Entity>, on: bool| {
            if let Some(e) = e
                && let Ok(mut n) = nodes.get_mut(e)
            {
                n.display = if on { Display::Flex } else { Display::None };
            }
        };
        // The tactical side and command buttons (reinforcements, special weapon, beacon, orders) stay a dimmed icon when unavailable, as in the original;
        // everywhere else the disabled texture replaces the icon.
        let dimmed_icon = (matches!(b.name.as_str(), "b_reinforcement" | "b_special_weapon" | "b_special_weapon2" | "b_retreat") || b.name.starts_with("c_button")) && l.icon.is_some();
        if dimmed_icon
            && let Some(e) = l.icon
            && let Ok(mut img) = images.get_mut(e)
        {
            img.color = if st.disabled { Color::srgba(0.5, 0.5, 0.5, 1.0) } else { Color::WHITE };
        }
        let show_disabled = st.disabled && l.disabled.is_some() && !dimmed_icon;
        let show_selected = !st.disabled && pressed && l.selected.is_some();
        set(&mut nodes, l.disabled, show_disabled);
        set(&mut nodes, l.selected, show_selected);
        set(&mut nodes, l.icon, !show_disabled && !show_selected);
        set(&mut nodes, l.over, hover && !st.disabled && !show_selected);
    }
}

/// Writes credits, planet name etc. into bound text fields and the allegiance symbol.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bound_fields(
    fresh: Query<(), Added<BoundField>>,
    data: Res<CommandBarData>,
    mut texts: Query<(Entity, &BoundField, &mut Text, Option<&mut TextColor>, Option<&Children>)>,
    fonts: Query<&TextFont>,
    spans: Query<(), With<SmallCapsSpan>>,
    mut commands: Commands,
    mut icons: Query<(&BoundField, &mut ImageNode), Without<Text>>,
    mut tex: ResMut<UiTextures>,
    db: Res<CommandBarDb>,
    mut images: ResMut<Assets<Image>>,
) {
    if !data.is_changed() && fresh.is_empty() {
        return;
    }
    for (entity, f, mut t, mut color, children) in &mut texts {
        if matches!(f.0, Field::PlanetName | Field::Income) && let (Some(c), Some(tc)) = (data.planet_color, color.as_deref_mut()) {
            tc.0 = c;
        }
        t.0 = match f.0 {
            Field::Credits => group_digits(data.credits),
            Field::Income => if data.planet_name.is_empty() { String::new() } else { group_digits(data.planet_income) },
            Field::PlanetName => data.planet_name.to_uppercase(),
            Field::Day => format!("MONTH: {}", data.day),
            Field::MonthIncome => group_digits(data.month_income),
            Field::TechLevel => format!("TECH: {}", data.tech_level),
            Field::Pop => format!("{}/{}", data.pop_current, data.pop_cap),
            Field::Allegiance => String::new(),
        };
        if f.0 == Field::PlanetName
            && let Ok(font) = fonts.get(entity)
        {
            let c = color.as_deref().map_or(Color::WHITE, |c| c.0);
            apply_small_caps(&mut commands, entity, &mut t, font, c, children, &spans);
        }
    }
    if let Some(alt) = db.get("Text_Allegiance_A") {
        for (f, mut img) in &mut icons {
            if f.0 == Field::Income {
                img.color = data.planet_color.unwrap_or(PALE);
            }
            if f.0 == Field::Allegiance
                && let Some(owner) = data.planet_owner
                && let Some(n) = CommandBarComponent::pick(&alt.icon_alt, owner.list_index())
                && let Some(t) = tex.get(&mut images, n)
            {
                img.image = t.handle;
            }
        }
    }
}

/// Numbers like the original bar: plain up to four digits, dot-grouped from five ("4634", "40.000").
/// Hover text of an info card number (title line, then the explanation).
fn field_tip(f: Field) -> Option<&'static str> {
    Some(match f {
        Field::Credits => "Credits\nYour treasury: the credits you can spend right now.",
        Field::MonthIncome => "Income per month\nCredits your empire earns per galactic month (4 weeks), upkeep already deducted.",
        Field::Income => "Planet income\nCredits per week the selected planet yields.",
        Field::Day => "Galactic month\nThe bar shows how far the current month has progressed.",
        Field::TechLevel => "Tech level\nYour current technology level.",
        Field::Pop => "Population\nUnits you field, out of the population cap.",
        Field::PlanetName | Field::Allegiance => return None,
    })
}

fn group_digits(v: i64) -> String {
    let s = v.abs().to_string();
    if s.len() <= 4 {
        return v.to_string();
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    if v < 0 { format!("-{out}") } else { out }
}

/// Space (design px) between two ability buttons of one card, and between two rows.
const ABILITY_ICON_GAP: f32 = 4.0;
/// Space (design px) at least kept free between the ability buttons of neighbouring cards. The tactical bar's cards are 55 design px
/// apart and its buttons 24 px wide (`tactical_shell_layout` in tests/real_data.rs); at the component's Scale 0.8 a pair of buttons
/// leaves 12.6 px to the next card, against 4 px inside the pair.
const ABILITY_CARD_GAP: f32 = 8.0;

/// Where ability `nth` of the `of` abilities of a card sits: `center` is the card's centre, `pitch` the room of the card (distance to
/// the nearer neighbouring card), `w` the button width (design px) and `base` the scale the buttons are drawn at. Returns (left edge,
/// row above the bottom row, scale). One or two abilities sit side by side centred above the card, as the Remake centres a group's
/// abilities on top of the group (video b0700); more go in two rows. Buttons that would not fit the card with [`ABILITY_CARD_GAP`] to
/// spare are drawn smaller, so the buttons of neighbouring cards never overlap and never run together into one evenly spaced row.
pub(crate) fn ability_place(center: f32, pitch: f32, w: f32, base: f32, nth: usize, of: usize) -> (f32, usize, f32) {
    let of = of.max(1);
    let cols = if of <= 2 { of } else { of.div_ceil(2) };
    let scale = ((pitch - ABILITY_CARD_GAP - (cols - 1) as f32 * ABILITY_ICON_GAP) / (cols as f32 * w)).min(base);
    let (row, col) = (nth / cols, nth % cols);
    // A last row that is not full is centred on its own.
    let in_row = if row == (of - 1) / cols { of - row * cols } else { cols };
    let row_w = in_row as f32 * w * scale + (in_row - 1) as f32 * ABILITY_ICON_GAP;
    (center - row_w * 0.5 + col as f32 * (w * scale + ABILITY_ICON_GAP), row, scale)
}

/// Puts every ability button above its own card ([`ability_place`]), drawn at the `<Scale>` of its component. The bar layout has
/// `special_button_{2n}` on the centre of card `n`, which gives the card centres and the room between them. Runs after [`slots`]
/// (lib.rs): a new selection's buttons are placed in the frame they appear, not one frame late at the old places.
pub(crate) fn ability_columns(data: Res<CommandBarData>, mut buttons: Query<(&Slot, &AbilityHome, &mut Node, &mut UiTransform)>) {
    if !data.is_changed() {
        return;
    }
    let homes: HashMap<usize, AbilityHome> = buttons.iter().filter(|(s, ..)| s.kind == SlotKind::Ability).map(|(s, h, ..)| (s.index, *h)).collect();
    let centre = |card: usize| homes.get(&(card * 2)).map(|h| h.corner.x);
    let room = |card: usize| {
        let c = centre(card)?;
        let left = card.checked_sub(1).and_then(centre).map(|p| c - p);
        let right = centre(card + 1).map(|n| n - c);
        left.into_iter().chain(right).reduce(f32::min)
    };
    for (slot, home, mut node, mut tf) in &mut buttons {
        if slot.kind != SlotKind::Ability {
            continue;
        }
        let (w, h) = match (node.width, node.height) {
            (Val::Px(w), Val::Px(h)) => (w, h),
            _ => (24.0, 25.0),
        };
        let entry = data.abilities.get(slot.index).filter(|a| !a.icon.is_empty());
        let (left, top, scale) = match entry.and_then(|a| homes.get(&(a.card * 2)).map(|c| (a, c.corner))) {
            Some((a, card_corner)) => {
                let pitch = room(a.card).unwrap_or(2.0 * w + ABILITY_ICON_GAP + ABILITY_CARD_GAP);
                let (left, row, scale) = ability_place(card_corner.x + w * 0.5, pitch, w, home.scale, a.nth, a.of);
                // The node scales around its centre, which stays on the shell's button row for the bottom row (the Remake's icon centres
                // sit there, b0700); shift it so the scaled button's left edge lands where it belongs.
                (left - w * (1.0 - scale) * 0.5, home.corner.y - row as f32 * (h * scale + ABILITY_ICON_GAP), scale)
            }
            None => (home.corner.x, home.corner.y, home.scale),
        };
        if node.left != Val::Px(left) {
            node.left = Val::Px(left);
        }
        if node.top != Val::Px(top) {
            node.top = Val::Px(top);
        }
        if tf.scale != Vec2::splat(scale) {
            tf.scale = Vec2::splat(scale);
        }
    }
}

/// Tint of the group frame pieces (estimate, see [`border_pieces`]).
const BORDER_TINT: Color = Color::srgba(0.35, 0.42, 0.55, 0.55);

/// Joins the column frames (`special_border_NN`, two selected units each) of one bar section into one panel:
/// left / centre / right pieces at the ends and in between, the full frame for a lone column (Remake video b0700).
pub(crate) fn border_pieces(
    data: Res<CommandBarData>,
    mut tex: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    borders: Query<(&Slot, &ButtonLayers)>,
    mut nodes: Query<&mut ImageNode>,
) {
    if !data.is_changed() {
        return;
    }
    const PIECES: [&str; 4] = [
        "i_special_border_full.tga",
        "i_special_border_left.tga",
        "i_special_border_center.tga",
        "i_special_border_right.tga",
    ];
    let section = |col: usize| data.selection.get(col * 2).filter(|s| !s.icon.is_empty()).map(|s| s.section);
    for (slot, layers) in &borders {
        if slot.kind != SlotKind::Border {
            continue;
        }
        let (Some(layer), Some(mine)) = (layers.icon, section(slot.index)) else { continue };
        let prev = slot.index.checked_sub(1).and_then(section) == Some(mine);
        let next = section(slot.index + 1) == Some(mine);
        let piece = match (prev, next) {
            (false, false) => 0,
            (false, true) => 1,
            (true, true) => 2,
            (true, false) => 3,
        };
        if let (Some(t), Ok(mut img)) = (tex.get(&mut images, PIECES[piece]), nodes.get_mut(layer)) {
            img.image = t.handle;
            // The frame art is near white: as a bright outline around a group (and bright seams between its cards) it is not what the Remake shows,
            // where grouped cards keep their own dark slot frames. A dark translucent tint (estimate) keeps the frame but drops the outline.
            img.color = BORDER_TINT;
        }
    }
}

/// Pad entities of the bottom row (largest `top`), ordered left to right.
fn bottom_row_order<'a>(pads: impl Iterator<Item = (Entity, usize, &'a Node)>) -> Vec<Entity> {
    let px = |v: Val| if let Val::Px(x) = v { x } else { 0.0 };
    let mut v: Vec<(Entity, usize, f32, f32)> = pads.map(|(e, i, n)| (e, i, px(n.left), px(n.top))).collect();
    let Some(max_top) = v.iter().map(|p| p.3).reduce(f32::max) else { return Vec::new() };
    v.retain(|p| (p.3 - max_top).abs() < 4.0);
    v.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.1.cmp(&b.1)));
    v.into_iter().map(|p| p.0).collect()
}

/// Fills queue, pad, selection and ability slots from [`CommandBarData`].
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn slots(
    fresh: Query<(), Added<Slot>>,
    data: Res<CommandBarData>,
    mut remap: ResMut<PadRemap>,
    panel: Res<PanelData>,
    mut tex: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut slots: Query<
        (
            Entity,
            &Slot,
            Option<&PanelSlotName>,
            &mut Node,
            &mut EawButtonState,
            &mut Visibility,
            &mut EawButton,
        ),
        Without<SlotPart>,
    >,
    children: Query<&Children>,
    mut parts: Query<
        (&SlotPart, &mut Node, Option<&mut ImageNode>, Option<&mut Text>, Option<&mut BackgroundColor>),
        Without<Slot>,
    >,
) {
    if !data.is_changed() && !panel.is_changed() && fresh.is_empty() {
        return;
    }
    // Structure list of the planet view: entries fill the bottom pad row left to right, other pads stay empty.
    let bottom_row = if data.pads_bottom_row {
        bottom_row_order(slots.iter().filter(|s| s.1.kind == SlotKind::Pad).map(|s| (s.0, s.1.index, &*s.3)))
    } else {
        Vec::new()
    };
    let map: std::collections::HashMap<usize, usize> = slots
        .iter()
        .filter(|s| s.1.kind == SlotKind::Pad)
        .filter_map(|s| bottom_row.iter().position(|&b| b == s.0).map(|r| (s.1.index, r)))
        .collect();
    if remap.0 != map {
        remap.0 = map;
    }
    for (e, slot, pname, mut node, mut st, mut vis, mut button) in &mut slots {
        let slot = &if data.pads_bottom_row && slot.kind == SlotKind::Pad {
            Slot { kind: slot.kind, index: bottom_row.iter().position(|&b| b == e).unwrap_or(usize::MAX) }
        } else {
            *slot
        };
        button.tooltip_literal = match slot.kind {
            SlotKind::Ability => data.abilities.get(slot.index).filter(|a| !a.icon.is_empty()).map(|a| a.tooltip.clone()).filter(|t| !t.is_empty()),
            SlotKind::Pad => data.pads.get(slot.index).map(|p| p.tooltip.clone()).filter(|t| !t.is_empty()),
            SlotKind::Queue => data.queue.get(slot.index).filter(|q| q.count > 0).map(|q| q.tooltip.clone()).filter(|t| !t.is_empty()),
            _ => None,
        };
        button.tooltip_icon = match slot.kind {
            SlotKind::Ability => data.abilities.get(slot.index).filter(|a| !a.icon.is_empty()).map(|a| a.icon.clone()),
            SlotKind::Pad => data.pads.get(slot.index).map(|p| p.icon.clone()),
            SlotKind::Queue => data.queue.get(slot.index).filter(|q| q.count > 0).map(|q| q.icon.clone()),
            _ => None,
        }
        .filter(|i| !i.is_empty());
        button.marker_tooltip = match slot.kind {
            SlotKind::Pad => data.pads.get(slot.index).map(|p| p.marker_tooltip.clone()).filter(|t| !t.is_empty()),
            _ => None,
        };
        // (icon, progress, label, disabled, selected)
        let content: Option<(String, f32, String, bool, bool)> = match slot.kind {
            SlotKind::Queue => data.queue.get(slot.index).filter(|q| q.count > 0).map(|q| {
                (
                    q.icon.clone(),
                    q.progress,
                    // Like the Remake: the production percentage under the icon (the count of a stack in front).
                    {
                        let pct = format!("{}%", (q.progress.clamp(0.0, 1.0) * 100.0).floor() as u32);
                        if q.count > 1 { format!("{}x {pct}", q.count) } else { pct }
                    },
                    false,
                    false,
                )
            }),
            SlotKind::Pad => data.pads.get(slot.index).map(|p| {
                (
                    p.icon.clone(),
                    if p.building { 1.0 } else { 0.0 },
                    if p.label.is_empty() { p.price.map(|v| v.to_string()).unwrap_or_default() } else { p.label.clone() },
                    p.disabled,
                    false,
                )
            }),
            SlotKind::Select => data.selection.get(slot.index).filter(|s| !s.icon.is_empty()).map(|s| {
                (
                    s.icon.clone(),
                    0.0,
                    if s.count > 1 { s.count.to_string() } else { String::new() },
                    false,
                    s.selected,
                )
            }),
            SlotKind::Panel => pname.and_then(|n| panel.slots.get(&n.0)).map(|s| {
                (s.icon.clone(), s.progress, s.label.clone(), s.disabled, s.selected)
            }),
            SlotKind::Border => data.selection.get(slot.index * 2).is_some_and(|s| !s.icon.is_empty())
                .then(|| (String::new(), 0.0, String::new(), false, false)),
            SlotKind::Ability => data.abilities.get(slot.index).filter(|a| !a.icon.is_empty()).map(|a| {
                (a.icon.clone(), 1.0 - a.ready, String::new(), a.disabled, a.active)
            }),
        };
        let (upkeep, level, locked) = match slot.kind {
            SlotKind::Pad => data.pads.get(slot.index).map_or((0.0, 0, false), |p| (p.upkeep, p.level, p.locked)),
            _ => (0.0, 0, false),
        };
        let Some((icon, progress, label, disabled, selected)) = content else {
            node.display = Display::None;
            *vis = Visibility::Hidden;
            continue;
        };
        node.display = Display::Flex;
        *vis = Visibility::Inherited;
        st.disabled = disabled;
        st.selected = selected;
        let lane = if slot.kind == SlotKind::Queue { data.queue.get(slot.index).map_or(0, |q| q.lane) } else { 0 };
        let look = if slot.kind == SlotKind::Panel { pname.and_then(|n| panel.slots.get(&n.0)).map_or_else(Default::default, |s| s.look) } else { crate::panels::SlotLook::Tile };
        let (slot_w, slot_h) = (node.width, node.height);
        for k in children.iter_descendants(e) {
            let Ok((part, mut n, img, text, bg)) = parts.get_mut(k) else {
                continue;
            };
            match part {
                SlotPart::Icon => {
                    if look != crate::panels::SlotLook::Tile && let (Val::Px(w), Val::Px(h)) = (slot_w, slot_h) {
                        // Fleet boxes: bare units and round hero portraits, larger than the tile icon.
                        let s = w.max(h) * 1.25;
                        n.width = Val::Px(s);
                        n.height = Val::Px(s);
                        n.left = Val::Px(w * 0.5 - s * 0.5);
                        n.top = Val::Px(h * 0.5 - s * 0.5);
                        n.border_radius = if look == crate::panels::SlotLook::Hero { BorderRadius::MAX } else { BorderRadius::ZERO };
                    }
                    if let Some(mut img) = img {
                        match tex.get(&mut images, &icon) {
                            Some(t) => {
                                img.image = t.handle;
                                img.color = if locked {
                                    Color::srgba(0.22, 0.22, 0.25, 1.0)
                                } else if disabled {
                                    Color::srgba(0.45, 0.45, 0.45, 1.0)
                                } else {
                                    Color::WHITE
                                };
                                n.display = Display::Flex;
                            }
                            None => n.display = Display::None,
                        }
                    }
                }
                SlotPart::Progress => {
                    if let Val::Px(w) = n.width {
                        n.height = Val::Px(w * progress.clamp(0.0, 1.0));
                    }
                    if let Some(mut bg) = bg {
                        bg.0 = if lane == 1 { Color::srgba(1.0, 0.55, 0.1, 0.45) } else { Color::srgba(0.1, 0.5, 0.9, 0.45) };
                    }
                }
                SlotPart::HeroBadge => {
                    n.display = if look == crate::panels::SlotLook::Hero { Display::Flex } else { Display::None };
                }
                SlotPart::LaneMark => {
                    if let Some(mut bg) = bg {
                        bg.0 = if lane == 1 { LANE_GROUND } else { LANE_SPACE };
                    }
                }
                SlotPart::Label => {
                    if let Some(mut t) = text {
                        t.0 = label.clone();
                    }
                }
                SlotPart::UpkeepIcon => {
                    n.display = if upkeep.abs() > 0.5 { Display::Flex } else { Display::None };
                    if let Some(mut img) = img {
                        img.color = if upkeep < 0.0 { Color::srgb(1.0, 0.3, 0.25) } else { Color::srgb(0.4, 1.0, 0.45) };
                    }
                }
                SlotPart::UpkeepPlus => {
                    if let Some(mut t) = text {
                        t.0 = if upkeep.abs() > 0.5 { "+".to_owned() } else { String::new() };
                    }
                }
                SlotPart::Level => {
                    if let Some(mut t) = text {
                        t.0 = roman(level);
                    }
                }
            }
        }
    }
}

/// Picks the level texture of status bars from the selection.
#[allow(clippy::type_complexity)]
pub(crate) fn status_bars(
    fresh: Query<(), Added<StatusBar>>,
    data: Res<CommandBarData>,
    panel: Res<PanelData>,
    mut bars: Query<
        (Entity, &StatusBar, Option<&PanelBarName>, &BarTextures, &mut Node),
        Without<BarLayer>,
    >,
    children: Query<&Children>,
    mut layers: Query<(&BarLayer, &mut ImageNode)>,
) {
    if !data.is_changed() && !panel.is_changed() && fresh.is_empty() {
        return;
    }
    for (e, bar, pname, tex, mut node) in &mut bars {
        let frac = match pname {
            Some(n) => panel.bars.get(&n.0).copied(),
            None => data.selection.get(bar.index).filter(|s| !s.icon.is_empty()).and_then(|s| {
                if bar.shield { s.shield } else { Some(s.health) }
            }),
        };
        let Some(frac) = frac else {
            node.display = Display::None;
            continue;
        };
        node.display = Display::Flex;
        let level = (frac.clamp(0.0, 1.0) * bar.max_level as f32).round() as usize;
        let Ok(kids) = children.get(e) else { continue };
        for &k in kids {
            if let Ok((layer, mut img)) = layers.get_mut(k) {
                let list = if layer.fill { &tex.fill } else { &tex.frame };
                if let Some(h) = list.get(level).or_else(|| list.last()) {
                    img.image = h.clone();
                }
            }
        }
    }
}

/// Segments of the month progress bar (`PanelData::bars["fight_bar_g"]`).
#[derive(Component, Debug, Clone, Copy)]
pub struct BarSegment {
    index: usize,
    count: usize,
}

const SEGMENTS: usize = 10;

impl Builder<'_> {
    fn spawn_segment_bar(&mut self, center: Vec2, size: Vec2) {
        let holder = self
            .world
            .spawn((
                Name::new("fight_bar_g"),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(center.x - size.x * 0.5),
                    top: Val::Px(self.root_h - center.y - size.y * 0.5),
                    width: Val::Px(size.x * 0.65),
                    height: Val::Px(size.y),
                    column_gap: Val::Px(1.5),
                    padding: UiRect::axes(Val::Px(1.0), Val::Px(3.0)),
                    ..default()
                },
                FocusPolicy::Pass,
                ChildOf(self.root),
            ))
            .id();
        for index in 0..SEGMENTS {
            self.world.spawn((
                BarSegment { index, count: SEGMENTS },
                Node { flex_grow: 1.0, ..default() },
                BackgroundColor(SEGMENT_OFF),
                FocusPolicy::Pass,
                ChildOf(holder),
            ));
        }
    }
}

/// Text and icon colour of the bar read-outs (`Text_Color` 189 216 245).
const PALE: Color = Color::srgb(0.74, 0.85, 0.96);
const SEGMENT_ON: Color = Color::srgb(0.25, 0.9, 0.3);
const SEGMENT_OFF: Color = Color::srgba(0.05, 0.22, 0.08, 0.9);

/// Lights the month bar's segments.
pub(crate) fn segment_bars(panel: Res<PanelData>, fresh: Query<(), Added<BarSegment>>, mut segments: Query<(&BarSegment, &mut BackgroundColor)>) {
    if !panel.is_changed() && fresh.is_empty() {
        return;
    }
    let frac = panel.bars.get("fight_bar_g").copied().unwrap_or(0.0);
    for (s, mut bg) in &mut segments {
        let lit = (s.index as f32 + 0.5) / s.count as f32 <= frac + 1e-3;
        bg.0 = if lit { SEGMENT_ON } else { SEGMENT_OFF };
    }
}

/// Parts of the tooltip panel (children of the [`Tooltip`] root).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipPart {
    Icon,
    Title,
    Divider,
    Body,
    StatsRow,
    StatLabels,
    StatValues,
    Warn,
}

/// A tooltip text split into its panel parts.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TooltipLayout {
    /// First line.
    pub title: String,
    /// Wrapped paragraphs.
    pub body: String,
    /// Left column of the stat rows.
    pub labels: String,
    /// Right column of the stat rows.
    pub values: String,
    /// Warning lines (red).
    pub warn: String,
}

impl TooltipLayout {
    /// Splits `text`: the first line is the title; lines starting with `@` are stat rows (`@Label<TAB>value`), lines
    /// starting with `!` are warnings, everything else is body text (blank lines separate paragraphs).
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut lines = text.lines();
        out.title = lines.by_ref().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned();
        let mut body: Vec<&str> = Vec::new();
        for l in lines {
            if let Some(rest) = l.strip_prefix('@') {
                let (k, v) = rest.split_once('\t').unwrap_or((rest, ""));
                if !out.labels.is_empty() {
                    out.labels.push('\n');
                    out.values.push('\n');
                }
                out.labels.push_str(k.trim());
                out.values.push_str(v.trim());
            } else if let Some(rest) = l.strip_prefix('!') {
                if !out.warn.is_empty() {
                    out.warn.push('\n');
                }
                out.warn.push_str(rest.trim());
            } else {
                body.push(l.trim_end());
            }
        }
        while body.first().is_some_and(|l| l.trim().is_empty()) {
            body.remove(0);
        }
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
        }
        out.body = body.join("\n");
        out
    }
}

const TIP_BG: Color = Color::srgba(0.04, 0.09, 0.13, 0.95);
const TIP_BORDER: Color = Color::srgb(0.0, 0.76, 0.99);

/// Spawns the global tooltip panel: icon and title row, body text, stat rows and a warning line, anchored next to the advisor droid.
pub(crate) fn spawn_tooltip(mut commands: Commands, settings: Res<EawUiSettings>) {
    let camera = settings.camera;
    let mut tip = commands.spawn((
        Tooltip,
        Name::new("EaW tooltip"),
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            padding: UiRect::axes(Val::Px(9.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(1.0)),
            min_width: Val::Px(150.0),
            max_width: Val::Px(340.0),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            overflow: Overflow::clip_y(),
            display: Display::None,
            ..default()
        },
        BackgroundColor(TIP_BG),
        BorderColor::all(TIP_BORDER),
        Visibility::Hidden,
        GlobalZIndex(1000),
        FocusPolicy::Pass,
    ));
    if let Some(c) = camera {
        tip.insert(UiTargetCamera(c));
    }
}

/// Fills the (emptied) tooltip panel with the parts of `layout`. The panel is rebuilt on every change: a cached layout of
/// reused text nodes kept the panel one row too short.
fn fill_tooltip(p: &mut ChildSpawnerCommands, layout: &TooltipLayout, icon: Option<Handle<Image>>, font: &dyn Fn(f32) -> TextFont) {
    p.spawn(Node { column_gap: Val::Px(7.0), align_items: AlignItems::Center, ..default() }).with_children(|h| {
        if let Some(handle) = icon {
            h.spawn((TipPart::Icon, ImageNode::new(handle), Node { width: Val::Px(36.0), height: Val::Px(36.0), flex_shrink: 0.0, ..default() }, FocusPolicy::Pass));
        }
        h.spawn((TipPart::Title, Text::new(layout.title.clone()), font(13.0), TextColor(Color::srgb(1.0, 0.86, 0.38)), FocusPolicy::Pass));
    });
    if !layout.body.is_empty() || !layout.labels.is_empty() || !layout.warn.is_empty() {
        p.spawn((TipPart::Divider, Node { height: Val::Px(1.0), flex_shrink: 0.0, ..default() }, BackgroundColor(Color::srgba(0.0, 0.62, 0.85, 0.55))));
    }
    if !layout.body.is_empty() {
        p.spawn((TipPart::Body, Text::new(layout.body.clone()), font(11.0), TextColor(Color::srgb(0.78, 0.88, 0.97)), FocusPolicy::Pass));
    }
    if !layout.labels.is_empty() {
        p.spawn((TipPart::StatsRow, Node { column_gap: Val::Px(14.0), justify_content: JustifyContent::SpaceBetween, ..default() }, FocusPolicy::Pass)).with_children(|r| {
            r.spawn((TipPart::StatLabels, Text::new(layout.labels.clone()), font(11.0), TextColor(Color::srgb(0.52, 0.68, 0.82)), FocusPolicy::Pass));
            r.spawn((TipPart::StatValues, Text::new(layout.values.clone()), font(11.0), TextColor(Color::srgb(0.95, 0.97, 1.0)), TextLayout::justify(Justify::Right), FocusPolicy::Pass));
        });
    }
    if !layout.warn.is_empty() {
        p.spawn((TipPart::Warn, Text::new(layout.warn.clone()), font(11.0), TextColor(Color::srgb(1.0, 0.5, 0.38)), FocusPolicy::Pass));
    }
}

/// Shows the tooltip of the hovered (or forced) button, else the [`WorldTooltip`].
///
/// Text format (see [`TooltipLayout::parse`]): first line title, `@Label<TAB>value` stat rows, `!` warnings, the rest
/// wrapped body text. Build pad, queue and world tooltips are anchored next to the advisor droid (lower left, above the
/// info panel); other buttons show next to the cursor.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn tooltip(
    assets: Res<EawUiAssets>,
    forced: Res<ForcedTooltip>,
    suppress: Res<TooltipSuppress>,
    keep_out: Res<TooltipKeepOut>,
    world_tip: Res<WorldTooltip>,
    buttons: Query<(Entity, &EawButton, &Interaction, &ComputedNode, &UiGlobalTransform)>,
    markers: Query<(&SlotPart, &Node, &ComputedNode, &UiGlobalTransform, &ChildOf)>,
    mut tip: Query<(Entity, &mut Node, &mut Visibility, &ComputedNode), (With<Tooltip>, Without<SlotPart>, Without<TipPart>)>,
    window: Query<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    mut tex: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<(String, u8)>,
    mut commands: Commands,
    settings: Res<EawUiSettings>,
    fields: Query<(&BoundField, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
) {
    let Ok((root, mut node, mut vis, own)) = tip.single_mut() else {
        return;
    };
    if suppress.0 && forced.0.is_none() {
        node.display = Display::None;
        return;
    }
    let cursor_phys = window.single().ok().and_then(Window::physical_cursor_position);
    let mut hit = None;
    for (e, b, i, cn, gt) in &buttons {
        let forced_hit = forced.0.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&b.name));
        if (*i != Interaction::None || forced_hit) && (!b.tooltips.is_empty() || b.tooltip_literal.is_some()) {
            hit = Some((e, b, cn, gt, forced_hit));
            if !forced_hit {
                break;
            }
        }
    }
    // (text, icon, hovered bar button)
    let (text, icon, button) = match hit {
        Some((e, b, cn, gt, forced_hit)) => {
            // The small upkeep/income marker of a pad has its own explanation.
            let over_marker = cursor_phys.is_some_and(|c| {
                b.marker_tooltip.is_some()
                    && markers.iter().any(|(p, n, mn, mg, parent)| {
                        *p == SlotPart::UpkeepIcon && parent.parent() == e && n.display != Display::None && {
                            let half = mn.size() * 0.5 + Vec2::splat(4.0);
                            let d = (c - mg.translation).abs();
                            d.x <= half.x && d.y <= half.y
                        }
                    })
            });
            let text = if over_marker {
                b.marker_tooltip.clone().unwrap_or_default()
            } else {
                b.tooltip_literal
                    .clone()
                    .or_else(|| b.tooltips.iter().find_map(|k| assets.text.get(k).filter(|t| !t.is_empty()).map(str::to_owned)))
                    .unwrap_or_default()
            };
            let icon = if over_marker { None } else { b.tooltip_icon.clone() };
            (text, icon, Some((b, cn, gt, forced_hit)))
        }
        // A number of the info card (credits, income, tech, population) explains itself while the cursor is on it.
        None => match cursor_phys.and_then(|c| fields.iter().find(|(_, n, g, v)| v.get() && n.contains_point(**g, c)).and_then(|(f, ..)| field_tip(f.0))) {
            Some(t) => (t.to_owned(), None, None),
            None => (world_tip.text.clone().unwrap_or_default(), world_tip.icon.clone(), None),
        },
    };
    if text.is_empty() {
        node.display = Display::None;
        return;
    }
    let key = format!("{text}\u{0}{}", icon.as_deref().unwrap_or(""));
    if last.0 != key || node.display == Display::None {
        let layout = TooltipLayout::parse(&text);
        key.clone_into(&mut last.0);
        last.1 = 0;
        let icon_tex = icon.as_deref().and_then(|i| tex.get(&mut images, i)).map(|t| t.handle);
        let font = |size: f32| match &settings.tooltip_font_family {
            Some(f) => TextFont { font: FontSource::Family(f.as_str().into()), font_size: FontSize::Px(size), ..default() },
            None => TextFont::from_font_size(size),
        };
        // A fixed width: text in a shrink-to-fit panel is measured on one line and the panel ends too short.
        node.width = Val::Px(tooltip_width(&layout, icon_tex.is_some()));
        commands.entity(root).despawn_children().with_children(|p| fill_tooltip(p, &layout, icon_tex, &font));
        // The size is only known after the next layout passes: stay invisible for two frames so the panel does not jump.
        *vis = Visibility::Hidden;
    } else if last.1 < 2 {
        last.1 += 1;
        *vis = Visibility::Hidden;
    } else {
        *vis = Visibility::Inherited;
    }
    node.display = Display::Flex;
    let drop_anchor = match &button {
        Some((b, ..)) => b.name.starts_with("b_create") || b.name.starts_with("queue"),
        None => true,
    };
    // Only the droid that is laid out (another bar may carry a hidden copy of the button).
    let droid = buttons.iter().find(|b| b.1.name.eq_ignore_ascii_case("b_droid_help") && b.3.size().x > 1.0).map(|b| (b.3, b.4));
    let inv = own.inverse_scale_factor();
    let tip_w = own.size().x * inv;
    let tip_h = own.size().y * inv;

    match window.single() {
        Ok(w) => {
            let screen_w = w.physical_width() as f32 * inv;
            let screen_h = w.physical_height() as f32 * inv;
            node.max_height = Val::Px((screen_h - 20.0).max(50.0));

            if drop_anchor && let Some((dn, dg)) = droid {
                let ppd = scale.0 / dn.inverse_scale_factor().max(1e-6);
                let top_left = dg.translation.xy() - dn.size() * 0.5;
                let left_ui = (top_left.x + 62.0 * ppd) * inv;
                let bottom_phys = top_left.y - 12.0 * ppd;
                let mut dist_from_bottom_ui = ((w.physical_height() as f32 - bottom_phys) * inv).max(0.0);
                // The advisor log grows upward from the same spot: the card sits above its text.
                dist_from_bottom_ui = card_bottom_above_log(dist_from_bottom_ui, keep_out.0, screen_h);
                let space_above = screen_h - dist_from_bottom_ui - 10.0;

                let clamped_x = left_ui.clamp(4.0, (screen_w - tip_w - 4.0).max(4.0));
                node.left = Val::Px(clamped_x);

                if tip_h > space_above {
                    node.top = Val::Px(10.0);
                    node.bottom = Val::Auto;
                } else {
                    node.top = Val::Auto;
                    node.bottom = Val::Px(dist_from_bottom_ui);
                }
            } else if let Some((_, cn, gt, true)) = &button {
                let c_ui = gt.translation.xy() * inv;
                let clamped_x = (c_ui.x - 10.0).clamp(4.0, (screen_w - tip_w - 4.0).max(4.0));
                let top_ui = c_ui.y - cn.size().y * inv * 0.5 - 30.0;
                let clamped_y = top_ui.clamp(10.0, (screen_h - tip_h - 10.0).max(10.0));
                node.left = Val::Px(clamped_x);
                node.top = Val::Px(clamped_y);
                node.bottom = Val::Auto;
            } else {
                let cursor_ui = w.physical_cursor_position().map(|p| p * inv);
                let at = cursor_ui.map_or(Vec2::ZERO, |p| p + Vec2::new(14.0, 18.0));
                let at = clamp_to_screen(at, cursor_ui.unwrap_or(at), Vec2::new(tip_w, tip_h), Vec2::new(screen_w, screen_h));
                node.left = Val::Px(at.x);
                node.top = Val::Px(at.y);
                node.bottom = Val::Auto;
            }
        }
        Err(_) => {
            node.left = Val::Px(10.0);
            node.top = Val::Auto;
            node.bottom = Val::Px(60.0);
        }
    }
}

/// Keeps [`UiScale`] in sync with the target height (`Auto` mode).
pub(crate) fn ui_scale(
    settings: Res<EawUiSettings>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut scale: ResMut<UiScale>,
) {
    let v = match settings.scale {
        crate::UiScaleMode::Fixed(v) => v,
        crate::UiScaleMode::Auto => match window.single() {
            Ok(w) => (w.width() / 1360.0).min(w.height() / crate::shell::REFERENCE_HEIGHT * 1.012),
            Err(_) => return,
        },
    };
    if (scale.0 - v).abs() > 1e-4 {
        scale.0 = v;
    }
}















/// Writes [`PanelData::texts`] into text components of shell panels.
pub(crate) fn panel_texts(
    panel: Res<PanelData>,
    fresh: Query<(), Added<PanelText>>,
    mut texts: Query<(&PanelText, &mut Text)>,
) {
    if !panel.is_changed() && fresh.is_empty() {
        return;
    }
    for (n, mut t) in &mut texts {
        t.0 = panel.texts.get(&n.0).cloned().unwrap_or_default();
    }
}





/// Keeps a tooltip of `size` (logical px) on the screen: shifted left at the right edge, and flipped above the
/// cursor when it would run over the bottom edge. Also ensures top and left edges stay on-screen.
/// Distance of a droid-anchored card from the screen bottom (UI px): at least `dist`, and above the advisor log whose top edge is `log_top` (px from the top).
fn card_bottom_above_log(dist: f32, log_top: Option<f32>, screen_h: f32) -> f32 {
    log_top.map_or(dist, |top| dist.max(screen_h - top + 4.0))
}

pub fn clamp_to_screen(anchor: Vec2, cursor: Vec2, size: Vec2, screen: Vec2) -> Vec2 {
    let mut p = anchor;
    if p.x + size.x > screen.x - 4.0 {
        p.x = (screen.x - size.x - 4.0).max(4.0);
    } else {
        p.x = p.x.max(4.0);
    }
    if p.y + size.y > screen.y - 4.0 {
        p.y = (cursor.y - size.y - 12.0).max(4.0);
    } else {
        p.y = p.y.max(4.0);
    }
    p
}

/// Roman numeral of a small level ("" for 0).
fn roman(level: u8) -> String {
    ["", "I", "II", "III", "IV", "V"].get(level as usize).copied().unwrap_or("").to_owned()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ability_that_is_on_shows_the_running_outline_and_off_hides_it() {
        use crate::data::AbilityEntry;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(50)))
            .init_resource::<CommandBarData>()
            .add_systems(Update, active_outlines);
        let frames: Vec<Handle<Image>> = (0..13u128).map(|i| Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(i + 1), std::marker::PhantomData)).collect();
        let button = app.world_mut().spawn((Slot { kind: SlotKind::Ability, index: 1 }, Node::default())).id();
        let outline = app
            .world_mut()
            .spawn((ActiveOutline { frames: frames.clone(), fps: 8.0 }, Node { display: Display::None, ..default() }, ImageNode::default(), ChildOf(button)))
            .id();
        let entry = |active: bool| AbilityEntry { icon: "I_SA_HUNT.TGA".into(), ready: 1.0, active, kind: "HUNT".into(), ..default() };
        app.world_mut().resource_mut::<CommandBarData>().abilities = vec![AbilityEntry::default(), entry(false)];
        app.update();
        assert_eq!(app.world().get::<Node>(outline).unwrap().display, Display::None, "off: no outline");
        // Toggled on (Hunt): the outline shows and runs through its frames at 8 per second.
        app.world_mut().resource_mut::<CommandBarData>().abilities[1].active = true;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..12 {
            app.update();
            assert_eq!(app.world().get::<Node>(outline).unwrap().display, Display::Flex, "on: the outline stays up");
            seen.insert(app.world().get::<ImageNode>(outline).unwrap().image.id());
        }
        assert!(seen.len() >= 4, "the outline animates: {} frames seen", seen.len());
        // Toggled off again.
        app.world_mut().resource_mut::<CommandBarData>().abilities[1].active = false;
        app.update();
        assert_eq!(app.world().get::<Node>(outline).unwrap().display, Display::None);
    }

    #[test]
    fn ability_buttons_sit_above_their_own_card_and_keep_off_the_neighbours() {
        // The tactical bar: cards 55 design px apart, buttons 24 px wide (tests/real_data.rs `tactical_shell_layout`), Scale 0.8.
        let (pitch, w, base) = (55.0, 24.0, 0.8);
        let share = (pitch - ABILITY_CARD_GAP) * 0.5;
        for of in 1..=6 {
            let place: Vec<(f32, usize, f32)> = (0..of).map(|nth| ability_place(0.0, pitch, w, base, nth, of)).collect();
            let scale = place[0].2;
            assert!(place.iter().all(|p| p.2 == scale), "one size per card");
            assert_eq!(scale == base, of <= 4, "{of} abilities: the component's size up to two rows of two, smaller beyond");
            for (i, a) in place.iter().enumerate() {
                let (l, r) = (a.0, a.0 + w * scale);
                assert!(l >= -share - 1e-3 && r <= share + 1e-3, "{of} abilities: button {i} spans {l}..{r}, the card's share is ±{share}");
                for b in place[i + 1..].iter().filter(|b| b.1 == a.1) {
                    assert!(b.0 >= r + ABILITY_ICON_GAP - 1e-3 || a.0 >= b.0 + w * scale + ABILITY_ICON_GAP - 1e-3, "{of} abilities: buttons of one row overlap");
                }
            }
            for row in 0..2 {
                let in_row: Vec<_> = place.iter().filter(|p| p.1 == row).collect();
                if let (Some(first), Some(last)) = (in_row.first(), in_row.last()) {
                    assert!((first.0 + last.0 + w * scale).abs() < 1e-3, "{of} abilities: row {row} is centred on the card");
                }
            }
        }
        // Two neighbouring cards with two abilities each: the gap between the cards is three times the gap inside a card.
        let at = |c: f32, nth| ability_place(c, pitch, w, base, nth, 2);
        let (within, across) = (at(0.0, 1).0 - (at(0.0, 0).0 + w * base), at(pitch, 0).0 - (at(0.0, 1).0 + w * base));
        assert!(across >= 3.0 * within, "gap between cards {across}, inside a card {within}");
    }

    #[test]
    fn ability_columns_moves_every_button_above_its_card() {
        use crate::data::AbilityEntry;
        /// Ability button nodes written per frame.
        #[derive(Resource, Default)]
        struct Moves(usize);
        let mut app = App::new();
        app.init_resource::<CommandBarData>().init_resource::<Moves>().add_systems(
            Update,
            (ability_columns, (|q: Query<(), (Changed<Node>, With<AbilityHome>)>, mut m: ResMut<Moves>| m.0 += q.iter().count()).after(ability_columns)),
        );
        // The shell's button row: 24 buttons 27.5 design px apart, special_button_{2n} on the centre of card n (x 380 + 55 n).
        let corner = |i: usize| Vec2::new(380.0 + 27.5 * i as f32 - 12.0, 40.0);
        let buttons: Vec<Entity> = (0..24)
            .map(|i| {
                let node = Node { left: Val::Px(corner(i).x), top: Val::Px(corner(i).y), width: Val::Px(24.0), height: Val::Px(25.0), ..default() };
                app.world_mut().spawn((Slot { kind: SlotKind::Ability, index: i }, AbilityHome { corner: corner(i), scale: 0.8 }, node)).id()
            })
            .collect();
        let entry = |card, nth, of| AbilityEntry { icon: "I_SA_HUNT.TGA".into(), card, nth, of, ..default() };
        // Card 0 with two abilities, card 1 with one, card 2 with five.
        let mut abilities = vec![entry(0, 0, 2), entry(0, 1, 2), entry(1, 0, 1)];
        abilities.extend((0..5).map(|n| entry(2, n, 5)));
        abilities.resize(24, AbilityEntry::default());
        app.world_mut().resource_mut::<CommandBarData>().abilities = abilities;
        app.update();
        // Visible box (left, right, top, bottom) of button i: the node scales around its centre.
        let at = |app: &App, i: usize| {
            let n = app.world().get::<Node>(buttons[i]).unwrap();
            let s = app.world().get::<UiTransform>(buttons[i]).unwrap().scale.x;
            let (Val::Px(l), Val::Px(t)) = (n.left, n.top) else { panic!("px") };
            (l + 12.0 * (1.0 - s), l + 12.0 * (1.0 + s), t + 12.5 * (1.0 - s), t + 12.5 * (1.0 + s))
        };
        let at0 = |i| at(&app, i);
        assert!((at0(0).1 - at0(0).0 - 24.0 * 0.8).abs() < 1e-3, "drawn at the component's Scale");
        assert!(((at0(0).0 + at0(1).1) * 0.5 - 380.0).abs() < 1e-3, "the pair is centred on card 0");
        assert!(((at0(2).0 + at0(2).1) * 0.5 - 435.0).abs() < 1e-3, "the single button on card 1");
        assert!(at0(3).1 - at0(3).0 < 24.0 * 0.8, "five abilities are drawn smaller");
        assert!((0..6).all(|i| ((at0(i).2 + at0(i).3) * 0.5 - 52.5).abs() < 1e-3), "the bottom row stays centred on the shell's button row");
        assert!(at0(6).3 < at0(3).2, "abilities 4 and 5 of card 2 go in a row above");
        for (i, card) in [0, 0, 1, 2, 2, 2, 2, 2].into_iter().enumerate() {
            let (l, r, ..) = at0(i);
            let c = 380.0 + 55.0 * card as f32;
            assert!(l >= c - 23.5 - 1e-3 && r <= c + 23.5 + 1e-3, "button {i} of card {card} spans {l}..{r}");
        }
        // Unused buttons stay where the shell has them.
        assert_eq!(app.world().get::<Node>(buttons[10]).unwrap().left, Val::Px(corner(10).x));
        // The bar data changes every frame in a battle (cooldowns, health): the buttons stay where they are and their nodes are not
        // written again.
        let placed: Vec<_> = (0..24).map(at0).collect();
        app.world_mut().resource_mut::<Moves>().0 = 0;
        for _ in 0..8 {
            app.world_mut().resource_mut::<CommandBarData>().set_changed();
            app.update();
            assert_eq!((0..24).map(|i| at(&app, i)).collect::<Vec<_>>(), placed);
        }
        assert_eq!(app.world().resource::<Moves>().0, 0, "no layout writes while nothing moves");
    }

    #[test]
    fn only_the_opaque_art_of_a_shell_counts_as_interface() {
        // A 40x20 design px panel shown at 2x (80x40 physical px, centred at 100,100) with one art image of 16x8 design px at (8, 4):
        // its left half opaque, its right half see-through (the faceplate above the build bar is like that).
        let mut opaque = ShellOpaque::new(Vec2::new(40.0, 20.0));
        let (w, h) = (32, 16);
        let pixels = (0..w * h).flat_map(|i| [255, 255, 255, if i % w < w / 2 { 255 } else { 10 }]).collect();
        opaque.add(&alamo_model::image::Rgba8 { width: w, height: h, pixels }, Vec2::new(8.0, 4.0), 2.0);
        let node = ComputedNode { size: Vec2::new(80.0, 40.0), ..ComputedNode::DEFAULT };
        let tf = UiGlobalTransform::from_xy(100.0, 100.0);
        // Physical px of a design point.
        let at = |x: f32, y: f32| Vec2::new(60.0, 80.0) + Vec2::new(x, y) * 2.0;
        assert!(opaque.hit(&node, &tf, at(10.0, 6.0)));
        assert!(opaque.hit(&node, &tf, at(15.0, 11.0)));
        // Inside the panel's box but on the see-through half or outside the art: open map.
        assert!(!opaque.hit(&node, &tf, at(21.0, 6.0)));
        assert!(!opaque.hit(&node, &tf, at(30.0, 15.0)));
        assert!(!opaque.hit(&node, &tf, at(2.0, 2.0)));
        assert!(!opaque.hit(&node, &tf, at(-5.0, 6.0)));
        // Rectangles: one touching the opaque half overlaps, one beside it does not.
        assert!(opaque.overlaps(&node, &tf, at(14.0, 2.0), at(18.0, 5.0)));
        assert!(!opaque.overlaps(&node, &tf, at(24.0, 2.0), at(39.0, 19.0)));
        // As rectangles: the opaque half is one 8x8 design px block (cells 2..4, rows 1..3).
        assert_eq!(opaque.rects(&node, &tf), vec![(at(8.0, 4.0), at(16.0, 12.0))]);
    }

    #[test]
    fn opaque_rows_merge_into_few_rectangles() {
        // An L shape: a 3-cell column on the left, a full-width strip at the bottom.
        let mut opaque = ShellOpaque::new(Vec2::new(16.0, 16.0));
        for (c, r) in [(0, 0), (0, 1), (0, 2), (0, 3), (1, 3), (2, 3), (3, 3)] {
            opaque.cells[r * 4 + c] = true;
        }
        let node = ComputedNode { size: Vec2::new(16.0, 16.0), ..ComputedNode::DEFAULT };
        let tf = UiGlobalTransform::from_xy(8.0, 8.0);
        assert_eq!(opaque.rects(&node, &tf), vec![(Vec2::ZERO, Vec2::new(4.0, 12.0)), (Vec2::new(0.0, 12.0), Vec2::new(16.0, 16.0))]);
    }

    #[test]
    fn card_stays_above_the_advisor_log() {
        assert_eq!(card_bottom_above_log(100.0, None, 1000.0), 100.0);
        assert_eq!(card_bottom_above_log(100.0, Some(900.0), 1000.0), 104.0, "log top 100 px above the bottom: card at least 104 px up");
        assert_eq!(card_bottom_above_log(100.0, Some(700.0), 1000.0), 304.0, "three log lines (300 px) push the card up");
        assert_eq!(card_bottom_above_log(500.0, Some(900.0), 1000.0), 500.0);
    }

    #[test]
    fn clamp_to_screen_keeps_tooltip_on_screen() {
        let screen = Vec2::new(1920.0, 1080.0);
        let size = Vec2::new(300.0, 400.0);
        // Anchor near top-left
        let p = clamp_to_screen(Vec2::new(-10.0, -20.0), Vec2::new(100.0, 100.0), size, screen);
        assert!(p.x >= 4.0 && p.y >= 4.0);
        // Anchor near bottom-right
        let p2 = clamp_to_screen(Vec2::new(1900.0, 1000.0), Vec2::new(1800.0, 950.0), size, screen);
        assert!(p2.x + size.x <= screen.x);
        assert!(p2.y >= 4.0 && p2.y + size.y <= screen.y);
    }

    #[test]
    fn fleet_box_unit_slots_exclude_the_pathfinder_plate() {
        assert!(is_fleet_box_unit("q1_0001") && is_fleet_box_unit("q3_0406"));
        assert!(!is_fleet_box_unit("q1_0000"));
        assert!(!is_fleet_box_unit("queue00") && !is_fleet_box_unit("b_create_01"));
    }

    #[test]
    fn every_card_number_has_a_hover_text() {
        for f in [Field::Credits, Field::MonthIncome, Field::Income, Field::Day, Field::TechLevel, Field::Pop] {
            let t = field_tip(f).expect("number field has a tip");
            assert!(t.contains('\n'), "{f:?}: title line plus explanation");
        }
        assert!(field_tip(Field::PlanetName).is_none());
    }

    #[test]
    fn digits_are_grouped() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(4634), "4634");
        assert_eq!(group_digits(12_450), "12.450");
        assert_eq!(group_digits(-1_234_567), "-1.234.567");
    }

    #[test]
    fn slot_names_map_to_kinds() {
        assert_eq!(slot_for("queue03").map(|s| (s.kind, s.index)), Some((SlotKind::Queue, 3)));
        assert_eq!(slot_for("tqueue09").map(|s| s.index), Some(9));
        assert_eq!(slot_for("b_create_17").map(|s| s.kind), Some(SlotKind::Pad));
        assert_eq!(slot_for("s_select_05").map(|s| s.kind), Some(SlotKind::Select));
        assert!(slot_for("b_zoom").is_none());
    }

    #[test]
    fn bounds_union_and_within() {
        let (lo, hi) = bounds_of([
            (Vec2::new(0.0, 0.0), Vec2::new(10.0, 5.0)),
            (Vec2::new(-3.0, 2.0), Vec2::new(4.0, 9.0)),
        ].into_iter());
        assert_eq!((lo, hi), (Vec2::new(-3.0, 0.0), Vec2::new(10.0, 9.0)));
        assert_eq!(bounds_of(std::iter::empty()), (Vec2::ZERO, Vec2::ONE));
        assert!(within(Vec2::new(1.0, 1.0), lo, hi));
        assert!(!within(Vec2::new(11.0, 1.0), lo, hi));
    }

    #[test]
    fn faction_indices_follow_the_data() {
        assert_eq!(UiFaction::Rebel.list_index(), 0);
        assert_eq!(UiFaction::Empire.list_index(), 1);
        assert_eq!(UiFaction::Empire.mesh_alt(), 0);
        assert_eq!(UiFaction::Rebel.mesh_alt(), 1);
        assert_eq!(UiFaction::from_name(" REBEL "), UiFaction::Rebel);
        assert_eq!(UiFaction::from_name("Hutts"), UiFaction::Underworld);
        assert_eq!(UiFaction::from_name("Republic"), UiFaction::Rebel);
        assert_eq!(UiFaction::from_name("CIS"), UiFaction::Underworld);
    }
}






/// Marks the text spans that [`apply_small_caps`] adds below a text.
#[derive(Component)]
pub struct SmallCapsSpan;

/// Splits a text into small-caps runs: every word starts with a full-size capital, the rest of the word is a smaller capital.
/// Returns (text, is_small) with neighbouring runs of the same size merged; the first run is always full size.
pub fn small_caps_segments(text: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut start = true;
    for c in text.chars() {
        let small = !start && !c.is_whitespace();
        // Spaces take the size of the run before them.
        let small = if c.is_whitespace() { out.last().is_some_and(|l| l.1) } else { small };
        for u in c.to_uppercase() {
            match out.last_mut() {
                Some(l) if l.1 == small => l.0.push(u),
                _ => out.push((u.to_string(), small)),
            }
        }
        start = c.is_whitespace();
    }
    out
}

/// Writes `text` in small caps into the `Text` of `entity`: the first run goes into the text itself, the others become [`SmallCapsSpan`]
/// children (old ones, listed in `old`, are removed). Small runs use 78% of the font size.
pub fn apply_small_caps(commands: &mut Commands, entity: Entity, text: &mut Text, font: &TextFont, color: Color, old: Option<&Children>, spans: &Query<(), With<SmallCapsSpan>>) {
    if let Some(old) = old {
        for &k in old {
            if spans.contains(k) {
                commands.entity(k).try_despawn();
            }
        }
    }
    let segs = small_caps_segments(text.0.as_str());
    text.0 = segs.first().map(|s| s.0.clone()).unwrap_or_default();
    for (s, small) in segs.into_iter().skip(1) {
        let mut f = font.clone();
        if small && let FontSize::Px(px) = f.font_size {
            f.font_size = FontSize::Px(px * 0.78);
        }
        commands.spawn((SmallCapsSpan, TextSpan::new(s), f, TextColor(color), ChildOf(entity)));
    }
}

/// Width (design px) of a tooltip: the longest line (estimated from the glyph count), within 170..340.
fn tooltip_width(layout: &TooltipLayout, icon: bool) -> f32 {
    let longest = |s: &str, per_char: f32| s.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f32 * per_char;
    let title = longest(&layout.title, 7.4) + if icon { 43.0 } else { 0.0 };
    let stats = if layout.labels.is_empty() { 0.0 } else { longest(&layout.labels, 5.8) + longest(&layout.values, 5.8) + 16.0 };
    (title.max(longest(&layout.body, 5.8)).max(stats).max(longest(&layout.warn, 5.8)) + 20.0).clamp(170.0, 340.0)
}
