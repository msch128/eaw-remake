//! The F2 cheat menu: a small panel in the cyan glass of the command bar, on the galaxy map and in space battles. Every
//! cheat is a journaled sim command (`galaxy_sim::cheats`, `battle_sim::BattleCheat`), so saves and replays stay
//! consistent; any cheat also marks the galaxy game as cheated.

use battle_view::{BattleRuntime, BattleViewSettings};
use bevy::prelude::*;
use galaxy_play::{PlayAction, PlayerSelection, SimState};
use galaxy_sim::cheats::Cheat;

use crate::flow::AppScreen;

/// Credits per click of the credit cheat.
const CREDIT_STEP: u32 = 5000;
/// Game speeds of the speed row.
const SPEEDS: [f32; 4] = [1.0, 2.0, 4.0, 10.0];

/// Registers the menu.
pub(crate) fn plugin(app: &mut App) {
    let galaxy = in_state(AppScreen::Galaxy).and_then(resource_exists::<SimState>);
    let battle = in_state(AppScreen::SpaceBattle).and_then(resource_exists::<BattleRuntime>);
    app.init_resource::<CheatMenu>()
        .init_resource::<BattleCheats>()
        .add_systems(Update, (toggle, open_by_cli).chain().run_if(in_state(AppScreen::Galaxy).or_else(in_state(AppScreen::SpaceBattle))))
        .add_systems(Update, (rebuild_galaxy, clicks_galaxy, labels_galaxy).chain().after(toggle).run_if(galaxy))
        .add_systems(Update, (rebuild_battle, clicks_battle, labels_battle).chain().after(toggle).run_if(battle))
        .add_systems(OnExit(AppScreen::Galaxy), close)
        .add_systems(OnExit(AppScreen::SpaceBattle), close)
        .add_systems(OnEnter(AppScreen::SpaceBattle), |mut b: ResMut<BattleCheats>| *b = BattleCheats::default());
}

/// Whether the panel is open.
#[derive(Resource, Default)]
pub(crate) struct CheatMenu {
    pub(crate) open: bool,
}

/// Which battle cheats are on (the sim has the truth; this drives the captions).
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BattleCheats {
    invulnerable: bool,
    no_cooldowns: bool,
    no_pop_cap: bool,
    ai_paused: bool,
}

#[derive(Component)]
pub(crate) struct CheatRoot;

/// What a button does.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(crate) enum CheatAction {
    Credits,
    FinishBuild,
    TechUp,
    AiPause,
    Speed(f32),
    WinNow,
    Invulnerable,
    NoCooldowns,
    NoPopCap,
    BattleAiPause,
}

/// A text that shows the state of a cheat.
#[derive(Component)]
struct CheatLabel(CheatAction);

fn on_off(on: bool) -> &'static str {
    if on { "ON" } else { "OFF" }
}

/// The text of a button.
fn caption(action: CheatAction, galaxy_ai_paused: bool, battle: &BattleCheats) -> String {
    match action {
        CheatAction::Credits => format!("+{CREDIT_STEP} credits"),
        CheatAction::FinishBuild => "Finish current build".to_owned(),
        CheatAction::TechUp => "Tech level +1".to_owned(),
        CheatAction::AiPause => format!("Pause AI: {}", on_off(galaxy_ai_paused)),
        CheatAction::Speed(s) => format!("{s}x"),
        CheatAction::WinNow => "Win the battle now".to_owned(),
        CheatAction::Invulnerable => format!("Own units invulnerable: {}", on_off(battle.invulnerable)),
        CheatAction::NoCooldowns => format!("No ability cooldowns: {}", on_off(battle.no_cooldowns)),
        CheatAction::NoPopCap => format!("No population limit: {}", on_off(battle.no_pop_cap)),
        CheatAction::BattleAiPause => format!("Pause AI: {}", on_off(battle.ai_paused)),
    }
}

fn close(mut menu: ResMut<CheatMenu>) {
    menu.open = false;
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut menu: ResMut<CheatMenu>) {
    if keys.just_pressed(KeyCode::F2) {
        menu.open = !menu.open;
    }
}

/// `--cheat-menu` opens the panel once per screen (offscreen renders).
fn open_by_cli(args: Res<crate::cli::Args>, screen: Res<State<AppScreen>>, mut menu: ResMut<CheatMenu>, mut done: Local<Option<AppScreen>>) {
    if args.cheat_menu && *done != Some(*screen.get()) {
        *done = Some(*screen.get());
        menu.open = true;
    }
}

/// Builds the panel with `sections` (title, buttons) and the speed row.
fn spawn_panel(commands: &mut Commands, sections: &[(&str, &[CheatAction])], text: &dyn Fn(CheatAction) -> String) {
    let font = |size: f32| TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(size), ..default() };
    let border = Color::srgb(0.0, 0.76, 0.99);
    let button = |p: &mut ChildSpawnerCommands, action: CheatAction, width: Val| {
        p.spawn((
            Button,
            galaxy_play::PlayUiBlock,
            action,
            Node { width, padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), justify_content: JustifyContent::Center, border: UiRect::all(Val::Px(1.0)), border_radius: BorderRadius::all(Val::Px(3.0)), ..default() },
            BackgroundColor(Color::srgba(0.05, 0.16, 0.22, 0.95)),
            BorderColor::all(border),
        ))
        .with_child((Text::new(text(action)), font(13.0), TextColor(Color::srgb(0.85, 0.93, 1.0)), CheatLabel(action)));
    };
    commands
        .spawn((
            CheatRoot,
            galaxy_play::PlayUiBlock,
            Name::new("cheat menu"),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(16.0),
                top: Val::Px(96.0),
                width: Val::Px(250.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.09, 0.13, 0.95)),
            BorderColor::all(border),
            GlobalZIndex(650),
        ))
        .with_children(|p| {
            p.spawn((Text::new("CHEAT MENU  (F2)"), font(15.0), TextColor(Color::srgb(1.0, 0.86, 0.38))));
            p.spawn((Text::new("Using a cheat marks the game as cheated."), font(10.5), TextColor(Color::srgb(0.52, 0.68, 0.82))));
            for (title, actions) in sections {
                p.spawn((Text::new(*title), font(11.0), TextColor(Color::srgb(0.52, 0.68, 0.82))));
                for &a in *actions {
                    button(p, a, Val::Percent(100.0));
                }
            }
            p.spawn((Text::new("GAME SPEED"), font(11.0), TextColor(Color::srgb(0.52, 0.68, 0.82))));
            p.spawn(Node { column_gap: Val::Px(4.0), ..default() }).with_children(|row| {
                for s in SPEEDS {
                    button(row, CheatAction::Speed(s), Val::Percent(25.0));
                }
            });
        });
}

fn rebuild_galaxy(mut commands: Commands, menu: Res<CheatMenu>, existing: Query<Entity, With<CheatRoot>>, state: Res<SimState>) {
    if !menu.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }
    if menu.open {
        let ai = state.galaxy().ai_paused;
        let galaxy = [CheatAction::Credits, CheatAction::FinishBuild, CheatAction::TechUp, CheatAction::AiPause];
        spawn_panel(&mut commands, &[("GALAXY", &galaxy)], &|a| caption(a, ai, &BattleCheats::default()));
    }
}

fn rebuild_battle(mut commands: Commands, menu: Res<CheatMenu>, existing: Query<Entity, With<CheatRoot>>, state: Res<BattleCheats>) {
    if !menu.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }
    if menu.open {
        let battle = [CheatAction::WinNow, CheatAction::Invulnerable, CheatAction::NoCooldowns, CheatAction::NoPopCap, CheatAction::BattleAiPause];
        spawn_panel(&mut commands, &[("BATTLE", &battle)], &|a| caption(a, false, &state));
    }
}

fn clicks_galaxy(
    buttons: Query<(&Interaction, &CheatAction), Changed<Interaction>>,
    selection: Res<PlayerSelection>,
    state: Res<SimState>,
    mut actions: MessageWriter<PlayAction>,
    mut toast: ResMut<crate::play_ui::Toast>,
) {
    for (i, a) in &buttons {
        if *i != Interaction::Pressed {
            continue;
        }
        match *a {
            CheatAction::Credits => {
                actions.write(PlayAction::Cheat(Cheat::Credits(CREDIT_STEP)));
            }
            CheatAction::FinishBuild => match selection.planet {
                Some(planet) => {
                    actions.write(PlayAction::Cheat(Cheat::FinishProduction { planet }));
                }
                None => {
                    toast.0 = "Select one of your planets first".to_owned();
                    toast.1 = 3.0;
                }
            },
            CheatAction::TechUp => {
                actions.write(PlayAction::Cheat(Cheat::TechUp));
            }
            CheatAction::AiPause => {
                actions.write(PlayAction::Cheat(Cheat::AiPaused(!state.galaxy().ai_paused)));
            }
            CheatAction::Speed(s) => {
                actions.write(PlayAction::SetSpeedMultiplier(s));
            }
            _ => {}
        }
    }
}

fn clicks_battle(
    buttons: Query<(&Interaction, &CheatAction), Changed<Interaction>>,
    rt: Res<BattleRuntime>,
    mut settings: ResMut<BattleViewSettings>,
    mut state: ResMut<BattleCheats>,
    mut actions: MessageWriter<PlayAction>,
) {
    use battle_sim::{BattleCheat, Command};
    let side = settings.player_side;
    for (i, a) in &buttons {
        if *i != Interaction::Pressed {
            continue;
        }
        let cheat = match *a {
            CheatAction::WinNow => Some(BattleCheat::WinNow),
            CheatAction::Invulnerable => {
                state.invulnerable = !state.invulnerable;
                Some(BattleCheat::Invulnerable(state.invulnerable))
            }
            CheatAction::NoCooldowns => {
                state.no_cooldowns = !state.no_cooldowns;
                Some(BattleCheat::NoCooldowns(state.no_cooldowns))
            }
            CheatAction::NoPopCap => {
                state.no_pop_cap = !state.no_pop_cap;
                Some(BattleCheat::NoPopCap(state.no_pop_cap))
            }
            CheatAction::BattleAiPause => {
                state.ai_paused = !state.ai_paused;
                Some(BattleCheat::AiPaused(state.ai_paused))
            }
            CheatAction::Speed(s) => {
                settings.speed = s;
                settings.paused = false;
                None
            }
            _ => None,
        };
        if let Some(c) = cheat {
            rt.command(side, Command::Cheat(c));
            // The galaxy game is marked as cheated as well.
            actions.write(PlayAction::Cheat(Cheat::MarkCheated));
        }
    }
}

/// Keeps the toggle captions in line with the sim.
fn labels_galaxy(state: Res<SimState>, mut labels: Query<(&CheatLabel, &mut Text)>) {
    for (l, mut t) in &mut labels {
        let c = caption(l.0, state.galaxy().ai_paused, &BattleCheats::default());
        if t.0 != c {
            t.0 = c;
        }
    }
}

fn labels_battle(state: Res<BattleCheats>, mut labels: Query<(&CheatLabel, &mut Text)>) {
    for (l, mut t) in &mut labels {
        let c = caption(l.0, false, &state);
        if t.0 != c {
            t.0 = c;
        }
    }
}
