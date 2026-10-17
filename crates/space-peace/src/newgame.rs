//! New-game options (AI difficulty, classic AI) and save/load slots. The menus themselves are
//! `menu.rs` specs; the loading overlay lives in `play_ui.rs`.

use std::path::PathBuf;

use bevy::prelude::*;
use galaxy_play::{PlayAction, PlayEvent, SimState};
use galaxy_script::{DifficultyTier, SaveFile};

use crate::cli::Args;
use crate::flow::{AppScreen, CurrentCampaign, Paused, StartCampaign};
use crate::menu::{MenuAction, MenuColumn, MenuItem, MenuSpec, spawn_menu};

/// Save slots offered in the menus.
pub const SLOTS: u8 = 5;

/// Settings of the next new game.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct NewGameOptions {
    /// Difficulty of the Remake AI (Easy/Normal/Hard come from the mod, Brutal and Fair are custom).
    pub difficulty: DifficultyTier,
    /// Use galaxy-sim's built-in AI instead (no difficulty tiers).
    pub classic_ai: bool,
    /// "From the Ground Up" start of the next game.
    pub ground_up: bool,
    /// Override faction for the next game (e.g. From the Ground Up or submods).
    pub faction_override: Option<String>,
}

impl Default for NewGameOptions {
    fn default() -> Self {
        Self { difficulty: DifficultyTier::Normal, classic_ai: false, ground_up: false, faction_override: None }
    }
}

/// Display name of a tier.
pub fn tier_name(t: DifficultyTier) -> &'static str {
    match t {
        DifficultyTier::Easy => "Easy",
        DifficultyTier::Normal => "Normal",
        DifficultyTier::Hard => "Hard",
        DifficultyTier::Brutal => "Brutal (custom)",
        DifficultyTier::Fair => "Fair (custom)",
    }
}

impl NewGameOptions {
    /// Options from the command line.
    pub fn from_args(args: &Args) -> Self {
        let difficulty = match args.difficulty.as_deref() {
            Some("easy") => DifficultyTier::Easy,
            Some("hard") => DifficultyTier::Hard,
            Some("brutal") => DifficultyTier::Brutal,
            Some("fair") => DifficultyTier::Fair,
            _ => DifficultyTier::Normal,
        };
        Self { difficulty, classic_ai: args.classic_ai, ground_up: args.ground_up, faction_override: None }
    }
}

/// A save to start the next game from (set by the load menu, taken by `play::begin_play`).
#[derive(Resource, Debug, Clone)]
pub struct PendingLoad(pub PathBuf);

/// Which slot screen is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotMode {
    /// Write the running game into a slot.
    Save,
    /// Continue from a slot.
    Load,
}

/// The open slot screen, if any.
#[derive(Resource, Default, Debug)]
pub struct SlotScreen(pub Option<SlotMode>);

/// A slot button was clicked.
#[derive(Message, Debug, Clone, Copy)]
pub struct SlotChosen(pub SlotMode, pub u8);

/// A menu changed its content (labels) and has to be rebuilt.
#[derive(Message, Debug, Clone, Copy)]
pub struct MenuRefresh;

/// Root of the slot screen.
#[derive(Component)]
struct SlotMenu;

/// Saves folder of a headless run that names none (see [`isolate_offscreen_saves`]).
static OFFSCREEN_SAVES: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Sends the saves of a headless (`--offscreen`) run to a scratch folder unless `SP_SAVES_DIR` names one: a check run must never
/// overwrite the player's autosaves (2026-10-17 an offscreen battle replaced the player's pre-battle and rolling autosaves).
pub fn isolate_offscreen_saves() {
    let _ = OFFSCREEN_SAVES.set(std::env::temp_dir().join("space-peace-offscreen-saves"));
}

/// `%APPDATA%\SpacePeace\saves`, `None` when `%APPDATA%` is not set.
pub fn saves_dir() -> Option<PathBuf> {
    // Tests point this elsewhere so they never touch the player's saves.
    if let Some(d) = std::env::var_os("SP_SAVES_DIR") {
        return Some(PathBuf::from(d));
    }
    if let Some(d) = OFFSCREEN_SAVES.get() {
        return Some(d.clone());
    }
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("SpacePeace").join("saves"))
}

/// The three kinds of autosave slots (U43).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutosaveKind {
    /// Saved right before entering a space battle.
    PreBattle,
    /// Saved right after returning from a space battle.
    PostBattle,
    /// Saved every 5 minutes during galactic play.
    Rolling,
}

pub fn autosave_path(kind: AutosaveKind) -> Option<PathBuf> {
    let name = match kind {
        AutosaveKind::PreBattle => "autosave_pre_battle.json",
        AutosaveKind::PostBattle => "autosave_post_battle.json",
        AutosaveKind::Rolling => "autosave_rolling.json",
    };
    saves_dir().map(|d| d.join(name))
}

/// File of a slot (1..=5 manual, 6 PreBattle autosave, 7 PostBattle autosave, 8 Rolling autosave).
pub fn slot_path(slot: u8) -> Option<PathBuf> {
    match slot {
        1..=5 => saves_dir().map(|d| d.join(format!("slot{slot}.json"))),
        6 => autosave_path(AutosaveKind::PreBattle),
        7 => autosave_path(AutosaveKind::PostBattle),
        8 => autosave_path(AutosaveKind::Rolling),
        _ => None,
    }
}

/// What a slot file holds.
struct SlotInfo {
    campaign: String,
    /// Game content the save needs.
    content: crate::game::Content,
    week: u64,
    difficulty: String,
    /// Saved from a "From the Ground Up" start.
    ground_up: bool,
}

fn slot_info(slot: u8) -> Option<SlotInfo> {
    let path = slot_path(slot)?;
    if !path.exists() {
        return None;
    }
    let f = SaveFile::read(&path).ok()?;
    let difficulty = if f.header.config.remake_ai { f.header.config.difficulty.map_or("Normal", tier_name).to_owned() } else { "classic AI".to_owned() };
    Some(SlotInfo { campaign: f.header.campaign.clone(), content: crate::game::Content::from_mod_arg(f.header.config.content.as_deref()), week: f.checkpoint.week, difficulty, ground_up: f.header.sim_config.ground_up })
}

/// Readable campaign name for a save: the campaign group and the faction played, e.g. `Mini Standard (Empire)`.
/// Falls back to the raw campaign key when the campaign is not in the list.
fn campaign_label(campaign: &str, ground_up: bool, entries: &[crate::game::CampaignEntry]) -> String {
    match entries.iter().find(|e| e.name.eq_ignore_ascii_case(campaign)) {
        Some(e) => {
            let title = if ground_up { "From the Ground Up" } else if e.group_title.is_empty() { e.title.as_str() } else { e.group_title.as_str() };
            format!("{title} ({})", e.faction)
        }
        None => campaign.to_owned(),
    }
}

/// Adds the content of a Clone Wars save to its readable name; Remake 4.0 saves stay unmarked.
fn content_label(name: String, content: crate::game::Content) -> String {
    match content {
        crate::game::Content::Remake => name,
        c => format!("{name} [{}]", c.label()),
    }
}

/// One list line. The week is the galaxy's own counter, the number the game shows and logs while playing.
fn slot_line(prefix: &str, name: &str, i: &SlotInfo, note: &str) -> String {
    format!("{prefix} - {name}, week {}, {}{note}", i.week, i.difficulty)
}

/// The slot list. `only_campaign`: in a running game only saves of this campaign can be loaded.
pub fn slot_spec(mode: SlotMode, only_campaign: Option<&str>, entries: &[crate::game::CampaignEntry], active: crate::game::Content) -> MenuSpec {
    let mut items = Vec::new();
    for n in 1..=SLOTS {
        let label = match slot_info(n) {
            Some(i) => {
                let other = mode == SlotMode::Load && only_campaign.is_some_and(|c| !c.eq_ignore_ascii_case(&i.campaign) || i.content != active);
                let name = content_label(campaign_label(&i.campaign, i.ground_up, entries), i.content);
                let note = if other { " - different campaign or content (load it from the main menu)" } else { "" };
                slot_line(&format!("Slot {n}"), &name, &i, note)
            }
            None => format!("Slot {n} - empty"),
        };
        items.push(MenuItem::new(label, MenuAction::Slot(mode, n)));
    }
    if mode == SlotMode::Load {
        for (n, name_prefix) in [
            (6, "Autosave Pre-Battle"),
            (7, "Autosave Post-Battle"),
            (8, "Autosave Rolling (5 min)"),
        ] {
            let label = match slot_info(n) {
                Some(i) => {
                    let other = only_campaign.is_some_and(|c| !c.eq_ignore_ascii_case(&i.campaign) || i.content != active);
                    let name = content_label(campaign_label(&i.campaign, i.ground_up, entries), i.content);
                    let note = if other { " - different campaign or content" } else { "" };
                    slot_line(name_prefix, &name, &i, note)
                }
                None => format!("{name_prefix} - empty"),
            };
            items.push(MenuItem::new(label, MenuAction::Slot(mode, n)));
        }
    }
    MenuSpec {
        title: if mode == SlotMode::Save { "Save Game".into() } else { "Load Game".into() },
        columns: vec![MenuColumn { items, ..default() }],
        footer: vec![MenuItem::new("Back", MenuAction::CloseSlots)],
        dim_background: true,
        ..default()
    }
}

/// Periodic autosave timer (U43: rolling autosave every 5 minutes).
#[derive(Resource)]
pub struct AutosaveTimer(pub Timer);

impl Default for AutosaveTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(300.0, TimerMode::Repeating))
    }
}

/// Saves the running game every 5 minutes of play (galaxy view and space battle).
fn rolling_autosave(
    time: Res<Time>,
    screen: Res<State<AppScreen>>,
    state: Option<Res<SimState>>,
    mut timer: ResMut<AutosaveTimer>,
    mut actions: MessageWriter<PlayAction>,
) {
    if !matches!(*screen.get(), AppScreen::Galaxy | AppScreen::SpaceBattle) || state.is_none_or(|s| s.current.loading) {
        return;
    }
    if timer.0.tick(time.delta()).just_finished() {
        if let Some(path) = autosave_path(AutosaveKind::Rolling) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            info!("saves: rolling 5-minute autosave -> {}", path.display());
            actions.write(PlayAction::Save(path));
        }
    }
}

/// Options, slot screens and save/load handling.
pub struct NewGamePlugin;

impl Plugin for NewGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NewGameOptions>()
            .init_resource::<SlotScreen>()
            .init_resource::<AutosaveTimer>()
            .add_message::<SlotChosen>()
            .add_message::<MenuRefresh>()
            .add_systems(OnEnter(AppScreen::MainMenu), reset_slots)
            .add_systems(OnEnter(AppScreen::Galaxy), reset_slots)
            .add_systems(Update, (sync_slot_menu, handle_slot_chosen, report_events, rolling_autosave));
    }
}

fn reset_slots(mut slots: ResMut<SlotScreen>) {
    slots.0 = None;
}

/// Opens or closes the slot screen when [`SlotScreen`] changes.
fn sync_slot_menu(
    mut commands: Commands,
    slots: Res<SlotScreen>,
    menus: Query<Entity, With<SlotMenu>>,
    state: Option<Res<SimState>>,
    game: Res<crate::game::Game>,
    current: Res<CurrentCampaign>,
    ui: Option<Res<eaw_ui::EawUiSettings>>,
    content: Option<Res<crate::game::ContentChoice>>,
) {
    if !slots.is_changed() {
        return;
    }
    for e in &menus {
        commands.entity(e).despawn();
    }
    if let Some(mode) = slots.0 {
        let only = state.as_ref().and(current.name.as_deref());
        let entries = game.0.as_ref().map(|g| crate::game::campaign_entries(&g.data)).unwrap_or_default();
        let root = spawn_menu(&mut commands, &slot_spec(mode, only, &entries, content.map_or(crate::game::Content::Remake, |c| c.active)), SlotMenu);
        commands.entity(root).insert(GlobalZIndex(650));
        if let Some(cam) = ui.and_then(|u| u.camera) {
            commands.entity(root).insert(UiTargetCamera(cam));
        }
    }
}

/// A slot was chosen: save the running game, or continue from the file.
#[allow(clippy::too_many_arguments)]
fn handle_slot_chosen(
    mut chosen: MessageReader<SlotChosen>,
    mut slots: ResMut<SlotScreen>,
    mut paused: ResMut<Paused>,
    mut actions: MessageWriter<PlayAction>,
    mut start: MessageWriter<StartCampaign>,
    mut commands: Commands,
    state: Option<Res<SimState>>,
    current: Res<CurrentCampaign>,
    mut battle_menu: ResMut<crate::menu::BattleMenu>,
    screen: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    content: Option<Res<crate::game::ContentChoice>>,
    args: Res<crate::cli::Args>,
    mut exit: MessageWriter<AppExit>,
) {
    for SlotChosen(mode, n) in chosen.read() {
        let Some(path) = slot_path(*n) else {
            warn!("saves: %APPDATA% is not set");
            continue;
        };
        match mode {
            SlotMode::Save => {
                if state.is_none() {
                    continue;
                }
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                actions.write(PlayAction::Save(path));
                slots.0 = None;
                paused.0 = false;
                battle_menu.open = false;
            }
            SlotMode::Load => {
                let Ok(file) = SaveFile::read(&path) else { continue };
                let saved = crate::game::Content::from_mod_arg(file.header.config.content.as_deref());
                let active = content.as_ref().map_or(crate::game::Content::Remake, |c| c.active);
                if state.is_some() {
                    // Inside a game only the same galaxy can be replaced.
                    if current.name.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&file.header.campaign)) && saved == active {
                        actions.write(PlayAction::Load(path));
                        slots.0 = None;
                        paused.0 = false;
                        battle_menu.open = false;
                        if *screen.get() == AppScreen::SpaceBattle {
                            next.set(AppScreen::Galaxy);
                        }
                    }
                } else {
                    if saved != active {
                        // The save needs the other stack and the file system is fixed at start-up: start again on it, straight into the save.
                        let available = content.as_ref().is_some_and(|c| saved == crate::game::Content::Remake || c.clone_wars_available);
                        let extra = ["--campaign".to_owned(), file.header.campaign.clone(), "--load-from".to_owned(), path.display().to_string()];
                        if !available {
                            warn!("saves: {} needs {}, which is not installed", path.display(), saved.label());
                        } else if args.offscreen.is_some() {
                            warn!("saves: {} needs {}: restart skipped (offscreen run, use --mod)", path.display(), saved.label());
                        } else if crate::game::restart_with(saved, &extra) {
                            info!("saves: restarting on {} for {}", saved.label(), path.display());
                            exit.write(AppExit::Success);
                        }
                        continue;
                    }
                    commands.insert_resource(PendingLoad(path));
                    start.write(StartCampaign(file.header.campaign.clone()));
                    slots.0 = None;
                }
            }
        }
    }
}

fn report_events(mut events: MessageReader<PlayEvent>) {
    for e in events.read() {
        match e {
            PlayEvent::SaveFinished { path, result } => match result {
                Ok(()) => {
                    info!("saves: wrote {}", path.display());
                }
                Err(e) => warn!("saves: cannot write {}: {e}", path.display()),
            },
            PlayEvent::LoadFinished { path, result } => match result {
                Ok(week) => {
                    info!("saves: loaded {} (week {week})", path.display());
                }
                Err(e) => warn!("saves: cannot load {}: {e}", path.display()),
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_up_save_is_labelled_as_such() {
        let e = crate::game::CampaignEntry {
            name: "GC_Full_Web_Way_Underworld".into(),
            title: "Full".into(),
            group: "full".into(),
            group_title: "Full".into(),
            faction: "Underworld".into(),
            planets: 100,
        };
        assert_eq!(campaign_label("GC_Full_Web_Way_Underworld", false, std::slice::from_ref(&e)), "Full (Underworld)");
        assert_eq!(campaign_label("GC_Full_Web_Way_Underworld", true, std::slice::from_ref(&e)), "From the Ground Up (Underworld)");
    }

    #[test]
    fn slot_line_shows_the_in_game_week() {
        let i = SlotInfo { campaign: "X".into(), content: crate::game::Content::Remake, week: 1, difficulty: "Brutal".into(), ground_up: false };
        assert_eq!(slot_line("Slot 2", "Ground Up", &i, ""), "Slot 2 - Ground Up, week 1, Brutal");
    }

    #[test]
    fn autosave_paths_and_slot_mappings() {
        assert_eq!(
            autosave_path(AutosaveKind::PreBattle).unwrap().file_name().unwrap(),
            "autosave_pre_battle.json"
        );
        assert_eq!(
            autosave_path(AutosaveKind::PostBattle).unwrap().file_name().unwrap(),
            "autosave_post_battle.json"
        );
        assert_eq!(
            autosave_path(AutosaveKind::Rolling).unwrap().file_name().unwrap(),
            "autosave_rolling.json"
        );

        assert_eq!(slot_path(6), autosave_path(AutosaveKind::PreBattle));
        assert_eq!(slot_path(7), autosave_path(AutosaveKind::PostBattle));
        assert_eq!(slot_path(8), autosave_path(AutosaveKind::Rolling));
    }

    #[test]
    fn load_slot_spec_includes_autosaves() {
        let spec = slot_spec(SlotMode::Load, None, &[], crate::game::Content::Remake);
        assert_eq!(spec.columns[0].items.len(), 5 + 3);
        assert!(spec.columns[0].items[5].label.starts_with("Autosave Pre-Battle"));
        assert!(spec.columns[0].items[6].label.starts_with("Autosave Post-Battle"));
        assert!(spec.columns[0].items[7].label.starts_with("Autosave Rolling"));
    }

    #[test]
    fn save_slot_spec_only_shows_manual_slots() {
        let spec = slot_spec(SlotMode::Save, None, &[], crate::game::Content::Remake);
        assert_eq!(spec.columns[0].items.len(), 5);
    }
}
