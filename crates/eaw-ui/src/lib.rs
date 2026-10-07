//! Original Empire at War interface rebuilt in Bevy UI from the game's own data and textures.
//!
//! * [`textures`]: the `MT_CommandBar` mega texture and loose UI textures as Bevy images.
//! * [`components`]: `CommandBarComponents.xml` (look of every element).
//! * [`shell`]: the shell models (`.alo`) that hold the placement and the frame art.
//! * [`widgets`]: spawning of the command bars, buttons, slots and bars.
//! * [`data`]: [`CommandBarData`] (what to show) and [`EawButtonClicked`] (what was clicked).
//!
//! Add [`EawUiPlugin`], spawn a bar with [`widgets::spawn_command_bar`] (or the
//! [`EawUiCommandsExt`] helper), write to [`CommandBarData`] and listen for
//! [`EawButtonClicked`]. See `UI.md` for the findings behind it.

pub mod components;
pub mod data;
pub mod faction;
pub mod shell;
pub mod textures;
pub mod widgets;

use std::sync::Arc;

use alamo_data::{GameData, TextTable};
use alamo_fs::GameFs;
use bevy::prelude::*;

pub use components::{CommandBarComponent, CommandBarDb, ComponentKind};
pub use data::{
    AbilityEntry, BuildPadEntry, CommandBarData, EawButtonClicked, EawButtonState, ForcedTooltip,
    QueueEntry, SelectionEntry,
};
pub use faction::UiFaction;
pub use widgets::{CommandBarMode, CommandBarSpec, EawButton, EawCommandBar, spawn_command_bar};

/// How design pixels (1366x768 reference) map to screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiScaleMode {
    /// Window height / 768, like the original: 4K gets a 2.8x bar.
    Auto,
    /// Fixed physical pixels per design pixel (for render targets without a window).
    Fixed(f32),
}

/// Interface settings (resource).
#[derive(Resource, Debug, Clone)]
pub struct EawUiSettings {
    /// UI scale mode.
    pub scale: UiScaleMode,
    /// Frame art is rasterized from the original meshes at this many pixels per design pixel.
    /// 2 matches the 2048-wide dashboard textures; use 3 for 4K.
    pub faceplate_oversample: f32,
    /// System font family used for all UI text. The original fonts (`EmpireAtWar-*`) are not
    /// shipped with the game; this is looked up in the system font database at runtime.
    /// `None` uses Bevy's built-in default font.
    pub font_family: Option<String>,
    /// Font family of tooltips (`Tool_Tip_Font_Name` in `GameConstants.xml` is Arial).
    pub tooltip_font_family: Option<String>,
    /// Camera the UI is drawn by. `None` uses the default (the primary window's camera); set
    /// it for UI rendered into an image.
    pub camera: Option<Entity>,
}

impl Default for EawUiSettings {
    fn default() -> Self {
        Self {
            scale: UiScaleMode::Auto,
            faceplate_oversample: 2.0,
            font_family: Some("Bahnschrift".to_string()),
            tooltip_font_family: Some("Arial".to_string()),
            camera: None,
        }
    }
}

/// Game data the interface reads (resource).
#[derive(Resource)]
pub struct EawUiAssets {
    /// Game file system.
    pub fs: Arc<GameFs>,
    /// English string table (`MasterTextFile_ENGLISH.DAT`).
    pub text: TextTable,
    /// Optional game data, used to resolve unit icons ([`unit_icon`]).
    pub game: Option<Arc<GameData>>,
}

/// Icon texture name of a unit/structure (`Icon_Name` in its XML definition), `None` if it has none.
pub fn unit_icon(game: &GameData, unit: &str) -> Option<String> {
    let r = game.objects.resolve(unit)?;
    r.get("icon_name").map(str::to_string)
}

/// The plugin: resources, systems and the tooltip panel.
pub struct EawUiPlugin {
    /// Game file system (mod stack included).
    pub fs: Arc<GameFs>,
    /// Optional game data for unit icons.
    pub game: Option<Arc<GameData>>,
    /// Settings.
    pub settings: EawUiSettings,
}

impl EawUiPlugin {
    /// Creates the plugin with default settings.
    pub fn new(fs: Arc<GameFs>) -> Self {
        Self {
            fs,
            game: None,
            settings: EawUiSettings::default(),
        }
    }

    /// Shares already loaded game data (for unit icons).
    pub fn with_game_data(mut self, game: Arc<GameData>) -> Self {
        self.game = Some(game);
        self
    }

    /// Overrides the settings.
    pub fn with_settings(mut self, settings: EawUiSettings) -> Self {
        self.settings = settings;
        self
    }
}

impl Plugin for EawUiPlugin {
    fn build(&self, app: &mut App) {
        let (db, warnings) = CommandBarDb::load(&*self.fs);
        for w in warnings.iter().take(5) {
            warn!("command bar data: {w}");
        }
        let text = TextTable::load(&*self.fs, "Text/MasterTextFile_ENGLISH.DAT").unwrap_or_else(|e| {
            warn!("string table not loaded: {e}");
            TextTable::default()
        });
        app.insert_resource(db)
            .insert_resource(textures::UiTextures::new(self.fs.clone()))
            .insert_resource(EawUiAssets {
                fs: self.fs.clone(),
                text,
                game: self.game.clone(),
            })
            .insert_resource(self.settings.clone())
            .init_resource::<CommandBarData>()
            .init_resource::<ForcedTooltip>()
            .add_message::<EawButtonClicked>()
            .add_systems(Startup, widgets::spawn_tooltip)
            .add_systems(
                Update,
                (
                    widgets::ui_scale,
                    widgets::button_clicks,
                    widgets::button_visuals,
                    widgets::bound_fields,
                    widgets::slots,
                    widgets::status_bars,
                    widgets::tooltip,
                )
                    .chain(),
            );
    }
}

/// Convenience for spawning bars from systems.
pub trait EawUiCommandsExt {
    /// Queues [`spawn_command_bar`]; returns the (reserved) root entity of the bar's marker
    /// node, which receives [`EawCommandBar`] once the command is applied.
    fn spawn_eaw_command_bar(&mut self, spec: CommandBarSpec);
}

impl EawUiCommandsExt for Commands<'_, '_> {
    fn spawn_eaw_command_bar(&mut self, spec: CommandBarSpec) {
        self.queue(move |world: &mut World| {
            if spawn_command_bar(world, spec).is_none() {
                warn!("command bar could not be spawned (missing resources or shell model)");
            }
        });
    }
}



