//! Menus: main menu, campaign selection and the in-game pause menu.
//!
//! Screens are described as plain data ([`MenuSpec`]) and turned into Bevy UI by [`spawn_menu`];
//! that function is the one place to replace when the EaW-styled `eaw-ui` crate takes over the look.

use bevy::{app::AppExit, prelude::*};

use crate::camera::GalaxyCamera;
use crate::flow::{AppScreen, Paused, StartCampaign, UiSound};
use crate::galaxy_map::spawn_props;
use crate::game::{self, CampaignEntry, DEFAULT_CAMPAIGN, Game, GameAssets, StartupLog};
use crate::newgame::{MenuRefresh, NewGameOptions, SlotChosen, SlotMode, SlotScreen};
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
    /// Start the named campaign.
    Campaign(String),
    /// Back to the main menu.
    MainMenu,
    /// Close the pause menu.
    Resume,
    /// Quit the application.
    Quit,
    /// Leave the (placeholder) tactical battle screen.
    EndBattle,
    /// Next AI difficulty tier of a new game.
    CycleDifficulty,
    /// Switch between the Remake AI and the classic AI.
    ToggleClassicAi,
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

/// Faction colours used for the faction buttons.
fn faction_accent(faction: &str) -> [f32; 3] {
    match faction {
        "Rebel" => [1.0, 0.45, 0.4],
        "Empire" => [0.55, 0.95, 0.65],
        _ => [1.0, 0.8, 0.35],
    }
}

fn faction_label(faction: &str) -> &str {
    match faction {
        "Rebel" => "Rebel Alliance",
        "Empire" => "Galactic Empire",
        "Underworld" => "Underworld",
        other => other,
    }
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

/// The campaign selection: one column per map size, one button per human faction.
pub fn campaign_spec(campaigns: &[CampaignEntry], options: &NewGameOptions) -> MenuSpec {
    let mut columns: Vec<MenuColumn> = Vec::new();
    let mut groups: Vec<&str> = Vec::new();
    for c in campaigns {
        let idx = match groups.iter().position(|g| *g == c.group) {
            Some(i) => i,
            None => {
                groups.push(&c.group);
                columns.push(MenuColumn {
                    heading: c.group_title.clone(),
                    subheading: format!("{} planets", c.planets),
                    items: Vec::new(),
                });
                columns.len() - 1
            }
        };
        columns[idx].items.push(MenuItem {
            label: faction_label(&c.faction).to_owned(),
            action: MenuAction::Campaign(c.name.clone()),
            accent: Some(faction_accent(&c.faction)),
        });
    }
    MenuSpec {
        title: "Galactic Conquest".into(),
        subtitle: "Choose a galaxy and your side".into(),
        columns,
        footer: vec![
            MenuItem::new(options.difficulty_label(), MenuAction::CycleDifficulty),
            MenuItem::new(options.classic_label(), MenuAction::ToggleClassicAi),
            MenuItem::new("Back", MenuAction::MainMenu),
        ],
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
struct MenuButton(MenuAction);

/// Menu button was activated.
#[derive(Message)]
struct MenuClicked(MenuAction);

const PANEL_BG: Color = Color::srgba(0.02, 0.05, 0.10, 0.85);
const PANEL_BORDER: Color = Color::srgba(0.35, 0.65, 1.0, 0.8);
const BUTTON_IDLE: Color = Color::srgba(0.06, 0.12, 0.2, 0.9);
const BUTTON_HOVER: Color = Color::srgba(0.15, 0.32, 0.5, 0.95);
const BUTTON_PRESSED: Color = Color::srgba(0.25, 0.5, 0.75, 1.0);

/// Menu screens and their input handling.
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Campaigns>()
            .add_message::<MenuClicked>()
            .add_systems(OnEnter(AppScreen::MainMenu), (spawn_main_menu, spawn_backdrop))
            .add_systems(OnEnter(AppScreen::CampaignSelect), (spawn_campaign_menu, spawn_backdrop))
            .add_systems(OnEnter(AppScreen::Galaxy), despawn_backdrop)
            .add_systems(OnExit(AppScreen::MainMenu), despawn_screen_menu)
            .add_systems(OnExit(AppScreen::CampaignSelect), despawn_screen_menu)
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

fn spawn_main_menu(mut commands: Commands) {
    spawn_menu(&mut commands, &main_menu_spec(), ScreenMenu);
}

fn spawn_campaign_menu(mut commands: Commands, campaigns: Res<Campaigns>, options: Res<NewGameOptions>) {
    spawn_menu(&mut commands, &campaign_spec(&campaigns.0, &options), ScreenMenu);
}

/// Marks the 3D scene behind the menus: the galaxy art with its star dome.
#[derive(Component, Clone)]
pub struct MenuBackdrop;

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
fn backdrop_camera(time: Res<Time>, mut camera: Single<&mut Transform, With<GalaxyCamera>>) {
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
                row_gap: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(background),
            GlobalZIndex(20),
            marker,
        ))
        .with_children(|root| {
            root.spawn((
                Text::new(spec.title.clone()),
                TextFont { font_size: FontSize::Px(54.0), ..default() },
                TextColor(Color::srgb(0.85, 0.93, 1.0)),
                TextShadow { offset: Vec2::splat(2.0), color: Color::srgba(0.0, 0.1, 0.3, 0.9) },
            ));
            if !spec.subtitle.is_empty() {
                root.spawn((
                    Text::new(spec.subtitle.clone()),
                    TextFont { font_size: FontSize::Px(22.0), ..default() },
                    TextColor(Color::srgb(0.75, 0.86, 1.0)),
                    TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.1, 0.95) },
                ));
            }
            root.spawn(Node {
                margin: UiRect::top(Val::Px(24.0)),
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
                row_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(16.0)),
                border: UiRect::all(Val::Px(1.0)),
                min_width: Val::Px(240.0),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(PANEL_BORDER),
        ))
        .with_children(|col| {
            if !column.heading.is_empty() {
                col.spawn((
                    Text::new(column.heading.clone()),
                    TextFont { font_size: FontSize::Px(26.0), ..default() },
                    TextColor(Color::WHITE),
                ));
            }
            if !column.subheading.is_empty() {
                col.spawn((
                    Text::new(column.subheading.clone()),
                    TextFont { font_size: FontSize::Px(15.0), ..default() },
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
                min_width: Val::Px(210.0),
                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(BUTTON_IDLE),
            BorderColor::all(PANEL_BORDER),
            MenuButton(item.action.clone()),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(item.label.clone()),
                TextFont { font_size: FontSize::Px(20.0), ..default() },
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
    mut start: MessageWriter<StartCampaign>,
    mut exit: MessageWriter<AppExit>,
    mut newgame: ResMut<NewGameOptions>,
    mut slots: ResMut<SlotScreen>,
    mut chosen: MessageWriter<SlotChosen>,
    mut refresh: MessageWriter<MenuRefresh>,
) {
    for MenuClicked(action) in clicks.read() {
        match action {
            MenuAction::OpenCampaigns => next.set(AppScreen::CampaignSelect),
            MenuAction::Options => options.0 = true,
            MenuAction::Campaign(name) => {
                start.write(StartCampaign(name.clone()));
            }
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
            MenuAction::CycleDifficulty => {
                newgame.cycle();
                refresh.write(MenuRefresh);
            }
            MenuAction::ToggleClassicAi => {
                newgame.classic_ai = !newgame.classic_ai;
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
    campaigns: Res<Campaigns>,
    options: Res<NewGameOptions>,
) {
    if refresh.read().last().is_none() || *screen.get() != AppScreen::CampaignSelect {
        return;
    }
    for e in &menus {
        commands.entity(e).despawn();
    }
    spawn_menu(&mut commands, &campaign_spec(&campaigns.0, &options), ScreenMenu);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, group: &str, faction: &str) -> CampaignEntry {
        CampaignEntry {
            name: name.into(),
            title: name.into(),
            group: group.into(),
            group_title: group.into(),
            faction: faction.into(),
            planets: 100,
        }
    }

    #[test]
    fn campaign_spec_groups_factions_into_columns() {
        let spec = campaign_spec(&[
            entry("a_rebel", "A", "Rebel"),
            entry("a_empire", "A", "Empire"),
            entry("b_rebel", "B", "Rebel"),
        ], &NewGameOptions::default());
        assert_eq!(spec.columns.len(), 2);
        assert_eq!(spec.columns[0].items.len(), 2);
        assert_eq!(spec.columns[0].items[1].label, "Galactic Empire");
        assert_eq!(spec.columns[1].items[0].action, MenuAction::Campaign("b_rebel".into()));
        assert_eq!(spec.footer.len(), 3);
    }
}
