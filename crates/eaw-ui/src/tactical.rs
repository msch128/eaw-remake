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

/// Ability types that never get a button (`HUNT` "Hunt for Enemies" and `SPOILER_LOCK` "Lock S-Foils" are toggles the player uses).
const HIDDEN_ABILITIES: [&str; 1] = ["NONE"];

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
    // Types without an `I_SA_<TYPE>` texture: the engine picks the icon by its ability enum, which the data does not show (estimate:
    // the icon whose name or tooltip fits; units that name their own `Alternate_Icon_Name` override this).
    let fallback = match kind.to_ascii_uppercase().as_str() {
        "SPOILER_LOCK" => Some("I_SA_S_FOIL_MODE.TGA"),
        "TURBO" => Some("I_SA_POWER_TO_ENGINES.TGA"),
        "DEFEND" => Some("I_SA_SHIELD_FLARE.TGA"),
        "FOW_REVEAL_PING" => Some("I_SA_SENSOR_PING.TGA"),
        "MISSILE_SHIELD" => Some("I_SA_MISSILE_JAMMER.TGA"),
        "SUPER_LASER" | "ENERGY_WEAPON" => Some("I_SA_BEAM.TGA"),
        "CONCENTRATE_FIRE" => Some("I_SA_INTENSIFY_FIREPOWER.TGA"),
        "REPLENISH_WINGMEN" => Some("I_SA_COVER_ME.TGA"),
        _ => None,
    };
    if let Some(icon) = fallback {
        return icon.to_string();
    }
    format!("I_SA_{}.TGA", kind.to_uppercase())
}

/// Sort key of an ability's `Activation_Style`: 0 = the player triggers it, 1 = everything else (automatic effects).
fn activation_priority(style: &str) -> u8 {
    u8::from(!style.trim().eq_ignore_ascii_case("User_Input"))
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
    let mut priority: Vec<u8> = Vec::new();
    for a in &def.abilities {
        let kind = a.kind.trim().to_uppercase();
        if kind.is_empty()
            || HIDDEN_ABILITIES.contains(&kind.as_str())
            || out.iter().any(|o| o.kind == kind)
        {
            continue;
        }
        let icon = a.fields.get("alternate_icon_name")
            .or_else(|| a.fields.get("icon_name"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ability_icon(&kind));
        if !textures.library().in_mega_texture(&icon) && textures.library().load(&icon).is_none() {
            continue;
        }
        // A unit may name its own texts for an ability (`Alternate_Name_Text` / `Alternate_Description_Text`: the Subjugator's
        // ROCKET_ATTACK is its flak cannon); otherwise the ability type's texts apply.
        let own = |field: &str, suffix: &str| {
            a.fields
                .get(field)
                .and_then(|k| text.get(k.trim()))
                .or_else(|| text.get(&format!("TEXT_TOOLTIP_ABILITY_{kind}_{suffix}")))
        };
        priority.push(activation_priority(a.fields.get("activation_style").map_or("", String::as_str)));
        out.push(AbilityDef {
            name: own("alternate_name_text", "NAME").map_or_else(|| kind.replace('_', " "), str::to_string),
            description: own("alternate_description_text", "DESCRIPTION").unwrap_or_default().to_string(),
            icon,
            recharge_seconds: a.recharge_seconds.unwrap_or(0.0),
            duration_seconds: a.expiration_seconds.unwrap_or(0.0),
            kind,
        });
    }
    // The bar has two buttons per card: abilities the player triggers come before automatic ones (stable, so XML order holds inside a group).
    let mut keyed: Vec<(u8, AbilityDef)> = priority.into_iter().zip(out).collect();
    keyed.sort_by_key(|(p, _)| *p);
    keyed.into_iter().map(|(_, d)| d).collect()
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

/// Health of one unit of a selected type: one ship, or one squadron.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UnitStatus {
    /// Hull 0..1.
    pub health: f32,
    /// Shield 0..1 (`None` = no shields).
    pub shield: Option<f32>,
}

/// One selected unit type with its units.
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
    /// Bar section (see [`SelectionEntry::section`]).
    pub section: u8,
    /// Ability buttons (shown for the focused unit).
    pub abilities: Vec<AbilityState>,
    /// Each unit of the type (a ship, or a squadron) with its own health; the bar gives each its own card while they fit. Empty:
    /// `count` units at `health` / `shield`.
    pub units: Vec<UnitStatus>,
}

impl TacticalUnit {
    /// The units of the type: [`Self::units`], or `count` units at the type's health.
    fn unit_list(&self) -> Vec<UnitStatus> {
        if self.units.is_empty() {
            vec![UnitStatus { health: self.health, shield: self.shield }; self.count.max(1) as usize]
        } else {
            self.units.clone()
        }
    }
}

/// Columns of the selection bar; each holds two unit cards (`s_select_{2c}` on top, `s_select_{2c+1}` below).
pub const SELECT_COLUMNS: usize = 12;
/// Unit cards of the selection bar.
pub const SELECT_SLOTS: usize = SELECT_COLUMNS * 2;

/// What one card of the selection bar stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotUnits {
    /// Selection types (indices into [`TacticalData::selection`], ascending): one, or the rest of a selection with more types than cards.
    pub cards: Vec<usize>,
    /// Units of `cards[0]` the card shows (indices into its unit list); all units of every type for a card of several types.
    pub units: std::ops::Range<usize>,
}

impl SlotUnits {
    /// The first type the card stands for (its icon and bar section).
    pub fn first(&self) -> usize {
        self.cards[0]
    }
}

/// Start slot of every type when type `i` takes `k[i]` cards, or `None` when they need more than [`SELECT_SLOTS`]. `align`: every type
/// starts a column; `breaks`: every bar section starts a column (its own frame).
fn place_types(k: &[usize], sections: &[u8], align: bool, breaks: bool) -> Option<Vec<usize>> {
    let mut starts = Vec::with_capacity(k.len());
    let mut slot = 0usize;
    for (i, &n) in k.iter().enumerate() {
        if i > 0 && (align || (breaks && sections[i] != sections[i - 1])) {
            slot = slot.next_multiple_of(2);
        }
        starts.push(slot);
        slot += n;
    }
    (slot <= SELECT_SLOTS).then_some(starts)
}

/// The cards of the selection bar (one entry per `s_select_NN`, `None` = empty), the Remake's two rows (video v0700s, n93 f0011): every
/// unit (ship or squadron) has its own card with its own bars, filling a column top then bottom; the types of one bar section share a
/// frame. When the units need more than the 24 cards (estimate, the data has no paging for this bar: `I_Tactical_Controls_New.ALO` has
/// 24 `s_select_NN` and no scroll button), the largest groups are stacked: one card stands for several units of a type and shows their
/// number (`s_select_NN` is a `TextButton`). If the types still do not fit, a type no longer starts its own column, then the sections no
/// longer start their own; a selection of more than 24 types shows 23 and a last card for the rest (a click on it selects those).
pub fn selection_layout(tac: &TacticalData) -> Vec<Option<SlotUnits>> {
    let n_units: Vec<usize> = tac.selection.iter().map(|u| u.unit_list().len()).collect();
    let sections: Vec<u8> = tac.selection.iter().map(|u| u.section).collect();
    let mut out = vec![None; SELECT_SLOTS];
    let fill = |out: &mut Vec<Option<SlotUnits>>, k: &[usize], starts: &[usize]| {
        for (i, (&n, &start)) in k.iter().zip(starts).enumerate() {
            let total = n_units[i];
            for j in 0..n {
                out[start + j] = Some(SlotUnits { cards: vec![i], units: j * total / n..(j + 1) * total / n });
            }
        }
    };
    for (align, breaks) in [(true, true), (false, true), (false, false)] {
        let mut k = n_units.clone();
        loop {
            if let Some(starts) = place_types(&k, &sections, align, breaks) {
                fill(&mut out, &k, &starts);
                return out;
            }
            // Stack the largest group one card further (the later type on a tie, so the first types keep their cards longest).
            let Some(i) = (0..k.len()).filter(|&i| k[i] > 1).max_by_key(|&i| (k[i], i)) else { break };
            k[i] -= 1;
        }
    }
    // More types than cards (estimate, the data has no rule for it): the heroes keep a card of their own first, then the capital ships, the
    // small ships and the fighters; 23 types get one card each (their units stacked) and the last card stands for the other types (the
    // first one's icon, the number of all their units; a click selects them all). The cards stay in bar order, the rest card where its
    // first type would be.
    let n = tac.selection.len();
    let rank = |s: u8| match s {
        2 => 0,
        3 => 1,
        0 => 2,
        _ => 3,
    };
    let mut by_rank: Vec<usize> = (0..n).collect();
    by_rank.sort_by_key(|&i| (rank(sections[i]), i));
    let mut rest = by_rank.split_off(SELECT_SLOTS - 1);
    rest.sort_unstable();
    let mut cards: Vec<Vec<usize>> = by_rank.into_iter().map(|i| vec![i]).collect();
    cards.push(rest);
    cards.sort_by_key(|c| c[0]);
    for (entry, c) in out.iter_mut().zip(cards) {
        *entry = Some(SlotUnits { units: 0..n_units[c[0]], cards: c });
    }
    out
}

/// Bar columns (first, number) the cards of every selection type take in `layout`.
fn card_columns(layout: &[Option<SlotUnits>], cards: usize) -> Vec<Option<(usize, usize)>> {
    let mut cols: Vec<Option<(usize, usize)>> = vec![None; cards];
    for (slot, s) in layout.iter().enumerate() {
        let Some(s) = s.as_ref().filter(|s| s.cards.len() == 1) else { continue };
        let c = slot / 2;
        let e = &mut cols[s.first()];
        *e = Some(match *e {
            Some((first, _)) => (first, c - first + 1),
            None => (c, 1),
        });
    }
    cols
}

/// Bar section of a unit type: 0 small ships, 1 fighters and bombers, 2 heroes, 3 capital ships.
pub fn bar_section(def: &alamo_data::SpaceUnitDef) -> u8 {
    use alamo_data::ShipCategory as C;
    if def.kind.eq_ignore_ascii_case("UniqueUnit") {
        2
    } else if matches!(def.category, C::Fighter | C::Bomber) {
        1
    } else if def.category >= C::Capital {
        3
    } else {
        0
    }
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
        let Some(def) = game.objects.space_unit(unit_type) else { return Self::structure_from_game(game, text, textures, unit_type) };
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
            section: bar_section(&def),
            abilities: unit_abilities(game, text, textures, &def.name)
                .into_iter()
                .map(|def| AbilityState {
                    def,
                    ..default()
                })
                .collect(),
            units: Vec::new(),
        })
    }

    /// The card of a star base or another structure of a space battle (`StarBase`, `SpaceStructure`, ...: no space unit kind, so
    /// [`Self::from_game`] found none and selecting the own station left the bar empty). Name, icon, hull and shields come from the
    /// object's own `Text_ID`, `Icon_Name`, `Tactical_Health` and `Shield_Points`; it sits in the capital ships' section.
    fn structure_from_game(game: &GameData, text: &TextTable, textures: &UiTextures, unit_type: &str) -> Option<Self> {
        if !game.objects.is_structure(unit_type) {
            return None;
        }
        let r = game.objects.resolve(unit_type)?;
        let number = |tag: &str| r.get(tag).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let (hull, shield) = match game.objects.star_base(unit_type) {
            Some(b) => (b.hull, b.shield_points),
            None => (number("tactical_health"), number("shield_points")),
        };
        let name = r.get("text_id").and_then(|t| t.split_whitespace().next()).and_then(|t| text.get(t)).unwrap_or(unit_type).to_string();
        Some(Self {
            unit_type: unit_type.to_string(),
            name,
            icon: crate::unit_icon(game, unit_type).unwrap_or_default(),
            health: 1.0,
            shield: (shield > 0.0).then_some(1.0),
            hull_points: hull,
            shield_points: shield,
            count: 1,
            section: 3,
            abilities: unit_abilities(game, text, textures, unit_type).into_iter().map(|def| AbilityState { def, ..default() }).collect(),
            units: Vec::new(),
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
    /// Texture name of the unit icon: small units (fighters) are marked by a little icon box instead of brackets,
    /// like in the real game.
    pub icon: Option<String>,
    /// Nobody selected or hovers the unit: only the icon box is drawn (no brackets, bars or flag).
    pub icon_only: bool,
    /// The box of a picked squadron: a white frame with round corners and the unit's icon art ([`Self::icon`]) on no plate,
    /// [`SQUAD_BOX_DESIGN_PX`] across (Remake video v0700s: the same three-quarter art as the unit cards in every box, whatever way the
    /// craft flies). Before, the box held no art and showed the craft through it: a few pixels at the far zoom, a TIE seen from above
    /// next to nothing, so a dozen empty frames lay over a fight.
    pub squad_box: bool,
}

/// Size of a picked squadron's box in design pixels: the Remake's measures 28 x 27 px at 1080p (video v0700s), 19.9 design px.
pub const SQUAD_BOX_DESIGN_PX: f32 = 19.9;

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
    /// Tooltip of the Retreat button (`TooltipLayout` text): what it does and, while it is unavailable, why and until when.
    pub retreat_tip: Option<String>,
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

/// A unit card of the selection bar was clicked (sub-select the units it stands for).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct EawSelectionClicked {
    /// Index into [`TacticalData::selection`] (the first type of the card).
    pub index: usize,
    /// Unit type.
    pub unit_type: String,
    /// Shift held (remove from / add to selection).
    pub shift: bool,
    /// The types the card stands for (see [`SlotUnits::cards`]).
    pub cards: Vec<usize>,
    /// The units of `index` the card stands for (see [`SlotUnits::units`]); all units of every type when `cards` holds several.
    pub units: std::ops::Range<usize>,
}

/// A hardpoint marker was clicked.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct EawHardpointClicked {
    /// Hardpoint name.
    pub id: String,
}

/// Number of ability buttons of the tactical bar (`special_button_00..23`).
const ABILITY_BUTTONS: usize = 24;

/// The ability buttons (`special_button_NN`) for the cards of `tac`, in card order; each entry knows its card and its place among the
/// abilities that card shows, and `widgets::ability_columns` puts it above that card. Every card keeps its first two abilities (the bar
/// has two buttons per card); further abilities take the buttons other cards leave free. Before, buttons `2n` and `2n + 1` belonged to
/// card `n`: a third ability never showed, and the buttons of neighbouring cards ran together into one evenly spaced row.
pub fn ability_slots(tac: &TacticalData) -> Vec<AbilityEntry> {
    ability_slots_for(tac, &selection_layout(tac))
}

/// [`ability_slots`] for the cards `layout` gives the selection: the buttons of a type sit above the columns of its cards.
fn ability_slots_for(tac: &TacticalData, layout: &[Option<SlotUnits>]) -> Vec<AbilityEntry> {
    let cols = card_columns(layout, tac.selection.len());
    let cards: Vec<(usize, &TacticalUnit, (usize, usize))> = tac.selection.iter().enumerate().filter_map(|(i, u)| cols[i].map(|c| (i, u, c))).collect();
    let mut spare = ABILITY_BUTTONS;
    let mut shown: Vec<usize> = cards
        .iter()
        .map(|(_, u, _)| {
            let n = u.abilities.len().min(2).min(spare);
            spare -= n;
            n
        })
        .collect();
    for (n, (_, u, _)) in shown.iter_mut().zip(&cards) {
        let more = (u.abilities.len() - *n).min(spare);
        *n += more;
        spare -= more;
    }
    // Types whose cards share a column (one on the top card, one on the bottom card) have one group of buttons over their joint columns.
    let mut groups: Vec<(usize, usize, usize)> = Vec::new(); // (first column, end column, buttons)
    let mut group_of = Vec::with_capacity(cards.len());
    for (&(_, _, (column, span)), &n) in cards.iter().zip(&shown) {
        let end = column + span.max(1);
        match groups.last_mut() {
            Some(g) if column < g.1 => {
                g.1 = g.1.max(end);
                g.2 += n;
            }
            _ => groups.push((column, end, n)),
        }
        group_of.push(groups.len() - 1);
    }
    let mut placed = vec![0usize; groups.len()];
    let mut abilities = vec![AbilityEntry::default(); ABILITY_BUTTONS];
    let mut slots = abilities.iter_mut();
    for ((&(card, u, _), &of), &g) in cards.iter().zip(&shown).zip(&group_of) {
        let (column, end, among) = groups[g];
        let span = end - column;
        for (nth, a) in u.abilities.iter().take(of).enumerate() {
            let Some(slot) = slots.next() else { break };
            let at = placed[g];
            placed[g] += 1;
            *slot = AbilityEntry {
                icon: a.def.icon.clone(),
                ready: 1.0 - a.cooldown.clamp(0.0, 1.0),
                active: a.active,
                disabled: a.disabled,
                kind: a.def.kind.clone(),
                tooltip: a.def.tooltip(),
                card,
                column,
                span,
                nth,
                of,
                at,
                among,
            };
        }
    }
    abilities
}

/// Copies [`TacticalData`] into [`CommandBarData`] slots.
pub(crate) fn tactical_sync(tac: Res<TacticalData>, mut bar: ResMut<CommandBarData>) {
    if !tac.is_changed() || !tac.active {
        return;
    }
    // Unit cards: every unit its own while they fit, column by column, top then bottom (`selection_layout`).
    let layout = selection_layout(&tac);
    let lists: Vec<Vec<UnitStatus>> = tac.selection.iter().map(TacticalUnit::unit_list).collect();
    bar.selection = layout
        .iter()
        .map(|s| {
            let Some(s) = s else { return SelectionEntry::default() };
            let u = &tac.selection[s.first()];
            let units: Vec<UnitStatus> = if s.cards.len() == 1 {
                lists[s.first()][s.units.clone()].to_vec()
            } else {
                s.cards.iter().flat_map(|&c| lists[c].iter().copied()).collect()
            };
            let n = units.len().max(1) as f32;
            let shielded: Vec<f32> = units.iter().filter_map(|x| x.shield).collect();
            SelectionEntry {
                icon: u.icon.clone(),
                health: units.iter().map(|x| x.health).sum::<f32>() / n,
                shield: (!shielded.is_empty()).then(|| shielded.iter().sum::<f32>() / shielded.len() as f32),
                count: units.len() as u32,
                section: u.section,
                selected: s.cards.len() == 1 && s.first() == tac.focus,
            }
        })
        .collect();

    bar.abilities = ability_slots_for(&tac, &layout);
    bar.ability_column = 0;
    if let Some((cur, cap)) = tac.pop {
        bar.pop_current = cur;
        bar.pop_cap = cap;
    }
}

/// Reinforcement and retreat button states.
pub(crate) fn tactical_buttons(
    tac: Res<TacticalData>,
    mut buttons: Query<(&mut EawButton, &mut EawButtonState)>,
) {
    if !tac.is_changed() || !tac.active {
        return;
    }
    for (mut b, mut st) in &mut buttons {
        let name = b.name.clone();
        match name.as_str() {
            "b_retreat" => {
                st.disabled = tac.retreat == RetreatState::Unavailable;
                st.selected = matches!(tac.retreat, RetreatState::Countdown(_));
                if b.tooltip_literal != tac.retreat_tip {
                    b.tooltip_literal = tac.retreat_tip.clone();
                }
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
            && let Some(a) = bar.abilities.get(i).filter(|a| !a.icon.is_empty())
            && !a.disabled
            && let Some(u) = tac.selection.get(a.card)
        {
            abilities.write(EawAbilityClicked {
                unit_type: u.unit_type.clone(),
                ability: a.kind.clone(),
            });
        }
        if let Some(i) = c.name.strip_prefix("s_select_").and_then(|n| n.parse::<usize>().ok())
            && let Some(Some(s)) = selection_layout(&tac).get(i)
            && let Some(u) = tac.selection.get(s.first())
        {
            selections.write(EawSelectionClicked {
                index: s.first(),
                unit_type: u.unit_type.clone(),
                shift: c.shift,
                cards: s.cards.clone(),
                units: s.units.clone(),
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

/// Draw order of the world overlays (brackets, bars, icon boxes, names, hardpoint reticles; the hero boxes of the host use it too): below
/// every interface panel (the command bar, the minimap, the objectives and the hero portraits have none or a positive one) and above the
/// squadron icons. They belong to the world, which lies under the interface: before, the layer had 50 and its boxes and bars were drawn
/// over the command bar, the minimap and the unit panel.
pub const WORLD_OVERLAY_Z: i32 = -5;

/// One ship overlay node.
#[derive(Component)]
pub struct ShipOverlayNode {
    /// Ship id from [`ShipOverlay::id`].
    pub id: u64,
    health_img: Entity,
    health_bar: Entity,
    shield_bg: Entity,
    shield_clip: Entity,
    shield_img: Entity,
    label: Entity,
    flag: Entity,
    icon: Entity,
    brackets: [Entity; 4],
    /// Bar texture set: 0 small, 1 medium, 2 large.
    class: usize,
}

/// One hardpoint marker node.
#[derive(Component)]
pub struct HardpointNode {
    id: String,
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
        GlobalZIndex(WORLD_OVERLAY_Z),
    ));
    if let Some(c) = settings.camera {
        e.insert(UiTargetCamera(c));
    }
}

/// Health colour of a bar: green, yellow below 60 %, red below 30 %.
pub fn bar_color(frac: f32) -> Color {
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
    mut plates: Query<&mut BackgroundColor>,
    settings: Res<EawUiSettings>,
) {
    let Ok(root) = root.single() else { return };
    let live: Vec<u64> = tac.ships.iter().map(|o| o.id).collect();
    existing.retain(|id, e| {
        let keep = live.contains(id);
        if !keep {
            commands.entity(*e).try_despawn();
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
            let health_bar = commands.spawn((bar(8.0), FocusPolicy::Pass, ChildOf(id))).id();
            commands.spawn((full.clone(), img(set.frame.clone()), ChildOf(health_bar)));
            let health_img = commands
                .spawn((full.clone(), img(set.health.last().cloned()), ChildOf(health_bar)))
                .id();
            // Shield bar: frame + the full shield texture clipped to the fraction.
            let shield_bg = commands.spawn((bar(4.0), FocusPolicy::Pass, ChildOf(id))).id();
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
                width: Val::Px(7.0),
                height: Val::Px(7.0),
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
                    .spawn((corner(r, b), BorderColor::all(Color::NONE), FocusPolicy::Pass, ChildOf(id)))
                    .id();
            }
            let icon = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        border: UiRect::all(Val::Px(1.0)),
                        display: Display::None,
                        ..default()
                    },
                    ImageNode::default(),
                    BorderColor::all(accent),
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                    FocusPolicy::Pass,
                    ChildOf(id),
                ))
                .id();
            // Squadron flag: a small white pennant at the right end of the bars of an own squadron (Remake video b0700-b0760).
            let flag = commands
                .spawn((
                    Node { position_type: PositionType::Absolute, left: Val::Percent(100.0), margin: UiRect::left(Val::Px(3.0)), bottom: Val::Px(-12.0), width: Val::Px(8.0), height: Val::Px(10.0), display: Display::None, ..default() },
                    FocusPolicy::Pass,
                    ChildOf(id),
                ))
                .id();
            let white = Color::srgba(0.92, 0.94, 0.96, 0.9);
            commands.spawn((Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Px(1.5), height: Val::Px(10.0), ..default() }, BackgroundColor(white), FocusPolicy::Pass, ChildOf(flag)));
            commands.spawn((Node { position_type: PositionType::Absolute, left: Val::Px(1.5), top: Val::Px(0.0), width: Val::Px(6.0), height: Val::Px(5.0), ..default() }, BackgroundColor(white), FocusPolicy::Pass, ChildOf(flag)));
            commands.entity(id).insert(ShipOverlayNode {
                id: o.id,
                flag,
                health_img,
                health_bar,
                shield_bg,
                shield_clip,
                shield_img,
                label,
                icon,
                brackets,
                class,
            });
            existing.insert(o.id, id);
            continue; // positioned next frame, once its parts exist
        };
        let Ok(parts) = nodes.get(e) else { continue };
        // Small units get an icon box (24 design px) instead of brackets; big ones keep brackets and carry the bars above. An icon whose
        // art does not resolve leaves no empty frame: the box is not drawn (nor the whole overlay, when the box is all it shows).
        let icon_mode = o.icon.is_some() || o.squad_box;
        let art = if icon_mode { o.icon.as_deref().and_then(|name| textures.get(&mut images, name)) } else { None };
        let boxed = art.is_some();
        let (half, w) = if o.squad_box {
            (SQUAD_BOX_DESIGN_PX * 0.5 * s, SQUAD_BOX_DESIGN_PX * s)
        } else if icon_mode {
            (12.0 * s, 24.0 * s)
        } else {
            (o.half_size, w)
        };
        if let Ok((mut node, ..)) = styles.get_mut(e) {
            node.left = px(o.screen_pos.x - half, s);
            node.top = px(o.screen_pos.y - half, s);
            node.width = px(w, s);
            node.height = px(w, s);
            let only_box = o.icon_only || o.squad_box;
            node.display = if (o.selected || o.hovered || o.enemy || o.always || o.icon_only) && (boxed || !only_box) {
                Display::Flex
            } else {
                Display::None
            };
        }
        if let Ok((mut n, ..)) = styles.get_mut(parts.flag) {
            n.display = if o.always && !o.enemy && o.half_size < 24.0 && !o.icon_only { Display::Flex } else { Display::None };
        }
        let above = !icon_mode && o.half_size >= 40.0;
        if let Ok((mut n, ..)) = styles.get_mut(parts.health_bar) {
            n.display = if o.icon_only { Display::None } else { Display::Flex };
        }
        for (bar, below, over) in [(parts.health_bar, 8.0, 4.0), (parts.shield_bg, 4.0, 8.0)] {
            if let Ok((mut n, ..)) = styles.get_mut(bar) {
                if above {
                    n.bottom = Val::Auto;
                    n.top = Val::Px(-over);
                } else {
                    n.top = Val::Auto;
                    n.bottom = Val::Px(-below);
                }
            }
        }
        if let Ok((mut n, img, bc, _)) = styles.get_mut(parts.icon) {
            n.display = if boxed { Display::Flex } else { Display::None };
            n.border_radius = if o.squad_box { BorderRadius::all(Val::Px(4.0)) } else { BorderRadius::ZERO };
            if let Some(mut img) = img
                && let Some(t) = art
            {
                img.image = t.handle;
                img.color = Color::WHITE;
            }
            if let Some(mut bc) = bc {
                *bc = BorderColor::all(if o.enemy { accent } else { Color::WHITE });
            }
        }
        if let Ok(mut plate) = plates.get_mut(parts.icon) {
            plate.0 = if o.squad_box { Color::NONE } else { Color::srgba(0.0, 0.0, 0.0, 0.45) };
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
            n.display = if o.shield.is_some() && !o.icon_only { Display::Flex } else { Display::None };
        }
        if let Ok((.., Some(mut t))) = styles.get_mut(parts.label) {
            let name = o.name.clone().unwrap_or_default();
            if t.0 != name {
                t.0 = name;
            }
        }
        let brackets = if !icon_mode && (o.selected || o.hovered) && o.half_size >= 16.0 { accent } else { Color::NONE };
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

/// `HardPoint_Target_Reticle_Enemy_Screen_Size` of the Remake (`Gameconstants.xml`, the friendly one is the same): a reticle is this share
/// of the screen height (23.8 px at 1080p; the Remake video measures 20-24 px, b0853s). Used when the data has no value.
pub const RETICLE_SCREEN_SIZE: f32 = 0.022;

/// Reticle size in design pixels (1/768 of the screen height, the unit [`UiScale`] scales) for the data's screen share, if any.
pub fn reticle_design_px(game: Option<&GameData>) -> f32 {
    let share = game
        .and_then(|g| g.game_constants.get("hardpoint_target_reticle_enemy_screen_size"))
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v > 0.0 && *v < 1.0)
        .unwrap_or(RETICLE_SCREEN_SIZE);
    share * 768.0
}

/// Colour a reticle is drawn in: the white art tinted by the hardpoint's health like a bar (green in the Remake at full health, v0750s,
/// b0853s), dark once destroyed.
pub fn reticle_tint(health: f32, destroyed: bool) -> Color {
    if destroyed { Color::srgba(0.3, 0.3, 0.3, 0.55) } else { bar_color(health) }
}

/// Creates, moves and removes hardpoint markers; emits [`EawHardpointClicked`]. Each is the reticle art of its hardpoint at the data's
/// screen size, tinted by its health; there is no health line under it (the Remake shows a hardpoint's health in its tooltip).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn hardpoint_markers(
    tac: Res<TacticalOverlays>,
    scale: Res<UiScale>,
    assets: Option<Res<crate::EawUiAssets>>,
    root: Query<Entity, With<TacticalOverlayRoot>>,
    mut existing: Local<HashMap<String, Entity>>,
    clicks: Query<(&HardpointNode, &Interaction), Changed<Interaction>>,
    mut markers: Query<(&mut Node, &mut ImageNode), With<HardpointNode>>,
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
            commands.entity(*e).try_despawn();
        }
        keep
    });
    let s = scale.0;
    let size = reticle_design_px(assets.as_ref().and_then(|a| a.game.as_deref()));
    for h in &tac.hardpoints {
        let tint = reticle_tint(h.health, h.destroyed);
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
        // The node is `size` design px wide, `size * s` logical px: centred on the hardpoint.
        let (left, top) = (px(h.screen_pos.x - size * 0.5 * s, s), px(h.screen_pos.y - size * 0.5 * s, s));
        match existing.get(&h.id) {
            None => {
                let e = commands
                    .spawn((
                        Name::new(format!("hardpoint {}", h.id)),
                        Button,
                        Node {
                            position_type: PositionType::Absolute,
                            left,
                            top,
                            width: Val::Px(size),
                            height: Val::Px(size),
                            ..default()
                        },
                        image(),
                        FocusPolicy::Pass,
                        ChildOf(root),
                        HardpointNode { id: h.id.clone() },
                    ))
                    .id();
                existing.insert(h.id.clone(), e);
            }
            Some(&e) => {
                if let Ok((mut n, mut img)) = markers.get_mut(e) {
                    n.left = left;
                    n.top = top;
                    n.width = Val::Px(size);
                    n.height = Val::Px(size);
                    *img = image();
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
    fn ability_slots_keep_two_per_card_and_give_spare_buttons_to_cards_with_more() {
        let unit = |n: usize| TacticalUnit {
            abilities: (0..n).map(|i| AbilityState { def: AbilityDef { kind: format!("A{i}"), icon: format!("I_SA_A{i}.TGA"), ..default() }, ..default() }).collect(),
            ..default()
        };
        let shown = |slots: &[AbilityEntry]| slots.iter().filter(|a| !a.icon.is_empty()).map(|a| (a.card, a.nth, a.of)).collect::<Vec<_>>();
        // A few cards: every ability gets a button, in card order, the third and fourth ones too.
        let slots = ability_slots(&TacticalData { selection: vec![unit(3), unit(1), unit(0), unit(4)], ..default() });
        assert_eq!(slots.len(), 24);
        assert_eq!(shown(&slots), [(0, 0, 3), (0, 1, 3), (0, 2, 3), (1, 0, 1), (3, 0, 4), (3, 1, 4), (3, 2, 4), (3, 3, 4)]);
        assert_eq!((slots[2].kind.as_str(), slots[4].kind.as_str()), ("A2", "A0"));
        // A full bar: twelve cards keep their first two abilities; the third of the first card finds no button.
        let mut full: Vec<TacticalUnit> = (0..12).map(|_| unit(2)).collect();
        full[0] = unit(3);
        let slots = shown(&ability_slots(&TacticalData { selection: full, ..default() }));
        assert!((0..12).all(|c| slots.iter().filter(|x| x.0 == c).count() == 2 && slots.iter().all(|x| x.2 == 2)), "{slots:?}");
        // Ten cards with two leave two spare buttons for the card with five: it shows four.
        let mut some: Vec<TacticalUnit> = (0..10).map(|_| unit(2)).collect();
        some.insert(3, unit(5));
        let slots = shown(&ability_slots(&TacticalData { selection: some, ..default() }));
        assert_eq!(slots.len(), 24);
        assert_eq!(slots.iter().filter(|x| x.0 == 3).map(|x| x.2).collect::<Vec<_>>(), [4, 4, 4, 4]);
    }

    #[test]
    fn types_that_share_a_column_lay_their_ability_buttons_out_together() {
        // Two types on the top and bottom card of column 0 and a third in column 1, two abilities each: the four buttons over column 0
        // are one row of places 0..4 (before, both pairs were centred on column 0 and drawn on each other).
        let unit = |name: &str| TacticalUnit {
            unit_type: name.into(),
            abilities: (0..2).map(|i| AbilityState { def: AbilityDef { kind: format!("{name}{i}"), icon: format!("I_SA_{name}{i}.TGA"), ..default() }, ..default() }).collect(),
            ..default()
        };
        let tac = TacticalData { selection: vec![unit("A"), unit("B"), unit("C")], ..default() };
        let mut layout = vec![None; SELECT_SLOTS];
        for (slot, t) in [(0, 0), (1, 1), (2, 2)] {
            layout[slot] = Some(SlotUnits { cards: vec![t], units: 0..1 });
        }
        let placed: Vec<(usize, usize, usize, usize, usize)> =
            ability_slots_for(&tac, &layout).iter().filter(|a| !a.icon.is_empty()).map(|a| (a.card, a.column, a.span, a.at, a.among)).collect();
        assert_eq!(placed, [(0, 0, 1, 0, 4), (0, 0, 1, 1, 4), (1, 0, 1, 2, 4), (1, 0, 1, 3, 4), (2, 1, 1, 0, 2), (2, 1, 1, 1, 2)]);
    }

    /// `n` units of a type in bar section `section`.
    fn units_of(name: &str, section: u8, n: u32) -> TacticalUnit {
        TacticalUnit { unit_type: name.into(), icon: format!("I_{name}.TGA"), health: 1.0, count: n, section, ..default() }
    }

    /// (type, units) per card of the bar, `-` for an empty card.
    fn cards(tac: &TacticalData) -> Vec<String> {
        selection_layout(tac)
            .iter()
            .map(|s| s.as_ref().map_or("-".to_string(), |s| format!("{}:{}", tac.selection[s.first()].unit_type, s.units.len())))
            .collect()
    }

    #[test]
    fn every_unit_has_its_own_card_in_two_rows_while_they_fit() {
        // Three squadrons, a hero and two Star Destroyers: six cards, each type from the top of its own column (Remake video v0700s).
        let tac = TacticalData { selection: vec![units_of("TIE", 1, 3), units_of("Vader", 2, 1), units_of("ISD", 3, 2)], ..default() };
        let c = cards(&tac);
        assert_eq!(&c[..8], ["TIE:1", "TIE:1", "TIE:1", "-", "Vader:1", "-", "ISD:1", "ISD:1"]);
        assert!(c[8..].iter().all(|s| s == "-"));
        // The units keep their own health: the second card of the Star Destroyers shows the second one's.
        let mut tac = tac;
        tac.selection[2].units = vec![UnitStatus { health: 1.0, shield: Some(1.0) }, UnitStatus { health: 0.25, shield: Some(0.5) }];
        let mut app = App::new();
        app.init_resource::<CommandBarData>().insert_resource(TacticalData { active: true, ..tac }).add_systems(Update, tactical_sync);
        app.update();
        let bar = app.world().resource::<CommandBarData>();
        assert_eq!((bar.selection[7].health, bar.selection[7].shield, bar.selection[7].count), (0.25, Some(0.5), 1));
    }

    #[test]
    fn a_fleet_larger_than_the_bar_keeps_a_card_for_every_type_and_stacks_the_largest_groups() {
        // The defence of Brentaal in the play test: 16 types, 915 craft in squadrons, two Super Star Destroyers and the heroes; the
        // bar showed the first 12 types only.
        let mut sel = vec![
            units_of("Corvette", 0, 2), units_of("Lancer", 0, 3), units_of("Shuttle", 0, 6), units_of("Gunboat", 0, 7),
            units_of("TIE_Fighter", 1, 40), units_of("TIE_Bomber", 1, 7), units_of("TIE_Interceptor", 1, 24), units_of("TIE_Defender", 1, 70),
            units_of("TIE_Advanced", 1, 8), units_of("TIE_Hunter", 1, 16), units_of("Piett", 2, 1), units_of("Thrawn", 2, 1),
            units_of("Victory", 3, 4), units_of("Dominator", 3, 1), units_of("Vindicator", 3, 1), units_of("Executor", 3, 2),
        ];
        let tac = TacticalData { selection: sel.clone(), ..default() };
        let layout = selection_layout(&tac);
        assert_eq!(layout.len(), SELECT_SLOTS);
        for (t, u) in tac.selection.iter().enumerate() {
            let mine: Vec<&SlotUnits> = layout.iter().flatten().filter(|s| s.cards == [t]).collect();
            assert!(!mine.is_empty(), "{} has no card: {:?}", u.unit_type, cards(&tac));
            // Every unit is on exactly one card.
            assert_eq!(mine.iter().map(|s| s.units.len()).sum::<usize>(), u.count as usize, "{}", u.unit_type);
        }
        // The heroes and the single capital ships are not stacked; the 70 Defenders are.
        let c = cards(&tac);
        for one in ["Piett:1", "Thrawn:1", "Dominator:1", "Vindicator:1"] {
            assert!(c.iter().any(|s| s == one), "{one} in {c:?}");
        }
        assert!(c.iter().any(|s| s.starts_with("TIE_Defender:") && s != "TIE_Defender:1"), "{c:?}");
        // More types than cards (26): 23 of them have a card and one card stands for the rest. The heroes and every capital ship keep
        // their own (the play test's fleet at Fresia had 28 types: the Executors and the heroes fell into the rest), the last fighter
        // types share the rest card, which sits where they would.
        sel.extend((0..10).map(|i| units_of(&format!("Extra{i}"), 3, 1)));
        let tac = TacticalData { selection: sel, ..default() };
        let layout = selection_layout(&tac);
        assert!(layout.iter().all(Option::is_some));
        let rest: Vec<&SlotUnits> = layout.iter().flatten().filter(|s| s.cards.len() > 1).collect();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].cards, [7, 8, 9], "TIE_Defender, TIE_Advanced and TIE_Hunter share the rest card");
        assert_eq!(layout[7].as_ref().map(|s| s.cards.clone()), Some(vec![7, 8, 9]));
        for (t, u) in tac.selection.iter().enumerate().filter(|(_, u)| u.section >= 2) {
            assert!(layout.iter().flatten().any(|s| s.cards == [t]), "{} has no card of its own", u.unit_type);
        }
        // The rest card counts the units of all its types (70 + 8 + 16).
        let mut app = App::new();
        app.init_resource::<CommandBarData>().insert_resource(TacticalData { active: true, ..tac }).add_systems(Update, tactical_sync);
        app.update();
        assert_eq!(app.world().resource::<CommandBarData>().selection[7].count, 94);
    }

    #[test]
    fn a_picked_squadron_box_draws_its_icon_and_an_unresolved_icon_draws_no_frame() {
        // An empty game file system with one icon registered: `I_BUTTON_UNIT_TIE.TGA` resolves, `I_BUTTON_UNIT_NONE.TGA` does not.
        let dir = std::env::temp_dir().join("eaw-ui-overlay-icon-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let fs = std::sync::Arc::new(alamo_fs::GameFs::from_stacks(&[vec![dir]]).expect("fs"));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Assets<Image>>().init_resource::<UiScale>().init_resource::<crate::EawUiSettings>();
        let mut textures = UiTextures::new(fs);
        let icon = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            let art = alamo_model::image::Rgba8 { width: 4, height: 4, pixels: vec![200; 64] };
            textures.insert(&mut images, "I_BUTTON_UNIT_TIE.TGA", art).handle
        };
        app.insert_resource(textures);
        app.world_mut().spawn((TacticalOverlayRoot, Node::default()));
        let squad = |id: u64, icon: &str| ShipOverlay {
            id,
            screen_pos: Vec2::new(100.0 * id as f32, 300.0),
            half_size: 11.0,
            health: 1.0,
            shield: None,
            selected: true,
            hovered: false,
            enemy: false,
            name: None,
            always: false,
            icon: Some(icon.into()),
            icon_only: true,
            squad_box: true,
        };
        app.insert_resource(TacticalOverlays { ships: vec![squad(1, "I_BUTTON_UNIT_TIE.TGA"), squad(2, "I_BUTTON_UNIT_NONE.TGA")], hardpoints: Vec::new() })
            .add_systems(Update, ship_overlays);
        app.update();
        app.update();
        let world = app.world_mut();
        let found: Vec<(u64, Entity, Entity)> = world.query::<(Entity, &ShipOverlayNode)>().iter(world).map(|(e, n)| (n.id, e, n.icon)).collect();
        let shown = |world: &World, e: Entity| world.get::<Node>(e).map(|n| n.display) != Some(Display::None);
        let (_, tie, tie_icon) = *found.iter().find(|f| f.0 == 1).expect("overlay 1");
        let (_, none, _) = *found.iter().find(|f| f.0 == 2).expect("overlay 2");
        assert!(shown(world, tie) && shown(world, tie_icon), "the picked squadron shows its box");
        let img = world.get::<ImageNode>(tie_icon).expect("icon image");
        assert_eq!((img.image.id(), img.color), (icon.id(), Color::WHITE), "the unit's art, drawn as it is");
        assert_eq!(world.get::<BackgroundColor>(tie_icon).map(|b| b.0), Some(Color::NONE), "no plate");
        assert!(!shown(world, none), "an icon that does not resolve leaves no empty frame");
    }

    #[test]
    fn world_overlays_lie_under_every_interface_panel() {
        // The command bar, the minimap and the panels have no global z (0) or a positive one; the squadron icons lie at the bottom.
        let mut app = App::new();
        app.init_resource::<crate::EawUiSettings>().add_systems(Update, spawn_overlay_layer);
        app.update();
        let z: Vec<i32> = app.world_mut().query_filtered::<&GlobalZIndex, With<TacticalOverlayRoot>>().iter(app.world()).map(|z| z.0).collect();
        assert_eq!(z, [WORLD_OVERLAY_Z]);
        assert!(WORLD_OVERLAY_Z < 0, "under the command bar (0)");
        assert!(WORLD_OVERLAY_Z > crate::squad_icons::ICON_LAYER_Z, "over the squadron icons");
    }

    #[test]
    fn a_click_on_a_card_names_the_units_it_stands_for() {
        let mut app = App::new();
        app.init_resource::<CommandBarData>()
            .insert_resource(TacticalData { active: true, selection: vec![units_of("Vader", 2, 1), units_of("ISD", 3, 3)], ..default() })
            .add_message::<EawButtonClicked>()
            .add_message::<EawAbilityClicked>()
            .add_message::<EawSelectionClicked>()
            .add_systems(Update, (tactical_sync, tactical_clicks));
        app.update();
        // ISD cards are s_select_02, _03 (column 1) and _04 (column 2).
        app.world_mut().resource_mut::<Messages<EawButtonClicked>>().write(EawButtonClicked { entity: Entity::PLACEHOLDER, name: "s_select_03".into(), shift: false });
        app.update();
        let sel: Vec<_> = app.world_mut().resource_mut::<Messages<EawSelectionClicked>>().drain().collect();
        assert_eq!(sel.len(), 1);
        assert_eq!((sel[0].index, sel[0].cards.clone(), sel[0].units.clone()), (1, vec![1], 1..2));
    }

    #[test]
    fn player_triggered_abilities_sort_before_automatic_ones() {
        assert_eq!(activation_priority(" User_Input "), 0);
        assert_eq!(activation_priority("Space_Automatic"), 1);
        assert_eq!(activation_priority(""), 1);
        let mut v = vec![(activation_priority("Space_Automatic"), "a"), (activation_priority("User_Input"), "b"), (activation_priority("User_Input"), "c")];
        v.sort_by_key(|(p, _)| *p);
        assert_eq!(v.iter().map(|x| x.1).collect::<String>(), "bca");
    }

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
        assert_eq!(bar.selection.len(), 24);
        assert_eq!(bar.selection[0].shield, Some(0.25));
        assert!(bar.selection[0].selected);
        assert_eq!(bar.abilities[0].kind, "FULL_SALVO");
        assert!((bar.abilities[0].ready - 0.75).abs() < 1e-6);
        assert!(bar.abilities[0].active);
        assert_eq!((bar.pop_current, bar.pop_cap), (3, 9));
    }

    #[test]
    fn sync_multi_selection_shows_abilities_for_all_selected_groups() {
        let mut app = App::new();
        app.init_resource::<CommandBarData>()
            .insert_resource(TacticalData {
                active: true,
                focus: 0,
                selection: vec![
                    TacticalUnit {
                        unit_type: "Thrawn_Victory".into(),
                        icon: "I_THRAWN.TGA".into(),
                        health: 1.0,
                        shield: Some(1.0),
                        count: 1,
                        abilities: vec![
                            AbilityState {
                                def: AbilityDef {
                                    kind: "POWER_TO_WEAPONS".into(),
                                    icon: ability_icon("POWER_TO_WEAPONS"),
                                    name: "Power to Weapons".into(),
                                    ..default()
                                },
                                cooldown: 0.0,
                                active: false,
                                disabled: false,
                            },
                            AbilityState {
                                def: AbilityDef {
                                    kind: "TRACTOR_BEAM".into(),
                                    icon: ability_icon("TRACTOR_BEAM"),
                                    name: "Tractor Beam".into(),
                                    ..default()
                                },
                                cooldown: 0.5,
                                active: false,
                                disabled: false,
                            },
                        ],
                        ..default()
                    },
                    TacticalUnit {
                        unit_type: "Generic_Victory".into(),
                        icon: "I_VICTORY.TGA".into(),
                        health: 0.8,
                        shield: Some(0.6),
                        count: 3,
                        abilities: vec![
                            AbilityState {
                                def: AbilityDef {
                                    kind: "FULL_SALVO".into(),
                                    icon: ability_icon("FULL_SALVO"),
                                    name: "Full Salvo".into(),
                                    ..default()
                                },
                                cooldown: 0.0,
                                active: false,
                                disabled: false,
                            },
                        ],
                        ..default()
                    },
                ],
                ..default()
            })
            .add_systems(Update, (tactical_sync, tactical_clicks));
        app.add_message::<EawButtonClicked>()
            .add_message::<EawAbilityClicked>()
            .add_message::<EawSelectionClicked>();
        app.update();

        let bar = app.world().resource::<CommandBarData>();
        // Group 0 (Thrawn) is at column 0: slot 0 (card), abilities 0 and 1
        assert_eq!(bar.selection[0].icon, "I_THRAWN.TGA");
        assert!(bar.selection[0].selected);
        assert_eq!(bar.abilities[0].kind, "POWER_TO_WEAPONS");
        assert_eq!(bar.abilities[1].kind, "TRACTOR_BEAM");

        // Group 1 (Victory) is at column 1: slot 2 (card), ability 2
        assert_eq!(bar.selection[2].icon, "I_VICTORY.TGA");
        assert!(!bar.selection[2].selected);
        assert_eq!(bar.abilities[2].kind, "FULL_SALVO");
        assert_eq!(bar.abilities[3].kind, "");

        // Clicking special_button_02 fires ability for Generic_Victory
        app.world_mut().resource_mut::<Messages<EawButtonClicked>>().write(EawButtonClicked {
            entity: Entity::PLACEHOLDER,
            name: "special_button_02".into(),
            shift: false,
        });
        app.update();

        let emitted: Vec<_> = app.world_mut().resource_mut::<Messages<EawAbilityClicked>>().drain().collect();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].unit_type, "Generic_Victory");
        assert_eq!(emitted[0].ability, "FULL_SALVO");

        // Clicking s_select_02 fires selection for Group 1
        app.world_mut().resource_mut::<Messages<EawButtonClicked>>().write(EawButtonClicked {
            entity: Entity::PLACEHOLDER,
            name: "s_select_02".into(),
            shift: false,
        });
        app.update();

        let emitted_sel: Vec<_> = app.world_mut().resource_mut::<Messages<EawSelectionClicked>>().drain().collect();
        assert_eq!(emitted_sel.len(), 1);
        assert_eq!(emitted_sel[0].index, 1);
        assert_eq!(emitted_sel[0].unit_type, "Generic_Victory");
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
    fn reticles_are_the_data_share_of_the_screen_height_and_tinted_by_health() {
        // 0.022 of the height: 16.9 design px, 23.8 px at 1080p (it was 30 design px, 42 px).
        assert!((reticle_design_px(None) - 16.896).abs() < 1e-3);
        let mut game = GameData::default();
        game.game_constants.insert("hardpoint_target_reticle_enemy_screen_size".into(), " 0.03 ".into());
        assert!((reticle_design_px(Some(&game)) - 23.04).abs() < 1e-3);
        // Green at full health like the Remake's (v0750s), the bar colours below, dark when destroyed; no grey for own hardpoints.
        assert_eq!(reticle_tint(1.0, false), bar_color(1.0));
        assert_eq!(reticle_tint(0.2, false), bar_color(0.2));
        assert_ne!(reticle_tint(1.0, true), bar_color(1.0));
    }

    #[test]
    fn ability_icon_names() {
        assert_eq!(ability_icon("full_salvo"), "I_SA_FULL_SALVO.TGA");
        assert_eq!(ability_icon("spoiler_lock"), "I_SA_S_FOIL_MODE.TGA");
        assert_eq!(ability_icon("TURBO"), "I_SA_POWER_TO_ENGINES.TGA");
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






