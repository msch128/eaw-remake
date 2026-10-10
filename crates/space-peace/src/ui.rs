//! HUD: planet info panel, FPS counter, controls hint and fullscreen toggling.

use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
    window::PrimaryWindow,
};

use crate::galaxy_map::{Hover, Selection};
use crate::flow::AppScreen;
use crate::settings::Settings;
use crate::galaxy_view::GalaxyView;

/// Heads-up display and window hotkeys.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .add_systems(Startup, spawn_fps)
            .add_systems(OnEnter(AppScreen::Galaxy), spawn_hud)
            .add_systems(
                OnTransition { exited: AppScreen::Galaxy, entered: AppScreen::MainMenu },
                despawn_hud,
            )
            .add_systems(
                OnTransition { exited: AppScreen::Galaxy, entered: AppScreen::CampaignSelect },
                despawn_hud,
            )
            .add_systems(
                OnTransition { exited: AppScreen::SpaceBattle, entered: AppScreen::MainMenu },
                despawn_hud,
            )
            .add_systems(
                OnTransition { exited: AppScreen::SpaceBattle, entered: AppScreen::CampaignSelect },
                despawn_hud,
            )
            .add_systems(Update, (toggle_fullscreen, toggle_fps, update_fps))
            .add_systems(
                Update,
                (update_info_panel, update_faction_bar, update_tooltip)
                    .run_if(in_state(AppScreen::Galaxy))
                    .run_if(not(resource_exists::<galaxy_play::SimState>)),
            );
    }
}

#[derive(Component)]
struct FpsText;

/// Marks the galaxy HUD (info panel, faction bar, tooltip, hint); removed when leaving the map.
#[derive(Component)]
pub(crate) struct HudEntity;

/// Panels that take the colours of the player faction skin (see `play_ui::skin_panels`).
#[derive(Component)]
pub(crate) struct SkinPanel;

/// Root of the faction overview bar at the top (hidden once the EaW command bar shows the same numbers).
#[derive(Component)]
pub(crate) struct TopBar;

/// The controls hint text.
#[derive(Component)]
pub(crate) struct HintText;

fn despawn_hud(mut commands: Commands, hud: Query<Entity, With<HudEntity>>) {
    for e in &hud {
        commands.entity(e).despawn();
    }
}

#[derive(Component)]
pub struct InfoPanel;

/// Which line of the info panel a text entity shows.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InfoLine {
    Name,
    Owner,
    Income,
    Slots,
    Ability,
    Forces,
}

/// Which field of the top faction bar a text entity shows.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum BarField {
    Faction,
    Credits,
    Planets,
}

/// Mouse-follow tooltip root.
#[derive(Component)]
pub(crate) struct Tooltip;

/// Text of the tooltip.
#[derive(Component)]
struct TooltipText;

/// Formats an integer with thousands separators (`15000` -> `15,000`).
pub(crate) fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

const PANEL_BG: Color = Color::srgba(0.02, 0.05, 0.10, 0.82);
const PANEL_BORDER: Color = Color::srgba(0.35, 0.65, 1.0, 0.8);

fn spawn_fps(mut commands: Commands) {
    // FPS counter, top left like the original (F3); the hero bar owns the top right.
    commands.spawn((
        Text::new("FPS --"),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.6, 1.0, 0.6)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(4.0),
            left: Val::Px(8.0),
            ..default()
        },
        FpsText,
        GlobalZIndex(30),
    ));
}

fn spawn_hud(mut commands: Commands, existing: Query<(), With<HudEntity>>) {
    // Coming back from a tactical battle the HUD is still there.
    if !existing.is_empty() {
        return;
    }
    // Controls hint, bottom right.
    commands.spawn((
        HintText,
        HudEntity,
        Text::new("LMB select | RMB/MMB drag or WASD pan | Wheel zoom | Esc menu | F3 fps | F11 fullscreen"),
        TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(12.0), ..default() },
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
            SkinPanel,
            BackgroundColor(PANEL_BG),
            BorderColor::all(PANEL_BORDER),
            GlobalZIndex(6),
            Visibility::Hidden,
            HudEntity,
            InfoPanel,
        ))
        .with_children(|p| {
            let line = |p: &mut ChildSpawnerCommands, which: InfoLine, size: f32, color: Color| {
                p.spawn((
                    Text::new(""),
                    TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(size), ..default() },
                    TextColor(color),
                    which,
                ));
            };
            line(p, InfoLine::Name, 22.0, Color::WHITE);
            line(p, InfoLine::Owner, 15.0, Color::WHITE);
            line(p, InfoLine::Income, 15.0, Color::srgb(1.0, 0.9, 0.5));
            line(p, InfoLine::Slots, 14.0, Color::srgb(0.75, 0.85, 1.0));
            line(p, InfoLine::Ability, 14.0, Color::srgb(0.6, 1.0, 0.8));
            line(p, InfoLine::Forces, 13.0, Color::srgb(0.8, 0.82, 0.86));
        });

    // Faction overview bar, top centre.
    commands
        .spawn((
            HudEntity,
            TopBar,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    column_gap: Val::Px(26.0),
                    ..default()
                },
                SkinPanel,
            BackgroundColor(PANEL_BG),
                BorderColor::all(PANEL_BORDER),
            ))
            .with_children(|bar| {
                for (field, color) in [
                    (BarField::Faction, Color::WHITE),
                    (BarField::Credits, Color::srgb(1.0, 0.9, 0.5)),
                    (BarField::Planets, Color::srgb(0.75, 0.85, 1.0)),
                ] {
                    bar.spawn((
                        Text::new(""),
                        TextFont { font: FontSource::Family("Bahnschrift".into()), font_size: FontSize::Px(17.0), ..default() },
                        TextColor(color),
                        field,
                    ));
                }
            });
        });

    // Hover tooltip (name, owner, income) following the cursor.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                padding: UiRect::all(Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            SkinPanel,
            BackgroundColor(PANEL_BG),
            BorderColor::all(PANEL_BORDER),
            Visibility::Hidden,
            GlobalZIndex(10),
            HudEntity,
            Tooltip,
        ))
        .with_children(|t| {
            t.spawn((
                Text::new(""),
                TextFont { font_size: FontSize::Px(14.0), ..default() },
                TextColor(Color::WHITE),
                TooltipText,
            ));
        });
}
/// Fills the info panel from the selected planet: owner, income, structure slots, ability and
/// the forces the campaign starts with there.
pub(crate) fn update_info_panel(
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
            InfoLine::Income => text.0 = format!("Income: {} credits", thousands(i64::from(p.income))),
            InfoLine::Slots => {
                text.0 = format!(
                    "Space structures: {} slots (+{} special)",
                    p.max_space_base, p.special_slots
                );
            }
            InfoLine::Ability => {
                text.0 = match &p.ability {
                    Some((name, desc)) if desc.is_empty() => format!("Ability: {name}"),
                    Some((name, desc)) => format!("Ability: {name}\n{desc}"),
                    None => String::new(),
                };
            }
            InfoLine::Forces => {
                const SHOWN: usize = 8;
                text.0 = if p.starting_forces.is_empty() {
                    String::new()
                } else {
                    let mut s = String::from("Starting forces:");
                    for f in p.starting_forces.iter().take(SHOWN) {
                        s.push_str("\n  ");
                        s.push_str(f);
                    }
                    if p.starting_forces.len() > SHOWN {
                        s.push_str(&format!("\n  +{} more", p.starting_forces.len() - SHOWN));
                    }
                    s
                };
            }
        }
    }
}

/// Fills the top bar with the human faction, its credits and owned planet count.
fn update_faction_bar(
    view: Res<GalaxyView>,
    mut fields: Query<(&BarField, &mut Text, &mut TextColor)>,
) {
    if !view.is_changed() {
        return;
    }
    let faction = view.player_faction.and_then(|f| view.factions.get(f));
    let owned = view
        .planets
        .iter()
        .filter(|p| p.owner.is_some() && p.owner == view.player_faction)
        .count();
    for (field, mut text, mut color) in &mut fields {
        match (field, faction) {
            (BarField::Faction, Some(f)) => {
                text.0 = f.name.clone();
                color.0 = Color::srgb(f.color[0], f.color[1], f.color[2]);
            }
            (BarField::Faction, None) => text.0 = "Spectator".into(),
            (BarField::Credits, Some(f)) => text.0 = format!("Credits {}", thousands(f.credits)),
            (BarField::Planets, Some(_)) => text.0 = format!("Planets {owned} / {}", view.planets.len()),
            (BarField::Planets, None) => text.0 = format!("Planets {}", view.planets.len()),
            (BarField::Credits, None) => text.0.clear(),
        }
    }
}

/// Shows name, owner and income of the hovered planet next to the cursor.
fn update_tooltip(
    hover: Res<Hover>,
    view: Res<GalaxyView>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut root: Single<(&mut Node, &mut Visibility), With<Tooltip>>,
    mut text: Single<&mut Text, With<TooltipText>>,
) {
    let (node, vis) = &mut *root;
    let cursor = window.and_then(|w| w.cursor_position());
    match (hover.0.and_then(|i| view.planets.get(i).map(|p| (i, p))), cursor) {
        (Some((i, p)), Some(c)) => {
            let owner = view.owner_of(i).map_or("Neutral", |f| f.name.as_str());
            text.0 = format!("{}\nOwner: {owner}\nIncome: {}", p.display_name, thousands(i64::from(p.income)));
            node.left = Val::Px(c.x + 18.0);
            node.top = Val::Px(c.y + 18.0);
            **vis = Visibility::Inherited;
        }
        _ => **vis = Visibility::Hidden,
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

/// F11 or Alt+Enter toggles borderless fullscreen (applied by the settings plugin).
fn toggle_fullscreen(keys: Res<ButtonInput<KeyCode>>, mut settings: ResMut<Settings>) {
    let alt_enter = keys.just_pressed(KeyCode::Enter)
        && keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if keys.just_pressed(KeyCode::F11) || alt_enter {
        settings.borderless = !settings.borderless;
    }
}