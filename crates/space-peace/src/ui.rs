//! HUD: planet info panel, FPS counter, controls hint and fullscreen toggling.

use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
    window::{MonitorSelection, PrimaryWindow, WindowMode},
};

use crate::galaxy_map::Selection;
use crate::galaxy_view::GalaxyView;

/// Heads-up display and window hotkeys.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, spawn_hud)
            .add_systems(
                Update,
                (toggle_fullscreen, toggle_fps, update_fps, update_info_panel),
            );
    }
}

#[derive(Component)]
struct FpsText;

#[derive(Component)]
struct InfoPanel;

/// Which line of the info panel a text entity shows.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum InfoLine {
    Name,
    Owner,
    Income,
}

const PANEL_BG: Color = Color::srgba(0.02, 0.05, 0.10, 0.82);
const PANEL_BORDER: Color = Color::srgba(0.35, 0.65, 1.0, 0.8);

fn spawn_hud(mut commands: Commands) {
    // FPS counter, top right (F3).
    commands.spawn((
        Text::new("FPS --"),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.6, 1.0, 0.6)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(12.0),
            ..default()
        },
        FpsText,
    ));

    // Controls hint, bottom right.
    commands.spawn((
        Text::new("LMB select | RMB/MMB drag or WASD pan | Wheel zoom | F3 fps | F11 fullscreen | N next track"),
        TextFont { font_size: FontSize::Px(12.0), ..default() },
        TextColor(Color::srgba(0.7, 0.8, 0.9, 0.55)),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(8.0), right: Val::Px(12.0), ..default() },
    ));

    // Planet info panel, bottom left (hidden until a planet is selected).
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0),
                bottom: Val::Px(16.0),
                min_width: Val::Px(240.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(PANEL_BORDER),
            Visibility::Hidden,
            InfoPanel,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                InfoLine::Name,
            ));
            p.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                InfoLine::Owner,
            ));
            p.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.9, 0.5)),
                InfoLine::Income,
            ));
        });
}

/// Fills the info panel from the selected planet.
fn update_info_panel(
    selection: Res<Selection>,
    view: Res<GalaxyView>,
    mut panel: Single<&mut Visibility, With<InfoPanel>>,
    mut lines: Query<(&InfoLine, &mut Text, &mut TextColor)>,
) {
    if !selection.is_changed() && !view.is_changed() {
        return;
    }
    let Some(i) = selection.0.filter(|&i| i < view.planets.len()) else {
        **panel = Visibility::Hidden;
        return;
    };
    let p = &view.planets[i];
    **panel = Visibility::Inherited;
    for (line, mut text, mut color) in &mut lines {
        match line {
            InfoLine::Name => text.0 = p.display_name.clone(),
            InfoLine::Owner => match view.owner_of(i) {
                Some(f) => {
                    text.0 = format!("Owner: {}", f.name);
                    color.0 = Color::srgb(f.color[0], f.color[1], f.color[2]);
                }
                None => {
                    text.0 = "Owner: Neutral".into();
                    color.0 = Color::srgb(0.7, 0.75, 0.8);
                }
            },
            InfoLine::Income => text.0 = format!("Income: {} credits", p.income),
        }
    }
}

fn toggle_fps(keys: Res<ButtonInput<KeyCode>>, mut fps: Single<&mut Visibility, With<FpsText>>) {
    if keys.just_pressed(KeyCode::F3) {
        **fps = if **fps == Visibility::Hidden {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

fn update_fps(
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    mut fps: Single<&mut Text, With<FpsText>>,
    mut acc: Local<f32>,
) {
    // Refresh a few times per second so the number is readable.
    *acc += time.delta_secs();
    if *acc < 0.25 {
        return;
    }
    *acc = 0.0;
    let fps_val = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed());
    let ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed());
    if let (Some(f), Some(ms)) = (fps_val, ms) {
        fps.0 = format!("{f:.0} FPS  {ms:.2} ms");
    }
}

/// F11 or Alt+Enter toggles borderless fullscreen.
fn toggle_fullscreen(
    keys: Res<ButtonInput<KeyCode>>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
) {
    let alt_enter = keys.just_pressed(KeyCode::Enter)
        && keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if keys.just_pressed(KeyCode::F11) || alt_enter {
        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
}
