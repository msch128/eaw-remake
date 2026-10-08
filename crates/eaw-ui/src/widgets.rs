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
use crate::data::{CommandBarData, EawButtonClicked, EawButtonState, ForcedTooltip};
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
    for art in arts {
        let tex = decoded
            .entry(art.texture.to_lowercase())
            .or_insert_with(|| b.textures.library().load(&art.texture));
        let Some(tex) = tex else { continue };
        let (alo, ahi) = art.bounds();
        let img = rasterize(&art.triangles, tex, alo, ahi, oversample);
        let handle = b.upload_raw(&art.name, img);
        b.image_at(root, &handle, (alo + ahi) * 0.5 - lo, ahi - alo, Color::WHITE, Some(&art.name));
    }

    // Components in draw order (z, then base layer). Components far outside the art belong to
    // other panels (e.g. the ground filters below the galactic bar).
    let mut items: Vec<_> = layout.order.iter().filter_map(|n| layout.item(n)).collect();
    items.sort_by(|a, c| {
        let la = b.db.get(&a.name).map_or(0, |d| d.base_layer);
        let lc = b.db.get(&c.name).map_or(0, |d| d.base_layer);
        a.z.total_cmp(&c.z).then(la.cmp(&lc))
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
                    left: Val::Px(offset.x),
                    top: Val::Px(-offset.y),
                    width: Val::Px(rect_size.x),
                    height: Val::Px(rect_size.y),
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
            if matches!(field, Field::Credits | Field::Income | Field::MonthIncome)
                && let Some(tx) = self.tex("i_icon_credit.tga")
            {
                let sz = tx.size.as_vec2() * 0.8;
                self.layer(holder, size, &tx, sz, Vec2::new(-size.x * 0.5 + sz.x * 0.5 + 2.0, 0.0), Color::WHITE);
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
        let alt_icons = CommandBarComponent::pick(&def.icon_alt, 0).is_some();

        let button = self
            .world
            .spawn((
                Name::new(def.name.clone()),
                EawButton {
                    name: lname.clone(),
                    tooltips: def.tooltips.clone(),
                    tooltip_literal: None,
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
        if let Some(t) = CommandBarComponent::pick(&def.blank, fi).and_then(|n| self.tex(n)) {
            let mut sz = scaled(&t);
            icon_px = (sz.min_element() * 0.84).max(20.0);
            if slot.is_some_and(|s| s.kind == SlotKind::Ability) {
                // The ability border is drawn at its natural size and the icon fills it
                // (the shell rectangle is 24x25, the textures are 26x26).
                sz = t.size.as_vec2();
                icon_px = sz.min_element() * 0.88;
            }
            self.layer(button, size, &t, sz, Vec2::ZERO, tint);
        }
        let icon_tex = CommandBarComponent::pick(&def.icon, fi)
            .or_else(|| {
                alt_icons
                    .then(|| CommandBarComponent::pick(&def.icon_alt, 0))
                    .flatten()
            })
            .and_then(|n| self.tex(n));
        if let Some(t) = &icon_tex {
            let sz = scaled(t);
            layers.icon = Some(self.layer(button, size, t, sz, def.icon_offset, tint));
        }
        if let Some(t) = CommandBarComponent::pick(&def.disabled, fi).and_then(|n| self.tex(n)) {
            let sz = scaled(&t);
            let e = self.layer(button, size, &t, sz, def.icon_offset, tint);
            self.hide(e);
            layers.disabled = Some(e);
        }
        if let Some(t) = CommandBarComponent::pick(&def.selected, fi).and_then(|n| self.tex(n)) {
            let sz = scaled(&t);
            let e = self.layer(button, size, &t, sz, def.icon_offset, tint);
            self.hide(e);
            layers.selected = Some(e);
        }
        if let Some(t) = CommandBarComponent::pick(&def.mouse_over, fi).and_then(|n| self.tex(n)) {
            let sz = scaled(&t);
            let e = self.layer(button, size, &t, sz, Vec2::ZERO, tint);
            self.hide(e);
            layers.over = Some(e);
        }
        self.world.entity_mut(button).insert(layers);
        if lname.starts_with("choice_button_") {
            // Labels of the battle banner buttons come from `PanelData::texts`.
            let color = def.text_color.map_or(Color::WHITE, rgba);
            let l = self.text_layer(button, size, def, "", Vec2::ZERO, color);
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
            let color = def
                .text_color2
                .or(def.text_color)
                .map_or(Color::srgb(0.85, 0.9, 1.0), rgba);
            let l = self.text_layer(button, size, def, "", label_off, color);
            self.world.entity_mut(l).insert(SlotPart::Label);
            if s.kind == SlotKind::Pad {
                // Upkeep marker (top left) and level numeral (top right) of build pads.
                if let Some(t) = self.tex("i_icon_credit.tga") {
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
            let _ = s;
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
        (&ButtonLayers, &Interaction, &EawButtonState),
        Or<(Changed<Interaction>, Changed<EawButtonState>)>,
    >,
    mut nodes: Query<&mut Node>,
) {
    for (l, i, st) in &buttons {
        let hover = matches!(i, Interaction::Hovered | Interaction::Pressed) || st.force_hover;
        let pressed = matches!(i, Interaction::Pressed) || st.selected;
        let set = |nodes: &mut Query<&mut Node>, e: Option<Entity>, on: bool| {
            if let Some(e) = e
                && let Ok(mut n) = nodes.get_mut(e)
            {
                n.display = if on { Display::Flex } else { Display::None };
            }
        };
        let show_disabled = st.disabled && l.disabled.is_some();
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
    mut texts: Query<(&BoundField, &mut Text, Option<&mut TextColor>)>,
    mut icons: Query<(&BoundField, &mut ImageNode), Without<Text>>,
    mut tex: ResMut<UiTextures>,
    db: Res<CommandBarDb>,
    mut images: ResMut<Assets<Image>>,
) {
    if !data.is_changed() && fresh.is_empty() {
        return;
    }
    for (f, mut t, color) in &mut texts {
        if f.0 == Field::PlanetName && let (Some(c), Some(mut tc)) = (data.planet_color, color) {
            tc.0 = c;
        }
        t.0 = match f.0 {
            Field::Credits => group_digits(data.credits),
            Field::Income => group_digits(data.planet_income),
            Field::PlanetName => data.planet_name.clone(),
            Field::Day => format!("MONTH: {}", data.day),
            Field::MonthIncome => group_digits(data.month_income),
            Field::TechLevel => format!("TECH: {}", data.tech_level),
            Field::Pop => format!("{} / {}", data.pop_current, data.pop_cap),
            Field::Allegiance => String::new(),
        };
    }
    if let Some(alt) = db.get("Text_Allegiance_A") {
        for (f, mut img) in &mut icons {
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

/// Fills queue, pad, selection and ability slots from [`CommandBarData`].
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn slots(
    fresh: Query<(), Added<Slot>>,
    data: Res<CommandBarData>,
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
        (&SlotPart, &mut Node, Option<&mut ImageNode>, Option<&mut Text>),
        Without<Slot>,
    >,
) {
    if !data.is_changed() && !panel.is_changed() && fresh.is_empty() {
        return;
    }
    for (e, slot, pname, mut node, mut st, mut vis, mut button) in &mut slots {
        button.tooltip_literal = match slot.kind {
            SlotKind::Ability => data.abilities.get(slot.index).map(|a| a.tooltip.clone()),
            SlotKind::Pad => data.pads.get(slot.index).map(|p| p.tooltip.clone()).filter(|t| !t.is_empty()),
            SlotKind::Queue => data.queue.get(slot.index).map(|q| q.tooltip.clone()).filter(|t| !t.is_empty()),
            _ => None,
        };
        // (icon, progress, label, disabled, selected)
        let content: Option<(String, f32, String, bool, bool)> = match slot.kind {
            SlotKind::Queue => data.queue.get(slot.index).map(|q| {
                (
                    q.icon.clone(),
                    q.progress,
                    if q.count > 1 { q.count.to_string() } else { String::new() },
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
            SlotKind::Select => data.selection.get(slot.index).map(|s| {
                (
                    s.icon.clone(),
                    0.0,
                    if s.count > 1 { s.count.to_string() } else { String::new() },
                    false,
                    false,
                )
            }),
            SlotKind::Panel => pname.and_then(|n| panel.slots.get(&n.0)).map(|s| {
                (s.icon.clone(), s.progress, s.label.clone(), s.disabled, s.selected)
            }),
            SlotKind::Border => (data.selection.len() > slot.index * 2)
                .then(|| (String::new(), 0.0, String::new(), false, false)),
            SlotKind::Ability => data.abilities.get(slot.index).map(|a| {
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
        for k in children.iter_descendants(e) {
            let Ok((part, mut n, img, text)) = parts.get_mut(k) else {
                continue;
            };
            match part {
                SlotPart::Icon => {
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
            None => data.selection.get(bar.index).and_then(|s| {
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
                    width: Val::Px(size.x),
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

const SEGMENT_ON: Color = Color::srgb(0.25, 0.95, 0.3);
const SEGMENT_OFF: Color = Color::srgba(0.1, 0.3, 0.12, 0.8);

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

/// Spawns the global tooltip panel.
pub(crate) fn spawn_tooltip(mut commands: Commands, settings: Res<EawUiSettings>) {
    let font = match &settings.tooltip_font_family {
        Some(f) => TextFont {
            font: FontSource::Family(f.as_str().into()),
            font_size: FontSize::Px(10.5),
            ..default()
        },
        None => TextFont::from_font_size(10.5),
    };
    let mut tip = commands.spawn((
            Tooltip,
            Name::new("EaW tooltip"),
            Node {
                position_type: PositionType::Absolute,
                padding: UiRect::axes(Val::Px(7.0), Val::Px(4.0)),
                border: UiRect::all(Val::Px(1.0)),
                max_width: Val::Px(330.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.87)),
            BorderColor::all(Color::srgb(0.0, 0.76, 0.99)),
            GlobalZIndex(1000),
            FocusPolicy::Pass,
        ));
    if let Some(c) = settings.camera {
        tip.insert(UiTargetCamera(c));
    }
    tip.with_children(|p| {
        p.spawn((
            Text::new(""),
            font,
            TextColor(Color::srgb(0.74, 0.85, 0.96)),
            FocusPolicy::Pass,
        ));
    });
}

/// Shows the tooltip of the hovered (or forced) button next to it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tooltip(
    assets: Res<EawUiAssets>,
    forced: Res<ForcedTooltip>,
    buttons: Query<(&EawButton, &Interaction, &ComputedNode, &UiGlobalTransform)>,
    mut tip: Query<(&mut Node, &Children, &ComputedNode), With<Tooltip>>,
    mut texts: Query<&mut Text>,
    window: Query<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
) {
    let Ok((mut node, kids, own)) = tip.single_mut() else {
        return;
    };
    let mut hit = None;
    for (b, i, cn, gt) in &buttons {
        let forced_hit = forced.0.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&b.name));
        if (*i != Interaction::None || forced_hit)
            && (!b.tooltips.is_empty() || b.tooltip_literal.is_some())
        {
            hit = Some((b, cn, gt, forced_hit));
            if !forced_hit {
                break;
            }
        }
    }
    let Some((b, cn, gt, forced_hit)) = hit else {
        node.display = Display::None;
        return;
    };
    let text = b
        .tooltip_literal
        .as_deref()
        .or_else(|| b.tooltips.iter().find_map(|k| assets.text.get(k).filter(|t| !t.is_empty())))
        .unwrap_or("");
    if text.is_empty() {
        node.display = Display::None;
        return;
    }
    if let Some(&k) = kids.first()
        && let Ok(mut t) = texts.get_mut(k)
    {
        t.0 = text.to_string();
    }
    node.display = Display::Flex;
    let inv = cn.inverse_scale_factor();
    let anchor = if forced_hit {
        // Above the button centre (physical px -> logical).
        let c = gt.translation * inv;
        Vec2::new(c.x - 10.0, c.y - cn.size().y * inv * 0.5 - 30.0 * scale.0)
    } else {
        window
            .single()
            .ok()
            .and_then(Window::cursor_position)
            .map_or(Vec2::ZERO, |p| p + Vec2::new(14.0, 18.0))
    };
    let anchor = match window.single() {
        Ok(w) if !forced_hit => clamp_to_screen(anchor, w.cursor_position().unwrap_or(anchor), own.size() / w.scale_factor(), w.size()),
        _ => anchor,
    };
    // Bevy positions logical px; UiScale is applied to Val::Px so undo it.
    node.left = Val::Px(anchor.x / scale.0);
    node.top = Val::Px(anchor.y / scale.0);
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
            Ok(w) => w.height() / crate::shell::REFERENCE_HEIGHT,
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
/// cursor when it would run over the bottom edge.
pub fn clamp_to_screen(anchor: Vec2, cursor: Vec2, size: Vec2, screen: Vec2) -> Vec2 {
    let mut p = anchor;
    if p.x + size.x > screen.x - 4.0 {
        p.x = (screen.x - size.x - 4.0).max(0.0);
    }
    if p.y + size.y > screen.y - 4.0 {
        p.y = (cursor.y - size.y - 12.0).max(0.0);
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
    }
}





