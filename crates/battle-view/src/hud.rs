//! Extra parts of the EaW space tactical screen that are not in the command bar model: the R2 advisor hint
//! bubble, the "Mission Objectives" box (bottom right) and the hero portraits (top right).
//!
//! Texts come from the game's string table: `TEXT_DROID_ADVISOR_<FACTION>_SPACE_NN`, `TEXT_TEST_OBJECTIVE_HEADER`,
//! `TEXT_TACTICAL_PRIMARY_SPACE_ATTACK` / `_DEFEND[_<ATTACKER>]`. Positions are design pixels (1366x768, scaled by
//! eaw-ui's `UiScale`) measured from the reference screenshot of the real game.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

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

/// Marks the root nodes this module spawns (children go with them).
#[derive(Component)]
pub struct TacticalHud;

/// The advisor bubble and its text.
#[derive(Component)]
pub(crate) struct AdvisorBubble;

#[derive(Component)]
pub(crate) struct AdvisorText;

/// Portrait ring: dark when the hero is not selected, orange glow when selected (Remake ref 07 and video b0700).
const HERO_RING_IDLE: Color = Color::srgba(0.25, 0.3, 0.36, 0.9);
const HERO_RING_SELECTED: Color = Color::srgb(1.0, 0.58, 0.22);
const HERO_BAR_W: f32 = 42.0;

/// A hero portrait of the player's side.
#[derive(Component)]
pub(crate) struct HeroPortrait(u32);

/// Health fill inside a hero portrait.
#[derive(Component)]
pub(crate) struct HeroHealth(u32);

/// A hero's face floating next to its ship in the battle (a little portrait box, like the real game).
#[derive(Component)]
pub(crate) struct HeroBillboard(u32);

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
    /// Ship under the cursor and since when (the tooltip waits a moment).
    hover_since: Option<(u32, f32)>,
    /// The droid button was pressed: show the next hint now.
    pub show_now: bool,
    /// The objectives box is shown (`b_story_arc_t` toggles it).
    pub objectives: bool,
}

/// The panel of the unit under the cursor and its parts.
#[derive(Component)]
pub(crate) struct UnitPanel;
#[derive(Component)]
pub(crate) struct UnitPanelIcon;
#[derive(Component)]
pub(crate) struct UnitPanelName;
#[derive(Component)]
pub(crate) struct UnitPanelClass;
#[derive(Component)]
pub(crate) struct UnitPanelDesc;
#[derive(Component)]
pub(crate) struct UnitPanelStats;
/// Tooltip of the hardpoint reticle under the cursor.
#[derive(Component)]
pub(crate) struct HardpointTip;

/// Radius of a weapon hit area as a share of the hull radius (estimate).
const HARDPOINT_RADIUS_OF_HULL: f32 = 0.04;

/// Seconds the cursor rests on a ship before its tooltip shows (estimate).
const TOOLTIP_DELAY: f32 = 0.35;

/// The mission objectives box.
#[derive(Component)]
pub(crate) struct ObjectivesBox;

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
    state.objectives = true;
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
        ObjectivesBox,
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
    // Small caps like the original heading: a full-size first letter per word, smaller capitals after it.
    let head_font = font(&ui, 14.0);
    let head_color = Color::srgb(0.84, 0.9, 0.98);
    let segs = eaw_ui::small_caps_segments(&header);
    let head = commands.spawn((Text::new(segs.first().map(|s| s.0.clone()).unwrap_or_default()), head_font.clone(), TextColor(head_color), ChildOf(boxed))).id();
    for (s, small) in segs.into_iter().skip(1) {
        let mut f = head_font.clone();
        if small && let FontSize::Px(px) = f.font_size {
            f.font_size = FontSize::Px(px * 0.78);
        }
        commands.spawn((TextSpan::new(s), f, TextColor(head_color), ChildOf(head)));
    }
    commands.spawn((
        Text::new(format!("1. {body}")),
        font(&ui, 10.5),
        TextColor(Color::srgb(0.9, 0.93, 0.97)),
        Node { margin: UiRect::left(Val::Px(6.0)), ..default() },
        ChildOf(boxed),
    ));

    // Unit panel: the real game shows this cyan glass panel above R2 for the unit under the cursor (icon, name, class, description).
    let mut panel = commands.spawn((
        TacticalHud,
        UnitPanel,
        Name::new("unit panel"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(17.0),
            bottom: Val::Px(309.0),
            width: Val::Px(423.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(2.0)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgba(0.06, 0.3, 0.34, 0.6)),
        BorderColor::all(Color::srgb(0.3, 0.85, 0.95)),
        GlobalZIndex(75),
        bevy::ui::FocusPolicy::Pass,
    ));
    target(&mut panel);
    let panel = panel.id();
    let head = commands.spawn((Node { column_gap: Val::Px(8.0), align_items: AlignItems::Center, ..default() }, ChildOf(panel))).id();
    commands.spawn((UnitPanelIcon, Node { width: Val::Px(40.0), height: Val::Px(34.0), ..default() }, ImageNode::default(), ChildOf(head)));
    commands.spawn((UnitPanelName, Text::new(""), font(&ui, 14.0), TextColor(Color::srgb(0.88, 0.95, 1.0)), ChildOf(head)));
    commands.spawn((UnitPanelClass, Text::new(""), font(&ui, 11.0), TextColor(Color::srgb(0.9, 0.95, 1.0)), ChildOf(panel)));
    commands.spawn((UnitPanelDesc, Text::new(""), font(&ui, 11.0), TextColor(Color::srgb(0.9, 0.95, 1.0)), ChildOf(panel)));
    commands.spawn((UnitPanelStats, Text::new(""), font(&ui, 11.0), TextColor(Color::srgb(0.3, 1.0, 0.4)), ChildOf(panel)));

    // Hardpoint tooltip ("Quad Turbo Laser Battery - 120 DMG - 100%"), follows the cursor.
    let mut tip = commands.spawn((
        TacticalHud,
        HardpointTip,
        Name::new("hardpoint tip"),
        Text::new(""),
        font(&ui, 11.0),
        TextColor(Color::srgb(0.9, 1.0, 0.95)),
        Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.03, 0.12, 0.14, 0.92)),
        BorderColor::all(Color::srgb(0.3, 0.85, 0.95)),
        GlobalZIndex(80),
        bevy::ui::FocusPolicy::Pass,
    ));
    target(&mut tip);

    // Hero portraits, top right.
    let mut row = commands.spawn((
        TacticalHud,
        HeroRow,
        Name::new("hero portraits"),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(8.0),
            top: Val::Px(4.0),
            column_gap: Val::Px(8.0),
            ..default()
        },
        GlobalZIndex(60),
    ));
    target(&mut row);
}

/// The unit panel (icon, name, class, description, hull and shield) for the ship under the cursor and the hardpoint tooltip for a reticle.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn unit_tooltip(
    time: Res<Time>,
    inputs: (Res<crate::control::Hover>, Res<crate::control::Pointer>, Res<crate::control::UiBlock>),
    camera: Single<(&Camera, &GlobalTransform), With<crate::camera::BattleCamera>>,
    rt: Res<BattleRuntime>,
    ships: Res<RenderedShips>,
    assets: Option<Res<EawUiAssets>>,
    textures: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    scale: Res<UiScale>,
    overlays: Res<eaw_ui::TacticalOverlays>,
    mut state: ResMut<HudState>,
    mut panel: Query<&mut Node, (With<UnitPanel>, Without<HardpointTip>)>,
    mut tip: Query<(&mut Node, &mut Text), (With<HardpointTip>, Without<UnitPanel>)>,
    mut parts: ParamSet<(
        Query<&mut ImageNode, With<UnitPanelIcon>>,
        Query<&mut Text, (With<UnitPanelName>, Without<HardpointTip>)>,
        Query<&mut Text, (With<UnitPanelClass>, Without<HardpointTip>)>,
        Query<&mut Text, (With<UnitPanelDesc>, Without<HardpointTip>)>,
        Query<&mut Text, (With<UnitPanelStats>, Without<HardpointTip>)>,
    )>,
    mut bubble: Query<&mut Visibility, With<AdvisorBubble>>,
) {
    let (Ok(mut panel), Ok((mut tip_node, mut tip_text))) = (panel.single_mut(), tip.single_mut()) else { return };
    let (hover, pointer, block) = inputs;
    let (cam, gt) = *camera;
    let now = time.elapsed_secs();
    let hovered = hover.ship.filter(|_| !block.0);
    match (hovered, state.hover_since) {
        (Some(id), Some((cur, _))) if id == cur => {}
        (Some(id), _) => state.hover_since = Some((id, now)),
        (None, _) => state.hover_since = None,
    }
    let Some(cursor) = pointer.cursor else {
        if panel.display != Display::None {
            panel.display = Display::None;
        }
        if tip_node.display != Display::None {
            tip_node.display = Display::None;
        }
        return;
    };
    let game = assets.as_ref().and_then(|a| a.game.clone());

    // A hardpoint reticle under the cursor wins.
    // The hit area of a weapon is its projected size (a few percent of the hull radius, estimate: the bone has no extent in the data), at least 24 px
    // around the reticle so that the edge of a turret counts too; the closest one wins.
    let near = overlays
        .hardpoints
        .iter()
        .filter_map(|h| {
            let sid = h.id.split_once(':').and_then(|(a, _)| a.parse::<u32>().ok())?;
            let s = ships.0.get(&sid)?;
            let c = cam.world_to_viewport(gt, s.pos).ok()?;
            let e = cam.world_to_viewport(gt, s.pos + gt.right() * s.radius * HARDPOINT_RADIUS_OF_HULL).ok()?;
            let reach = (e - c).length().max(24.0 * scale.0);
            let d = h.screen_pos.distance(cursor);
            (d < reach).then_some((h, d / reach))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let (Some((marker, _)), false) = (near, block.0) {
        let parsed = marker.id.split_once(':').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<usize>().ok()?)));
        if let Some((sid, hp)) = parsed
            && let Some(s) = ships.0.get(&sid)
            && let Some(hdef) = rt.catalog.ship(s.def).hardpoints.get(hp)
        {
            let name = game
                .as_ref()
                .and_then(|g| g.objects.resolve(&hdef.name).and_then(|r| r.get("tooltip_text").or_else(|| r.get("text_id")).and_then(|v| v.split_whitespace().next().map(str::to_string))))
                .and_then(|k| assets.as_ref().and_then(|a| a.text.get(k.trim()).map(str::to_string)))
                .unwrap_or_else(|| hdef.name.replace('_', " "));
            let dmg = hdef.projectile.and_then(|p| rt.catalog.projectiles.get(p as usize)).map(|p| p.damage);
            let mut msg = name;
            // Data tooltips usually carry their own damage ("... - 100 DMG - ... - 65 DMG"); ours only fills the gap.
            if let Some(d) = dmg.filter(|_| !msg.to_ascii_uppercase().contains("DMG")) {
                msg.push_str(&format!(" - {d:.0} DMG"));
            }
            msg.push_str(&format!(" - {:.0}%", marker.health * 100.0));
            let glyphs = msg.chars().count();
            if tip_text.0 != msg {
                tip_text.0 = msg;
            }
            tip_node.display = Display::Flex;
            // Keep the box on screen: the width is estimated from the text (11 px font, about 0.6 em per glyph, plus padding).
            let est = (glyphs as f32 * 11.0 * 0.6 + 14.0) * scale.0;
            let (vw, vh) = cam.physical_viewport_size().map(|v| (v.x as f32, v.y as f32)).unwrap_or((f32::MAX, f32::MAX));
            let x = (cursor.x + 16.0).min(vw - est - 4.0).max(0.0);
            let y = (cursor.y + 18.0).min(vh - 26.0 * scale.0).max(0.0);
            tip_node.left = Val::Px(x / scale.0);
            tip_node.top = Val::Px(y / scale.0);
            if panel.display != Display::None {
                panel.display = Display::None;
            }
            return;
        }
    }
    if tip_node.display != Display::None {
        tip_node.display = Display::None;
    }

    let show = state.hover_since.filter(|(_, at)| now - at >= TOOLTIP_DELAY);
    let Some(s) = show.and_then(|(id, _)| ships.0.get(&id)) else {
        if panel.display != Display::None {
            panel.display = Display::None;
        }
        for mut v in &mut bubble {
            if *v == Visibility::Hidden {
                *v = Visibility::Inherited;
            }
        }
        return;
    };
    let def = rt.catalog.ship(s.def);
    let text = |key: &str| assets.as_ref().and_then(|a| a.text.get(key)).filter(|t| !t.is_empty()).map(str::to_string);
    let obj = game.as_ref().and_then(|g| g.objects.resolve(&def.name));
    let name = def.text_id.as_deref().and_then(text).unwrap_or_else(|| def.name.replace('_', " "));
    let class = obj
        .as_ref()
        .and_then(|r| r.get("encyclopedia_unit_class"))
        .and_then(|k| text(k.trim()))
        .map(|c| format!("Class: {c}"))
        .unwrap_or_default();
    let desc = obj
        .as_ref()
        .and_then(|r| r.get("encyclopedia_text"))
        .and_then(|v| v.split_whitespace().next().and_then(text))
        .unwrap_or_default();
    let mut stats = format!("Hull {:.0} / {:.0}", s.hull * def.hull, def.hull);
    if def.shield > 0.0 {
        stats.push_str(&format!("   Shield {:.0} / {:.0}", s.shield * def.shield, def.shield));
    }
    // Carriers and stations: squadrons still waiting in the hangars.
    if let Some(n) = rt.cur.as_ref().and_then(|c| c.ship(s.id)).map(|ss| ss.reserve_squadrons).filter(|n| *n > 0) {
        stats.push_str(&format!("
Squadrons in reserve: {n}"));
    }
    if let Ok(mut t) = parts.p1().single_mut()
        && t.0 != name.to_uppercase()
    {
        t.0 = name.to_uppercase();
    }
    if let Ok(mut t) = parts.p2().single_mut()
        && t.0 != class
    {
        t.0 = class;
    }
    if let Ok(mut t) = parts.p3().single_mut()
        && t.0 != desc
    {
        t.0 = desc;
    }
    if let Ok(mut t) = parts.p4().single_mut()
        && t.0 != stats
    {
        t.0 = stats;
    }
    if let (Some(g), Some(mut tex)) = (game.as_ref(), textures)
        && let Some(icon) = eaw_ui::unit_icon(g, &def.name)
        && let Some(t) = tex.get(&mut images, &icon)
        && let Ok(mut img) = parts.p0().single_mut()
    {
        img.image = t.handle;
    }
    panel.display = Display::Flex;
    for mut v in &mut bubble {
        *v = Visibility::Hidden;
    }
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
    let due = std::mem::take(&mut state.show_now)
        || match state.hint_at {
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
    ui: Res<EawUiSettings>,
    old_billboards: Query<Entity, With<HeroBillboard>>,
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
            commands.entity(*k).try_despawn();
        }
        for e in &old_billboards {
            commands.entity(e).try_despawn();
        }
        for (id, name) in &now {
            let icon = unit_icon(&game, name).and_then(|n| textures.get(&mut images, &n)).map(|t| t.handle);
            let portrait = commands
                .spawn((
                    HeroPortrait(*id),
                    Name::new(format!("hero {name}")),
                    Button,
                    Node {
                        width: Val::Px(46.0),
                        height: Val::Px(46.0),
                        border: UiRect::all(Val::Px(3.0)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BorderColor::all(HERO_RING_IDLE),
                    BackgroundColor(Color::srgba(0.0, 0.05, 0.1, 0.8)),
                    ChildOf(row),
                ))
                .id();
            if let Some(h) = icon.clone() {
                let mut b = commands.spawn((
                    TacticalHud,
                    HeroBillboard(*id),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        border: UiRect::all(Val::Px(1.0)),
                        display: Display::None,
                        ..default()
                    },
                    ImageNode::new(h),
                    BorderColor::all(Color::srgb(0.85, 0.95, 0.9)),
                    GlobalZIndex(40),
                    bevy::ui::FocusPolicy::Pass,
                ));
                if let Some(cam) = ui.camera {
                    b.insert(UiTargetCamera(cam));
                }
            }
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
                    left: Val::Px(2.0),
                    bottom: Val::Px(-6.0),
                    width: Val::Px(HERO_BAR_W),
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
        *border = BorderColor::all(if selected { HERO_RING_SELECTED } else { HERO_RING_IDLE });
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
            node.width = Val::Px(HERO_BAR_W * s.hull.clamp(0.0, 1.0));
        }
    }
}

/// Shows or hides the objectives box.
pub(crate) fn objectives_visibility(state: Res<HudState>, mut boxes: Query<&mut Node, With<ObjectivesBox>>) {
    for mut n in &mut boxes {
        let want = if state.objectives { Display::Flex } else { Display::None };
        if n.display != want {
            n.display = want;
        }
    }
}

/// Keeps the hero faces next to their ships.
pub(crate) fn hero_billboards(
    ships: Res<RenderedShips>,
    scale: Res<UiScale>,
    camera: Single<(&Camera, &GlobalTransform), With<crate::camera::BattleCamera>>,
    mut boards: Query<(&HeroBillboard, &mut Node)>,
) {
    let (cam, gt) = *camera;
    let vp = cam.logical_viewport_size().unwrap_or(Vec2::new(1920.0, 1080.0));
    for (b, mut node) in &mut boards {
        let shown = ships.0.get(&b.0).and_then(|s| cam.world_to_viewport(gt, s.pos + Vec3::Y * s.radius * 0.5).ok()).filter(|p| p.x > 0.0 && p.y > 0.0 && p.x < vp.x && p.y < vp.y);
        match shown {
            Some(p) => {
                node.display = Display::Flex;
                node.left = Val::Px((p.x + 10.0 * scale.0) / scale.0);
                node.top = Val::Px((p.y - 30.0 * scale.0) / scale.0);
            }
            None => node.display = Display::None,
        }
    }
}

/// Removes the HUD with the battle.
pub fn remove_hud(mut commands: Commands, hud: Query<Entity, With<TacticalHud>>, mut state: ResMut<HudState>) {
    for e in &hud {
        commands.entity(e).try_despawn();
    }
    *state = HudState::default();
}
