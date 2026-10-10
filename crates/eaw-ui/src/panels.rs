//! Panels beyond the command bars: planet screen, fleet organizer, reinforcement and hero
//! frames, minimaps, story dialogs, and slot drag and drop.
//!
//! The shell panels (`ShellSpec`) are filled through [`PanelData`], keyed by the original
//! component names, so the application never needs to know screen coordinates:
//!
//! * planet info: `planet_structure_00..08` (built structures), `slot_00/01`, `Pad_00..02`,
//!   texts `Planet_name_text`, `unit_cap_number`, `building_cap_number`, `Planetary_Income_number`
//! * fleet organizer (`I_Galactic_Organize_New.ALO`): `q1_RRCC`, `q2_RRCC`, `q3_RRCC` (fleet 1..3,
//!   row `RR` 00..04, column `CC` 00..06), see [`fleet_slot_name`]
//! * reinforcement window (`I_Main_Reinforce.ALO`): `r_RRCC`, `r_pop_text`
//! * hero frames: `hero_slot_good00..10` / `hero_slot_evil00..10` with bars `hero_hb_*`

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::ui::{ComputedNode, FocusPolicy, RelativeCursorPosition, UiGlobalTransform};
use bevy::window::PrimaryWindow;

use crate::data::EawButtonClicked;
use crate::faction::UiFaction;
use crate::textures::UiTextures;
use crate::widgets::EawButton;
use crate::{EawUiAssets, EawUiSettings};

/// Content of one panel slot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PanelSlot {
    /// Icon texture name (`Icon_Name` of a unit or structure).
    pub icon: String,
    /// Small label (count, price ...).
    pub label: String,
    /// Progress 0..1 (build / recharge), 0 = none.
    pub progress: f32,
    /// Greyed out.
    pub disabled: bool,
    /// Highlighted (selected).
    pub selected: bool,
}

/// Content of all shell panels, keyed by lower-cased component name (resource).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct PanelData {
    /// Slot contents; a slot without an entry stays hidden.
    pub slots: HashMap<String, PanelSlot>,
    /// Text of text components (`planet_name_text` ...).
    pub texts: HashMap<String, String>,
    /// Bar fractions 0..1 (`hero_hb_good00` ...).
    pub bars: HashMap<String, f32>,
}

impl PanelData {
    /// Sets a slot.
    pub fn set_slot(&mut self, name: &str, slot: PanelSlot) {
        self.slots.insert(name.to_lowercase(), slot);
    }

    /// Sets a text.
    pub fn set_text(&mut self, name: &str, text: impl Into<String>) {
        self.texts.insert(name.to_lowercase(), text.into());
    }
}

/// Name of a slot of the fleet organizer: `fleet` 1..=3, `row` 0..=4, `col` 0..=6.
pub fn fleet_slot_name(fleet: u32, row: u32, col: u32) -> String {
    format!("q{fleet}_{row:02}{col:02}")
}

/// True for components that are filled from [`PanelData::slots`].
pub(crate) fn is_panel_slot(lname: &str) -> bool {
    const PREFIXES: [&str; 12] = [
        "build_pad",
        "unit_pad",
        "space_pad_",
        "q1_",
        "q2_",
        "q3_",
        "planet_structure_",
        "slot_",
        "pad_",
        "land_fleet_",
        "r_0",
        "hero_slot_",
    ];
    lname.ends_with(|c: char| c.is_ascii_digit()) && PREFIXES.iter().any(|p| lname.starts_with(p))
}

/// Slot driven by [`PanelData::slots`].
#[derive(Component, Debug, Clone)]
pub struct PanelSlotName(pub String);

/// Text driven by [`PanelData::texts`].
#[derive(Component, Debug, Clone)]
pub struct PanelText(pub String);

/// Bar driven by [`PanelData::bars`].
#[derive(Component, Debug, Clone)]
pub struct PanelBarName(pub String);

/// A slot was dragged onto another slot (or released over nothing: `to == None`).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct EawSlotDropped {
    /// Lower-cased component name the drag started on.
    pub from: String,
    /// Lower-cased component name under the cursor on release.
    pub to: Option<String>,
    /// Shift held (split instead of move in the fleet organizer).
    pub shift: bool,
}

#[derive(Resource, Default)]
pub(crate) struct DragState {
    from: Option<String>,
}

/// Tracks drag and drop between buttons flagged `Drag_And_Drop`.
pub(crate) fn slot_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Query<&Window, With<PrimaryWindow>>,
    buttons: Query<(&EawButton, &Interaction, &ComputedNode, &UiGlobalTransform)>,
    mut state: ResMut<DragState>,
    mut out: MessageWriter<EawSlotDropped>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        state.from = buttons
            .iter()
            .find(|(b, i, ..)| b.drag && **i == Interaction::Pressed)
            .map(|(b, ..)| b.name.clone());
    }
    if mouse.just_released(MouseButton::Left)
        && let Some(from) = state.from.take()
    {
        let cursor = window
            .single()
            .ok()
            .and_then(|w| w.physical_cursor_position());
        let to = cursor.and_then(|c| {
            buttons
                .iter()
                .find(|(b, _, n, t)| b.drag && n.contains_point(**t, c))
                .map(|(b, ..)| b.name.clone())
        });
        if to.as_deref() != Some(from.as_str()) {
            out.write(EawSlotDropped {
                from,
                to,
                shift: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
            });
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Minimap
// ---------------------------------------------------------------------------------------------

/// What kind of minimap a surface is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinimapKind {
    /// Galactic map (`radar_galactic`): planets and fleets.
    Galactic,
    /// Space battle radar (`radar`): units and map bounds.
    Tactical,
}

/// A point on a minimap, in world coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinimapDot {
    /// World position (x right, y up).
    pub pos: Vec2,
    /// Colour, normally the owner's colour.
    pub color: Color,
    /// Larger, with a highlight ring (selected planet).
    pub selected: bool,
}

/// Minimap content (resource).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct MinimapData {
    /// World rectangle shown by the minimap: `(min, max)`.
    pub bounds: Option<(Vec2, Vec2)>,
    /// Planets (galactic) or large objects.
    pub planets: Vec<MinimapDot>,
    /// Units (tactical) or fleets (galactic).
    pub units: Vec<MinimapDot>,
    /// Active expanding radar pings or scan waves.
    pub pings: Vec<MinimapPing>,
    /// The camera's view rectangle in world coordinates: `(min, max)`.
    pub camera: Option<(Vec2, Vec2)>,
    /// Fill drawn behind the marks (covers the picture of the bar art, e.g. a tint for a tactical map).
    pub background: Option<Color>,
}

/// An expanding radar wave ring drawn on the minimap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinimapPing {
    /// Position in minimap world coordinates.
    pub pos: Vec2,
    /// Ring radius in world units.
    pub radius: f32,
    /// Color and alpha transparency.
    pub color: Color,
}

/// Fog of war on the tactical minimap: when `enabled`, the map is drawn dark except inside the `reveal` discs (world position, radius).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct MinimapFog {
    /// Draw the fog.
    pub enabled: bool,
    /// Areas the player currently sees.
    pub reveal: Vec<(Vec2, f32)>,
}

/// The minimap click target of a command bar.
#[derive(Component, Debug, Clone, Copy)]
pub struct MinimapSurface {
    /// Kind.
    pub kind: MinimapKind,
}

/// The user clicked or dragged on a minimap.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct EawMinimapClicked {
    /// World position under the cursor (needs [`MinimapData::bounds`]).
    pub world: Vec2,
    /// Normalized position inside the minimap, (0,0) bottom-left, (1,1) top-right.
    pub normalized: Vec2,
    /// True on the frame the button went down.
    pub just_pressed: bool,
}

#[derive(Component)]
pub struct MinimapMark;

/// Maps a world position into normalized minimap coordinates (0,0 bottom-left).
pub fn world_to_minimap(bounds: (Vec2, Vec2), p: Vec2) -> Vec2 {
    let size = (bounds.1 - bounds.0).max(Vec2::splat(1e-6));
    (p - bounds.0) / size
}

/// Inverse of [`world_to_minimap`].
pub fn minimap_to_world(bounds: (Vec2, Vec2), n: Vec2) -> Vec2 {
    bounds.0 + n * (bounds.1 - bounds.0)
}

/// Rebuilds the dots and the camera frustum when the data changes.
pub(crate) fn minimap_marks(
    data: Res<MinimapData>,
    fog: Res<MinimapFog>,
    fresh: Query<(), Added<MinimapSurface>>,
    surfaces: Query<(Entity, &MinimapSurface, Option<&Children>)>,
    marks: Query<(), With<MinimapMark>>,
    mut commands: Commands,
) {
    if !data.is_changed() && !fog.is_changed() && fresh.is_empty() {
        return;
    }
    let Some(bounds) = data.bounds else { return };
    for (e, surf, kids) in &surfaces {
        for k in kids.into_iter().flatten() {
            if marks.contains(*k) {
                commands.entity(*k).despawn();
            }
        }
        if let Some(bg) = data.background {
            // Fog: unexplored areas read as clearly dark/hidden (0.08 tint), explored sensor discs show the lit background tint.
            let base = if fog.enabled { bg.to_linear().with_alpha(1.0).to_vec3() * 0.08 } else { bg.to_linear().to_vec3() };
            let base = Color::linear_rgba(base.x, base.y, base.z, bg.alpha());
            commands.spawn((
                MinimapMark,
                Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                BackgroundColor(base),
                FocusPolicy::Pass,
                ChildOf(e),
            ));
            if fog.enabled {
                let size = (bounds.1 - bounds.0).max(Vec2::splat(1e-6));
                for (pos, radius) in &fog.reveal {
                    let n = world_to_minimap(bounds, *pos);
                    let (rw, rh) = (radius / size.x * 100.0, radius / size.y * 100.0);
                    commands.spawn((
                        MinimapMark,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Percent(n.x * 100.0 - rw),
                            bottom: Val::Percent(n.y * 100.0 - rh),
                            width: Val::Percent(rw * 2.0),
                            height: Val::Percent(rh * 2.0),
                            border_radius: BorderRadius::MAX,
                            ..default()
                        },
                        BackgroundColor(bg),
                        FocusPolicy::Pass,
                        ChildOf(e),
                    ));
                }
            }
        }
        let dot = |commands: &mut Commands, d: &MinimapDot, size: f32| {
            let n = world_to_minimap(bounds, d.pos);
            let size = if d.selected { size * 1.8 } else { size };
            let mut c = commands.spawn((
                MinimapMark,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(n.x * 100.0),
                    bottom: Val::Percent(n.y * 100.0),
                    width: Val::Px(size),
                    height: Val::Px(size),
                    margin: UiRect::all(Val::Px(-size * 0.5)),
                    border: if d.selected {
                        UiRect::all(Val::Px(1.0))
                    } else {
                        UiRect::ZERO
                    },
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(d.color),
                FocusPolicy::Pass,
                ChildOf(e),
            ));
            if d.selected {
                c.insert(BorderColor::all(Color::WHITE));
            }
        };
        let (psize, usize_) = match surf.kind {
            MinimapKind::Galactic => (4.0, 3.0),
            MinimapKind::Tactical => (5.0, 2.0),
        };
        for p in &data.planets {
            dot(&mut commands, p, psize);
        }
        for u in &data.units {
            dot(&mut commands, u, usize_);
        }
        if let Some((lo, hi)) = data.camera {
            let (a, b) = (world_to_minimap(bounds, lo), world_to_minimap(bounds, hi));
            let (min, max) = (a.min(b), a.max(b));
            commands.spawn((
                MinimapMark,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(min.x * 100.0),
                    bottom: Val::Percent(min.y * 100.0),
                    width: Val::Percent((max.x - min.x) * 100.0),
                    height: Val::Percent((max.y - min.y) * 100.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.9)),
                FocusPolicy::Pass,
                ChildOf(e),
            ));
        }
        let size = (bounds.1 - bounds.0).max(Vec2::splat(1e-6));
        for ping in &data.pings {
            let n = world_to_minimap(bounds, ping.pos);
            let (rw, rh) = (ping.radius / size.x * 100.0, ping.radius / size.y * 100.0);
            commands.spawn((
                MinimapMark,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(n.x * 100.0 - rw),
                    bottom: Val::Percent(n.y * 100.0 - rh),
                    width: Val::Percent(rw * 2.0),
                    height: Val::Percent(rh * 2.0),
                    border: UiRect::all(Val::Px(1.5)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BorderColor::all(ping.color),
                FocusPolicy::Pass,
                ChildOf(e),
            ));
        }
    }
}

/// Emits [`EawMinimapClicked`] while the mouse is pressed on a minimap.
pub(crate) fn minimap_clicks(
    data: Res<MinimapData>,
    surfaces: Query<(&Interaction, &RelativeCursorPosition), With<MinimapSurface>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut out: MessageWriter<EawMinimapClicked>,
) {
    for (i, rel) in &surfaces {
        if *i != Interaction::Pressed {
            continue;
        }
        let Some(n) = rel.normalized else { continue };
        // RelativeCursorPosition is centred and y-down: convert to 0..1, y up.
        let normalized = Vec2::new(n.x + 0.5, 0.5 - n.y);
        let world = data
            .bounds
            .map_or(normalized, |b| minimap_to_world(b, normalized));
        out.write(EawMinimapClicked {
            world,
            normalized,
            just_pressed: mouse.just_pressed(MouseButton::Left),
        });
    }
}

// ---------------------------------------------------------------------------------------------
// Galactic conquest message / advisor dialog
// ---------------------------------------------------------------------------------------------

/// A galactic conquest message box (GC intro transmissions, game messages) with optional portrait.
#[derive(Debug, Clone, Default)]
pub struct DialogSpec {
    /// Title: a text id (`TEXT_...`, resolved) or literal text.
    pub title: String,
    /// Body: a text id or literal text.
    pub text: String,
    /// Portrait texture name (`None` shows the faction symbol).
    pub portrait: Option<String>,
    /// Faction skin (colours).
    pub faction: UiFaction,
    /// Button labels (text ids or literals); empty gives one "OK".
    pub buttons: Vec<String>,
}

/// A dialog button was clicked; the dialog closes itself.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EawDialogResult {
    /// The dialog root (already despawned when you read this).
    pub dialog: Entity,
    /// Index into [`DialogSpec::buttons`].
    pub button: usize,
}

/// Root of a dialog.
#[derive(Component, Debug, Clone, Copy)]
pub struct EawDialog;

#[derive(Component)]
pub struct DialogButton {
    dialog: Entity,
    index: usize,
}

/// Panel colours of a faction skin: `(background, border, text accent)`.
pub fn faction_colors(f: UiFaction) -> (Color, Color, Color) {
    match f {
        UiFaction::Empire => (
            Color::srgba(0.02, 0.05, 0.12, 0.93),
            Color::srgb(0.25, 0.5, 0.9),
            Color::srgb(0.75, 0.88, 1.0),
        ),
        UiFaction::Rebel => (
            Color::srgba(0.1, 0.03, 0.03, 0.93),
            Color::srgb(0.85, 0.35, 0.3),
            Color::srgb(1.0, 0.85, 0.8),
        ),
        UiFaction::Underworld => (
            Color::srgba(0.1, 0.07, 0.02, 0.93),
            Color::srgb(0.9, 0.65, 0.2),
            Color::srgb(1.0, 0.92, 0.75),
        ),
    }
}

/// Spawns a centred GC message dialog; returns its root.
pub fn spawn_dialog(world: &mut World, spec: &DialogSpec) -> Option<Entity> {
    // The original dialogs share one frame whatever the faction: navy glass, blue glow, black buttons.
    let _ = faction_colors(spec.faction);
    let (bg, border, accent) = (Color::srgba(0.03, 0.05, 0.12, 0.93), Color::srgb(0.42, 0.58, 0.88), Color::srgb(0.95, 0.96, 0.98));
    let (font_family, camera) = {
        let s = world.get_resource::<EawUiSettings>()?;
        (s.font_family.clone(), s.camera)
    };
    let resolve = |w: &World, s: &str| -> String {
        w.get_resource::<EawUiAssets>()
            .and_then(|a| a.text.get(s).filter(|t| !t.is_empty()).map(str::to_string))
            .unwrap_or_else(|| s.to_string())
    };
    let title = resolve(world, &spec.title);
    let body = resolve(world, &spec.text);
    let labels: Vec<String> = if spec.buttons.is_empty() {
        vec!["OK".to_string()]
    } else {
        spec.buttons.iter().map(|b| resolve(world, b)).collect()
    };
    // Portrait: explicit texture or the faction symbol from the command bar atlas.
    let portrait_name = spec.portrait.clone().unwrap_or_else(|| {
        match spec.faction {
            UiFaction::Rebel => "i_button_command_bar_rebel_symbol.tga",
            UiFaction::Empire => "i_button_command_bar_empire_symbol.tga",
            UiFaction::Underworld => "i_button_command_bar_underworld_symbol.tga",
        }
        .to_string()
    });
    let portrait = {
        let mut textures = world.remove_resource::<UiTextures>()?;
        let tex = {
            let mut images = world.get_resource_mut::<Assets<Image>>()?;
            textures.get(&mut images, &portrait_name)
        };
        world.insert_resource(textures);
        tex
    };
    let font = |size: f32| match &font_family {
        Some(f) => TextFont {
            font: FontSource::Family(f.as_str().into()),
            font_size: FontSize::Px(size),
            ..default()
        },
        None => TextFont::from_font_size(size),
    };

    let root = world
        .spawn((
            EawDialog,
            Name::new("EaW dialog"),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                margin: UiRect {
                    left: Val::Px(-260.0),
                    top: Val::Px(-95.0),
                    ..default()
                },
                width: Val::Px(520.0),
                min_height: Val::Px(150.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(2.0)),
                column_gap: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(bg),
            BorderColor::all(border),
            BoxShadow::new(Color::srgba(0.3, 0.5, 1.0, 0.4), Val::Px(0.0), Val::Px(0.0), Val::Px(3.0), Val::Px(14.0)),
            GlobalZIndex(500),
        ))
        .id();
    if let Some(c) = camera {
        world.entity_mut(root).insert(UiTargetCamera(c));
    }
    if let Some(p) = portrait {
        world.spawn((
            ImageNode::new(p.handle),
            Node {
                width: Val::Px(96.0),
                height: Val::Px(96.0),
                flex_shrink: 0.0,
                ..default()
            },
            ChildOf(root),
        ));
    }
    let col = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                flex_grow: 1.0,
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    world.spawn((
        Text::new(title.to_uppercase()),
        font(16.0),
        TextColor(accent),
        ChildOf(col),
    ));
    world.spawn((
        Text::new(body),
        font(13.0),
        TextColor(Color::srgb(0.9, 0.93, 0.98)),
        ChildOf(col),
    ));
    let row = world
        .spawn((
            Node {
                column_gap: Val::Px(8.0),
                justify_content: JustifyContent::FlexEnd,
                margin: UiRect::top(Val::Px(6.0)),
                ..default()
            },
            ChildOf(col),
        ))
        .id();
    for (index, label) in labels.into_iter().enumerate() {
        let btn = world
            .spawn((
                Button,
                DialogButton { dialog: root, index },
                Node {
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(5.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.01, 0.01, 0.03)),
                BorderColor::all(Color::srgb(0.6, 0.65, 0.75)),
                ChildOf(row),
            ))
            .id();
        world.spawn((Text::new(label.to_uppercase()), font(13.0), TextColor(accent), ChildOf(btn)));
    }
    Some(root)
}

/// One side of the battle banner: a stack of discs with a portrait on top.
#[derive(Debug, Clone)]
pub struct BannerSide {
    /// Number of discs (1..=5, by fleet size).
    pub stacks: u32,
    /// Portrait or ship icon texture name.
    pub icon: Option<String>,
    /// Disc tint (green for the player, red for the enemy).
    pub tint: Color,
}

/// The "<PLANET>: SPACE BATTLE IMMINENT!" banner at the top of the galaxy map.
#[derive(Debug, Clone)]
pub struct BattleBannerSpec {
    /// Headline (literal, already upper case).
    pub title: String,
    /// Faction skin.
    pub faction: UiFaction,
    /// Label of the left button (Begin).
    pub begin: String,
    /// Label of the right button (Auto-Resolve).
    pub auto: String,
    /// Left disc stack (the player).
    pub left: BannerSide,
    /// Right disc stack (the opponent).
    pub right: BannerSide,
}

/// Names of the banner buttons in [`EawButtonClicked`].
pub const BANNER_BEGIN: &str = "choice_button_left";
/// See [`BANNER_BEGIN`].
pub const BANNER_AUTO: &str = "choice_button_right";

/// Spawns the original battle banner (`i_attack_mode_choice.alo`); returns its root. The buttons report
/// [`BANNER_BEGIN`] / [`BANNER_AUTO`] through [`EawButtonClicked`]; the caller closes the banner.
pub fn spawn_battle_banner(world: &mut World, spec: &BattleBannerSpec) -> Option<Entity> {
    use crate::widgets::{ShellAnchor, ShellSpec, spawn_shell};
    let font_family = world.get_resource::<EawUiSettings>()?.font_family.clone();
    {
        let mut data = world.get_resource_mut::<PanelData>()?;
        data.set_text(BANNER_BEGIN, spec.begin.clone());
        data.set_text(BANNER_AUTO, spec.auto.clone());
    }
    let mut shell = ShellSpec::new("i_attack_mode_choice.alo", spec.faction);
    shell.anchor = ShellAnchor::TopCenter;
    shell.offset = Vec2::new(0.0, -6.0);
    let root = spawn_shell(world, &shell)?;
    world.entity_mut(root).insert(GlobalZIndex(500));
    let (width, height) = {
        let n = world.get::<Node>(root)?;
        match (n.width, n.height) {
            (Val::Px(w), Val::Px(h)) => (w, h),
            _ => (436.0, 110.0),
        }
    };
    let font = |size: f32| match &font_family {
        Some(f) => TextFont { font: FontSource::Family(f.as_str().into()), font_size: FontSize::Px(size), ..default() },
        None => TextFont::from_font_size(size),
    };
    world
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(8.0),
                width: Val::Px(width),
                height: Val::Px(26.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            FocusPolicy::Pass,
            ChildOf(root),
        ))
        .with_children(|p| {
            p.spawn((Text::new(spec.title.clone()), font(14.0), TextColor(Color::srgb(0.95, 0.97, 1.0)), bevy::text::LetterSpacing::Rem(0.08), FocusPolicy::Pass));
        });
    // Disc centres sit on the button row, either side of the centre column.
    let row_y = height - 35.0;
    for (side, dx) in [(&spec.left, -23.0), (&spec.right, 23.0)] {
        let stack = format!("fleet_stacks_{:02}.tga", side.stacks.clamp(1, 5));
        let discs = {
            let mut textures = world.remove_resource::<UiTextures>()?;
            let t = {
                let mut images = world.get_resource_mut::<Assets<Image>>()?;
                (textures.get(&mut images, &stack), side.icon.as_deref().and_then(|n| textures.get(&mut images, n)))
            };
            world.insert_resource(textures);
            t
        };
        let cx = width * 0.5 + dx;
        let image = |world: &mut World, tex: &crate::textures::UiTexture, size: Vec2, centre: Vec2, tint: Color| {
            world.spawn((
                ImageNode::new(tex.handle.clone()).with_color(tint),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(centre.x - size.x * 0.5),
                    top: Val::Px(centre.y - size.y * 0.5),
                    width: Val::Px(size.x),
                    height: Val::Px(size.y),
                    ..default()
                },
                FocusPolicy::Pass,
                ChildOf(root),
            ));
        };
        if let Some(t) = &discs.0 {
            image(world, t, Vec2::new(42.0, 52.5), Vec2::new(cx, row_y + 2.0), side.tint);
        }
        if let Some(t) = &discs.1 {
            image(world, t, Vec2::splat(30.0), Vec2::new(cx, row_y - 3.0), Color::WHITE);
        }
    }
    Some(root)
}

/// Closes dialogs when a button is pressed and reports the choice. Firing on the press (not the
/// release) cannot be lost when press and release land in one frame, or when the cursor leaves
/// the button in between.
pub(crate) fn dialog_buttons(
    mut q: Query<(&DialogButton, &Interaction, &mut BackgroundColor), Changed<Interaction>>,
    mut out: MessageWriter<EawDialogResult>,
    mut clicked: MessageWriter<EawButtonClicked>,
    mut commands: Commands,
) {
    for (b, i, mut bg) in &mut q {
        match i {
            Interaction::Pressed => {
                out.write(EawDialogResult {
                    dialog: b.dialog,
                    button: b.index,
                });
                clicked.write(EawButtonClicked {
                    name: format!("dialog_button_{}", b.index),
                    entity: b.dialog,
                    shift: false,
                });
                commands.entity(b.dialog).despawn();
            }
            Interaction::Hovered => bg.0 = Color::srgb(0.1, 0.13, 0.24),
            Interaction::None => bg.0 = Color::srgb(0.01, 0.01, 0.03),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressing_a_dialog_button_reports_and_closes() {
        let mut app = App::new();
        app.add_message::<EawDialogResult>()
            .add_message::<EawButtonClicked>()
            .add_systems(Update, dialog_buttons);
        let dialog = app.world_mut().spawn(EawDialog).id();
        let btn = app
            .world_mut()
            .spawn((DialogButton { dialog, index: 1 }, Interaction::Hovered, BackgroundColor(Color::NONE)))
            .id();
        app.update();
        *app.world_mut().get_mut::<Interaction>(btn).unwrap() = Interaction::Pressed;
        app.update();
        let got: Vec<_> = app.world_mut().resource_mut::<Messages<EawDialogResult>>().drain().collect();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].dialog, got[0].button), (dialog, 1));
        assert!(app.world().get_entity(dialog).is_err());
    }

    #[test]
    fn minimap_mapping_roundtrips() {
        let b = (Vec2::new(-100.0, -50.0), Vec2::new(300.0, 150.0));
        assert_eq!(world_to_minimap(b, Vec2::new(-100.0, -50.0)), Vec2::ZERO);
        assert_eq!(world_to_minimap(b, Vec2::new(300.0, 150.0)), Vec2::ONE);
        let p = Vec2::new(40.0, 25.0);
        let n = world_to_minimap(b, p);
        assert!((minimap_to_world(b, n) - p).length() < 1e-4);
        assert_eq!(n, Vec2::new(0.35, 0.375));
    }

    #[test]
    fn fleet_slot_names() {
        assert_eq!(fleet_slot_name(1, 0, 0), "q1_0000");
        assert_eq!(fleet_slot_name(3, 4, 6), "q3_0406");
    }

    #[test]
    fn panel_slot_detection() {
        for n in [
            "q1_0203",
            "planet_structure_08",
            "slot_01",
            "pad_02",
            "r_0302",
            "hero_slot_good10",
            "land_fleet_19",
        ] {
            assert!(is_panel_slot(n), "{n}");
        }
        for n in ["q1_frame", "q1_fleet", "r_close", "b_zoom", "r_pop_text", "queue03"] {
            assert!(!is_panel_slot(n), "{n}");
        }
    }
}




