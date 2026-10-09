//! Data binding: what the application tells the interface to show, and what the interface tells
//! the application back.
//!
//! The interface never owns game state. The application writes the [`CommandBarData`] resource
//! (credits, selected planet, build queue ...) and reads [`EawButtonClicked`] messages.

use bevy::prelude::*;

use crate::faction::UiFaction;

/// A button was clicked. `name` is the original component name, lower-cased
/// (`b_zoom`, `queue03`, `b_create_07`, `c_button00`, `b_option_g` ...).
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct EawButtonClicked {
    /// Lower-cased component name.
    pub name: String,
    /// The button entity.
    pub entity: Entity,
    /// True if Shift was held (`Click_Shift` buttons act on the whole selection / queue).
    pub shift: bool,
}

/// A greyed-out (disabled) button was pressed: the application can say why it is locked.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct EawLockedClicked {
    /// Lower-cased component name.
    pub name: String,
}

/// One entry of the production queue (`queue00..09` galactic, `tqueue00..09` tactical).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueueEntry {
    /// Icon texture name (`Icon_Name` of the unit XML, e.g. `I_BUTTON_X-WING.tga`).
    pub icon: String,
    /// Build progress 0..1 (the front entry is the one being built).
    pub progress: f32,
    /// Number of queued items of this kind (shown when above 1).
    pub count: u32,
    /// Tooltip text (empty keeps the slot's engine text).
    pub tooltip: String,
}

/// One build pad (`b_create_NN`): a unit or structure that can be produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildPadEntry {
    /// Icon texture name.
    pub icon: String,
    /// Price text (credits).
    pub price: Option<u32>,
    /// Greyed out (cannot be built right now: queue full, pop cap, funds ...).
    pub disabled: bool,
    /// Production in progress for this item (shows the build overlay).
    pub building: bool,
    /// Tooltip text (empty keeps the slot's engine text).
    pub tooltip: String,
    /// Income change per week while the unit exists (negative = upkeep); shows a red or green marker.
    pub upkeep: f32,
    /// Starbase level (1..) shown as a roman numeral; 0 shows none.
    pub level: u8,
    /// A requirement is missing (star base level, structure ...): drawn dark with the required level.
    pub locked: bool,
    /// Text under the icon instead of the price (fleet unit counts).
    pub label: String,
}

/// One slot of the tactical selection (`s_select_NN` with `s_health_NN` / `s_shield_NN`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SelectionEntry {
    /// Icon texture name.
    pub icon: String,
    /// Hull 0..1.
    pub health: f32,
    /// Shield 0..1 (`None` for units without shields).
    pub shield: Option<f32>,
    /// Number of selected units of this type (shown when above 1).
    pub count: u32,
    /// Section of the bar (0 small ships, 1 fighters, 2 heroes, 3 capitals): neighbours of one section share one frame.
    pub section: u8,
}

/// A special ability button of the tactical bar (`special_button_NN`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AbilityEntry {
    /// Icon texture name.
    pub icon: String,
    /// Recharge progress 0..1 (1 = ready).
    pub ready: f32,
    /// Ability active (toggled on).
    pub active: bool,
    /// Ability cannot be used.
    pub disabled: bool,
    /// Ability `Type` (`FULL_SALVO`), reported when the button is clicked.
    pub kind: String,
    /// Tooltip text (already localized).
    pub tooltip: String,
}

/// Everything the command bars display (resource).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct CommandBarData {
    /// Credits in the galactic reserve.
    pub credits: i64,
    /// Income of the selected planet (`Text_Planetary_Income`).
    pub planet_income: i64,
    /// Name of the selected planet.
    pub planet_name: String,
    /// Owner of the selected planet (allegiance symbol).
    pub planet_owner: Option<UiFaction>,
    /// Credits per month (`Text_Day_Credits`).
    pub month_income: i64,
    /// Progress through the current month, 0..1 (the green bar next to "MONTH").
    pub month_progress: f32,
    /// Colour of the planet name (owner relation); `None` keeps the data default.
    pub planet_color: Option<Color>,
    /// Galactic month counter (shown as "MONTH: n").
    pub day: u32,
    /// Tech level of the player.
    pub tech_level: u32,
    /// Current unit population.
    pub pop_current: u32,
    /// Population cap.
    pub pop_cap: u32,
    /// Production queue.
    pub queue: Vec<QueueEntry>,
    /// Build pads.
    pub pads: Vec<BuildPadEntry>,
    /// Tactical selection.
    pub selection: Vec<SelectionEntry>,
    /// Special abilities.
    pub abilities: Vec<AbilityEntry>,
    /// First bar column (two slots wide) the ability buttons are drawn above: the section of the focused unit.
    pub ability_column: usize,
    /// Which build tab is open (`b_space_tab` / `b_planet_tab`): true for space production.
    pub space_tab: bool,
}

/// Programmatic override of a button's look (the application owns the state).
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EawButtonState {
    /// Toggled on / pressed look.
    pub selected: bool,
    /// Greyed out, ignores clicks.
    pub disabled: bool,
    /// Force the hover look (used for offscreen renders and gamepads).
    pub force_hover: bool,
}

/// Debug/offscreen helper: show the tooltip of this button as if the mouse hovered it.
#[derive(Resource, Debug, Clone, Default)]
pub struct ForcedTooltip(pub Option<String>);

