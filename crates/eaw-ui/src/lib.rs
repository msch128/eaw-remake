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
pub mod panels;
pub mod tactical;
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
pub use panels::{
    BANNER_AUTO, BANNER_BEGIN, BannerSide, BattleBannerSpec, DialogSpec, EawDialogResult, EawMinimapClicked, EawSlotDropped, MinimapData, MinimapDot,
    MinimapKind, PanelData, PanelSlot, fleet_slot_name, spawn_battle_banner, spawn_dialog,
};
pub use tactical::{
    AbilityDef, AbilityState, EawAbilityClicked, EawHardpointClicked, EawSelectionClicked,
    HardpointMarker, RetreatState, ShipOverlay, TacticalData, TacticalUnit, ability_icon,
    spawn_unit_panel, unit_abilities, hardpoint_reticle, bar_class,
};
pub use widgets::{
    CommandBarMode, CommandBarSpec, EawButton, EawCommandBar, EawPanel, ShellAnchor, ShellSpec,
    spawn_command_bar, spawn_shell,
};

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
        let textures = textures::UiTextures::new(self.fs.clone());
        // Decode the 64 MB mega texture off the main thread.
        let slot = textures.library().mega_slot();
        let fs = self.fs.clone();
        let task = bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default)
            .spawn(async move {
                let _ = slot.set(textures::decode_mega_texture(&fs));
            });
        app.insert_resource(MegaTextureTask(Some(task)))
            .init_resource::<PendingBars>()
            .add_message::<UiTexturesLoaded>()
            .insert_resource(db)
            .insert_resource(textures)
            .insert_resource(EawUiAssets {
                fs: self.fs.clone(),
                text,
                game: self.game.clone(),
            })
            .insert_resource(self.settings.clone())
            .init_resource::<CommandBarData>()
            .init_resource::<ForcedTooltip>()
            .init_resource::<PanelData>()
            .init_resource::<TacticalData>()
            .add_message::<EawAbilityClicked>()
            .add_message::<EawSelectionClicked>()
            .add_message::<EawHardpointClicked>()
            .init_resource::<MinimapData>()
            .init_resource::<panels::DragState>()
            .add_message::<EawSlotDropped>()
            .add_message::<EawMinimapClicked>()
            .add_message::<EawDialogResult>()
            .add_message::<EawButtonClicked>()
            .add_systems(Startup, (widgets::spawn_tooltip, tactical::spawn_overlay_layer))
            .add_systems(
                Update,
                (
                    poll_textures,
                    widgets::ui_scale,
                    tactical::tactical_sync,
                    tactical::tactical_buttons,
                    tactical::button_labels,
                    tactical::tactical_clicks,
                    tactical::ship_overlays,
                    tactical::hardpoint_markers,
                    tactical::unit_panel,
                    widgets::button_clicks,
                    widgets::button_visuals,
                    widgets::bound_fields,
                    widgets::slots,
                    widgets::status_bars,
                    widgets::panel_texts,
                    panels::minimap_marks,
                    panels::minimap_clicks,
                    panels::slot_drag,
                    panels::dialog_buttons,
                    widgets::tooltip,
                )
                    .chain(),
            );
    }
}

/// Sent once the mega texture finished decoding; textures packed into it are available from now on.
#[derive(Message, Debug, Clone, Copy)]
pub struct UiTexturesLoaded;

/// Keeps the background decode task alive.
#[derive(Resource)]
struct MegaTextureTask(Option<bevy::tasks::Task<()>>);

/// Bars requested before the textures were ready; spawned by [`poll_textures`].
#[derive(Resource, Default)]
struct PendingBars(Vec<CommandBarSpec>);

fn poll_textures(
    mut task: ResMut<MegaTextureTask>,
    textures: Res<textures::UiTextures>,
    mut loaded: MessageWriter<UiTexturesLoaded>,
    mut commands: Commands,
    mut pending: ResMut<PendingBars>,
) {
    if task.0.is_none() || !textures.library().is_ready() {
        return;
    }
    task.0 = None;
    loaded.write(UiTexturesLoaded);
    for spec in pending.0.drain(..) {
        commands.queue(move |world: &mut World| {
            if spawn_command_bar(world, spec).is_none() {
                warn!("command bar could not be spawned (missing resources or shell model)");
            }
        });
    }
}

/// Convenience for spawning bars from systems.
pub trait EawUiCommandsExt {
    /// Queues [`spawn_command_bar`]; if the textures are still decoding the bar is spawned
    /// as soon as [`UiTexturesLoaded`] fires. The root receives [`EawCommandBar`].
    fn spawn_eaw_command_bar(&mut self, spec: CommandBarSpec);
}

impl EawUiCommandsExt for Commands<'_, '_> {
    fn spawn_eaw_command_bar(&mut self, spec: CommandBarSpec) {
        self.queue(move |world: &mut World| {
            let ready = world
                .get_resource::<textures::UiTextures>()
                .is_some_and(|t| t.library().is_ready());
            if !ready {
                world.resource_mut::<PendingBars>().0.push(spec);
            } else if spawn_command_bar(world, spec).is_none() {
                warn!("command bar could not be spawned (missing resources or shell model)");
            }
        });
    }
}







