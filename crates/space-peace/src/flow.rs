//! Application flow: screens (main menu, campaign selection, galaxy map), pause, campaign
//! start, music scenes and UI sounds.

use alamo_audio::{MusicDirector, MusicScene, SfxCommands, plugin::AlamoAudio};
use bevy::prelude::*;

use crate::galaxy_view::GalaxyView;
use crate::game::{self, DEMO_CAMPAIGN, Game, StartupLog};

/// The screen the application is on.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppScreen {
    /// Title screen.
    #[default]
    MainMenu,
    /// Galaxy and faction selection.
    CampaignSelect,
    /// The galactic conquest map.
    Galaxy,
}

/// True while the pause menu is open on the galaxy map.
#[derive(Resource, Default, Debug)]
pub struct Paused(pub bool);

/// Run condition: the game is not paused.
pub fn not_paused(paused: Res<Paused>) -> bool {
    !paused.0
}

/// The campaign being played.
#[derive(Resource, Default, Debug, Clone)]
pub struct CurrentCampaign {
    /// Campaign name.
    pub name: Option<String>,
    /// Human faction (`Rebel`, `Empire`, `Underworld`).
    pub faction: Option<String>,
}

/// Request to build the named campaign and switch to the galaxy map.
#[derive(Message, Debug, Clone)]
pub struct StartCampaign(pub String);

/// Request to play a GUI sound event; played by `alamo-audio` when it is active.
#[derive(Message, Debug, Clone, Copy)]
pub struct UiSound(pub &'static str);

/// Wires screens, pause handling, campaign start, music and UI sounds.
pub struct FlowPlugin;

impl Plugin for FlowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Paused>()
            .init_resource::<CurrentCampaign>()
            .add_message::<StartCampaign>()
            .add_message::<UiSound>()
            .add_systems(
                Update,
                (
                    escape_key,
                    start_campaign,
                    play_ui_sounds.run_if(resource_exists::<AlamoAudio>),
                ),
            )
            .add_systems(OnEnter(AppScreen::MainMenu), menu_music)
            .add_systems(OnEnter(AppScreen::CampaignSelect), menu_music)
            .add_systems(OnEnter(AppScreen::Galaxy), galaxy_music)
            .add_systems(OnExit(AppScreen::Galaxy), unpause);
    }
}

/// Esc: pause menu on the map, back from the campaign list.
fn escape_key(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<AppScreen>>,
    mut next: ResMut<NextState<AppScreen>>,
    mut paused: ResMut<Paused>,
    mut sound: MessageWriter<UiSound>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    match state.get() {
        AppScreen::Galaxy => {
            paused.0 = !paused.0;
            sound.write(UiSound(crate::menu::SFX_BACK));
        }
        AppScreen::CampaignSelect => {
            next.set(AppScreen::MainMenu);
            sound.write(UiSound(crate::menu::SFX_BACK));
        }
        AppScreen::MainMenu => {}
    }
}

fn unpause(mut paused: ResMut<Paused>) {
    paused.0 = false;
}

/// Builds the requested campaign and enters the galaxy map.
fn start_campaign(
    mut requests: MessageReader<StartCampaign>,
    game: Res<Game>,
    mut commands: Commands,
    mut next: ResMut<NextState<AppScreen>>,
    mut current: ResMut<CurrentCampaign>,
) {
    let Some(StartCampaign(name)) = requests.read().last() else {
        return;
    };
    let mut log = StartupLog::default();
    let result = if name == DEMO_CAMPAIGN {
        Ok((GalaxyView::demo(), Some("Rebel".to_owned())))
    } else if let Some(assets) = &game.0 {
        game::build_campaign(assets, name, &mut log)
    } else {
        Err("no game installed".to_owned())
    };
    log.emit();
    match result {
        Ok((view, faction)) => {
            commands.insert_resource(view);
            current.name = Some(name.clone());
            current.faction = faction;
            next.set(AppScreen::Galaxy);
        }
        Err(e) => error!("cannot start campaign `{name}`: {e}"),
    }
}

fn menu_music(director: Option<ResMut<MusicDirector>>) {
    if let Some(mut d) = director {
        d.set_scene(MusicScene::MainMenu);
    }
}

fn galaxy_music(director: Option<ResMut<MusicDirector>>, current: Res<CurrentCampaign>) {
    if let Some(mut d) = director {
        let faction = current.faction.clone().unwrap_or_else(|| "Rebel".to_owned());
        d.set_scene(MusicScene::Galactic(faction));
    }
}

fn play_ui_sounds(mut requests: MessageReader<UiSound>, mut sfx: SfxCommands) {
    for s in requests.read() {
        sfx.play(s.0);
    }
}
