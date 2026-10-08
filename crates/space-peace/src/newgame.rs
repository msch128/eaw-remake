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
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct NewGameOptions {
    /// Difficulty of the Remake AI (Easy/Normal/Hard come from the mod, Brutal and Fair are custom).
    pub difficulty: DifficultyTier,
    /// Use galaxy-sim's built-in AI instead (no difficulty tiers).
    pub classic_ai: bool,
}

impl Default for NewGameOptions {
    fn default() -> Self {
        Self { difficulty: DifficultyTier::Normal, classic_ai: false }
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
        Self { difficulty, classic_ai: args.classic_ai }
    }

    /// Next tier in the order Easy, Normal, Hard, Brutal, Fair.
    pub fn cycle(&mut self) {
        self.difficulty = match self.difficulty {
            DifficultyTier::Easy => DifficultyTier::Normal,
            DifficultyTier::Normal => DifficultyTier::Hard,
            DifficultyTier::Hard => DifficultyTier::Brutal,
            DifficultyTier::Brutal => DifficultyTier::Fair,
            DifficultyTier::Fair => DifficultyTier::Easy,
        };
    }

    /// Label of the difficulty button.
    pub fn difficulty_label(&self) -> String {
        if self.classic_ai { "AI difficulty: - (classic AI)".to_owned() } else { format!("AI difficulty: {}", tier_name(self.difficulty)) }
    }

    /// Label of the classic-AI button.
    pub fn classic_label(&self) -> String {
        format!("Classic AI: {}", if self.classic_ai { "on" } else { "off" })
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

/// `%APPDATA%\SpacePeace\saves`, `None` when `%APPDATA%` is not set.
pub fn saves_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("SpacePeace").join("saves"))
}

/// File of a slot.
pub fn slot_path(slot: u8) -> Option<PathBuf> {
    saves_dir().map(|d| d.join(format!("slot{slot}.json")))
}

/// What a slot file holds.
struct SlotInfo {
    campaign: String,
    week: u64,
    difficulty: String,
}

fn slot_info(slot: u8) -> Option<SlotInfo> {
    let path = slot_path(slot)?;
    if !path.exists() {
        return None;
    }
    let f = SaveFile::read(&path).ok()?;
    let difficulty = if f.header.config.remake_ai { f.header.config.difficulty.map_or("Normal", tier_name).to_owned() } else { "classic AI".to_owned() };
    Some(SlotInfo { campaign: f.header.campaign.clone(), week: f.checkpoint.week, difficulty })
}

/// The slot list. `only_campaign`: in a running game only saves of this campaign can be loaded.
pub fn slot_spec(mode: SlotMode, only_campaign: Option<&str>) -> MenuSpec {
    let mut items = Vec::new();
    for n in 1..=SLOTS {
        let label = match slot_info(n) {
            Some(i) => {
                let other = mode == SlotMode::Load && only_campaign.is_some_and(|c| !c.eq_ignore_ascii_case(&i.campaign));
                format!("Slot {n} - week {}, {}, {}{}", i.week + 1, i.campaign, i.difficulty, if other { " (other galaxy)" } else { "" })
            }
            None => format!("Slot {n} - empty"),
        };
        items.push(MenuItem::new(label, MenuAction::Slot(mode, n)));
    }
    MenuSpec {
        title: if mode == SlotMode::Save { "Save Game".into() } else { "Load Game".into() },
        columns: vec![MenuColumn { items, ..default() }],
        footer: vec![MenuItem::new("Back", MenuAction::CloseSlots)],
        dim_background: true,
        ..default()
    }
}

/// Options, slot screens and save/load handling.
pub struct NewGamePlugin;

impl Plugin for NewGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NewGameOptions>()
            .init_resource::<SlotScreen>()
            .add_message::<SlotChosen>()
            .add_message::<MenuRefresh>()
            .add_systems(OnEnter(AppScreen::MainMenu), reset_slots)
            .add_systems(OnEnter(AppScreen::Galaxy), reset_slots)
            .add_systems(Update, (sync_slot_menu, handle_slot_chosen, report_events));
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
    current: Res<CurrentCampaign>,
) {
    if !slots.is_changed() {
        return;
    }
    for e in &menus {
        commands.entity(e).despawn();
    }
    if let Some(mode) = slots.0 {
        let only = state.as_ref().and(current.name.as_deref());
        spawn_menu(&mut commands, &slot_spec(mode, only), SlotMenu);
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
            }
            SlotMode::Load => {
                let Ok(file) = SaveFile::read(&path) else { continue };
                if state.is_some() {
                    // Inside a game only the same galaxy can be replaced.
                    if current.name.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&file.header.campaign)) {
                        actions.write(PlayAction::Load(path));
                        slots.0 = None;
                        paused.0 = false;
                    }
                } else {
                    commands.insert_resource(PendingLoad(path));
                    start.write(StartCampaign(file.header.campaign.clone()));
                    slots.0 = None;
                }
            }
        }
    }
}

fn report_events(mut events: MessageReader<PlayEvent>, mut end: ResMut<crate::play_ui::EndState>) {
    for e in events.read() {
        match e {
            PlayEvent::SaveFinished { path, result } => match result {
                Ok(()) => {
                    info!("saves: wrote {}", path.display());
                    let _ = std::fs::write(crate::play_ui::GameStats::sidecar(path), end.stats.to_text());
                }
                Err(e) => warn!("saves: cannot write {}: {e}", path.display()),
            },
            PlayEvent::LoadFinished { path, result } => match result {
                Ok(week) => {
                    info!("saves: loaded {} (week {week})", path.display());
                    end.stats = std::fs::read_to_string(crate::play_ui::GameStats::sidecar(path)).ok().and_then(|t| crate::play_ui::GameStats::from_text(&t)).unwrap_or_default();
                }
                Err(e) => warn!("saves: cannot load {}: {e}", path.display()),
            },
            _ => {}
        }
    }
}
