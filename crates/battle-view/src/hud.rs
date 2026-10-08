//! Extra parts of the EaW space tactical screen that are not in the command bar model: the R2 advisor hint
//! bubble, the "Mission Objectives" box (bottom right) and the hero portraits (top right).
//!
//! Texts come from the game's string table: `TEXT_DROID_ADVISOR_<FACTION>_SPACE_NN`, `TEXT_TEST_OBJECTIVE_HEADER`,
//! `TEXT_TACTICAL_PRIMARY_SPACE_ATTACK` / `_DEFEND[_<ATTACKER>]`. Positions are design pixels (1366x768, scaled by
//! eaw-ui's `UiScale`) measured from the reference screenshot of the real game.

use std::collections::HashMap;

use bevy::prelude::*;
use eaw_ui::textures::UiTextures;
use eaw_ui::{EawUiAssets, EawUiSettings, UiFaction, unit_icon};

use crate::control::{FocusCamera, Selection};
use crate::ships::RenderedShips;
use crate::{BattleRuntime, BattleStatus, BattleViewSettings};

/// Battle seconds before the first hint appears (estimate: the real game shows it right at the start).
const FIRST_HINT_AT: f32 = 1.0;
/// Seconds a hint stays up (estimate).
const HINT_SECONDS: f32 = 14.0;
/// Battle seconds between two hints (estimate).
const HINT_EVERY: f32 = 75.0;
/// Hint numbers that apply to any space battle, in order (`_SPACE_00`, `_09`, `_12`, `_13` need a station, an
/// upgradeable station, the Emperor or a Broadside Cruiser and are skipped).
const HINTS: [u32; 9] = [1, 2, 3, 4, 5, 8, 10, 11, 6];

/// Marks everything this module spawns.
#[derive(Component)]
pub struct TacticalHud;

/// The advisor bubble and its text.
#[derive(Component)]
pub(crate) struct AdvisorBubble;

#[derive(Component)]
pub(crate) struct AdvisorText;

/// A hero portrait of the player's side.
#[derive(Component)]
pub(crate) struct HeroPortrait(u32);

/// Health fill inside a hero portrait.
#[derive(Component)]
pub(crate) struct HeroHealth(u32);

/// Row that holds the hero portraits.
#[derive(Component)]
pub(crate) struct HeroRow;

/// State of the HUD of the current battle.
#[derive(Resource, Default)]
pub struct HudState {
    spawned: bool,
    frames: u32,
    /// Heroes currently shown (ship id, object name).
    heroes: Vec<(u32, String)>,
    /// Hint rotation: index into [`HINTS`] and the battle time the current hint was shown at.
    next_hint: usize,
    hint_at: Option<f32>,
    /// Result cache of "is this unit a named hero".
    hero_cache: HashMap<String, bool>,
    last_click: Option<(u32, f32)>,
}

fn font(settings: &EawUiSettings, size: f32) -> TextFont {
    match &settings.font_family {
        Some(f) => TextFont { font: FontSource::Family(f.as_str().into()), font_size: FontSize::Px(size), ..default() },
        None => TextFont::from_font_size(size),
    }
}

fn faction_key(faction: UiFaction) -> &'static str {
    match faction {
        UiFaction::Rebel => "REBEL",
        UiFaction::Empire => "EMPIRE",
        UiFaction::Underworld => "UNDERWORLD",
    }
}

/// Objective line of the player's side: attackers destroy everything, defenders kill the invader.
fn objective_key(rt: &BattleRuntime, player: u8) -> String {
    let sides = rt.request.as_ref().map(|r| r.sides.as_slice()).unwrap_or(&[]);
    let defender = sides.len().saturating_sub(1);
    if sides.len() < 2 || (player as usize) < defender {
        return "TEXT_TACTICAL_PRIMARY_SPACE_ATTACK".to_string();
    }
    let attacker = sides[0].faction_name.to_uppercase();
    format!("TEXT_TACTICAL_PRIMARY_SPACE_DEFEND_{attacker}")
}

/// Spawns the bubble and the objectives box once the battle runs.
#[allow(clippy::too_many_arguments)]
pub fn spawn_hud(
    mut commands: Commands,
    mut state: ResMut<HudState>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    ui: Res<EawUiSettings>,
    assets: Option<Res<EawUiAssets>>,
) {
    if state.spawned {
        return;
    }
    state.frames += 1;
    let Some(assets) = assets else { return };
    // The UI camera of the battle is set a frame or two after the session starts.
    if ui.camera.is_none() && state.frames < 10 {
        return;
    }
    state.spawned = true;
    state.heroes.clear();
    state.next_hint = 0;
    state.hint_at = None;
    let target = |c: &mut EntityCommands| {
        if let Some(cam) = ui.camera {
            c.insert(UiTargetCamera(cam));
        }
    };

    // R2 advisor bubble above the droid portrait of the bar; hidden until the first hint.
    let mut bubble = commands.spawn((
        TacticalHud,
        AdvisorBubble,
        Name::new("advisor bubble"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(65.0),
            bottom: Val::Px(309.0),
            width: Val::Px(265.0),
            min_height: Val::Px(47.0),
            padding: UiRect::axes(Val::Px(6.0), Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgba(0.07, 0.2, 0.32, 0.78)),
        BorderColor::all(Color::srgb(0.27, 0.6, 0.85)),
        GlobalZIndex(60),
    ));
    target(&mut bubble);
    let bubble = bubble.id();
    commands.spawn((
        TacticalHud,
        AdvisorText,
        Text::new(""),
        font(&ui, 11.0),
        TextColor(Color::srgb(0.88, 0.94, 1.0)),
        ChildOf(bubble),
    ));

    // Mission objectives, bottom right.
    let key = objective_key(&rt, settings.player_side);
    let header = assets.text.get("TEXT_TEST_OBJECTIVE_HEADER").unwrap_or("Mission Objectives").to_uppercase();
    let body = assets.text.get(&key).or_else(|| assets.text.get("TEXT_TACTICAL_PRIMARY_SPACE_DEFEND")).unwrap_or("").to_string();
    let mut boxed = commands.spawn((
        TacticalHud,
        Name::new("mission objectives"),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(36.0),
            bottom: Val::Px(29.0),
            width: Val::Px(265.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            row_gap: Val::Px(3.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.04, 0.09, 0.17, 0.5)),
        GlobalZIndex(60),
    ));
    target(&mut boxed);
    let boxed = boxed.id();
    commands.spawn((TacticalHud, Text::new(header), font(&ui, 13.0), TextColor(Color::srgb(0.84, 0.9, 0.98)), ChildOf(boxed)));
    commands.spawn((
        TacticalHud,
        Text::new(format!("1. {body}")),
        font(&ui, 10.5),
        TextColor(Color::srgb(0.9, 0.93, 0.97)),
        Node { margin: UiRect::left(Val::Px(6.0)), ..default() },
        ChildOf(boxed),
    ));

    // Hero portraits, top right.
    let mut row = commands.spawn((
        TacticalHud,
        HeroRow,
        Name::new("hero portraits"),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(8.0),
            top: Val::Px(4.0),
            column_gap: Val::Px(5.0),
            ..default()
        },
        GlobalZIndex(60),
    ));
    target(&mut row);
}

/// Shows the hints one after the other.
pub(crate) fn advisor(
    status: Res<BattleStatus>,
    rt: Res<BattleRuntime>,
    assets: Option<Res<EawUiAssets>>,
    mut state: ResMut<HudState>,
    mut bubble: Query<&mut Node, With<AdvisorBubble>>,
    mut text: Query<&mut Text, With<AdvisorText>>,
) {
    let (Some(assets), Ok(mut node), Ok(mut text)) = (assets, bubble.single_mut(), text.single_mut()) else { return };
    if status.over {
        node.display = Display::None;
        return;
    }
    let now = status.time;
    let due = match state.hint_at {
        None => now >= FIRST_HINT_AT,
        Some(at) => now >= at + HINT_EVERY,
    };
    if due {
        let faction = UiFaction::from_name(rt.player_faction.as_deref().unwrap_or("Empire"));
        let n = HINTS[state.next_hint % HINTS.len()];
        let key = format!("TEXT_DROID_ADVISOR_{}_SPACE_{n:02}", faction_key(faction));
        if let Some(t) = assets.text.get(&key).filter(|t| !t.is_empty()) {
            text.0 = t.to_string();
            node.display = Display::Flex;
        }
        state.next_hint += 1;
        state.hint_at = Some(now);
    } else if state.hint_at.is_some_and(|at| now >= at + HINT_SECONDS) && node.display != Display::None {
        node.display = Display::None;
    }
}

fn is_hero(game: &alamo_data::GameData, cache: &mut HashMap<String, bool>, name: &str) -> bool {
    *cache.entry(name.to_string()).or_insert_with(|| {
        game.objects
            .resolve(name)
            .and_then(|r| r.get("is_named_hero").map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1")))
            .unwrap_or(false)
    })
}

/// Keeps one portrait per living hero of the player's side and updates its health and selection ring.
#[allow(clippy::too_many_arguments)]
pub(crate) fn heroes(
    mut commands: Commands,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    ships: Res<RenderedShips>,
    assets: Option<Res<EawUiAssets>>,
    textures: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut state: ResMut<HudState>,
    row: Query<(Entity, Option<&Children>), With<HeroRow>>,
    mut portraits: Query<(&HeroPortrait, &Interaction, &mut BorderColor)>,
    mut fills: Query<(&HeroHealth, &mut Node)>,
    time: Res<Time>,
    mut focus: MessageWriter<FocusCamera>,
    mut selection: ResMut<Selection>,
) {
    let (Some(assets), Some(mut textures), Ok((row, kids))) = (assets, textures, row.single()) else { return };
    let Some(game) = assets.game.clone() else { return };
    let player = settings.player_side;
    let mut now: Vec<(u32, String)> = ships
        .0
        .values()
        .filter(|s| s.side == player)
        .filter_map(|s| {
            let name = rt.catalog.ship(s.def).name.clone();
            is_hero(&game, &mut state.hero_cache, &name).then_some((s.id, name))
        })
        .collect();
    now.sort();
    if now != state.heroes {
        for k in kids.into_iter().flatten() {
            commands.entity(*k).despawn();
        }
        for (id, name) in &now {
            let icon = unit_icon(&game, name).and_then(|n| textures.get(&mut images, &n)).map(|t| t.handle);
            let portrait = commands
                .spawn((
                    TacticalHud,
                    HeroPortrait(*id),
                    Name::new(format!("hero {name}")),
                    Button,
                    Node {
                        width: Val::Px(36.0),
                        height: Val::Px(36.0),
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BorderColor::all(Color::srgb(0.3, 0.9, 0.4)),
                    BackgroundColor(Color::srgba(0.0, 0.05, 0.1, 0.8)),
                    ChildOf(row),
                ))
                .id();
            if let Some(h) = icon {
                commands.spawn((
                    Node { width: Val::Percent(100.0), height: Val::Percent(100.0), border_radius: BorderRadius::MAX, ..default() },
                    ImageNode::new(h),
                    ChildOf(portrait),
                ));
            }
            commands.spawn((
                HeroHealth(*id),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(4.0),
                    bottom: Val::Px(-5.0),
                    width: Val::Px(26.0),
                    height: Val::Px(2.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.25, 0.95, 0.3)),
                ChildOf(portrait),
            ));
        }
        state.heroes = now;
        return;
    }
    let t = time.elapsed_secs();
    for (p, interaction, mut border) in &mut portraits {
        let selected = selection.ships.contains(&p.0);
        *border = BorderColor::all(if selected { Color::srgb(1.0, 0.6, 0.25) } else { Color::srgb(0.3, 0.9, 0.4) });
        if *interaction == Interaction::Pressed {
            let double = state.last_click.is_some_and(|(id, at)| id == p.0 && t - at < 0.4);
            selection.ships = vec![p.0];
            selection.enemy = None;
            if double {
                focus.write(FocusCamera);
            }
            state.last_click = Some((p.0, t));
        }
    }
    for (h, mut node) in &mut fills {
        if let Some(s) = ships.0.get(&h.0) {
            node.width = Val::Px(26.0 * s.hull.clamp(0.0, 1.0));
        }
    }
}

/// Removes the HUD with the battle.
pub fn remove_hud(mut commands: Commands, hud: Query<Entity, With<TacticalHud>>, mut state: ResMut<HudState>) {
    for e in &hud {
        commands.entity(e).despawn();
    }
    *state = HudState::default();
}
