//! Space tactical interface: selected unit panel, ability buttons, multi-selection grid,
//! world-space overlays (selection brackets, health/shield bars, hardpoint markers),
//! reinforcement and retreat displays.
//!
//! The application describes the situation in [`TacticalData`]; the plugin turns it into the
//! command bar slots (`s_select_NN`, `s_health_NN`, `special_button_NN` ...) and into overlay nodes,
//! and answers with [`EawAbilityClicked`], [`EawSelectionClicked`] and [`EawHardpointClicked`].
//!
//! Ability buttons follow the engine convention found in the data: the ability `Type` of
//! `Unit_Abilities_Data` (`FULL_SALVO`) selects the icon `I_SA_FULL_SALVO.TGA` and the strings
//! `TEXT_TOOLTIP_ABILITY_FULL_SALVO_NAME` / `_DESCRIPTION`.

use std::collections::HashMap;

use alamo_data::{GameData, TextTable};
use bevy::prelude::*;
use bevy::ui::FocusPolicy;

use crate::data::{AbilityEntry, CommandBarData, EawButtonClicked, EawButtonState, SelectionEntry};
use crate::textures::UiTextures;
use crate::widgets::{ButtonLabel, EawButton};
use crate::{EawUiAssets, EawUiSettings};

/// Ability types that are AI behaviours or passives, never shown as buttons.
const HIDDEN_ABILITIES: [&str; 3] = ["HUNT", "SPOILER_LOCK", "NONE"];

/// An ability button of a unit type, resolved from the XML and the string table.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AbilityDef {
    /// `Type`, e.g. `FULL_SALVO`.
    pub kind: String,
    /// Icon texture name (`I_SA_<TYPE>.TGA`).
    pub icon: String,
    /// Localized name.
    pub name: String,
    /// Localized description.
    pub description: String,
    /// `Recharge_Seconds`.
    pub recharge_seconds: f32,
    /// `Expiration_Seconds` (active duration).
    pub duration_seconds: f32,
}

impl AbilityDef {
    /// Tooltip text: name, a blank line, description.
    pub fn tooltip(&self) -> String {
        if self.description.is_empty() {
            self.name.clone()
        } else {
            format!("{}\n{}", self.name, self.description)
        }
    }
}

/// Icon texture name of an ability type.
pub fn ability_icon(kind: &str) -> String {
    format!("I_SA_{}.TGA", kind.to_uppercase())
}

/// The ability buttons of a unit type (those with a known icon), in XML order.
pub fn unit_abilities(
    game: &GameData,
    text: &TextTable,
    textures: &UiTextures,
    unit: &str,
) -> Vec<AbilityDef> {
    let Some(def) = game.objects.space_unit(unit) else {
        return Vec::new();
    };
    let mut out: Vec<AbilityDef> = Vec::new();
    for a in &def.abilities {
        let kind = a.kind.trim().to_uppercase();
        if kind.is_empty()
            || HIDDEN_ABILITIES.contains(&kind.as_str())
            || out.iter().any(|o| o.kind == kind)
        {
            continue;
        }
        let icon = ability_icon(&kind);
        if !textures.library().in_mega_texture(&icon) {
            continue;
        }
        let key = |suffix: &str| format!("TEXT_TOOLTIP_ABILITY_{kind}_{suffix}");
        out.push(AbilityDef {
            name: text
                .get(&key("NAME"))
                .map_or_else(|| kind.replace('_', " "), str::to_string),
            description: text.get(&key("DESCRIPTION")).unwrap_or_default().to_string(),
            icon,
            recharge_seconds: a.recharge_seconds.unwrap_or(0.0),
            duration_seconds: a.expiration_seconds.unwrap_or(0.0),
            kind,
        });
    }
    out
}

/// State of one ability of a selected unit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AbilityState {
    /// The ability.
    pub def: AbilityDef,
    /// Remaining cooldown 0..1 (0 = ready).
    pub cooldown: f32,
    /// Currently active (toggled on / running).
    pub active: bool,
    /// Cannot be used right now.
    pub disabled: bool,
}

/// One selected unit (or a stack of identical units).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TacticalUnit {
    /// Unit type (object name).
    pub unit_type: String,
    /// Localized name.
    pub name: String,
    /// Icon texture name.
    pub icon: String,
    /// Hull 0..1.
    pub health: f32,
    /// Shield 0..1 (`None` = no shields).
    pub shield: Option<f32>,
    /// Hull points (for the numeric display), 0 = unknown.
    pub hull_points: f32,
    /// Shield points, 0 = unknown.
    pub shield_points: f32,
    /// Number of units of this type in the selection.
    pub count: u32,
    /// Ability buttons (shown for the focused unit).
    pub abilities: Vec<AbilityState>,
}

impl TacticalUnit {
    /// Builds a full-health unit from the game data: icon, localized name, hull/shield points and
    /// ability buttons.
    pub fn from_game(
        game: &GameData,
        text: &TextTable,
        textures: &UiTextures,
        unit_type: &str,
    ) -> Option<Self> {
        let def = game.objects.space_unit(unit_type)?;
        Some(Self {
            unit_type: def.name.clone(),
            name: def
                .text_id
                .as_deref()
                .and_then(|t| text.get(t))
                .unwrap_or(&def.name)
                .to_string(),
            icon: def.icon.clone().unwrap_or_default(),
            health: 1.0,
            shield: (def.shield_points > 0.0).then_some(1.0),
            hull_points: def.hull,
            shield_points: def.shield_points,
            count: 1,
            abilities: unit_abilities(game, text, textures, &def.name)
                .into_iter()
                .map(|def| AbilityState {
                    def,
                    ..default()
                })
                .collect(),
        })
    }
}

/// Retreat button state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum RetreatState {
    /// Button hidden / unavailable.
    #[default]
    Unavailable,
    /// Ready to be pressed.
    Ready,
    /// Countdown running, seconds left.
    Countdown(f32),
}

/// A selection bracket with bars above a ship, in screen space.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipOverlay {
    /// Stable id (entity bits of the ship); keeps the overlay node alive between frames.
    pub id: u64,
    /// Centre in logical window pixels (y down), before UI scaling.
    pub screen_pos: Vec2,
    /// Half the on-screen size of the ship in the same pixels (bracket radius).
    pub half_size: f32,
    /// Hull 0..1.
    pub health: f32,
    /// Shield 0..1.
    pub shield: Option<f32>,
    /// Selected (white brackets) ...
    pub selected: bool,
    /// ... or only hovered (dimmer brackets).
    pub hovered: bool,
    /// Enemy ships use red brackets.
    pub enemy: bool,
    /// Floating name label above the bars (the `Game_Object_Name` font of the game).
    pub name: Option<String>,
    /// Shown with bars only (no brackets) even when neither selected nor hovered: own units in the
    /// real game keep a health line under them.
    pub always: bool,
}

/// A hardpoint marker on a hovered capital ship.
#[derive(Debug, Clone, PartialEq)]
pub struct HardpointMarker {
    /// Hardpoint name (reported in [`EawHardpointClicked`]).
    pub id: String,
    /// Reticle kind: `GENERIC`, `ENGINES`, `SHIELD_GEN`, `DOCKING_BAY`, `GRAV_WELL`, `WEAPONS`,
    /// `WEAPONS_ION`, `WEAPONS_MISSILE`, `WEAPONS_TORPEDO`, `WEAPONS_TURBO`, `WEAPONS_MASS_DRIVER`.
    pub kind: String,
    /// Currently tracked / targeted by the player (`_TRACKED` reticle).
    pub tracked: bool,
    /// Centre in logical window pixels (y down), before UI scaling.
    pub screen_pos: Vec2,
    /// Health 0..1.
    pub health: f32,
    /// Can be targeted (shown bright and clickable).
    pub targetable: bool,
    /// Destroyed (shown dark).
    pub destroyed: bool,
}

/// Everything the space tactical interface shows (resource). Set `active` to let it drive the
/// command bar slots (`CommandBarData::selection` / `abilities` / `pop_*`).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct TacticalData {
    /// Drive the command bar from this data.
    pub active: bool,
    /// The selection, one entry per unit type.
    pub selection: Vec<TacticalUnit>,
    /// Index into `selection` whose abilities and details are shown.
    pub focus: usize,
    /// Reinforcement points available (`b_reinforcement` label).
    pub reinforcement_points: Option<u32>,
    /// Current population / cap (`Text_Planetary_Pop`).
    pub pop: Option<(u32, u32)>,
    /// Retreat button.
    pub retreat: RetreatState,
}

/// World-space overlays of the frame (own resource so that moving ships do not mark [`TacticalData`] changed).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct TacticalOverlays {
    /// Ship overlays for the frame.
    pub ships: Vec<ShipOverlay>,
    /// Hardpoint markers for the frame.
    pub hardpoints: Vec<HardpointMarker>,
}

/// An ability button was clicked.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct EawAbilityClicked {
    /// Unit type of the focused unit.
    pub unit_type: String,
    /// Ability `Type`, e.g. `FULL_SALVO`.
    pub ability: String,
}

/// A slot of the multi-selection grid was clicked (sub-select that unit type).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct EawSelectionClicked {
    /// Index into [`TacticalData::selection`].
    pub index: usize,
    /// Unit type.
    pub unit_type: String,
    /// Shift held (remove from / add to selection).
    pub shift: bool,
}

/// A hardpoint marker was clicked.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct EawHardpointClicked {
    /// Hardpoint name.
    pub id: String,
}

/// Copies [`TacticalData`] into [`CommandBarData`] slots.
pub(crate) fn tactical_sync(tac: Res<TacticalData>, mut bar: ResMut<CommandBarData>) {
    if !tac.is_changed() || !tac.active {
        return;
    }
    bar.selection = tac
        .selection
        .iter()
        .map(|u| SelectionEntry {
            icon: u.icon.clone(),
            health: u.health,
            shield: u.shield,
            count: u.count,
        })
        .collect();
    bar.abilities = tac
        .selection
        .get(tac.focus)
        .map(|u| {
            u.abilities
                .iter()
                .map(|a| AbilityEntry {
                    icon: a.def.icon.clone(),
                    ready: 1.0 - a.cooldown.clamp(0.0, 1.0),
                    active: a.active,
                    disabled: a.disabled,
                    kind: a.def.kind.clone(),
                    tooltip: a.def.tooltip(),
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some((cur, cap)) = tac.pop {
        bar.pop_current = cur;
        bar.pop_cap = cap;
    }
}

/// Reinforcement and retreat button states.
pub(crate) fn tactical_buttons(
    tac: Res<TacticalData>,
    mut buttons: Query<(&EawButton, &mut EawButtonState)>,
) {
    if !tac.is_changed() || !tac.active {
        return;
    }
    for (b, mut st) in &mut buttons {
        match b.name.as_str() {
            "b_retreat" => {
                st.disabled = tac.retreat == RetreatState::Unavailable;
                st.selected = matches!(tac.retreat, RetreatState::Countdown(_));
            }
            "b_reinforcement" => st.disabled = tac.reinforcement_points.is_none(),
            _ => {}
        }
    }
}
/// Writes the label texts of `b_retreat` / `b_reinforcement`.
pub(crate) fn button_labels(
    tac: Res<TacticalData>,
    buttons: Query<&EawButton>,
    mut labels: Query<(&ButtonLabel, &mut Text)>,
) {
    if !tac.is_changed() {
        return;
    }
    for (l, mut t) in &mut labels {
        let Some(b) = buttons.iter().find(|b| b.name == l.0) else {
            continue;
        };
        t.0 = match b.name.as_str() {
            "b_retreat" => match tac.retreat {
                RetreatState::Countdown(s) => format!("{}:{:02}", s as u32 / 60, s as u32 % 60),
                _ => String::new(),
            },
            "b_reinforcement" => tac.reinforcement_points.map(|p| p.to_string()).unwrap_or_default(),
            _ => String::new(),
        };
    }
}

/// Translates button clicks into ability / selection messages.
pub(crate) fn tactical_clicks(
    mut clicks: MessageReader<EawButtonClicked>,
    tac: Res<TacticalData>,
    bar: Res<CommandBarData>,
    mut abilities: MessageWriter<EawAbilityClicked>,
    mut selections: MessageWriter<EawSelectionClicked>,
) {
    for c in clicks.read() {
        if let Some(i) = c.name.strip_prefix("special_button_").and_then(|n| n.parse::<usize>().ok())
            && let Some(a) = bar.abilities.get(i)
            && !a.disabled
            && let Some(u) = tac.selection.get(tac.focus)
        {
            abilities.write(EawAbilityClicked {
                unit_type: u.unit_type.clone(),
                ability: a.kind.clone(),
            });
        }
        if let Some(i) = c.name.strip_prefix("s_select_").and_then(|n| n.parse::<usize>().ok())
            && let Some(u) = tac.selection.get(i)
        {
            selections.write(EawSelectionClicked {
                index: i,
                unit_type: u.unit_type.clone(),
                shift: c.shift,
            });
        }
    }
}

// ---------------------------------------------------------------------------------------------
// World-space overlays
// ---------------------------------------------------------------------------------------------

/// Full-screen layer that holds the overlays.
#[derive(Component)]
pub struct TacticalOverlayRoot;

/// One ship overlay node.
#[derive(Component)]
pub struct ShipOverlayNode {
    /// Ship id from [`ShipOverlay::id`].
    pub id: u64,
    health_img: Entity,
    shield_bg: Entity,
    shield_clip: Entity,
    shield_img: Entity,
    label: Entity,
    brackets: [Entity; 4],
    /// Bar texture set: 0 small, 1 medium, 2 large.
    class: usize,
}

/// One hardpoint marker node.
#[derive(Component)]
pub struct HardpointNode {
    id: String,
    fill: Entity,
}

/// Spawns the overlay layer (Startup).
pub(crate) fn spawn_overlay_layer(mut commands: Commands, settings: Res<EawUiSettings>) {
    let mut e = commands.spawn((
        TacticalOverlayRoot,
        Name::new("EaW tactical overlays"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        FocusPolicy::Pass,
        GlobalZIndex(50),
    ));
    if let Some(c) = settings.camera {
        e.insert(UiTargetCamera(c));
    }
}

fn bar_color(frac: f32) -> Color {
    if frac > 0.6 {
        Color::srgb(0.2, 0.85, 0.25)
    } else if frac > 0.3 {
        Color::srgb(0.95, 0.8, 0.15)
    } else {
        Color::srgb(0.9, 0.2, 0.15)
    }
}

fn px(v: f32, scale: f32) -> Val {
    Val::Px(v / scale.max(0.01))
}

/// Bar texture size class by on-screen ship size: the game has `small`, `medium` and `large`
/// (34, 68 and 102 px wide) `i_bar_health_*` / `i_bar_shield_*` sets.
pub fn bar_class(half_size: f32) -> usize {
    match half_size {
        h if h < 32.0 => 0,
        h if h < 70.0 => 1,
        _ => 2,
    }
}

const BAR_CLASSES: [&str; 3] = ["small", "medium", "large"];

/// Level textures of the original health/shield bars (internal cache entry).
#[derive(Clone)]
pub struct BarSet {
    health: Vec<Handle<Image>>,
    shield: Option<Handle<Image>>,
    frame: Option<Handle<Image>>,
}

fn bar_set(
    cache: &mut HashMap<usize, BarSet>,
    textures: &mut UiTextures,
    images: &mut Assets<Image>,
    class: usize,
) -> BarSet {
    cache
        .entry(class)
        .or_insert_with(|| {
            let name = BAR_CLASSES[class];
            let mut get = |n: String| textures.get(images, &n).map(|t| t.handle);
            BarSet {
                health: (0..=10)
                    .filter_map(|l| get(format!("i_bar_health_{name}{l}.tga")))
                    .collect(),
                shield: get(format!("i_bar_shield_{name}10.tga")),
                frame: get("i_bar_control_wide.tga".to_string()),
            }
        })
        .clone()
}

/// Creates, moves and removes ship overlay nodes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn ship_overlays(
    tac: Res<TacticalOverlays>,
    scale: Res<UiScale>,
    root: Query<Entity, With<TacticalOverlayRoot>>,
    mut existing: Local<HashMap<u64, Entity>>,
    mut sets: Local<HashMap<usize, BarSet>>,
    nodes: Query<&ShipOverlayNode>,
    mut textures: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    mut styles: Query<(&mut Node, Option<&mut ImageNode>, Option<&mut BorderColor>, Option<&mut Text>)>,
    settings: Res<EawUiSettings>,
) {
    let Ok(root) = root.single() else { return };
    let live: Vec<u64> = tac.ships.iter().map(|o| o.id).collect();
    existing.retain(|id, e| {
        let keep = live.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    let s = scale.0;
    for o in &tac.ships {
        let w = o.half_size * 2.0;
        let accent = match (o.enemy, o.selected) {
            (true, _) => Color::srgb(1.0, 0.25, 0.2),
            (false, true) => Color::srgb(0.3, 1.0, 0.4),
            (false, false) => Color::srgba(0.45, 0.9, 0.55, 0.8),
        };
        let Some(&e) = existing.get(&o.id) else {
            let class = bar_class(o.half_size);
            let set = bar_set(&mut sets, &mut textures, &mut images, class);
            let id = commands
                .spawn((
                    Name::new("ship overlay"),
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    FocusPolicy::Pass,
                    ChildOf(root),
                ))
                .id();
            let bar = |below: f32| Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                bottom: Val::Px(-below),
                height: Val::Px(2.0),
                width: Val::Percent(100.0),
                ..default()
            };
            let full = Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                height: Val::Percent(100.0),
                width: Val::Percent(100.0),
                ..default()
            };
            let img = |h: Option<Handle<Image>>| h.map_or_else(ImageNode::default, ImageNode::new);
            // Health bar: frame + discrete level texture.
            let health_bar = commands.spawn((bar(5.0), FocusPolicy::Pass, ChildOf(id))).id();
            commands.spawn((full.clone(), img(set.frame.clone()), ChildOf(health_bar)));
            let health_img = commands
                .spawn((full.clone(), img(set.health.last().cloned()), ChildOf(health_bar)))
                .id();
            // Shield bar: frame + the full shield texture clipped to the fraction.
            let shield_bg = commands.spawn((bar(8.0), FocusPolicy::Pass, ChildOf(id))).id();
            commands.spawn((full.clone(), img(set.frame.clone()), ChildOf(shield_bg)));
            let shield_clip = commands
                .spawn((
                    Node {
                        overflow: Overflow::clip(),
                        ..full.clone()
                    },
                    ChildOf(shield_bg),
                ))
                .id();
            let shield_img = commands
                .spawn((
                    Node {
                        width: Val::Px(w),
                        ..full.clone()
                    },
                    img(set.shield.clone()),
                    ChildOf(shield_clip),
                ))
                .id();
            // Floating name label (Game_Object_Name font of GameConstants).
            let label_holder = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(-24.0),
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    FocusPolicy::Pass,
                    ChildOf(id),
                ))
                .id();
            let font = match &settings.font_family {
                Some(f) => TextFont {
                    font: FontSource::Family(f.as_str().into()),
                    font_size: FontSize::Px(10.0),
                    ..default()
                },
                None => TextFont::from_font_size(10.0),
            };
            let label = commands
                .spawn((
                    Text::new(""),
                    font,
                    TextLayout::no_wrap(),
                    TextColor(accent),
                    TextShadow {
                        offset: Vec2::splat(1.0),
                        color: Color::BLACK,
                    },
                    FocusPolicy::Pass,
                    ChildOf(label_holder),
                ))
                .id();
            let corner = |right: bool, bottom: bool| Node {
                position_type: PositionType::Absolute,
                left: if right { Val::Auto } else { Val::Px(0.0) },
                right: if right { Val::Px(0.0) } else { Val::Auto },
                top: if bottom { Val::Auto } else { Val::Px(0.0) },
                bottom: if bottom { Val::Px(0.0) } else { Val::Auto },
                width: Val::Px(8.0),
                height: Val::Px(8.0),
                border: UiRect {
                    left: if right { Val::ZERO } else { Val::Px(2.0) },
                    right: if right { Val::Px(2.0) } else { Val::ZERO },
                    top: if bottom { Val::ZERO } else { Val::Px(2.0) },
                    bottom: if bottom { Val::Px(2.0) } else { Val::ZERO },
                },
                ..default()
            };
            let mut brackets = [Entity::PLACEHOLDER; 4];
            for (i, (r, b)) in [(false, false), (true, false), (false, true), (true, true)]
                .into_iter()
                .enumerate()
            {
                brackets[i] = commands
                    .spawn((corner(r, b), BorderColor::all(accent), FocusPolicy::Pass, ChildOf(id)))
                    .id();
            }
            commands.entity(id).insert(ShipOverlayNode {
                id: o.id,
                health_img,
                shield_bg,
                shield_clip,
                shield_img,
                label,
                brackets,
                class,
            });
            existing.insert(o.id, id);
            continue; // positioned next frame, once its parts exist
        };
        let Ok(parts) = nodes.get(e) else { continue };
        if let Ok((mut node, ..)) = styles.get_mut(e) {
            node.left = px(o.screen_pos.x - o.half_size, s);
            node.top = px(o.screen_pos.y - o.half_size, s);
            node.width = px(w, s);
            node.height = px(w, s);
            node.display = if o.selected || o.hovered || o.enemy || o.always {
                Display::Flex
            } else {
                Display::None
            };
        }
        let set = bar_set(&mut sets, &mut textures, &mut images, parts.class);
        if let Ok((_, Some(mut img), ..)) = styles.get_mut(parts.health_img) {
            let level = (o.health.clamp(0.0, 1.0) * 10.0).round() as usize;
            if let Some(h) = set.health.get(level).or_else(|| set.health.last()) {
                img.image = h.clone();
            }
        }
        if let Ok((mut n, ..)) = styles.get_mut(parts.shield_clip) {
            n.width = Val::Percent(o.shield.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0);
        }
        if let Ok((mut n, ..)) = styles.get_mut(parts.shield_img) {
            n.width = Val::Px(w / s.max(0.01));
        }
        if let Ok((mut n, ..)) = styles.get_mut(parts.shield_bg) {
            n.display = if o.shield.is_some() { Display::Flex } else { Display::None };
        }
        if let Ok((.., Some(mut t))) = styles.get_mut(parts.label) {
            let name = o.name.clone().unwrap_or_default();
            if t.0 != name {
                t.0 = name;
            }
        }
        commands.entity(parts.label).insert(TextColor(accent));
        let brackets = if o.selected || o.hovered || o.enemy { accent } else { Color::NONE };
        for b in parts.brackets {
            if let Ok((_, _, Some(mut bc), _)) = styles.get_mut(b) {
                *bc = BorderColor::all(brackets);
            }
        }
    }
}

/// Texture name of a hardpoint reticle: `I_HARD_POINT_RETICLE_<KIND>[_TRACKED].DDS`.
pub fn hardpoint_reticle(kind: &str, tracked: bool) -> String {
    let kind = if kind.is_empty() { "GENERIC" } else { kind };
    format!(
        "I_HARD_POINT_RETICLE_{}{}.DDS",
        kind.to_uppercase(),
        if tracked { "_TRACKED" } else { "" }
    )
}

/// Creates, moves and removes hardpoint markers; emits [`EawHardpointClicked`].
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn hardpoint_markers(
    tac: Res<TacticalOverlays>,
    scale: Res<UiScale>,
    root: Query<Entity, With<TacticalOverlayRoot>>,
    mut existing: Local<HashMap<String, Entity>>,
    clicks: Query<(&HardpointNode, &Interaction), Changed<Interaction>>,
    mut markers: Query<(&HardpointNode, &mut Node, &mut ImageNode)>,
    mut fills: Query<(&mut Node, &mut BackgroundColor), Without<HardpointNode>>,
    mut textures: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    mut out: MessageWriter<EawHardpointClicked>,
) {
    let Ok(root) = root.single() else { return };
    for (m, i) in &clicks {
        if *i == Interaction::Pressed {
            out.write(EawHardpointClicked { id: m.id.clone() });
        }
    }
    existing.retain(|id, e| {
        let keep = tac.hardpoints.iter().any(|h| &h.id == id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    let s = scale.0;
    const SIZE: f32 = 30.0;
    for h in &tac.hardpoints {
        let tint = if h.destroyed {
            Color::srgba(0.3, 0.3, 0.3, 0.55)
        } else if h.targetable {
            Color::WHITE
        } else {
            Color::srgba(0.6, 0.6, 0.65, 0.7)
        };
        let handle = textures
            .get(&mut images, &hardpoint_reticle(&h.kind, h.tracked))
            .or_else(|| textures.get(&mut images, &hardpoint_reticle("GENERIC", h.tracked)))
            .map(|t| t.handle);
        let image = || {
            handle
                .clone()
                .map_or_else(ImageNode::default, ImageNode::new)
                .with_color(tint)
        };
        match existing.get(&h.id) {
            None => {
                let fill = commands
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(3.0),
                            bottom: Val::Px(-3.0),
                            height: Val::Px(2.0),
                            width: Val::Percent(h.health.clamp(0.0, 1.0) * 84.0),
                            ..default()
                        },
                        BackgroundColor(bar_color(h.health)),
                        FocusPolicy::Pass,
                    ))
                    .id();
                let e = commands
                    .spawn((
                        Name::new(format!("hardpoint {}", h.id)),
                        HardpointNode {
                            id: h.id.clone(),
                            fill,
                        },
                        Button,
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(h.screen_pos.x - SIZE * 0.5, s),
                            top: px(h.screen_pos.y - SIZE * 0.5, s),
                            width: Val::Px(SIZE),
                            height: Val::Px(SIZE),
                            ..default()
                        },
                        image(),
                        ChildOf(root),
                    ))
                    .id();
                commands.entity(fill).insert(ChildOf(e));
                existing.insert(h.id.clone(), e);
            }
            Some(&e) => {
                if let Ok((m, mut n, mut img)) = markers.get_mut(e) {
                    n.left = px(h.screen_pos.x - SIZE * 0.5, s);
                    n.top = px(h.screen_pos.y - SIZE * 0.5, s);
                    *img = image();
                    if let Ok((mut f, mut bg)) = fills.get_mut(m.fill) {
                        f.width = Val::Percent(h.health.clamp(0.0, 1.0) * 84.0);
                        bg.0 = bar_color(h.health);
                    }
                }
            }
        }
    }
}
// ---------------------------------------------------------------------------------------------
// Selected unit panel
// ---------------------------------------------------------------------------------------------

/// Root of the selected unit panel.
#[derive(Component)]
pub struct UnitPanel;

#[derive(Component)]
pub struct UnitPanelPart(UnitPanelKind);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnitPanelKind {
    Icon,
    Name,
    HullFill,
    HullText,
    ShieldBg,
    ShieldFill,
    ShieldText,
}

/// Spawns the selected unit panel above the tactical bar (icon, name, hull and shield bars).
pub fn spawn_unit_panel(world: &mut World) -> Option<Entity> {
    let (font_family, camera) = {
        let s = world.get_resource::<EawUiSettings>()?;
        (s.font_family.clone(), s.camera)
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
            UnitPanel,
            Name::new("EaW selected unit panel"),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(380.0),
                bottom: Val::Px(196.0),
                width: Val::Px(250.0),
                padding: UiRect::all(Val::Px(6.0)),
                column_gap: Val::Px(8.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.04, 0.09, 0.82)),
            FocusPolicy::Pass,
        ))
        .id();
    if let Some(c) = camera {
        world.entity_mut(root).insert(UiTargetCamera(c));
    }
    world.spawn((
        UnitPanelPart(UnitPanelKind::Icon),
        ImageNode::default(),
        Node {
            width: Val::Px(46.0),
            height: Val::Px(46.0),
            flex_shrink: 0.0,
            ..default()
        },
        ChildOf(root),
    ));
    let col = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                flex_grow: 1.0,
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    world.spawn((
        UnitPanelPart(UnitPanelKind::Name),
        Text::new(""),
        font(13.0),
        TextColor(Color::srgb(0.85, 0.93, 1.0)),
        ChildOf(col),
    ));
    let bar = |world: &mut World, fill: UnitPanelKind, text: UnitPanelKind, color: Color| {
        let row = world
            .spawn((
                Node {
                    column_gap: Val::Px(5.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
                ChildOf(col),
            ))
            .id();
        let bg = world
            .spawn((
                Node {
                    width: Val::Px(120.0),
                    height: Val::Px(7.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
                BorderColor::all(Color::srgba(0.4, 0.55, 0.8, 0.7)),
                ChildOf(row),
            ))
            .id();
        if fill == UnitPanelKind::ShieldFill {
            world.entity_mut(bg).insert(UnitPanelPart(UnitPanelKind::ShieldBg));
        }
        world.spawn((
            UnitPanelPart(fill),
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(color),
            ChildOf(bg),
        ));
        world.spawn((
            UnitPanelPart(text),
            Text::new(""),
            font(10.0),
            TextColor(Color::srgb(0.8, 0.88, 1.0)),
            ChildOf(row),
        ));
    };
    bar(world, UnitPanelKind::HullFill, UnitPanelKind::HullText, Color::srgb(0.2, 0.85, 0.25));
    bar(world, UnitPanelKind::ShieldFill, UnitPanelKind::ShieldText, Color::srgb(0.25, 0.6, 1.0));
    Some(root)
}

/// Fills the unit panel from the focused unit.
#[allow(clippy::type_complexity)]
pub(crate) fn unit_panel(
    tac: Res<TacticalData>,
    mut textures: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut panel: Query<&mut Node, With<UnitPanel>>,
    mut parts: Query<
        (&UnitPanelPart, Option<&mut Text>, Option<&mut ImageNode>, &mut Node, Option<&mut BackgroundColor>),
        Without<UnitPanel>,
    >,
    assets: Res<EawUiAssets>,
) {
    let _ = &assets;
    if !tac.is_changed() {
        return;
    }
    let unit = tac.selection.get(tac.focus);
    for mut n in &mut panel {
        n.display = if unit.is_some() { Display::Flex } else { Display::None };
    }
    let Some(u) = unit else { return };
    for (p, text, img, mut node, bg) in &mut parts {
        match p.0 {
            UnitPanelKind::Icon => {
                if let (Some(mut img), Some(t)) = (img, textures.get(&mut images, &u.icon)) {
                    img.image = t.handle;
                }
            }
            UnitPanelKind::Name => {
                if let Some(mut t) = text {
                    t.0 = if u.count > 1 {
                        format!("{} x{}", u.name, u.count)
                    } else {
                        u.name.clone()
                    };
                }
            }
            UnitPanelKind::HullFill => {
                node.width = Val::Percent(u.health.clamp(0.0, 1.0) * 100.0);
                if let Some(mut bg) = bg {
                    bg.0 = bar_color(u.health);
                }
            }
            UnitPanelKind::HullText => {
                if let Some(mut t) = text {
                    t.0 = if u.hull_points > 0.0 {
                        format!("{:.0}/{:.0}", u.hull_points * u.health, u.hull_points)
                    } else {
                        format!("{:.0}%", u.health * 100.0)
                    };
                }
            }
            UnitPanelKind::ShieldBg => {
                node.display = if u.shield.is_some() { Display::Flex } else { Display::None };
            }
            UnitPanelKind::ShieldFill => {
                node.width = Val::Percent(u.shield.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0);
            }
            UnitPanelKind::ShieldText => {
                if let Some(mut t) = text {
                    t.0 = match u.shield {
                        Some(s) if u.shield_points > 0.0 => {
                            format!("{:.0}/{:.0}", u.shield_points * s, u.shield_points)
                        }
                        Some(s) => format!("{:.0}%", s * 100.0),
                        None => String::new(),
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_copies_selection_and_abilities() {
        let mut app = App::new();
        app.init_resource::<CommandBarData>()
            .insert_resource(TacticalData {
                active: true,
                selection: vec![TacticalUnit {
                    unit_type: "Star_Destroyer".into(),
                    icon: "I_X.TGA".into(),
                    health: 0.5,
                    shield: Some(0.25),
                    count: 2,
                    abilities: vec![AbilityState {
                        def: AbilityDef {
                            kind: "FULL_SALVO".into(),
                            icon: ability_icon("FULL_SALVO"),
                            name: "Full Salvo".into(),
                            ..default()
                        },
                        cooldown: 0.25,
                        active: true,
                        disabled: false,
                    }],
                    ..default()
                }],
                pop: Some((3, 9)),
                ..default()
            })
            .add_systems(Update, tactical_sync);
        app.update();
        let bar = app.world().resource::<CommandBarData>();
        assert_eq!(bar.selection.len(), 1);
        assert_eq!(bar.selection[0].shield, Some(0.25));
        assert_eq!(bar.abilities[0].kind, "FULL_SALVO");
        assert!((bar.abilities[0].ready - 0.75).abs() < 1e-6);
        assert!(bar.abilities[0].active);
        assert_eq!((bar.pop_current, bar.pop_cap), (3, 9));
    }

    #[test]
    fn inactive_tactical_data_leaves_the_bar_alone() {
        let mut app = App::new();
        app.insert_resource(CommandBarData {
            credits: 7,
            ..default()
        })
        .init_resource::<TacticalData>()
        .add_systems(Update, tactical_sync);
        app.update();
        assert_eq!(app.world().resource::<CommandBarData>().credits, 7);
    }

    #[test]
    fn reticle_names_and_bar_classes() {
        assert_eq!(hardpoint_reticle("weapons_ion", true), "I_HARD_POINT_RETICLE_WEAPONS_ION_TRACKED.DDS");
        assert_eq!(hardpoint_reticle("", false), "I_HARD_POINT_RETICLE_GENERIC.DDS");
        assert_eq!((bar_class(10.0), bar_class(40.0), bar_class(120.0)), (0, 1, 2));
    }

    #[test]
    fn ability_icon_names() {
        assert_eq!(ability_icon("full_salvo"), "I_SA_FULL_SALVO.TGA");
    }

    #[test]
    fn tooltip_joins_name_and_description() {
        let a = AbilityDef {
            name: "Tractor Beam".into(),
            description: "Slows a ship.".into(),
            ..default()
        };
        assert_eq!(a.tooltip(), "Tractor Beam\nSlows a ship.");
        let b = AbilityDef {
            name: "X".into(),
            ..default()
        };
        assert_eq!(b.tooltip(), "X");
    }

    #[test]
    fn health_colors_change_with_level() {
        assert_ne!(bar_color(0.9), bar_color(0.5));
        assert_ne!(bar_color(0.5), bar_color(0.1));
    }
}






