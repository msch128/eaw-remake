//! The F2 cheat menu: a small panel in the cyan glass of the command bar. Every cheat is sent as a journaled sim command
//! (`galaxy_sim::cheats`), so saves and replays stay consistent, and marks the game as cheated.

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
    app.init_resource::<CheatMenu>().add_systems(
        Update,
        (toggle, open_by_cli, rebuild, clicks, labels)
            .chain()
            .run_if(in_state(AppScreen::Galaxy).and_then(resource_exists::<SimState>)),
    );
    app.add_systems(OnExit(AppScreen::Galaxy), close);
}

/// Whether the panel is open.
#[derive(Resource, Default)]
pub(crate) struct CheatMenu {
    pub(crate) open: bool,
}

#[derive(Component)]
struct CheatRoot;

/// What a button does.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(crate) enum CheatAction {
    Credits,
    FinishBuild,
    TechUp,
    AiPause,
    Speed(f32),
}

/// A text that shows the state of a cheat.
#[derive(Component)]
struct CheatLabel(CheatAction);

/// The text of a button (also the stable name used by tests).
fn caption(action: CheatAction, state: &SimState) -> String {
    match action {
        CheatAction::Credits => format!("+{CREDIT_STEP} credits"),
        CheatAction::FinishBuild => "Finish current build".to_owned(),
        CheatAction::TechUp => "Tech level +1".to_owned(),
        CheatAction::AiPause => format!("Pause AI: {}", if state.galaxy().ai_paused { "ON" } else { "OFF" }),
        CheatAction::Speed(s) => format!("{s}x"),
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

fn rebuild(mut commands: Commands, menu: Res<CheatMenu>, existing: Query<Entity, With<CheatRoot>>, state: Res<SimState>) {
    if !menu.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }
    if !menu.open {
        return;
    }
    let font = |size: f32| TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(size), ..default() };
    let border = Color::srgb(0.0, 0.76, 0.99);
    let button = |p: &mut ChildSpawnerCommands, action: CheatAction, width: Val| {
        p.spawn((
            Button,
            action,
            Node { width, padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), justify_content: JustifyContent::Center, border: UiRect::all(Val::Px(1.0)), border_radius: BorderRadius::all(Val::Px(3.0)), ..default() },
            BackgroundColor(Color::srgba(0.05, 0.16, 0.22, 0.95)),
            BorderColor::all(border),
        ))
        .with_child((Text::new(caption(action, &state)), font(13.0), TextColor(Color::srgb(0.85, 0.93, 1.0)), CheatLabel(action)));
    };
    commands
        .spawn((
            CheatRoot,
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
            p.spawn((Text::new("GALAXY"), font(11.0), TextColor(Color::srgb(0.52, 0.68, 0.82))));
            for a in [CheatAction::Credits, CheatAction::FinishBuild, CheatAction::TechUp, CheatAction::AiPause] {
                button(p, a, Val::Percent(100.0));
            }
            p.spawn((Text::new("GAME SPEED"), font(11.0), TextColor(Color::srgb(0.52, 0.68, 0.82))));
            p.spawn(Node { column_gap: Val::Px(4.0), ..default() }).with_children(|row| {
                for s in SPEEDS {
                    button(row, CheatAction::Speed(s), Val::Percent(25.0));
                }
            });
        });
}

fn clicks(
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
        }
    }
}

/// Keeps the toggle captions in line with the sim.
fn labels(state: Res<SimState>, mut labels: Query<(&CheatLabel, &mut Text)>) {
    for (l, mut t) in &mut labels {
        let c = caption(l.0, &state);
        if t.0 != c {
            t.0 = c;
        }
    }
}

/// `--cheat-menu` opens the panel once (offscreen renders).
fn open_by_cli(args: Res<crate::cli::Args>, mut menu: ResMut<CheatMenu>, mut done: Local<bool>) {
    if args.cheat_menu && !*done {
        *done = true;
        menu.open = true;
    }
}
