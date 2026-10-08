//! Menus: main menu, campaign selection and the in-game pause menu.
//!
//! Screens are described as plain data ([`MenuSpec`]) and turned into Bevy UI by [`spawn_menu`];
//! that function is the one place to replace when the EaW-styled `eaw-ui` crate takes over the look.

use bevy::{app::AppExit, prelude::*};

use crate::camera::GalaxyCamera;
use crate::flow::{AppScreen, Paused, UiSound};
use crate::galaxy_map::spawn_props;
use crate::game::{self, CampaignEntry, DEFAULT_CAMPAIGN, Game, GameAssets, StartupLog, StartupProblem};
use crate::newgame::{MenuRefresh, SlotChosen, SlotMode, SlotScreen};
use crate::options::OptionsOpen;

/// SFX event played when the cursor enters a button.
pub const SFX_HOVER: &str = "GUI_Common_Dialog_Button_Hilight";
/// SFX event played when a button is pressed.
pub const SFX_CLICK: &str = "GUI_Common_Dialog_Button_Press";
/// SFX event played when leaving a screen or closing a menu.
pub const SFX_BACK: &str = "GUI_Cancel_SFX";
/// SFX event played when a planet is selected on the map.
pub const SFX_PLANET_SELECT: &str = "GUI_Planet_Menu";
/// SFX event played when the cursor moves onto a planet.
pub const SFX_PLANET_HOVER: &str = "GUI_Planet_Mouse_Over";

/// What a menu button does.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    /// Open the campaign selection.
    OpenCampaigns,
    /// Open the options overlay.
    Options,
    /// Back to the main menu.
    MainMenu,
    /// Close the pause menu.
    Resume,
    /// Quit the application.
    Quit,
    /// Leave the (placeholder) tactical battle screen.
    EndBattle,
    /// Dismiss the startup problem and continue with the demo galaxy.
    AckProblem,
    /// Open the save or load slot list.
    OpenSlots(SlotMode),
    /// A slot button.
    Slot(SlotMode, u8),
    /// Close the slot list.
    CloseSlots,
}

/// One button.
#[derive(Debug, Clone)]
pub struct MenuItem {
    /// Button text.
    pub label: String,
    /// Action on click.
    pub action: MenuAction,
    /// Accent colour (sRGB) of the label, if any.
    pub accent: Option<[f32; 3]>,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, action: MenuAction) -> Self {
        Self { label: label.into(), action, accent: None }
    }
}

/// A titled column of buttons.
#[derive(Debug, Clone, Default)]
pub struct MenuColumn {
    /// Heading above the buttons (may be empty).
    pub heading: String,
    /// Smaller line below the heading (may be empty).
    pub subheading: String,
    /// Buttons, top to bottom.
    pub items: Vec<MenuItem>,
}

/// Description of a menu screen.
#[derive(Debug, Clone, Default)]
pub struct MenuSpec {
    /// Large title.
    pub title: String,
    /// Line below the title.
    pub subtitle: String,
    /// Button columns, left to right.
    pub columns: Vec<MenuColumn>,
    /// Buttons below the columns (e.g. Back).
    pub footer: Vec<MenuItem>,
    /// Darken the screen behind the menu (pause menu).
    pub dim_background: bool,
}



/// The title screen.
pub fn main_menu_spec() -> MenuSpec {
    MenuSpec {
        title: "SPACE PEACE".into(),
        subtitle: "Not at War".into(),
        columns: vec![MenuColumn {
            items: vec![
                MenuItem::new("Galactic Conquest", MenuAction::OpenCampaigns),
                MenuItem::new("Load Game", MenuAction::OpenSlots(SlotMode::Load)),
                MenuItem::new("Options", MenuAction::Options),
                MenuItem::new("Quit", MenuAction::Quit),
            ],
            ..default()
        }],
        ..default()
    }
}


/// Shown instead of the title screen when the game files cannot be used.
pub fn problem_spec(message: &str) -> MenuSpec {
    MenuSpec {
        title: "Cannot find the game".into(),
        subtitle: message.to_owned(),
        columns: vec![MenuColumn {
            items: vec![MenuItem::new("Continue with the demo galaxy", MenuAction::AckProblem), MenuItem::new("Quit", MenuAction::Quit)],
            ..default()
        }],
        ..default()
    }
}

/// The pause menu.
pub fn pause_spec() -> MenuSpec {
    MenuSpec {
        title: "Paused".into(),
        columns: vec![MenuColumn {
            items: vec![
                MenuItem::new("Resume", MenuAction::Resume),
                MenuItem::new("Save Game", MenuAction::OpenSlots(SlotMode::Save)),
                MenuItem::new("Load Game", MenuAction::OpenSlots(SlotMode::Load)),
                MenuItem::new("Options", MenuAction::Options),
                MenuItem::new("Main Menu", MenuAction::MainMenu),
                MenuItem::new("Quit", MenuAction::Quit),
            ],
            ..default()
        }],
        dim_background: true,
        ..default()
    }
}

/// Campaigns offered on the selection screen.
#[derive(Resource, Default, Debug)]
pub struct Campaigns(pub Vec<CampaignEntry>);

/// Root entity of the menu shown for the current screen.
#[derive(Component)]
struct ScreenMenu;

/// Root entity of the pause menu.
#[derive(Component)]
struct PauseMenu;

/// A clickable menu button.
#[derive(Component)]
pub(crate) struct MenuButton(pub(crate) MenuAction);

/// Menu button was activated.
#[derive(Message)]
struct MenuClicked(MenuAction);

const PANEL_BG: Color = Color::srgba(0.03, 0.05, 0.12, 0.9);
const PANEL_BORDER: Color = Color::srgb(0.42, 0.58, 0.88);
const BUTTON_BORDER: Color = Color::srgb(0.6, 0.65, 0.75);
const BUTTON_IDLE: Color = Color::srgb(0.01, 0.01, 0.03);
const BUTTON_HOVER: Color = Color::srgb(0.1, 0.13, 0.24);
const BUTTON_PRESSED: Color = Color::srgb(0.2, 0.28, 0.5);

/// Menu screens and their input handling.
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Campaigns>()
            .add_message::<MenuClicked>()
            .add_systems(OnEnter(AppScreen::MainMenu), (spawn_main_menu, spawn_backdrop_unless_scene))
            .add_systems(OnEnter(AppScreen::CampaignSelect), spawn_backdrop)
            .add_systems(OnEnter(AppScreen::Galaxy), despawn_backdrop)
            .add_systems(OnEnter(AppScreen::Galaxy), sync_pause_menu)
            .add_systems(OnExit(AppScreen::MainMenu), despawn_screen_menu)
            .add_systems(OnExit(AppScreen::Galaxy), despawn_pause_menu)
            .add_systems(
                Update,
                backdrop_camera.run_if(not(in_state(AppScreen::Galaxy))),
            )
            .add_systems(
                Update,
                (
                    sync_pause_menu
                        .run_if(resource_changed::<Paused>)
                        .run_if(in_state(AppScreen::Galaxy)),
                    button_interaction,
                    apply_clicks,
                    refresh_campaign_menu,
                )
                    .chain(),
            );
    }
}

pub(crate) fn spawn_main_menu(mut commands: Commands, problem: Res<StartupProblem>) {
    match &problem.0 {
        Some(m) => spawn_menu(&mut commands, &problem_spec(m), ScreenMenu),
        None => spawn_menu(&mut commands, &main_menu_spec(), ScreenMenu),
    };
}


/// Marks the 3D scene behind the menus: the galaxy art with its star dome.
#[derive(Component, Clone)]
pub struct MenuBackdrop;

/// The galaxy art stays away while the title scene runs (it is spawned right after this system on the main menu).
fn spawn_backdrop_unless_scene(commands: Commands, game: Res<Game>, existing: Query<(), With<MenuBackdrop>>) {
    if game.0.is_some() {
        return;
    }
    spawn_backdrop(commands, game, existing);
}

/// Spawns the galaxy art behind the menus (once; it stays while moving between menu screens).
fn spawn_backdrop(
    mut commands: Commands,
    game: Res<Game>,
    existing: Query<(), With<MenuBackdrop>>,
) {
    if !existing.is_empty() {
        return;
    }
    if let Some(assets) = &game.0 {
        spawn_backdrop_props(&mut commands, assets);
    }
}

/// Spawns the galaxy art (disc, nebulae, star dome; no planets) of the default campaign.
pub fn spawn_backdrop_props(commands: &mut Commands, assets: &GameAssets) {
    if let Ok((view, _)) = game::build_campaign(assets, DEFAULT_CAMPAIGN, &mut StartupLog::default()) {
        spawn_props(commands, &view.props, MenuBackdrop);
    }
}

fn despawn_backdrop(mut commands: Commands, backdrop: Query<Entity, With<MenuBackdrop>>) {
    for e in &backdrop {
        commands.entity(e).despawn();
    }
}

/// Slowly orbits the camera around the galaxy while a menu is shown.
fn backdrop_camera(time: Res<Time>, scene: Option<Res<crate::menu_scene::MenuSceneInfo>>, mut camera: Single<&mut Transform, With<GalaxyCamera>>) {
    if scene.is_some() {
        return;
    }
    let angle = time.elapsed_secs() * 0.03;
    let eye = Quat::from_rotation_y(angle) * Vec3::new(0.0, 1000.0, 1250.0);
    **camera = Transform::from_translation(eye).looking_at(Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
}

fn despawn_screen_menu(mut commands: Commands, menus: Query<Entity, With<ScreenMenu>>) {
    for e in &menus {
        commands.entity(e).despawn();
    }
}

fn despawn_pause_menu(mut commands: Commands, menus: Query<Entity, With<PauseMenu>>) {
    for e in &menus {
        commands.entity(e).despawn();
    }
}

/// Opens or closes the pause menu when [`Paused`] changes.
fn sync_pause_menu(
    mut commands: Commands,
    paused: Res<Paused>,
    menus: Query<Entity, With<PauseMenu>>,
) {
    for e in &menus {
        commands.entity(e).despawn();
    }
    if paused.0 {
        spawn_menu(&mut commands, &pause_spec(), PauseMenu);
    }
}

/// Builds the UI of `spec` below a full-screen root carrying `marker`.
///
/// This is the single replacement point for a different menu look.
pub fn spawn_menu(commands: &mut Commands, spec: &MenuSpec, marker: impl Component) -> Entity {
    // A single column carries the title inside its frame, like the original dialogs.
    let mut spec = spec.clone();
    let title_inside = spec.columns.len() == 1 && spec.columns[0].heading.is_empty();
    if title_inside {
        spec.columns[0].heading = std::mem::take(&mut spec.title);
    }
    let spec = &spec;
    let background = if spec.dim_background {
        Color::srgba(0.0, 0.0, 0.03, 0.6)
    } else {
        Color::NONE
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(background),
            GlobalZIndex(60),
            marker,
        ))
        .with_children(|root| {
            if !spec.title.is_empty() {
            root.spawn((
                Text::new(spec.title.to_uppercase()),
                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(26.0), ..default() },
                TextColor(Color::srgb(0.95, 0.96, 0.98)),
                TextShadow { offset: Vec2::splat(2.0), color: Color::srgba(0.0, 0.1, 0.3, 0.9) },
            ));
            }
            if !spec.subtitle.is_empty() {
                root.spawn((
                    Text::new(spec.subtitle.clone()),
                    TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(13.0), ..default() },
                    TextColor(Color::srgb(0.75, 0.86, 1.0)),
                    TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.1, 0.95) },
                ));
            }
            root.spawn(Node {
                margin: UiRect::top(Val::Px(14.0)),
                column_gap: Val::Px(26.0),
                align_items: AlignItems::FlexStart,
                ..default()
            })
            .with_children(|row| {
                for column in &spec.columns {
                    spawn_column(row, column);
                }
            });
            if !spec.footer.is_empty() {
                root.spawn(Node {
                    margin: UiRect::top(Val::Px(18.0)),
                    column_gap: Val::Px(14.0),
                    ..default()
                })
                .with_children(|row| {
                    for item in &spec.footer {
                        spawn_button(row, item);
                    }
                });
            }
        })
        .id()
}

fn spawn_column(parent: &mut ChildSpawnerCommands, column: &MenuColumn) {
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(7.0),
                padding: UiRect::all(Val::Px(14.0)),
                border: UiRect::all(Val::Px(2.0)),
                min_width: Val::Px(200.0),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(PANEL_BORDER),
            BoxShadow::new(Color::srgba(0.3, 0.5, 1.0, 0.4), Val::Px(0.0), Val::Px(0.0), Val::Px(3.0), Val::Px(14.0)),
        ))
        .with_children(|col| {
            if !column.heading.is_empty() {
                col.spawn((
                    Text::new(column.heading.to_uppercase()),
                    TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(18.0), ..default() },
                    TextColor(Color::WHITE),
                ));
            }
            if !column.subheading.is_empty() {
                col.spawn((
                    Text::new(column.subheading.clone()),
                    TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(15.0), ..default() },
                    TextColor(Color::srgb(0.6, 0.72, 0.85)),
                ));
            }
            for item in &column.items {
                spawn_button(col, item);
            }
        });
}

fn spawn_button(parent: &mut ChildSpawnerCommands, item: &MenuItem) {
    let label_color = item.accent.map_or(Color::WHITE, |c| Color::srgb(c[0], c[1], c[2]));
    parent
        .spawn((
            Button,
            Node {
                min_width: Val::Px(190.0),
                padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)),
                border: UiRect::all(Val::Px(1.0)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(BUTTON_IDLE),
            BorderColor::all(BUTTON_BORDER),
            MenuButton(item.action.clone()),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(item.label.to_uppercase()),
                TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(14.5), ..default() },
                TextColor(label_color),
            ));
        });
}

/// Hover / press feedback, sounds and click messages.
fn button_interaction(
    mut buttons: Query<
        (&Interaction, &MenuButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    mut clicks: MessageWriter<MenuClicked>,
    mut sounds: MessageWriter<UiSound>,
) {
    for (interaction, button, mut bg) in &mut buttons {
        match interaction {
            Interaction::Pressed => {
                bg.0 = BUTTON_PRESSED;
                sounds.write(UiSound(SFX_CLICK));
                clicks.write(MenuClicked(button.0.clone()));
            }
            Interaction::Hovered => {
                bg.0 = BUTTON_HOVER;
                sounds.write(UiSound(SFX_HOVER));
            }
            Interaction::None => bg.0 = BUTTON_IDLE,
        }
    }
}

/// Carries out the clicked button's action.
#[allow(clippy::too_many_arguments)]
fn apply_clicks(
    request: Option<Res<crate::battle_seam::BattleRequest>>,
    mut finished: MessageWriter<crate::battle_seam::BattleFinished>,
    mut clicks: MessageReader<MenuClicked>,
    mut next: ResMut<NextState<AppScreen>>,
    mut paused: ResMut<Paused>,
    mut options: ResMut<OptionsOpen>,
    mut exit: MessageWriter<AppExit>,
    mut slots: ResMut<SlotScreen>,
    mut chosen: MessageWriter<SlotChosen>,
    mut refresh: MessageWriter<MenuRefresh>,
    mut problem: ResMut<StartupProblem>,
) {
    for MenuClicked(action) in clicks.read() {
        match action {
            MenuAction::OpenCampaigns => next.set(AppScreen::CampaignSelect),
            MenuAction::Options => options.0 = true,
            MenuAction::MainMenu => {
                paused.0 = false;
                next.set(AppScreen::MainMenu);
            }
            MenuAction::Resume => paused.0 = false,
            MenuAction::EndBattle => {
                finished.write(crate::battle_seam::BattleFinished {
                    id: request.as_ref().map_or(0, |r| r.id),
                    outcome: crate::battle_seam::BattleOutcome::Aborted,
                });
            }
            MenuAction::AckProblem => {
                problem.0 = None;
                refresh.write(MenuRefresh);
            }
            MenuAction::OpenSlots(mode) => slots.0 = Some(*mode),
            MenuAction::Slot(mode, n) => {
                chosen.write(SlotChosen(*mode, *n));
            }
            MenuAction::CloseSlots => slots.0 = None,
            MenuAction::Quit => {
                exit.write(AppExit::Success);
            }
        }
    }
}

/// Rebuilds the campaign list when its option buttons changed their labels.
fn refresh_campaign_menu(
    mut refresh: MessageReader<MenuRefresh>,
    mut commands: Commands,
    screen: Res<State<AppScreen>>,
    menus: Query<Entity, With<ScreenMenu>>,
    problem: Res<StartupProblem>,
) {
    if refresh.read().last().is_none() {
        return;
    }
    let spec = match *screen.get() {
        AppScreen::MainMenu => match &problem.0 {
            Some(m) => problem_spec(m),
            None => main_menu_spec(),
        },
        _ => return,
    };
    for e in &menus {
        commands.entity(e).despawn();
    }
    spawn_menu(&mut commands, &spec, ScreenMenu);
}

