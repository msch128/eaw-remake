//! The Galactic Conquest set-up screen, laid out like the original: campaign list with set headers,
//! description text, "Choose your side" drop-down, difficulty radios and Cancel / Advanced / Start Game.
//!
//! The list comes from the game data: campaigns sharing a `Sort_Order` are one row (one campaign per
//! playable side), rows without a galaxy are set headers that only show their description.

use alamo_data::GameData;
use bevy::prelude::*;
use galaxy_script::DifficultyTier;

use crate::flow::{AppScreen, StartCampaign, UiSound};
use crate::game::Game;
use crate::menu::Campaigns;
use crate::newgame::NewGameOptions;

const BLUE: Color = Color::srgb(0.32, 0.52, 0.93);
const WHITE: Color = Color::srgb(0.95, 0.96, 0.98);
const DIM: Color = Color::srgb(0.38, 0.42, 0.52);
const PANEL: Color = Color::srgba(0.03, 0.05, 0.12, 0.84);
const BOX: Color = Color::srgba(0.02, 0.03, 0.07, 0.92);
const FRAME: Color = Color::srgb(0.42, 0.58, 0.88);
const SELECTED: Color = Color::srgba(0.58, 0.6, 0.64, 0.95);

/// One line of the campaign list.
#[derive(Debug, Clone, PartialEq)]
pub struct SetRow {
    /// Title shown in the list.
    pub title: String,
    /// Description text.
    pub description: String,
    /// Playable sides: faction key (`Rebel`, `Empire`, `Underworld`) and campaign name. Empty for headers.
    pub sides: Vec<(String, String)>,
}

/// Rows of the campaign list in the original's order.
pub fn rows_from_data(data: &GameData) -> Vec<SetRow> {
    let mut campaigns: Vec<&alamo_data::Campaign> = data.campaigns.iter().collect();
    campaigns.sort_by_key(|c| c.sort_order);
    let mut rows: Vec<(i32, SetRow)> = Vec::new();
    for c in campaigns {
        let title = c.text_id.as_deref().and_then(|id| data.text(id)).unwrap_or("").trim().to_owned();
        if title.is_empty() || title == "-" || c.name.to_ascii_lowercase().contains("obs") || c.name.to_ascii_lowercase().contains("_ai") {
            continue;
        }
        let description = c.description_id.as_deref().and_then(|id| data.text(id)).unwrap_or("").to_owned();
        let side = match c.starting_active_player.as_deref() {
            Some("Rebel") => Some("Rebel"),
            Some("Empire") => Some("Empire"),
            Some("Underworld") => Some("Underworld"),
            _ => None,
        };
        let playable = c.has_galaxy().then_some(side).flatten();
        if let Some(side) = playable {
            if let Some((_, row)) = rows.iter_mut().find(|(s, r)| *s == c.sort_order && !r.sides.is_empty()) {
                row.sides.push((side.to_owned(), c.name.clone()));
                continue;
            }
            rows.push((c.sort_order, SetRow { title, description, sides: vec![(side.to_owned(), c.name.clone())] }));
        } else if !c.has_galaxy() {
            rows.push((c.sort_order, SetRow { title, description, sides: Vec::new() }));
        }
    }
    rows.into_iter().map(|(_, r)| r).collect()
}

/// Rows without game data: the entries of the plain campaign list.
fn rows_from_entries(entries: &[crate::game::CampaignEntry]) -> Vec<SetRow> {
    let mut rows: Vec<SetRow> = Vec::new();
    for e in entries {
        let title = format!("{} ({} planets)", e.group_title, e.planets);
        match rows.iter_mut().find(|r| r.title == title) {
            Some(r) => r.sides.push((e.faction.clone(), e.name.clone())),
            None => rows.push(SetRow { title, description: String::new(), sides: vec![(e.faction.clone(), e.name.clone())] }),
        }
    }
    rows
}

/// State of the screen.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct SetupState {
    /// List rows.
    pub rows: Vec<SetRow>,
    /// Selected row.
    pub selected: usize,
    /// Chosen side (index into the row's sides).
    pub side: Option<usize>,
    /// Side drop-down open.
    pub dropdown: bool,
    /// The advanced options replace the description.
    pub advanced: bool,
}

#[derive(Component)]
struct SetupRoot;

#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum Click {
    Row(usize),
    SideToggle,
    Side(usize),
    Difficulty(DifficultyTier),
    Cancel,
    Advanced,
    Start,
    ClassicAi,
}

/// The set-up screen.
pub struct CampaignSelectPlugin;

impl Plugin for CampaignSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SetupState>()
            .add_systems(OnEnter(AppScreen::CampaignSelect), enter)
            .add_systems(OnExit(AppScreen::CampaignSelect), leave)
            .add_systems(OnEnter(AppScreen::Galaxy), restore_scale)
            .add_systems(Update, screen_scale.run_if(in_state(AppScreen::CampaignSelect).or_else(in_state(AppScreen::MainMenu))))
            .add_systems(
                Update,
                (clicks, respawn.run_if(resource_changed::<SetupState>.or_else(resource_changed::<NewGameOptions>))).chain().run_if(in_state(AppScreen::CampaignSelect)),
            );
    }
}

fn enter(mut state: ResMut<SetupState>, game: Res<Game>, campaigns: Res<Campaigns>) {
    let rows = match &game.0 {
        Some(g) => rows_from_data(&g.data),
        None => Vec::new(),
    };
    let rows = if rows.iter().any(|r| !r.sides.is_empty()) { rows } else { rows_from_entries(&campaigns.0) };
    let first = rows.iter().position(|r| !r.sides.is_empty()).unwrap_or(0);
    *state = SetupState { rows, selected: first, ..default() };
}

fn leave(mut commands: Commands, roots: Query<Entity, With<SetupRoot>>) {
    for e in &roots {
        commands.entity(e).despawn();
    }
}

fn font(size: f32) -> TextFont {
    TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(size), ..default() }
}

fn tx(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (Text::new(text), font(size), TextColor(color))
}

fn abs(left: f32, top: f32, width: f32, height: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(left),
        top: Val::Percent(top),
        width: Val::Percent(width),
        height: Val::Percent(height),
        ..default()
    }
}

fn game_text(game: &Game, id: &str, fallback: &str) -> String {
    game.0.as_ref().and_then(|g| g.data.text(id)).filter(|t| !t.is_empty()).unwrap_or(fallback).to_owned()
}

fn side_label(game: &Game, key: &str) -> String {
    match key {
        "Rebel" => game_text(game, "TEXT_FACTION_REBELS", "Rebellion"),
        "Empire" => "Empire".to_owned(),
        _ => game_text(game, "TEXT_FACTION_UNDERWORLD_NICKNAME", "Criminal Underworld"),
    }
}

fn respawn(mut commands: Commands, roots: Query<Entity, With<SetupRoot>>, state: Res<SetupState>, options: Res<NewGameOptions>, game: Res<Game>) {
    for e in &roots {
        commands.entity(e).despawn();
    }
    spawn_screen(&mut commands, &state, &options, &game);
}

fn spawn_screen(commands: &mut Commands, state: &SetupState, options: &NewGameOptions, game: &Game) {
    let row = state.rows.get(state.selected);
    let can_start = row.is_some_and(|r| state.side.is_some_and(|s| s < r.sides.len()));
    commands
        .spawn((
            SetupRoot,
            Node { position_type: PositionType::Absolute, left: Val::Percent(7.5), top: Val::Percent(11.8), width: Val::Percent(85.0), height: Val::Percent(75.8), border: UiRect::all(Val::Px(2.0)), ..default() },
            BackgroundColor(PANEL),
            BorderColor::all(FRAME),
            BoxShadow::new(Color::srgba(0.3, 0.5, 1.0, 0.45), Val::Px(0.0), Val::Px(0.0), Val::Px(4.0), Val::Px(18.0)),
            GlobalZIndex(20),
        ))
        .with_children(|f| {
            // Title.
            f.spawn((
                Node { position_type: PositionType::Absolute, left: Val::Px(0.0), right: Val::Px(0.0), top: Val::Percent(4.0), justify_content: JustifyContent::Center, ..default() },
                children![tx("GALACTIC CONQUEST", 21.0, WHITE)],
            ));
            // Campaign list.
            f.spawn((BackgroundColor(BOX), Node { flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(3.0)), overflow: Overflow::clip_y(), ..abs(4.4, 12.1, 24.6, 59.9) }))
                .with_children(|list| {
                    for (i, r) in state.rows.iter().enumerate() {
                        let selected = i == state.selected;
                        list.spawn((
                            Button,
                            Click::Row(i),
                            Node { padding: UiRect::axes(Val::Px(7.0), Val::Px(1.0)), ..default() },
                            BackgroundColor(if selected { SELECTED } else { Color::NONE }),
                            children![tx(r.title.clone(), 14.5, WHITE)],
                        ));
                    }
                });
            // Description box.
            f.spawn((BackgroundColor(BOX), BorderColor::all(Color::srgba(0.3, 0.45, 0.8, 0.5)), Node { border: UiRect::all(Val::Px(1.0)), padding: UiRect::all(Val::Px(8.0)), flex_direction: FlexDirection::Column, ..abs(30.0, 12.1, 66.5, 57.8) }))
                .with_children(|d| {
                    if state.advanced {
                        d.spawn(tx("ADVANCED OPTIONS", 12.5, WHITE));
                        let classic = options.classic_ai;
                        let radio = |on: bool| if on { "[x] " } else { "[ ] " };
                        for (label, click, on) in [
                            ("Brutal difficulty (custom)", Click::Difficulty(DifficultyTier::Brutal), options.difficulty == DifficultyTier::Brutal && !classic),
                            ("Fair difficulty (custom)", Click::Difficulty(DifficultyTier::Fair), options.difficulty == DifficultyTier::Fair && !classic),
                            ("Classic AI (the engine's built-in AI, no difficulty tiers)", Click::ClassicAi, classic),
                        ] {
                            d.spawn((Button, click, Node { margin: UiRect::top(Val::Px(8.0)), ..default() }, children![tx(format!("{}{label}", radio(on)), 13.0, BLUE)]));
                        }
                    } else if let Some(r) = row {
                        d.spawn(tx(r.description.replace("\\n", "\n").to_uppercase(), 12.5, BLUE));
                    }
                });
            // Side and difficulty.
            f.spawn((Node { justify_content: JustifyContent::Center, ..abs(38.9, 57.0, 22.7, 4.0) }, children![tx("CHOOSE YOUR SIDE", 13.0, WHITE)]));
            let side_text = row.and_then(|r| state.side.and_then(|s| r.sides.get(s))).map(|(k, _)| side_label(game, k)).unwrap_or_default();
            let drop_node = Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)), border: UiRect::all(Val::Px(1.0)), align_items: AlignItems::Center, justify_content: JustifyContent::SpaceBetween, ..abs(38.9, 61.5, 22.7, 4.7) };
            if row.is_some_and(|r| !r.sides.is_empty()) {
                f.spawn((Button, Click::SideToggle, drop_node, BackgroundColor(BOX), BorderColor::all(Color::srgb(0.25, 0.4, 0.85)), children![tx(side_text, 13.5, WHITE), tx("{25BC}", 11.0, BLUE)]));
            } else {
                f.spawn((drop_node, BackgroundColor(BOX), BorderColor::all(Color::srgb(0.15, 0.2, 0.35))));
            }
            if state.dropdown
                && let Some(r) = row
            {
                f.spawn((
                    Node { position_type: PositionType::Absolute, left: Val::Percent(38.9), top: Val::Percent(66.4), width: Val::Percent(22.7), flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(3.0)), ..default() },
                    BackgroundColor(BOX),
                    GlobalZIndex(25),
                ))
                .with_children(|l| {
                    for (i, (k, _)) in r.sides.iter().enumerate() {
                        l.spawn((Button, Click::Side(i), Node { padding: UiRect::axes(Val::Px(7.0), Val::Px(1.0)), ..default() }, children![tx(side_label(game, k), 14.0, WHITE)]));
                    }
                });
            }
            f.spawn((children![tx(game_text(game, "TEXT_CAMPAIGN_DIFFICULTY", "Difficulty:").to_uppercase(), 13.0, WHITE)], Node { justify_content: JustifyContent::Center, ..abs(70.0, 57.0, 28.0, 4.0) }));
            for (x, tier, id, fallback) in [
                (64.2, DifficultyTier::Easy, "TEXT_DIFFICULTY_EASY_NAME", "Kolto"),
                (72.8, DifficultyTier::Normal, "TEXT_DIFFICULTY_NORMAL_NAME", "Spice"),
                (81.6, DifficultyTier::Hard, "TEXT_DIFFICULTY_HARD_NAME", "Death Sticks"),
            ] {
                let on = options.difficulty == tier && !options.classic_ai;
                f.spawn((
                    Button,
                    Click::Difficulty(tier),
                    Node { position_type: PositionType::Absolute, left: Val::Percent(x), top: Val::Percent(62.0), align_items: AlignItems::Center, column_gap: Val::Px(6.0), ..default() },
                ))
                .with_children(|b| {
                    b.spawn((
                        Node { width: Val::Px(13.0), height: Val::Px(13.0), border: UiRect::all(Val::Px(2.0)), padding: UiRect::all(Val::Px(2.0)), border_radius: BorderRadius::all(Val::Px(4.0)), ..default() },
                        BorderColor::all(Color::srgb(0.8, 0.82, 0.88)),
                        BackgroundColor(BOX),
                        children![(Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(if on { BLUE } else { Color::NONE }))],
                    ));
                    b.spawn(tx(game_text(game, id, fallback).to_uppercase(), 13.0, BLUE));
                });
            }
            // Hint.
            f.spawn((abs(4.3, 77.5, 91.0, 10.0), children![tx(
                "HINT: BE CAREFUL TO SAVE YOUR GAME REGULARLY IF POSSIBLE. DO NOT SAVE WHILE IN PAUSE MODE OR IN BATTLE.",
                11.0,
                BLUE
            )]));
            // Buttons.
            let button = |f: &mut ChildSpawnerCommands, left: f32, label: String, click: Option<Click>| {
                let node = Node { border: UiRect::all(Val::Px(1.0)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..abs(left, 90.2, 24.2, 6.3) };
                let (bg, border, color) = match click {
                    Some(_) => (Color::srgb(0.01, 0.01, 0.03), Color::srgb(0.6, 0.65, 0.75), WHITE),
                    None => (Color::srgba(0.05, 0.07, 0.12, 0.8), Color::srgb(0.2, 0.25, 0.35), DIM),
                };
                let mut e = f.spawn((node, BackgroundColor(bg), BorderColor::all(border), children![tx(label.to_uppercase(), 15.0, color)]));
                if let Some(c) = click {
                    e.insert((Button, c));
                }
            };
            button(f, 4.3, game_text(game, "TEXT_BUTTON_CANCEL", "Cancel"), Some(Click::Cancel));
            button(f, 37.8, if state.advanced { "Back".to_owned() } else { "Advanced...".to_owned() }, (can_start || state.advanced).then_some(Click::Advanced));
            button(f, 71.5, game_text(game, "TEXT_BUTTON_START_GAME", "Start Game"), can_start.then_some(Click::Start));
        });
}

#[allow(clippy::too_many_arguments)]
fn clicks(
    pressed: Query<(&Interaction, &Click), Changed<Interaction>>,
    mut state: ResMut<SetupState>,
    mut options: ResMut<NewGameOptions>,
    mut start: MessageWriter<StartCampaign>,
    mut next: ResMut<NextState<AppScreen>>,
    mut sound: MessageWriter<UiSound>,
) {
    for (i, click) in &pressed {
        if *i != Interaction::Pressed {
            continue;
        }
        sound.write(UiSound(crate::menu::SFX_CLICK));
        match *click {
            Click::Row(r) => {
                if r != state.selected {
                    state.selected = r;
                    state.side = None;
                    state.dropdown = false;
                }
            }
            Click::SideToggle => state.dropdown = !state.dropdown,
            Click::Side(s) => {
                state.side = Some(s);
                state.dropdown = false;
            }
            Click::Difficulty(t) => {
                options.difficulty = t;
                options.classic_ai = false;
            }
            Click::ClassicAi => options.classic_ai = !options.classic_ai,
            Click::Cancel => next.set(AppScreen::MainMenu),
            Click::Advanced => state.advanced = !state.advanced,
            Click::Start => {
                let name = state.rows.get(state.selected).and_then(|r| state.side.and_then(|s| r.sides.get(s))).map(|(_, n)| n.clone());
                if let Some(name) = name {
                    start.write(StartCampaign(name));
                }
            }
        }
    }
}


/// The screen is laid out in 1366x768 design pixels and scales with the window height like the
/// in-game UI.
fn screen_scale(
    settings: Res<crate::settings::Settings>,
    window: Option<Single<&Window, With<bevy::window::PrimaryWindow>>>,
    offscreen: Option<Res<crate::offscreen::OffscreenTarget>>,
    mut scale: ResMut<UiScale>,
    eaw: Option<ResMut<eaw_ui::EawUiSettings>>,
) {
    let height = match (&window, &offscreen) {
        (Some(w), _) => w.height(),
        (None, Some(t)) => crate::offscreen::SIZE.1 as f32 / t.scale_factor,
        _ => 768.0,
    };
    let v = height / 768.0 * settings.ui_scale;
    match eaw {
        Some(mut e) => {
            if e.scale != eaw_ui::UiScaleMode::Fixed(v) {
                e.scale = eaw_ui::UiScaleMode::Fixed(v);
            }
        }
        None => {
            if scale.0 != v {
                scale.0 = v;
            }
        }
    }
}

fn restore_scale(settings: Res<crate::settings::Settings>, mut scale: ResMut<UiScale>, eaw: Option<ResMut<eaw_ui::EawUiSettings>>) {
    match eaw {
        Some(mut e) => e.scale = eaw_ui::UiScaleMode::Fixed(settings.ui_scale),
        None => scale.0 = settings.ui_scale,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_without_game_data_group_sides() {
        let e = |name: &str, faction: &str| crate::game::CampaignEntry {
            name: name.into(),
            title: name.into(),
            group: "g".into(),
            group_title: "Full".into(),
            faction: faction.into(),
            planets: 145,
        };
        let rows = rows_from_entries(&[e("a_rebel", "Rebel"), e("a_empire", "Empire")]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sides.len(), 2);
    }

    #[test]
    fn choosing_a_side_enables_start_and_starts_the_campaign() {
        let mut app = App::new();
        app.add_message::<StartCampaign>()
            .add_message::<UiSound>()
            .init_resource::<NewGameOptions>()
            .insert_resource(SetupState {
                rows: vec![SetRow { title: "A".into(), description: String::new(), sides: vec![("Rebel".into(), "c_rebel".into()), ("Empire".into(), "c_empire".into())] }],
                ..default()
            })
            .insert_resource(State::new(AppScreen::CampaignSelect))
            .insert_resource(NextState::<AppScreen>::default())
            .add_systems(Update, clicks);
        let press = |app: &mut App, click: Click| {
            let e = app.world_mut().spawn((click, Interaction::None)).id();
            app.update();
            *app.world_mut().get_mut::<Interaction>(e).unwrap() = Interaction::Pressed;
            app.update();
            app.world_mut().entity_mut(e).despawn();
        };
        press(&mut app, Click::Start);
        assert!(app.world_mut().resource_mut::<Messages<StartCampaign>>().drain().next().is_none() || app.world().resource::<SetupState>().side.is_none());
        press(&mut app, Click::Side(1));
        press(&mut app, Click::Start);
        let started: Vec<_> = app.world_mut().resource_mut::<Messages<StartCampaign>>().drain().collect();
        assert_eq!(started.last().map(|s| s.0.as_str()), Some("c_empire"));
    }
}
