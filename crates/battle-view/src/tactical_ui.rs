//! The original EaW space tactical bar (`eaw-ui`) for a running battle: selection panel with
//! health and shield bars, ability buttons, retreat countdown and population, and the clicks coming back
//! as sim commands.
//!
//! Needs `eaw_ui::EawUiPlugin` (with `with_game_data`) next to `BattleViewPlugin`. Add [`TacticalUiPlugin`]
//! to get the bar; it spawns with each [`BattleRuntime`] and despawns with it. Not covered: reinforcement
//! buttons (the sim supports `Command::Reinforce`, the galaxy request has no reinforcement pool yet) and
//! eaw-ui's world-space ship overlays (the gizmo bars of `control::overlays` stay in use).

use std::collections::BTreeMap;

use battle_sim::{Command, HpKind};
use bevy::prelude::*;
use eaw_ui::textures::UiTextures;
use eaw_ui::{
    AbilityState, CommandBarMode, CommandBarSpec, EawAbilityClicked, EawButtonClicked, EawCommandBar, EawHardpointClicked, EawMinimapClicked,
    EawSelectionClicked, EawUiAssets, EawUiCommandsExt, HardpointMarker, MinimapData, MinimapDot, PanelData, PanelSlot, RetreatState,
    ShellAnchor, ShellSpec, ShipOverlay, TacticalData, TacticalOverlays, TacticalUnit, UiFaction, spawn_shell,
};

use crate::camera::BattleCamera;

use crate::control::{self, Hover, OverlaysActive, Selection};
use crate::visuals::is_markable;
use crate::ships::RenderedShips;
use crate::{BattleRuntime, BattleStatus, BattleViewSettings};

/// Adds the tactical command bar to battles.
pub struct TacticalUiPlugin;

impl Plugin for TacticalUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TacticalUiState>().init_resource::<ReinforceUi>().init_resource::<crate::hud::HudState>().insert_resource(OverlaysActive);
        crate::input_test::build(app);
        app.add_systems(
            Update,
            (
                control::ui_block.after(control::read_pointer).before(control::selection).run_if(resource_exists::<BattleRuntime>),
                (crate::hud::spawn_hud, crate::hud::advisor, crate::hud::heroes).chain().run_if(resource_exists::<BattleRuntime>),
                crate::hud::remove_hud.run_if(resource_removed::<BattleRuntime>.or_eager(resource_removed::<crate::BattleSession>)),
                init_reinforce.run_if(resource_added::<BattleRuntime>),
                spawn_bar.run_if(resource_exists::<crate::BattleSession>.and_then(not(resource_exists::<TacticalBarSpawned>))),
                remove_reinforce.run_if(resource_removed::<BattleRuntime>),
                remove_bar.run_if(resource_removed::<BattleRuntime>.or_eager(resource_removed::<crate::BattleSession>)),
                (sync_data, sync_overlays, handle_clicks, hardpoint_clicks, reinforce_clicks, reinforce_events, reinforce_panel, sync_minimap, minimap_pan).chain().run_if(resource_exists::<BattleRuntime>),
            )
                .chain(),
        );
    }
}

/// Which ships each selection slot of the bar stands for (same order as [`TacticalData::selection`]).
#[derive(Resource, Default, Debug, Clone)]
pub struct TacticalUiState {
    /// Ships per slot.
    pub groups: Vec<Vec<u32>>,
}

/// Marks that the bar of the current battle was spawned.
#[derive(Resource)]
struct TacticalBarSpawned;

/// Spawns the bar as soon as the [`BattleSession`](crate::BattleSession) exists, i.e. while the loading screen
/// is still up, not in the frame where the ships spawn (the whole command bar tree is one costly command).
fn spawn_bar(mut commands: Commands, session: Res<crate::BattleSession>) {
    let faction = UiFaction::from_name(session.player_faction.as_deref().unwrap_or("Empire"));
    commands.spawn_eaw_command_bar(CommandBarSpec { mode: CommandBarMode::SpaceTactical, faction });
    commands.insert_resource(TacticalBarSpawned);
}

fn remove_bar(mut commands: Commands, bars: Query<Entity, With<EawCommandBar>>, mut tac: ResMut<TacticalData>) {
    for e in &bars {
        commands.entity(e).despawn();
    }
    tac.active = false;
    commands.remove_resource::<TacticalBarSpawned>();
}

/// Fills [`TacticalData`] from the selection and the displayed step.
#[allow(clippy::too_many_arguments)]
fn sync_data(
    rt: Res<BattleRuntime>,
    sel: Res<Selection>,
    status: Res<BattleStatus>,
    settings: Res<BattleViewSettings>,
    ships: Res<RenderedShips>,
    reinforce: Res<ReinforceUi>,
    assets: Option<Res<EawUiAssets>>,
    textures: Option<Res<UiTextures>>,
    mut tac: ResMut<TacticalData>,
    mut state: ResMut<TacticalUiState>,
) {
    let (Some(assets), Some(textures), Some(cur)) = (assets, textures, rt.cur.clone()) else { return };
    let Some(game) = assets.game.clone() else { return };
    let player = settings.player_side;

    // One slot per unit type of the selection.
    let mut by_def: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &id in &sel.ships {
        if let Some(s) = cur.ship(id).filter(|s| s.side == player) {
            by_def.entry(s.def).or_default().push(id);
        }
    }
    let mut selection = Vec::new();
    let mut groups = Vec::new();
    for (def, ids) in by_def {
        let sdef = rt.catalog.ship(def);
        let Some(mut unit) = TacticalUnit::from_game(&game, &assets.text, &textures, &sdef.name) else { continue };
        let n = ids.len() as f32;
        let (mut hull, mut shield) = (0.0, 0.0);
        for id in &ids {
            if let Some(s) = ships.0.get(id) {
                hull += s.hull;
                shield += s.shield;
            }
        }
        unit.health = hull / n;
        unit.shield = unit.shield.map(|_| shield / n);
        unit.count = ids.len() as u32;
        // Ability buttons follow the first ship of the slot.
        if let Some(first) = cur.ship(ids[0]) {
            for a in &mut unit.abilities {
                let Some((i, adef)) = sdef.abilities.iter().enumerate().find(|(_, d)| d.kind.eq_ignore_ascii_case(&a.def.kind)) else { continue };
                let (active, cooldown) = first.abilities.get(i).copied().unwrap_or((0.0, 0.0));
                *a = AbilityState {
                    def: a.def.clone(),
                    cooldown: if adef.recharge > 0.0 { (cooldown / adef.recharge).clamp(0.0, 1.0) } else { 0.0 },
                    active: active > 0.0,
                    disabled: status.over,
                };
            }
        }
        selection.push(unit);
        groups.push(ids);
    }

    let retreat = if status.over {
        RetreatState::Unavailable
    } else if status.time >= rt.retreat_countdown {
        RetreatState::Ready
    } else {
        RetreatState::Countdown(rt.retreat_countdown - status.time)
    };
    let new = TacticalData {
        active: true,
        focus: tac.focus.min(selection.len().saturating_sub(1)),
        selection,
        reinforcement_points: (!reinforce.rows.is_empty()).then(|| reinforce.rows.iter().map(|r| r.2).sum()),
        pop: Some((status.alive.get(player as usize).copied().unwrap_or(0), rt.unit_cap.max(0) as u32)),
        retreat,
    };
    if *tac != new {
        *tac = new;
    }
    state.groups = groups;
}

/// Turns clicks on the bar into selection changes and commands.
#[allow(clippy::too_many_arguments)]
fn handle_clicks(
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    state: Res<TacticalUiState>,
    mut sel: ResMut<Selection>,
    mut tac: ResMut<TacticalData>,
    mut buttons: MessageReader<EawButtonClicked>,
    mut abilities: MessageReader<EawAbilityClicked>,
    mut slots: MessageReader<EawSelectionClicked>,
) {
    let player = settings.player_side;
    for b in buttons.read() {
        if b.name == "b_retreat" {
            rt.command(player, Command::Retreat);
        }
    }
    for a in abilities.read() {
        let Some(group) = tac.selection.get(tac.focus).and_then(|_| state.groups.get(tac.focus)) else { continue };
        for &id in group {
            let Some(s) = rt.cur.as_ref().and_then(|c| c.ship(id)) else { continue };
            let defs = &rt.catalog.ship(s.def).abilities;
            if let Some(i) = defs.iter().position(|d| d.kind.eq_ignore_ascii_case(&a.ability)) {
                rt.command(player, Command::UseAbility { ship: id, ability: i as u16 });
            }
        }
    }
    for c in slots.read() {
        let Some(group) = state.groups.get(c.index) else { continue };
        if c.shift {
            sel.ships.retain(|id| !group.contains(id));
        } else {
            sel.ships = group.clone();
            tac.focus = c.index;
        }
    }
}

/// The tactical minimap: map bounds, every ship in its side's colour and the camera's view.
/// Minimap coordinates are map space, x right and y up (Bevy `-z`).
fn sync_minimap(
    time: Res<Time>,
    rt: Res<BattleRuntime>,
    ships: Res<RenderedShips>,
    camera: Single<(&Camera, &BattleCamera, &Projection)>,
    mut data: ResMut<MinimapData>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since < 0.1 && data.bounds.is_some() {
        return;
    }
    *since = 0.0;
    let (cam, orbit, projection) = *camera;
    let flip = |p: Vec3| Vec2::new(p.x, -p.z);
    let bounds = (Vec2::new(orbit.bounds_min.x, -orbit.bounds_max.y), Vec2::new(orbit.bounds_max.x, -orbit.bounds_min.y));
    let units = ships
        .0
        .values()
        .map(|s| MinimapDot { pos: flip(s.pos), color: rt.visuals.team_color(s.side), selected: false })
        .collect();
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => 0.8,
    };
    let aspect = cam.logical_viewport_size().map_or(1.78, |s| s.x / s.y.max(1.0));
    let half_h = orbit.distance * (fov * 0.5).tan() / orbit.pitch.sin().max(0.3);
    let center = flip(orbit.focus);
    let half = Vec2::new(half_h * aspect, half_h);
    let planets = rt.minimap_props.iter().map(|(pos, _)| MinimapDot { pos: *pos, color: Color::srgb(0.55, 0.5, 0.45), selected: false }).collect();
    let next = MinimapData { bounds: Some(bounds), planets, units, camera: Some((center - half, center + half)), background: Some(rt.minimap_tint) };
    if *data != next {
        *data = next;
    }
}

/// Click or drag on the minimap moves the battle camera.
fn minimap_pan(mut clicks: MessageReader<EawMinimapClicked>, mut camera: Single<&mut BattleCamera>) {
    if let Some(c) = clicks.read().last() {
        let y = camera.target_focus.y;
        camera.target_focus = Vec3::new(c.world.x, y, -c.world.y);
    }
}

/// The reinforcement window (`b_reinforcement`): the units of the player's fleets at the planet that did not fit under the
/// space population cap. A click on a slot calls one unit in (shift: all), it arrives after the reinforcement delay if the cap allows.
#[derive(Resource, Default, Debug)]
pub struct ReinforceUi {
    /// Root of the open window.
    pub root: Option<Entity>,
    /// Reserve per unit type: object name, battle ship id, units not called in yet.
    pub rows: Vec<(String, u32, u32)>,
    /// The refusal message on screen and the seconds it stays.
    notice: Option<(Entity, f32)>,
}

/// Reads the reserve of the player's side from the request.
fn init_reinforce(rt: Res<BattleRuntime>, settings: Res<BattleViewSettings>, mut ui: ResMut<ReinforceUi>) {
    ui.root = None;
    ui.rows.clear();
    let Some(side) = rt.request.as_ref().and_then(|r| r.sides.get(settings.player_side as usize)) else { return };
    for g in &side.reserve {
        let Some(ship) = rt.catalog.ship_id(&g.name) else { continue };
        match ui.rows.iter_mut().find(|r| r.1 == ship) {
            Some(r) => r.2 += g.count,
            None => ui.rows.push((g.name.clone(), ship, g.count)),
        }
    }
    info!("reinforcements: {} reserve unit types, cap {}", ui.rows.len(), rt.unit_cap);
}

fn remove_reinforce(mut commands: Commands, mut ui: ResMut<ReinforceUi>, mut panel: ResMut<PanelData>) {
    if let Some(e) = ui.root.take() {
        commands.entity(e).despawn();
    }
    ui.rows.clear();
    panel.slots.retain(|k, _| !k.starts_with("r_"));
    panel.texts.remove("r_pop_text");
}

/// Slot name `r_RRCC` to the index in the row list (4 columns).
fn reserve_slot(name: &str) -> Option<usize> {
    let digits = name.strip_prefix("r_")?;
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (row, col): (usize, usize) = (digits[..2].parse().ok()?, digits[2..].parse().ok()?);
    Some(row * 4 + col)
}

/// Opens and closes the window, and calls units in.
fn reinforce_clicks(
    mut clicks: MessageReader<EawButtonClicked>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    mut ui: ResMut<ReinforceUi>,
    mut commands: Commands,
) {
    for c in clicks.read() {
        match c.name.as_str() {
            "b_reinforcement" | "r_close" => {
                if let Some(e) = ui.root.take() {
                    commands.entity(e).despawn();
                } else if c.name == "b_reinforcement" && !ui.rows.is_empty() {
                    info!("reinforcements: opening the window ({} unit types in the reserve)", ui.rows.len());
                    let faction = UiFaction::from_name(rt.player_faction.as_deref().unwrap_or("Empire"));
                    // `reinforcement_window_ALTn` is the frame for n + 1 rows.
                    let rows = ui.rows.len().div_ceil(4).clamp(1, 3);
                    commands.queue(move |world: &mut World| {
                        let mut spec = ShellSpec::new("I_Main_Reinforce.ALO", faction);
                        spec.art_alt = Some(rows - 1);
                        spec.anchor = ShellAnchor::TopLeft;
                        spec.offset = Vec2::new(30.0, -60.0);
                        if let Some(e) = spawn_shell(world, &spec) {
                            world.resource_mut::<ReinforceUi>().root = Some(e);
                        }
                    });
                }
            }
            name => {
                let Some(idx) = reserve_slot(name) else { continue };
                if ui.root.is_none() {
                    continue;
                }
                if let Some(row) = ui.rows.get_mut(idx).filter(|r| r.2 > 0) {
                    let count = if c.shift { row.2 } else { 1 };
                    // The count drops when the simulation reports the call (`ReinforcementsCalled`).
                    rt.command(settings.player_side, Command::Reinforce { ship: row.1, count });
                }
            }
        }
    }
}

/// Fills the window slots (icon, units left) and the population text.
fn reinforce_panel(
    ui: Res<ReinforceUi>,
    rt: Res<BattleRuntime>,
    status: Res<BattleStatus>,
    settings: Res<BattleViewSettings>,
    assets: Option<Res<EawUiAssets>>,
    mut panel: ResMut<PanelData>,
) {
    if ui.root.is_none() {
        return;
    }
    let game = assets.and_then(|a| a.game.clone());
    for k in 0..12 {
        let name = format!("r_{:02}{:02}", k / 4, k % 4);
        match ui.rows.get(k) {
            Some((object, _, left)) => {
                let slot = PanelSlot {
                    icon: game.as_deref().and_then(|g| eaw_ui::unit_icon(g, object)).unwrap_or_default(),
                    label: left.to_string(),
                    disabled: *left == 0,
                    ..default()
                };
                if panel.slots.get(&name) != Some(&slot) {
                    panel.set_slot(&name, slot);
                }
            }
            None => {
                if panel.slots.contains_key(&name) {
                    panel.slots.remove(&name);
                }
            }
        }
    }
    let alive = status.alive.get(settings.player_side as usize).copied().unwrap_or(0);
    let cap = rt.request.as_ref().and_then(|r| r.sides.get(settings.player_side as usize)).map_or(0, |s| s.unit_cap);
    let text = format!("{alive}/{cap}");
    if panel.texts.get("r_pop_text") != Some(&text) {
        panel.set_text("r_pop_text", text);
    }
}

/// Follows the simulation: units leave the reserve when a call is accepted, a refusal shows a notice and plays a sound.
fn reinforce_events(
    time: Res<Time>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    mut ui: ResMut<ReinforceUi>,
    mut commands: Commands,
    mut sfx: MessageWriter<alamo_audio::PlaySfx>,
) {
    use battle_sim::Event;
    for ev in &rt.fresh_events {
        match ev {
            Event::ReinforcementsCalled { side, ship, count } if *side == settings.player_side => {
                info!("reinforcements: simulation accepted {count} of ship {ship}");
                if let Some(row) = ui.rows.iter_mut().find(|r| r.1 == *ship) {
                    row.2 = row.2.saturating_sub(*count);
                }
            }
            Event::ReinforcementsRefused { side, .. } if *side == settings.player_side => {
                info!("reinforcements: call refused by the simulation (unit cap)");
                if let Some((e, _)) = ui.notice.take() {
                    commands.entity(e).despawn();
                }
                let e = commands
                    .spawn((
                        crate::BattleEntity,
                        Text::new("Cannot call in reinforcements: unit cap reached"),
                        TextFont { font_size: FontSize::Px(20.0), ..default() },
                        TextColor(Color::srgb(1.0, 0.6, 0.45)),
                        Node { position_type: PositionType::Absolute, top: Val::Px(40.0), left: Val::Percent(35.0), ..default() },
                    ))
                    .id();
                ui.notice = Some((e, 3.0));
                sfx.write(alamo_audio::PlaySfx { event: "GUI_Cancel_SFX".to_string(), position: None, source: None });
            }
            _ => {}
        }
    }
    if let Some((e, left)) = ui.notice {
        let left = left - time.delta_secs();
        if left <= 0.0 {
            commands.entity(e).despawn();
            ui.notice = None;
        } else {
            ui.notice = Some((e, left));
        }
    }
}

/// Reticle kind of a hardpoint (`I_HARD_POINT_RETICLE_<KIND>`).
fn reticle_kind(kind: HpKind) -> &'static str {
    match kind {
        HpKind::Weapon => "WEAPONS",
        HpKind::ShieldGenerator => "SHIELD_GEN",
        HpKind::Engine => "ENGINES",
        HpKind::FighterBay => "DOCKING_BAY",
        HpKind::TractorBeam => "GRAV_WELL",
        _ => "GENERIC",
    }
}

/// Marker id of a hardpoint: `<ship>:<hardpoint index>`.
fn hardpoint_id(ship: u32, hp: usize) -> String {
    format!("{ship}:{hp}")
}

/// Most own ships that get an always-on health line (the rest only when selected or hovered).
const MAX_BAR_ONLY: usize = 200;

/// Feeds eaw-ui's world-space overlays: selection brackets, health/shield lines under own ships (always), the
/// hovered or selected enemy and the hardpoint reticles of the targeted enemy.
#[allow(clippy::too_many_arguments)]
fn sync_overlays(
    rt: Res<BattleRuntime>,
    sel: Res<Selection>,
    hover: Res<Hover>,
    ships: Res<RenderedShips>,
    settings: Res<BattleViewSettings>,
    assets: Option<Res<EawUiAssets>>,
    camera: Single<(&Camera, &GlobalTransform), With<BattleCamera>>,
    mut out: ResMut<TacticalOverlays>,
) {
    let (cam, gt) = *camera;
    let Some(vp) = cam.logical_viewport_size() else { return };
    let player = settings.player_side;
    let right = gt.right();
    let margin = 120.0;
    let mut next = TacticalOverlays::default();
    let mut bar_only = 0;
    let mut ids: Vec<u32> = ships.0.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let s = &ships.0[&id];
        let enemy = s.side != player;
        let selected = sel.ships.contains(&id) || sel.enemy == Some(id);
        let hovered = hover.ship == Some(id);
        if enemy && !selected && !hovered {
            continue;
        }
        let always = !enemy && !selected && !hovered;
        if always {
            if bar_only >= MAX_BAR_ONLY {
                continue;
            }
            bar_only += 1;
        }
        let Ok(c) = cam.world_to_viewport(gt, s.pos) else { continue };
        if c.x < -margin || c.y < -margin || c.x > vp.x + margin || c.y > vp.y + margin {
            continue;
        }
        let edge = cam.world_to_viewport(gt, s.pos + right * s.radius).map_or(8.0, |e| (e - c).length());
        let def = rt.catalog.ship(s.def);
        let name = (selected || hovered).then(|| {
            def.text_id
                .as_deref()
                .and_then(|k| assets.as_ref().and_then(|a| a.text.get(k)))
                .filter(|t| !t.is_empty())
                .map_or_else(|| def.name.clone(), str::to_string)
        });
        next.ships.push(ShipOverlay {
            id: u64::from(id),
            screen_pos: c,
            half_size: edge.clamp(8.0, 220.0),
            health: s.hull,
            shield: (def.shield > 0.0 && !always).then_some(s.shield),
            selected,
            hovered,
            enemy,
            name,
            always,
        });
    }
    // Hardpoint reticles on the enemy under the cursor (or the selected one) while own ships are selected.
    let target = hover.ship.filter(|id| ships.0.get(id).is_some_and(|s| s.side != player)).or(sel.enemy);
    if let Some(s) = target.and_then(|id| ships.0.get(&id)).filter(|_| !sel.ships.is_empty()) {
        let def = rt.catalog.ship(s.def);
        for (i, h) in def.hardpoints.iter().enumerate() {
            if !h.targetable || !h.destroyable || !is_markable(h.kind) || !s.hp_alive.get(i).copied().unwrap_or(false) {
                continue;
            }
            let Ok(p) = cam.world_to_viewport(gt, control::hardpoint_world(&rt, s, i)) else { continue };
            next.hardpoints.push(HardpointMarker {
                id: hardpoint_id(s.id, i),
                kind: reticle_kind(h.kind).to_string(),
                tracked: hover.hardpoint == Some((s.id, i as u16)),
                screen_pos: p,
                health: 1.0,
                targetable: true,
                destroyed: false,
            });
        }
    }
    if *out != next {
        *out = next;
    }
}

/// A click on a hardpoint reticle sends the selected ships to attack that hardpoint.
fn hardpoint_clicks(
    mut clicks: MessageReader<EawHardpointClicked>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    sel: Res<Selection>,
    ships: Res<RenderedShips>,
) {
    for c in clicks.read() {
        let Some((target, hp)) = c.id.split_once(':').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u16>().ok()?))) else { continue };
        let mine: Vec<u32> = sel
            .ships
            .iter()
            .copied()
            .filter(|id| ships.0.get(id).is_some_and(|s| s.side == settings.player_side && !rt.catalog.ship(s.def).is_static))
            .collect();
        if !mine.is_empty() {
            rt.command(settings.player_side, Command::AttackHardpoint { ships: mine, target, hardpoint: hp });
        }
    }
}
