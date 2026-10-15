//! The original EaW space tactical bar (`eaw-ui`) for a running battle: selection panel with
//! health and shield bars, ability buttons, retreat countdown and population, and the clicks coming back
//! as sim commands.
//!
//! Needs `eaw_ui::EawUiPlugin` (with `with_game_data`) next to `BattleViewPlugin`. Add [`TacticalUiPlugin`]
//! to get the bar; it spawns with each [`BattleRuntime`] and despawns with it. Not covered: reinforcement
//! buttons (the sim supports `Command::Reinforce`, the galaxy request has no reinforcement pool yet) and
//! eaw-ui's world-space ship overlays (the gizmo bars of `control::overlays` stay in use).
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

use std::collections::BTreeMap;

use battle_sim::{Command, HpKind};
use bevy::prelude::*;
use eaw_ui::textures::UiTextures;
use eaw_ui::{
    AbilityState, CommandBarMode, CommandBarSpec, EawAbilityClicked, EawButtonClicked, EawCommandBar, EawHardpointClicked, EawMinimapClicked,
    EawSelectionClicked, EawUiAssets, EawUiCommandsExt, HardpointMarker, MinimapData, MinimapDot, MinimapSurface, PanelData, PanelSlot, RetreatState,
    ShellAnchor, ShellSpec, ShipOverlay, TacticalData, TacticalOverlays, TacticalUnit, UiFaction, minimap_to_world, spawn_shell,
};

use crate::camera::BattleCamera;

use crate::control::{self, Hover, OverlaysActive, Selection};
use crate::visuals::is_markable;
use crate::ships::{RenderedShips, ShipRender};
use crate::{BattleRuntime, BattleStatus, BattleViewSettings};

/// Adds the tactical command bar to battles.
pub struct TacticalUiPlugin;

impl Plugin for TacticalUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RetreatNotice>().init_resource::<TacticalUiState>().init_resource::<ReinforceUi>().init_resource::<crate::hud::HudState>().insert_resource(OverlaysActive);
        crate::input_test::build(app);
        app.add_systems(Update, death_star_gauge.run_if(resource_exists::<BattleRuntime>));
        app.add_systems(
            Update,
            (
                control::ui_block.after(control::read_pointer).before(control::selection).run_if(resource_exists::<BattleRuntime>),
                (crate::hud::spawn_hud, crate::hud::advisor, crate::hud::heroes, crate::hud::objectives_visibility, crate::hud::hero_billboards, crate::hud::unit_tooltip, fix_tooltips).chain().run_if(resource_exists::<BattleRuntime>),
                check_teardown,
                crate::hud::remove_hud.run_if(resource_removed::<BattleRuntime>.or_eager(resource_removed::<crate::BattleSession>)),
                init_reinforce.run_if(resource_added::<BattleRuntime>),
                spawn_bar.run_if(resource_exists::<crate::BattleSession>.and_then(not(resource_exists::<TacticalBarSpawned>))),
                remove_reinforce.run_if(resource_removed::<BattleRuntime>),
                remove_bar.run_if(resource_removed::<BattleRuntime>.or_eager(resource_removed::<crate::BattleSession>)),
                (sync_data, sync_overlays, handle_clicks, ability_keys, autocast_toggle, autocast_rings, ability_at, hardpoint_clicks, reinforce_drag, reinforce_clicks, reinforce_place, reinforce_events, reinforce_panel, sync_minimap, minimap_pan, command_buttons_state, retreat_banner, victory_banner, pause_banner).chain().run_if(resource_exists::<BattleRuntime>),
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
    /// A targeted ability waits for a click on the map: the ships of the slot and the ability type.
    pub pending_ability: Option<(Vec<u32>, String)>,
    /// The cursor label while a targeted ability is armed.
    ability_label: Option<Entity>,
}

/// Counts frames after a battle ended, then checks that no battle UI is left (a leftover would draw over the galaxy).
#[derive(Resource)]
struct TeardownCheck(u32);

fn check_teardown(
    mut commands: Commands,
    check: Option<ResMut<TeardownCheck>>,
    runtime: Option<Res<BattleRuntime>>,
    leftovers: Query<(), Or<(With<eaw_ui::tactical::ShipOverlayNode>, With<eaw_ui::tactical::HardpointNode>, With<crate::hud::TacticalHud>)>>,
    bars: Query<&EawCommandBar>,
) {
    let Some(mut check) = check else { return };
    if runtime.is_some() {
        // A new battle started in the meantime.
        commands.remove_resource::<TeardownCheck>();
        return;
    }
    check.0 = check.0.saturating_sub(1);
    if check.0 == 0 {
        commands.remove_resource::<TeardownCheck>();
        // The galactic command bar of the map screen is expected; a tactical bar is not.
        let n = leftovers.iter().count() + bars.iter().filter(|b| b.mode == CommandBarMode::SpaceTactical).count();
        if n > 0 {
            error!("tactical: {n} battle UI entities left after the battle ended");
        } else {
            info!("tactical: battle UI cleaned up");
        }
    }
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

fn remove_bar(mut commands: Commands, bars: Query<Entity, With<EawCommandBar>>, mut tac: ResMut<TacticalData>, mut overlays: ResMut<TacticalOverlays>, mut fog: ResMut<eaw_ui::MinimapFog>, mut armed: ResMut<control::ArmedOrder>, mut action: ResMut<crate::camera::ActionCam>, mut sel: ResMut<Selection>, mut settings: ResMut<BattleViewSettings>) {
    // Nothing of this battle may leak into the next one (or into the galaxy).
    armed.0 = None;
    *action = crate::camera::ActionCam::default();
    *sel = Selection::default();
    settings.paused = false;
    if fog.enabled {
        *fog = eaw_ui::MinimapFog::default();
    }
    // The overlay layer outlives the battle: emptying its data lets eaw-ui despawn every ship bar and reticle.
    if !overlays.ships.is_empty() || !overlays.hardpoints.is_empty() {
        *overlays = TacticalOverlays::default();
    }
    commands.insert_resource(TeardownCheck(8));
    for e in &bars {
        commands.entity(e).try_despawn();
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
    // Nothing of ours selected: the bar shows the whole fleet as cards (no abilities); a click on a card selects that type.
    let overview = by_def.is_empty();
    if overview {
        for s in cur.ships.iter().filter(|s| s.side == player) {
            by_def.entry(s.def).or_default().push(s.id);
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
        if overview {
            unit.abilities.clear();
        }
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
    // Bar sections in the Remake order (small ships, fighters, heroes, capitals); stable, so types keep their id order inside one.
    let mut paired: Vec<_> = selection.into_iter().zip(groups).collect();
    paired.sort_by_key(|(u, _)| u.section);
    let (selection, groups): (Vec<_>, Vec<_>) = paired.into_iter().unzip();

    let retreat = if status.over {
        RetreatState::Unavailable
    } else if let Some(jump) = cur.retreat_jump_in.get(player as usize).copied().flatten() {
        // The order was given: the jump countdown runs (pressed button look and m:ss under it).
        RetreatState::Countdown(jump)
    } else if status.time >= rt.retreat_countdown {
        RetreatState::Ready
    } else {
        // Not available yet: the original shows the button dimmed and without text (Remake video b0662).
        RetreatState::Unavailable
    };
    let new = TacticalData {
        active: true,
        focus: if overview { usize::MAX } else { tac.focus.min(selection.len().saturating_sub(1)) },
        selection,
        reinforcement_points: (!reinforce.rows.is_empty()).then(|| reinforce.rows.iter().map(|r| r.2).sum()),
        pop: Some((rt.cur.as_ref().and_then(|c| c.pop.get(player as usize).copied()).unwrap_or(0).max(0) as u32, rt.unit_cap.max(0) as u32)),
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
    mut settings: ResMut<BattleViewSettings>,
    mut state: ResMut<TacticalUiState>,
    mut sel: ResMut<Selection>,
    mut tac: ResMut<TacticalData>,
    mut buttons: MessageReader<EawButtonClicked>,
    mut abilities: MessageReader<EawAbilityClicked>,
    mut slots: MessageReader<EawSelectionClicked>,
    ships: Res<RenderedShips>,
    mut armed: ResMut<control::ArmedOrder>,
    mut action: ResMut<crate::camera::ActionCam>,
    mut hud: ResMut<crate::hud::HudState>,
    mut issued: MessageWriter<control::OrderIssued>,
    audio: Option<Res<alamo_audio::plugin::AlamoAudio>>,
    mut sfx: MessageWriter<alamo_audio::PlaySfx>,
) {
    let player = settings.player_side;
    for b in buttons.read() {
        let mine = || control::selected_mobile(&rt, &ships, &sel, player);
        match b.name.as_str() {
            "b_retreat" => {
                // The announcement follows the sim's `RetreatOrdered` event (`audio::battle_audio`), so a refused order is silent.
                // Before the retreat countdown ran out the sim refuses: the faction's `Space_Retreat_Not_Allowed_SFXEvent` instead.
                if rt.cur.as_ref().is_some_and(|c| c.time < rt.retreat_countdown)
                    && let Some(n) = rt.player_faction.as_deref().zip(audio.as_ref()).and_then(|(f, a)| a.constants.faction_sfx(f, "space_retreat_not_allowed_sfxevent"))
                {
                    sfx.write(alamo_audio::PlaySfx { event: n.to_string(), position: None, source: None });
                }
                rt.command(player, Command::Retreat);
            }
            "b_play_pause_t" => settings.paused = !settings.paused,
            // 1x <-> 2x (the engine's list of speeds is not in the data; the galaxy bar uses the same toggle idea).
            "b_fast_forward_t" => settings.speed = if settings.speed > 1.5 { 1.0 } else { 2.0 },
            "b_camera_t" => action.request = true,
            "b_droid_help_tactical" => hud.show_now = true,
            "b_story_arc_t" => hud.objectives = !hud.objectives,
            "c_button00" => armed.0 = (armed.0 != Some(control::ArmedKind::Attack)).then_some(control::ArmedKind::Attack),
            "c_button01" => armed.0 = (armed.0 != Some(control::ArmedKind::AttackMove)).then_some(control::ArmedKind::AttackMove),
            // Waypoint mode (c_button03) needs path orders in the sim; until then it acts as a plain move.
            "c_button02" | "c_button03" => armed.0 = (armed.0 != Some(control::ArmedKind::Move)).then_some(control::ArmedKind::Move),
            "c_button04" | "c_button05" => {
                let ships_ordered = mine();
                if let Some(&lead) = ships_ordered.first() {
                    armed.0 = None;
                    if b.name == "c_button05" {
                        // Guard: hold the place and shoot what comes in range.
                        rt.command(player, Command::SetStance { ships: ships_ordered.clone(), stance: battle_sim::Stance::Defensive });
                    }
                    rt.command(player, Command::Stop { ships: ships_ordered });
                    let kind = if b.name == "c_button05" { control::OrderKind::Guard } else { control::OrderKind::Stop };
                    issued.write(control::OrderIssued { kind, lead });
                }
            }
            _ => {}
        }
    }
    for a in abilities.read() {
        let group_idx = tac
            .selection
            .iter()
            .position(|u| u.unit_type.eq_ignore_ascii_case(&a.unit_type))
            .unwrap_or(tac.focus);
        let Some(group) = state.groups.get(group_idx) else { continue };
        // Targeted abilities (reveal ping): the next click on the map is the target.
        let targeted = group.first().and_then(|&id| rt.cur.as_ref().and_then(|c| c.ship(id))).is_some_and(|s| {
            rt.catalog.ship(s.def).abilities.iter().any(|d| d.kind.eq_ignore_ascii_case(&a.ability) && is_targeted(d))
        });
        if targeted {
            // The unit's `SFXEvent_Target_Ability` line when the player is asked for a target.
            if let Some(def) = group.first().and_then(|&id| rt.cur.as_ref().and_then(|c| c.ship(id))).map(|s| s.def)
                && let Some(n) = rt.visuals.ship(def).sfx.abilities.iter().find(|x| x.kind.eq_ignore_ascii_case(&a.ability)).and_then(|x| x.target.as_deref())
            {
                sfx.write(alamo_audio::PlaySfx { event: n.to_string(), position: None, source: None });
            }
            state.pending_ability = Some((group.clone(), a.ability.clone()));
            armed.0 = Some(control::ArmedKind::AbilityAt);
            continue;
        }
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
    camera: Single<(&Camera, &GlobalTransform, &BattleCamera, &Projection)>,
    mut data: ResMut<MinimapData>,
    mut quad: ResMut<eaw_ui::MinimapCameraQuad>,
    mut fog: ResMut<eaw_ui::MinimapFog>,
    sel: Res<Selection>,
    settings: Res<BattleViewSettings>,
    pings: Query<&crate::effects::TacticalSensorPing>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    let has_pings = !pings.is_empty();
    let threshold = if has_pings { 0.03 } else { 0.1 };
    if *since < threshold && data.bounds.is_some() {
        return;
    }
    *since = 0.0;
    let (cam, cam_tf, orbit, projection) = *camera;
    let flip = |p: Vec3| Vec2::new(p.x, -p.z);
    let bounds = (Vec2::new(orbit.bounds_min.x, -orbit.bounds_max.y), Vec2::new(orbit.bounds_max.x, -orbit.bounds_min.y));
    let units = ships
        .0
        .values()
        .filter(|s| s.visible && rt.catalog.ship(s.def).special_weapon.is_none())
        .map(|s| MinimapDot { pos: flip(s.pos), color: relation_color(s.side, settings.player_side), selected: sel.ships.contains(&s.id) })
        .collect();
    // Fog of war: the sensor discs (`Space_FOW_Reveal_Range`) of the player's ships, one per grid cell of a quarter radius.
    let mut cells: std::collections::BTreeMap<(i32, i32, i32), (Vec2, f32)> = Default::default();
    if rt.fog_of_war {
        for s in ships.0.values().filter(|s| s.side == settings.player_side) {
            let r = rt.catalog.ship(s.def).sensor_range;
            let p = flip(s.pos);
            let q = (r * 0.25).max(1.0);
            cells.entry(((p.x / q).round() as i32, (p.y / q).round() as i32, (r / 100.0) as i32)).or_insert((p, r));
        }
    }
    let next_fog = eaw_ui::MinimapFog { enabled: rt.fog_of_war, reveal: cells.into_values().collect() };
    if *fog != next_fog {
        *fog = next_fog;
    }
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => 0.8,
    };
    let aspect = cam.logical_viewport_size().map_or(1.78, |s| s.x / s.y.max(1.0));
    let half_h = orbit.distance * (fov * 0.5).tan() / orbit.pitch.sin().max(0.3);
    let center = flip(orbit.focus);
    let half = Vec2::new(half_h * aspect, half_h);
    let planets: Vec<MinimapDot> = rt.minimap_props.iter().map(|(pos, _)| MinimapDot { pos: *pos, color: Color::srgb(0.55, 0.5, 0.45), selected: false }).collect();
    let minimap_pings: Vec<eaw_ui::MinimapPing> = pings
        .iter()
        .map(|p| {
            let t = (p.elapsed / p.duration).clamp(0.0, 1.0);
            let radius = p.max_radius * (1.0 - (1.0 - t).powi(2));
            let alpha = (1.0 - t).powi(2) * 0.9;
            eaw_ui::MinimapPing {
                pos: Vec2::new(p.pos.x, -p.pos.z),
                radius,
                color: p.color.with_alpha(alpha),
            }
        })
        .collect();
    // Footprint of the view on the battle plane: the screen corners cast onto it (far corners are limited so a flat camera stays on the map).
    let footprint = cam.logical_viewport_size().and_then(|vp| {
        let corners = [Vec2::ZERO, Vec2::new(vp.x, 0.0), vp, Vec2::new(0.0, vp.y)];
        let mut out = [Vec2::ZERO; 4];
        for (o, c) in out.iter_mut().zip(corners) {
            let ray = cam.viewport_to_world(cam_tf, c).ok()?;
            let dir = *ray.direction;
            let limit = orbit.distance * 4.0;
            let t = if dir.y < -1e-3 { ((orbit.focus.y - ray.origin.y) / dir.y).min(limit) } else { limit };
            *o = flip(ray.origin + dir * t);
        }
        Some(out)
    });
    let footprint = footprint.map(|q| q.map(|p| p.clamp(bounds.0, bounds.1)));
    if quad.0 != footprint {
        quad.0 = footprint;
    }
    let next = MinimapData { bounds: Some(bounds), planets, units, pings: minimap_pings, camera: Some((center - half, center + half)), background: Some(rt.minimap_tint) };
    if *data != next {
        *data = next;
    }
}

/// Click or drag on the minimap moves the battle camera (unless placing reinforcements).
fn minimap_pan(mut clicks: MessageReader<EawMinimapClicked>, mut camera: Single<&mut BattleCamera>, armed: Res<control::ArmedOrder>) {
    if armed.0 == Some(control::ArmedKind::Reinforce) {
        clicks.read().for_each(|_| ());
        return;
    }
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
    /// Row being placed (a click on the map calls it in) and whether the whole stack comes (Shift).
    placing: Option<(usize, bool)>,
    /// The cursor label while placing.
    label: Option<Entity>,
    /// Cursor position where the press on a reserve slot began while the button is still down (a drag from the slot into the battle).
    drag_from: Option<Vec2>,
    /// A drag from a slot was released over the map this frame: the unit is called in if the spot is allowed, and the pick ends either way.
    dropped: bool,
    /// What the ghost shows at the cursor this frame (`None` while no unit is being placed or the cursor is over the interface).
    pub ghost: Option<Ghost>,
    /// The ghost model: entity, whether it was built for an accepted spot (the team colour shows it), reserve row.
    ghost_model: Option<(Entity, bool, usize)>,
    /// Mouse wheel rotation offset clamped to ±15 degrees.
    pub ghost_yaw_offset: f32,
    /// The no-spawn zone discs that are drawn while a unit is carried: entity, centre, radius.
    zone_discs: Vec<(Entity, Vec3, f32)>,
    /// Mesh and material shared by the zone discs.
    zone_assets: Option<(Handle<Mesh>, Handle<StandardMaterial>)>,
}

/// Verdict of the placement rule at the cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ghost {
    /// Battle plane point under the cursor (Bevy space).
    pub ground: Vec3,
    /// Placement is allowed: green ghost. Otherwise red, with the reason.
    pub why_not: Option<&'static str>,
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
        commands.entity(e).try_despawn();
    }
    ui.rows.clear();
    ui.placing = None;
    ui.drag_from = None;
    ui.ghost = None;
    if let Some(e) = ui.label.take() {
        commands.entity(e).try_despawn();
    }
    if let Some((e, ..)) = ui.ghost_model.take() {
        commands.entity(e).try_despawn();
    }
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

/// Pressing a reserve slot picks its unit at once: holding the button and moving into the battle drags it (the ghost follows the cursor, the
/// button released over the map places it); a plain click on the slot keeps it picked for a later click on the map. A drag released over the
/// interface cancels.
fn reinforce_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    pointer: Res<control::Pointer>,
    block: Res<control::UiBlock>,
    slots: Query<(&eaw_ui::EawButton, &Interaction)>,
    mut ui: ResMut<ReinforceUi>,
    mut armed: ResMut<control::ArmedOrder>,
) {
    if ui.root.is_none() {
        ui.drag_from = None;
        return;
    }
    if mouse.just_pressed(MouseButton::Left) {
        let hit = slots.iter().find_map(|(b, i)| (*i == Interaction::Pressed).then(|| reserve_slot(&b.name)).flatten());
        if let Some(idx) = hit
            && ui.rows.get(idx).is_some_and(|r| r.2 > 0)
        {
            let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
            ui.placing = Some((idx, shift));
            ui.drag_from = pointer.cursor;
            armed.0 = Some(control::ArmedKind::Reinforce);
        }
    }
    if mouse.just_released(MouseButton::Left)
        && let Some(from) = ui.drag_from.take()
        && let Some(now) = pointer.cursor
        && now.distance(from) > 12.0
    {
        if block.0 {
            ui.placing = None;
            armed.0 = None;
        } else {
            ui.dropped = true;
        }
    }
}

/// Opens and closes the window, and calls units in.
fn reinforce_clicks(
    mut armed: ResMut<control::ArmedOrder>,
    mut clicks: MessageReader<EawButtonClicked>,
    rt: Res<BattleRuntime>,
    mut ui: ResMut<ReinforceUi>,
    mut commands: Commands,
    audio: Option<Res<alamo_audio::plugin::AlamoAudio>>,
    mut sfx: MessageWriter<alamo_audio::PlaySfx>,
) {
    // Faction announcements of `Factions.xml`: `Reinforcements_Selection_SFXEvent` (window opens) and `_Cancelled_` (closed while a unit was picked).
    let mut say = |tag: &str| {
        if let Some(n) = rt.player_faction.as_deref().zip(audio.as_ref()).and_then(|(f, a)| a.constants.faction_sfx(f, tag)) {
            sfx.write(alamo_audio::PlaySfx { event: n.to_string(), position: None, source: None });
        }
    };
    for c in clicks.read() {
        match c.name.as_str() {
            "b_reinforcement" | "r_close" => {
                if let Some(e) = ui.root.take() {
                    commands.entity(e).try_despawn();
                    if ui.placing.is_some() {
                        say("reinforcements_cancelled_sfxevent");
                    }
                } else if c.name == "b_reinforcement" && !ui.rows.is_empty() {
                    say("reinforcements_selection_sfxevent");
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
                // The unit is picked here; the click on the map (see `reinforce_place`) calls it in.
                if ui.rows.get(idx).is_some_and(|r| r.2 > 0) {
                    ui.placing = Some((idx, c.shift));
                    armed.0 = Some(control::ArmedKind::Reinforce);
                }
            }
        }
    }
}

/// Fills the window slots (icon, units left) and the population text.
fn reinforce_panel(
    ui: Res<ReinforceUi>,
    rt: Res<BattleRuntime>,
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
                    label: if *left > 1 { format!("x{left}") } else { String::new() },
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
    let pop = rt.cur.as_ref().and_then(|c| c.pop.get(settings.player_side as usize).copied()).unwrap_or(0);
    let text = format!("{pop}/{}", rt.unit_cap);
    if panel.texts.get("r_pop_text") != Some(&text) {
        panel.set_text("r_pop_text", text);
    }
}

/// Follows the simulation: units leave the reserve when a call is accepted, a refusal shows a notice and plays a sound.
fn reinforce_events(
    fonts: Res<eaw_ui::EawUiSettings>,
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
                    commands.entity(e).try_despawn();
                }
                let e = commands
                    .spawn((
                        crate::BattleEntity,
                        Text::new("Cannot call in reinforcements: unit cap reached"),
                        game_font(&fonts, 20.0),
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
            commands.entity(e).try_despawn();
            ui.notice = None;
        } else {
            ui.notice = Some((e, left));
        }
    }
}

/// `TEXT_MOVIE_MODE` of the data reads `BATTLECAM "!`: the button gets a real sentence built from the game's own help text (`TEXT_NEW_INTRO_MISC_9`).
fn fix_tooltips(mut buttons: Query<&mut eaw_ui::EawButton>) {
    for mut b in &mut buttons {
        if b.name == "b_camera_t" && b.tooltip_literal.is_none() {
            b.tooltip_literal = Some("Battle camera (C)
Follows the selected unit; you can rotate around it".to_string());
        }
    }
}

/// Abilities that are aimed at a point: the reveal ping, the lightning storm (zone) and the super laser.
fn is_targeted(d: &battle_sim::AbilityDef) -> bool {
    d.reveal_radius > 0.0 || d.zone.is_some() || d.super_laser.is_some()
}

/// Radius within which the sim looks for a super laser target around the aimed point (`Battle::command`, `UseAbilityAt`).
const SUPER_LASER_AIM_RADIUS: f32 = 600.0;

/// Completes a targeted ability: the cursor shows its area (reveal ping radius, storm radius with the placing range around the ship,
/// super laser aim circle); the click sends `UseAbilityAt` for the first ship of the slot that has it ready. A position that the sim would
/// refuse (out of the storm's range, no enemy capital / frigate / structure near the super laser aim) shows the reason and keeps the mode.
#[allow(clippy::too_many_arguments)]
fn ability_at(
    fonts: Res<eaw_ui::EawUiSettings>,
    mut state: ResMut<TacticalUiState>,
    mut armed: ResMut<control::ArmedOrder>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    hover: Res<Hover>,
    pointer: Res<control::Pointer>,
    block: Res<control::UiBlock>,
    scale: Res<UiScale>,
    mut labels: Query<(&mut Node, &mut Text, &mut TextColor)>,
    mut commands: Commands,
    mut gizmos: Gizmos,
) {
    if armed.0 != Some(control::ArmedKind::AbilityAt) {
        state.pending_ability = None;
        if let Some(e) = state.ability_label.take() {
            commands.entity(e).try_despawn();
        }
        return;
    }
    let Some((group, kind)) = state.pending_ability.clone() else {
        armed.0 = None;
        return;
    };
    let label = match state.ability_label {
        Some(e) => e,
        None => {
            let e = commands
                .spawn((
                    crate::BattleEntity,
                    Text::new(""),
                    game_font(&fonts, 12.0),
                    TextColor(Color::WHITE),
                    TextShadow { offset: Vec2::splat(1.0), color: Color::BLACK },
                    Node { position_type: PositionType::Absolute, display: Display::None, ..default() },
                    GlobalZIndex(70),
                    bevy::ui::FocusPolicy::Pass,
                ))
                .id();
            state.ability_label = Some(e);
            e
        }
    };
    let (Some(ground), Some(cursor)) = (hover.ground.filter(|_| !block.0), pointer.cursor) else {
        if let Ok((mut n, ..)) = labels.get_mut(label) {
            n.display = Display::None;
        }
        return;
    };
    let Some(cur) = rt.cur.as_ref() else { return };
    let shape = |g: Vec3, r: f32, c: Color, gizmos: &mut Gizmos| {
        gizmos.circle(Isometry3d::new(g, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), r, c).resolution(48);
    };
    let pos = crate::convert::bevy_to_sim(ground);
    // The caster: the first ship of the slot that has the ability ready (else the first one).
    let caster = group.iter().filter_map(|&id| cur.ship(id)).find_map(|s| {
        let i = rt.catalog.ship(s.def).abilities.iter().position(|d| d.kind.eq_ignore_ascii_case(&kind))?;
        Some((s, i))
    });
    let Some((caster, idx)) = caster else {
        armed.0 = None;
        return;
    };
    let def = &rt.catalog.ship(caster.def).abilities[idx];
    let ready = caster.abilities.get(idx).is_some_and(|a| a.1 <= 0.0 && a.0 <= 0.0);
    let mut verdict: Result<(), &str> = if ready { Ok(()) } else { Err("Not ready") };
    let (mut radius, mut colour) = (def.reveal_radius.max(60.0), Color::srgb(0.4, 0.8, 1.0));
    if let Some(zone) = &def.zone {
        radius = zone.radius;
        // The placing range around the caster.
        let from = crate::convert::sim_to_bevy(caster.pos);
        shape(from, zone.max_range, Color::srgba(0.6, 0.9, 1.0, 0.4), &mut gizmos);
        if caster.pos.dist(pos) > zone.max_range {
            verdict = verdict.and(Err("Out of range"));
        }
        colour = Color::srgb(0.8, 0.5, 1.0);
    }
    if def.super_laser.is_some() {
        radius = SUPER_LASER_AIM_RADIUS;
        colour = Color::srgb(1.0, 0.5, 0.2);
        let target = cur.ships.iter().any(|t| {
            t.side != settings.player_side
                && matches!(rt.catalog.ship(t.def).class, battle_sim::TargetClass::Capital | battle_sim::TargetClass::Frigate | battle_sim::TargetClass::Structure)
                && t.pos.dist(pos) <= SUPER_LASER_AIM_RADIUS
        });
        if !target {
            verdict = verdict.and(Err("No target"));
        }
    }
    if verdict.is_err() {
        colour = Color::srgb(1.0, 0.35, 0.3);
    }
    shape(ground, radius, colour, &mut gizmos);
    if let Ok((mut node, mut text, mut tc)) = labels.get_mut(label) {
        node.display = Display::Flex;
        node.left = Val::Px((cursor.x + 18.0) / scale.0);
        node.top = Val::Px((cursor.y + 14.0) / scale.0);
        let msg = match verdict {
            Ok(()) => "Click to use".to_string(),
            Err(why) => why.to_string(),
        };
        if text.0 != msg {
            text.0 = msg;
        }
        tc.0 = colour;
    }
    if !pointer.left_released || verdict.is_err() {
        return;
    }
    let casters: Vec<u32> = group.iter().copied().filter(|id| cur.ship(*id).is_some_and(|s| s.id == caster.id)).collect();
    if let Some(&id) = casters.first() {
        rt.command(settings.player_side, Command::UseAbilityAt { ship: id, ability: idx as u16, pos });
        if def.reveal_radius > 0.0 || def.kind.eq_ignore_ascii_case("FOW_REVEAL_PING") {
            commands.spawn((
                crate::BattleEntity,
                crate::effects::TacticalSensorPing {
                    pos: ground,
                    max_radius: def.reveal_radius.max(800.0),
                    duration: 2.5,
                    elapsed: 0.0,
                    color: Color::srgb(0.25, 0.85, 1.0),
                },
            ));
        }
    }
    state.pending_ability = None;
    armed.0 = None;
}

/// Shift + 1..6 press the ability buttons of the focused unit (the data has no ability hotkeys; the digits alone are control groups).
fn ability_keys(
    keys: Res<ButtonInput<KeyCode>>,
    tac: Res<TacticalData>,
    mut clicks: MessageWriter<EawAbilityClicked>,
) {
    if !(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)) {
        return;
    }
    const DIGITS: [KeyCode; 6] = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6];
    // The focused group first; in a mixed selection the first group that has that ability slot answers (U41).
    let focus = tac.selection.get(tac.focus).into_iter();
    let order: Vec<_> = focus.chain(tac.selection.iter().enumerate().filter(|(i, _)| *i != tac.focus).map(|(_, u)| u)).collect();
    for (k, key) in DIGITS.iter().enumerate() {
        if !keys.just_pressed(*key) {
            continue;
        }
        if let Some((unit, a)) = order.iter().find_map(|u| u.abilities.get(k).filter(|a| !a.disabled).map(|a| (*u, a))) {
            clicks.write(EawAbilityClicked { unit_type: unit.unit_type.clone(), ability: a.def.kind.clone() });
        }
    }
}

/// Ring on an ability button whose autocast is on.
#[derive(Component)]
pub(crate) struct AutocastRing;

/// The ability of the unit group behind `special_button_NN`: (ship slot group, kind of the ability).
fn button_ability(name: &str, tac: &TacticalData, state: &TacticalUiState) -> Option<(Vec<u32>, String)> {
    let k: usize = name.strip_prefix("special_button_")?.parse().ok()?;
    let group_idx = k / 2;
    let ability_idx = k % 2;
    let unit = tac.selection.get(group_idx)?;
    let kind = unit.abilities.get(ability_idx)?.def.kind.clone();
    Some((state.groups.get(group_idx)?.clone(), kind))
}

/// Right click on an ability button turns its autocast on or off for every ship of the slot (only abilities that support it).
#[allow(clippy::too_many_arguments)]
fn autocast_toggle(
    pointer: Res<control::Pointer>,
    tac: Res<TacticalData>,
    state: Res<TacticalUiState>,
    rt: Res<BattleRuntime>,
    settings: Res<BattleViewSettings>,
    camera: Single<&Camera, With<BattleCamera>>,
    buttons: Query<(&eaw_ui::EawButton, &ComputedNode, &UiGlobalTransform, &InheritedVisibility)>,
) {
    let (true, Some(cursor)) = (pointer.right_pressed, pointer.cursor) else { return };
    let sf = camera.target_scaling_factor().unwrap_or(1.0);
    let Some(name) = buttons
        .iter()
        .find(|(b, n, g, v)| v.get() && b.name.starts_with("special_button_") && n.contains_point(**g, cursor * sf))
        .map(|(b, ..)| b.name.clone())
    else {
        return;
    };
    let Some((group, kind)) = button_ability(&name, &tac, &state) else { return };
    let Some(cur) = rt.cur.as_ref() else { return };
    // The new state follows the first ship of the slot so that a mixed slot ends up uniform.
    let mut target: Option<bool> = None;
    for &id in &group {
        let Some(s) = cur.ship(id) else { continue };
        let defs = &rt.catalog.ship(s.def).abilities;
        let Some(i) = defs.iter().position(|d| d.kind.eq_ignore_ascii_case(&kind)) else { continue };
        if !defs[i].supports_autofire {
            continue;
        }
        let on = *target.get_or_insert(!s.autocast.get(i).copied().unwrap_or(false));
        rt.command(settings.player_side, Command::SetAutocast { ship: id, ability: i as u16, on });
    }
}

/// Keeps a ring on every ability button whose autocast is on (first ship of the focused slot).
fn autocast_rings(
    tac: Res<TacticalData>,
    state: Res<TacticalUiState>,
    rt: Res<BattleRuntime>,
    buttons: Query<(Entity, &eaw_ui::EawButton, Option<&Children>)>,
    rings: Query<(), With<AutocastRing>>,
    mut commands: Commands,
) {
    for (e, b, kids) in &buttons {
        if !b.name.starts_with("special_button_") {
            continue;
        }
        let on = button_ability(&b.name, &tac, &state).is_some_and(|(group, kind)| {
            group.first().and_then(|&id| rt.cur.as_ref().and_then(|c| c.ship(id))).is_some_and(|s| {
                let defs = &rt.catalog.ship(s.def).abilities;
                defs.iter().position(|d| d.kind.eq_ignore_ascii_case(&kind)).is_some_and(|i| defs[i].supports_autofire && s.autocast.get(i).copied().unwrap_or(false))
            })
        });
        let existing: Vec<Entity> = kids.into_iter().flatten().copied().filter(|k| rings.contains(*k)).collect();
        match (on, existing.is_empty()) {
            (true, true) => {
                commands.spawn((
                    AutocastRing,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(-2.0),
                        top: Val::Px(-2.0),
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BorderColor::all(Color::srgb(1.0, 0.85, 0.3)),
                    bevy::ui::FocusPolicy::Pass,
                    ChildOf(e),
                ));
            }
            (false, false) => {
                for k in existing {
                    commands.entity(k).try_despawn();
                }
            }
            _ => {}
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
    // One overlay per unit: a squadron is one unit, so its fighters share a single bracket and bar pair.
    let mut groups: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
    for (&id, s) in ships.0.iter().filter(|(_, s)| s.visible) {
        let def = rt.catalog.ship(s.def);
        let key = if s.squad != u32::MAX {
            (1 << 40) | (u64::from(s.side) << 32) | u64::from(s.squad)
        } else if matches!(def.class, battle_sim::TargetClass::Fighter | battle_sim::TargetClass::Bomber) {
            (2 << 40) | (u64::from(s.side) << 32) | (u64::from(s.group) << 16) | u64::from(s.def)
        } else {
            u64::from(id)
        };
        groups.entry(key).or_default().push(id);
    }
    for (key, members) in groups {
        let selected = members.iter().any(|id| sel.ships.contains(id) || sel.enemy == Some(*id));
        let hovered = members.iter().any(|id| hover.ship == Some(*id));
        let first = &ships.0[&members[0]];
        let def = rt.catalog.ship(first.def);
        let is_squad = first.squad != u32::MAX || matches!(def.class, battle_sim::TargetClass::Fighter | battle_sim::TargetClass::Bomber);
        // Units nobody selected or hovers (and that are not own squadrons with their health line) show only their icon.
        let icon_only = !selected && !hovered && (!is_squad || first.side != player);
        let enemy = first.side != player;
        let pts: Vec<(Vec2, f32)> = members
            .iter()
            .filter_map(|id| {
                let s = &ships.0[id];
                let c = cam.world_to_viewport(gt, s.pos).ok()?;
                let edge = cam.world_to_viewport(gt, s.pos + right * s.radius).map_or(8.0, |e| (e - c).length());
                Some((c, edge))
            })
            .collect();
        if pts.is_empty() {
            continue;
        }
        let n = pts.len() as f32;
        let centre = pts.iter().map(|p| p.0).sum::<Vec2>() / n;
        if centre.x < -margin || centre.y < -margin || centre.x > vp.x + margin || centre.y > vp.y + margin {
            continue;
        }
        let half = pts.iter().map(|(c, e)| (*c - centre).length() + e).fold(0.0f32, f32::max);
        let edge_max = pts.iter().map(|(_, e)| *e).fold(0.0f32, f32::max);
        // Own capital ships big enough to read keep just their health lines, no icon box (Remake video b0700: Star Destroyers carry bars,
        // only the fighters, heroes and tiny ships carry icon boxes). Estimate for the size: 3% of the picture height as screen radius.
        let icon_only = icon_only && !(!enemy && !is_squad && edge_max >= vp.y * 0.03);
        let live: Vec<&ShipRender> = members.iter().map(|id| &ships.0[id]).collect();
        if def.special_weapon.is_some() {
            continue;
        }
        let single = members.len() == 1;
        let mut name = (hovered || (selected && single && sel.ships.len() + usize::from(sel.enemy.is_some()) <= 1)).then(|| {
            def.text_id
                .as_deref()
                .and_then(|k| assets.as_ref().and_then(|a| a.text.get(k)))
                .filter(|t| !t.is_empty())
                .map_or_else(|| def.name.clone(), str::to_string)
        });
        if let Some(n) = name.as_mut() {
            let retreat_active = rt.cur.as_ref().and_then(|c| c.retreat_jump_in.get(first.side as usize).copied().flatten()).is_some();
            if retreat_active && !live.iter().all(|s| s.can_retreat) {
                n.push_str(" [ENGINES DISABLED]");
            }
        }
        // The icon marks tiny units, squadrons and every unit nobody has selected or hovered (so that no unit is invisible).
        let icon = (icon_only || is_squad || edge_max < 9.0).then(|| assets.as_ref().and_then(|a| a.game.as_deref()).and_then(|g| eaw_ui::unit_icon(g, &def.name))).flatten()
            .or_else(|| is_squad.then(|| "I_BUTTON_UNIT_GENERIC_FIGHTER.TGA".to_string()));
        if icon_only && icon.is_none() {
            continue;
        }
        next.ships.push(ShipOverlay {
            id: key,
            screen_pos: centre,
            half_size: if is_squad { 11.0 } else { half.clamp(11.0, 220.0) },
            health: live.iter().map(|s| s.hull).sum::<f32>() / n,
            shield: (def.shield > 0.0).then(|| live.iter().map(|s| s.shield).sum::<f32>() / n),
            selected,
            hovered,
            enemy,
            name,
            always: !enemy && is_squad,
            // Far zoom (units a few pixels wide): a small box with the unit icon marks the unit, as in the Remake (video b0700).
            // Close up, fighters keep only the short hull bar and the flag (ref 07).
            icon,
            icon_only,
        });
    }
    // Hardpoint reticles with their health on the hovered ship, the selected enemy or a lone selected ship. They are clickable
    // (attack that hardpoint) when they belong to an enemy and own ships are selected.
    let lone = (sel.ships.len() == 1).then(|| sel.ships[0]);
    let target = hover.ship.or(sel.enemy).or(lone);
    if let Some(s) = target.and_then(|id| ships.0.get(&id)) {
        let def = rt.catalog.ship(s.def);
        let can_attack = s.side != player && !sel.ships.is_empty();
        for (i, h) in def.hardpoints.iter().enumerate() {
            if !h.destroyable || !is_markable(h.kind) {
                continue;
            }
            let alive = s.hp_alive.get(i).copied().unwrap_or(false);
            if !alive {
                continue;
            }
            let Ok(p) = cam.world_to_viewport(gt, control::hardpoint_world(&rt, s, i)) else { continue };
            next.hardpoints.push(HardpointMarker {
                id: hardpoint_id(s.id, i),
                kind: reticle_kind(h.kind).to_string(),
                tracked: hover.hardpoint == Some((s.id, i as u16)),
                screen_pos: p,
                health: s.hp_health.get(i).copied().unwrap_or(1.0),
                targetable: can_attack && h.targetable,
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
    mut hardpoint_orders: MessageWriter<control::HardpointOrderIssued>,
) {
    for c in clicks.read() {
        let Some((target, hp)) = c.id.split_once(':').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u16>().ok()?))) else { continue };
        let mine = control::selected_mobile(&rt, &ships, &sel, settings.player_side);
        let enemy = ships.0.get(&target).is_some_and(|s| s.side != settings.player_side);
        if enemy && !mine.is_empty() {
            if !mine.is_empty() {
                let lead = control::voice_lead(&rt, &ships, &mine);
                hardpoint_orders.write(control::HardpointOrderIssued { lead, target, hardpoint: hp });
            }
            rt.command(settings.player_side, Command::AttackHardpoint { ships: mine, target, hardpoint: hp });
        }
    }
}

/// Why the placement rule refuses a position (the sim answers with these `CommandError`s).
fn refusal_text(e: battle_sim::CommandError) -> &'static str {
    use battle_sim::CommandError as E;
    match e {
        E::OutsideBattleArea => "Outside the battle area",
        E::InNoSpawnZone => "Too close to an enemy structure",
        E::NotVisible => "Area not explored",
        _ => "Not allowed here",
    }
}

/// Everything the placement rule looks at, taken from the displayed snapshot.
fn placement_units(rt: &BattleRuntime) -> Vec<battle_sim::PlacementUnit> {
    rt.cur
        .as_ref()
        .map(|c| c.ships.iter().map(|s| battle_sim::PlacementUnit::of(&rt.catalog, s.def, s.side, s.pos, rt.no_spawn_radius)).collect())
        .unwrap_or_default()
}

/// The area rules of `Battle::reinforcement_zone_check` evaluated on the displayed snapshot so that the ghost can show them before the click: inside
/// the battle area, outside the no-spawn zones of enemy structures, inside an area the side sees (sensor discs of its ships and structures).
pub(crate) fn zone_check(rt: &BattleRuntime, side: u8, pos: battle_sim::Vec3) -> Result<(), &'static str> {
    if rt.cur.is_none() {
        return Err("Not ready");
    }
    battle_sim::placement_check(rt.arena_half - 300.0, rt.fog_of_war, side, pos, &placement_units(rt), &[]).map_err(refusal_text)
}

/// [`zone_check`] plus the population cap for `count` units of `ship` (what the sim checks in `Command::ReinforceAt`).
pub(crate) fn place_check(rt: &BattleRuntime, side: u8, pos: battle_sim::Vec3, ship: u32, count: u32) -> Result<(), &'static str> {
    zone_check(rt, side, pos)?;
    let pop = rt.cur.as_ref().and_then(|c| c.pop.get(side as usize).copied()).unwrap_or(0);
    if pop + rt.catalog.ship(ship).population * count as i32 > rt.unit_cap {
        return Err("Population cap reached");
    }
    Ok(())
}

/// The no-spawn zones (Bevy centre, radius) that enemy structures cast on `side`.
pub(crate) fn no_spawn_zones(rt: &BattleRuntime, side: u8) -> Vec<(Vec3, f32)> {
    let units = placement_units(rt);
    battle_sim::no_spawn_zones(side, &units).into_iter().map(|(c, r)| (crate::convert::sim_to_bevy(c), r)).collect()
}

/// Texture of a no-spawn zone disc (EaW look): a translucent red fill with a red hatch, a brighter rim and a soft edge.
fn zone_texture() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const N: usize = 512;
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = ((x as f32 + 0.5) / N as f32 * 2.0 - 1.0, (y as f32 + 0.5) / N as f32 * 2.0 - 1.0);
            let r = (u * u + v * v).sqrt();
            let hatch = if (x + y) % 20 < 5 { 0.22 } else { 0.0 };
            let feather = ((1.0 - r) / 0.25).clamp(0.0, 1.0);
            let rim = if (0.955..=1.0).contains(&r) { 0.55 } else { 0.0 };
            let alpha = if r > 1.0 { 0.0 } else { ((0.14 + hatch) * (0.6 + 0.4 * feather) + rim * (1.0 - (r - 0.955).abs() / 0.05).clamp(0.0, 1.0)).min(0.9) };
            px.extend_from_slice(&[245, 45, 35, (alpha * 255.0) as u8]);
        }
    }
    let mut img = Image::new(Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD);
    img.sampler = bevy::image::ImageSampler::linear();
    img
}

/// Keeps one flat translucent disc per no-spawn zone on the battle plane (`y`), and removes the ones that are gone.
fn sync_zone_discs(commands: &mut Commands, ui: &mut ReinforceUi, zones: &[(Vec3, f32)], y: f32, meshes: &mut Assets<Mesh>, images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) {
    let (mesh, material) = ui
        .zone_assets
        .get_or_insert_with(|| {
            let tex = images.add(zone_texture());
            let material = materials.add(StandardMaterial { base_color_texture: Some(tex), unlit: true, alpha_mode: AlphaMode::Blend, cull_mode: None, ..default() });
            (meshes.add(Rectangle::new(2.0, 2.0)), material)
        })
        .clone();
    for (i, (centre, radius)) in zones.iter().enumerate() {
        let tf = Transform { translation: Vec3::new(centre.x, y + 0.5, centre.z), rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2), scale: Vec3::splat(*radius) };
        match ui.zone_discs.get_mut(i) {
            Some(d) => {
                d.1 = *centre;
                d.2 = *radius;
                let e = d.0;
                commands.queue(move |world: &mut World| {
                    if let Some(mut t) = world.get_mut::<Transform>(e) {
                        *t = tf;
                    }
                });
            }
            None => {
                let e = commands.spawn((crate::BattleEntity, Name::new("no-spawn zone"), Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), tf, bevy::light::NotShadowCaster)).id();
                ui.zone_discs.push((e, *centre, *radius));
            }
        }
    }
    while ui.zone_discs.len() > zones.len() {
        if let Some((e, ..)) = ui.zone_discs.pop() {
            commands.entity(e).try_despawn();
        }
    }
}

/// Model of a reserve unit for its ghost (a squadron shows its first fighter type).
fn ghost_model_path(rt: &BattleRuntime, ship: u32) -> Option<String> {
    let def = rt.catalog.ship(ship);
    def.model.clone().or_else(|| def.members.first().and_then(|m| rt.catalog.ship(m.0).model.clone()))
}

#[derive(bevy::ecs::system::SystemParam)]
struct ReinforcePlaceCtx<'w> {
    fonts: Res<'w, eaw_ui::EawUiSettings>,
    rt: Res<'w, BattleRuntime>,
    settings: Res<'w, BattleViewSettings>,
    hover: Res<'w, Hover>,
    pointer: Res<'w, control::Pointer>,
    block: Res<'w, control::UiBlock>,
    assets: Option<Res<'w, EawUiAssets>>,
    scale: Res<'w, UiScale>,
    ships: Res<'w, RenderedShips>,
    minimap_data: Res<'w, MinimapData>,
}

/// Reinforcement placement: after a press (drag) or click on a reserve slot the cursor carries a ghost of the unit (its model with a green team colour
/// where the wave may arrive, red where the rule refuses, plus a ring and the reason); while it is carried the no-spawn zones of enemy structures are drawn
/// as red hatched areas. Releasing the dragged button (or a left click) on an accepted spot calls the unit in (hyperspace arrival after the reinforcement
/// delay). Right click / Esc cancels.
#[allow(clippy::too_many_arguments)]
fn reinforce_place(
    ctx: ReinforcePlaceCtx,
    mut ui: ResMut<ReinforceUi>,
    mut armed: ResMut<control::ArmedOrder>,
    mut labels: Query<(&mut Node, &mut Text, &mut TextColor)>,
    mut commands: Commands,
    mut gizmos: Gizmos,
    minimap_surfaces: Query<(&Interaction, &bevy::ui::RelativeCursorPosition), With<MinimapSurface>>,
    mut scroll: MessageReader<bevy::input::mouse::MouseWheel>,
    mut hold_cam: Single<&mut crate::camera::BattleCamera>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let placing = ui.placing.filter(|_| armed.0 == Some(control::ArmedKind::Reinforce));
    let Some((idx, all)) = placing else {
        ui.placing = None;
        ui.dropped = false;
        ui.ghost = None;
        ui.ghost_yaw_offset = 0.0;
        for (e, ..) in ui.zone_discs.drain(..) {
            commands.entity(e).try_despawn();
        }
        if let Some(e) = ui.label.take() {
            commands.entity(e).try_despawn();
        }
        if let Some((e, ..)) = ui.ghost_model.take() {
            commands.entity(e).try_despawn();
        }
        return;
    };
    let Some(row) = ui.rows.get(idx).cloned() else {
        armed.0 = None;
        return;
    };
    let player = ctx.settings.player_side;
    let count = if all { row.2 } else { 1 };
    let label = match ui.label {
        Some(e) => e,
        None => {
            let e = commands
                .spawn((
                    crate::BattleEntity,
                    Text::new(""),
                    game_font(&ctx.fonts, 12.0),
                    TextColor(Color::WHITE),
                    TextShadow { offset: Vec2::splat(1.0), color: Color::BLACK },
                    Node { position_type: PositionType::Absolute, display: Display::None, ..default() },
                    GlobalZIndex(70),
                    bevy::ui::FocusPolicy::Pass,
                ))
                .id();
            ui.label = Some(e);
            e
        }
    };
    // Test hook `SP_PLACE_HOLD=1`: the scripted placement keeps the ghost armed on its spot (for an offscreen capture) instead of calling the wave in.
    let mut hold_spot: Option<Vec3> = None;
    let hold = ctx.settings.synthetic_pointer && std::env::var_os("SP_PLACE_HOLD").is_some();
    if ctx.settings.synthetic_pointer {
        // Scripted offscreen runs have no mouse: the wave goes to the first spot next to an own ship that the placement rule accepts.
        let spot = ctx.ships.0.values().filter(|s| s.side == player && !ctx.rt.catalog.ship(s.def).is_static).min_by_key(|s| s.id).and_then(|o| {
            [(300.0, 0.0), (-300.0, 0.0), (0.0, 300.0), (0.0, -300.0), (600.0, 0.0), (-600.0, 0.0), (0.0, 600.0), (0.0, -600.0)]
                .iter()
                .map(|(dx, dz)| crate::convert::bevy_to_sim(o.pos + Vec3::new(*dx, 0.0, *dz)))
                .find(|p| place_check(&ctx.rt, player, *p, row.1, count).is_ok())
        });
        if let Some(pos) = spot {
            if hold {
                hold_spot = Some(crate::convert::sim_to_bevy(pos));
            } else {
                ctx.rt.command(player, Command::ReinforceAt { ship: row.1, count, pos, yaw_offset: 0.0 });
                info!("reinforcements: placing {} at {pos:?} (scripted)", row.0);
                armed.0 = None;
            }
        }
        if hold_spot.is_none() {
            return;
        }
    }
    // Hold mode: show the first enemy no-spawn zone with the ghost inside it (red) so the zone look can be judged.
    if hold_spot.is_some() {
        if let Some((c, r)) = no_spawn_zones(&ctx.rt, player).first().copied() {
            let spot = c + Vec3::X * (r * 0.7);
            hold_spot = Some(spot);
            hold_cam.focus = c + Vec3::X * (r * 0.3);
            hold_cam.target_focus = hold_cam.focus;
            hold_cam.distance = (r * 2.6).clamp(900.0, 9000.0);
            hold_cam.target_distance = hold_cam.distance;
        }
    }
    let y = hold_spot.or(ctx.hover.ground).map_or(0.0, |g| g.y);
    let zones = no_spawn_zones(&ctx.rt, player);
    sync_zone_discs(&mut commands, &mut ui, &zones, y, &mut meshes, &mut images, &mut materials);
    let cursor = match (hold_spot, ctx.pointer.cursor) {
        (Some(_), _) => Vec2::new(960.0, 540.0),
        (None, Some(c)) => c,
        (None, None) => return,
    };
    let minimap_hit = minimap_surfaces.iter().find_map(|(_, rel)| {
        let n = rel.normalized?;
        if !(-0.5..=0.5).contains(&n.x) || !(-0.5..=0.5).contains(&n.y) {
            return None;
        }
        let normalized = Vec2::new(n.x + 0.5, 0.5 - n.y);
        let world = ctx.minimap_data.bounds.map_or(normalized, |b| minimap_to_world(b, normalized));
        Some((Vec3::new(world.x, 0.0, -world.y), battle_sim::Vec3::new(world.x, world.y, 0.0)))
    });
    let (ground, sim_pos) = if let Some(h) = hold_spot {
        (h, crate::convert::bevy_to_sim(h))
    } else if let Some((m_ground, m_sim)) = minimap_hit {
        (m_ground, m_sim)
    } else {
        let Some(ground) = ctx.hover.ground else { return };
        if ctx.block.0 {
            ui.ghost = None;
            if let Ok((mut n, ..)) = labels.get_mut(label) {
                n.display = Display::None;
            }
            if let Some((e, ..)) = ui.ghost_model.take() {
                commands.entity(e).try_despawn();
            }
            return;
        }
        (ground, crate::convert::bevy_to_sim(ground))
    };
    let verdict = place_check(&ctx.rt, player, sim_pos, row.1, count);
    let ok = verdict.is_ok();
    ui.ghost = Some(Ghost { ground, why_not: verdict.err() });
    let colour = if ok { Color::srgb(0.3, 1.0, 0.4) } else { Color::srgb(1.0, 0.35, 0.3) };
    let def = ctx.rt.catalog.ship(row.1);
    let ring = (def.radius * ctx.rt.visuals.ship(row.1).scale).max(ctx.rt.reinforce_clearance).max(60.0);
    gizmos.circle(Isometry3d::new(ground, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), ring, colour).resolution(48);
    // The ghost model faces the arrival direction (from the own spawn edge), not turning toward the enemy base;
    // the mouse wheel may rotate it at most ±15°.
    for ev in scroll.read() {
        ui.ghost_yaw_offset = (ui.ghost_yaw_offset + ev.y * 5.0f32.to_radians()).clamp(-15.0f32.to_radians(), 15.0f32.to_radians());
    }
    let base_facing = ctx
        .rt
        .cur
        .as_ref()
        .and_then(|c| c.spawn_yaw.get(player as usize).copied())
        .unwrap_or(if player == 0 { 0.0 } else { std::f32::consts::PI });
    let facing = base_facing + ui.ghost_yaw_offset;
    let rotation = crate::convert::ship_rotation(facing);
    match ui.ghost_model {
        Some((e, built_ok, built_idx)) if built_ok == ok && built_idx == idx => {
            commands.queue(move |world: &mut World| {
                if let Some(mut tf) = world.get_mut::<Transform>(e) {
                    tf.translation = ground;
                    tf.rotation = rotation;
                }
            });
        }
        old => {
            if let Some((e, ..)) = old {
                commands.entity(e).try_despawn();
            }
            ui.ghost_model = ghost_model_path(&ctx.rt, row.1).map(|path| {
                let e = commands
                    .spawn((
                        crate::BattleEntity,
                        Name::new("reinforcement ghost"),
                        alamo_render::AlamoModel::new(path).with_team_color(colour),
                        Transform { translation: ground, rotation, scale: Vec3::splat(ctx.rt.visuals.ship(row.1).scale) },
                    ))
                    .id();
                (e, ok, idx)
            });
        }
    }
    let name = ctx
        .assets
        .as_ref()
        .and_then(|a| def.text_id.as_deref().and_then(|k| a.text.get(k)).filter(|t| !t.is_empty()).map(str::to_string))
        .unwrap_or_else(|| row.0.clone());
    if let Ok((mut node, mut text, mut tc)) = labels.get_mut(label) {
        node.display = Display::Flex;
        node.left = Val::Px((cursor.x + 18.0) / ctx.scale.0);
        node.top = Val::Px((cursor.y + 14.0) / ctx.scale.0);
        let msg = match verdict {
            Ok(()) => format!("{name}: release to call in{}", if all { " (all)" } else { "" }),
            Err(why) => format!("{name}: {why}"),
        };
        if text.0 != msg {
            text.0 = msg;
        }
        tc.0 = colour;
    }
    let dropped = std::mem::take(&mut ui.dropped);
    if hold {
        return;
    }
    if (ctx.pointer.left_released && ok) || dropped {
        if let Err(why) = verdict {
            info!("reinforcements: drop of {} refused: {why}", row.0);
        } else {
            // The count drops when the simulation reports the call (`ReinforcementsCalled`).
            ctx.rt.command(player, Command::ReinforceAt { ship: row.1, count, pos: sim_pos, yaw_offset: ui.ghost_yaw_offset });
            info!("reinforcements: placing {count} x {} at {sim_pos:?}", row.0);
        }
        armed.0 = None;
    }
}

/// The order buttons are dimmed while nothing mobile is selected (as in the Remake video with an empty selection).
fn command_buttons_state(
    rt: Res<BattleRuntime>,
    sel: Res<Selection>,
    ships: Res<RenderedShips>,
    settings: Res<BattleViewSettings>,
    mut buttons: Query<(&eaw_ui::EawButton, &mut eaw_ui::EawButtonState, &mut Node)>,
) {
    let any = !control::selected_mobile(&rt, &ships, &sel, settings.player_side).is_empty();
    for (b, mut st, mut node) in &mut buttons {
        if b.name.starts_with("c_button") && st.disabled == any {
            st.disabled = !any;
        }
        // Galaxy leftovers of the shared shell (superweapon / bombardment / landing panels) have no place in a space battle.
        if matches!(b.name.as_str(), "sl_switch" | "pb_switch" | "deathstar_switch") && node.display != Display::None {
            node.display = Display::None;
        }
    }
}


/// The banner that shows the retreat jump countdown at the top of the screen after the Retreat order.
#[derive(Component)]
pub(crate) struct RetreatBanner;

/// Seconds a "retreat not possible yet" notice stays on screen (set by the Esc menu while the retreat lock runs).
#[derive(Resource, Default)]
pub struct RetreatNotice(pub f32);

fn retreat_banner(
    mut commands: Commands,
    ui: Res<eaw_ui::EawUiSettings>,
    settings: Res<BattleViewSettings>,
    time: Res<Time<Real>>,
    rt: Res<BattleRuntime>,
    status: Res<BattleStatus>,
    mut notice: ResMut<RetreatNotice>,
    tac: Res<TacticalData>,
    mut banner: Query<(Entity, &mut Text), With<RetreatBanner>>,
) {
    let mss = |s: f32| format!("{}:{:02}", s.ceil() as u32 / 60, s.ceil() as u32 % 60);
    notice.0 = (notice.0 - time.delta_secs()).max(0.0);
    let player = settings.player_side;
    let enemy_retreat = rt.cur.as_ref().and_then(|c| {
        c.retreat_jump_in.iter().enumerate().find_map(|(side, j)| {
            if side as u8 != player { *j } else { None }
        })
    });
    let want = match tac.retreat {
        RetreatState::Countdown(s) if tac.active => {
            if s <= 60.0 {
                Some(format!("TACTICAL RETREAT - HYPERSPACE JUMP IN {:.0}s", s.ceil()))
            } else {
                Some(format!("TACTICAL RETREAT - HYPERSPACE JUMP IN {}", mss(s)))
            }
        }
        _ if enemy_retreat.is_some() => {
            let s = enemy_retreat.unwrap();
            if s <= 60.0 {
                Some(format!("ENEMY FLEET RETREATING - HYPERSPACE JUMP IN {:.0}s", s.ceil()))
            } else {
                Some(format!("ENEMY FLEET RETREATING - HYPERSPACE JUMP IN {}", mss(s)))
            }
        }
        _ if notice.0 > 0.0 => Some(format!("RETREAT NOT POSSIBLE YET - AVAILABLE IN {}", mss((rt.retreat_countdown - status.time).max(0.0)))),
        _ => None,
    };
    match (want, banner.single_mut()) {
        (Some(text), Ok((_, mut t))) => {
            if t.0 != text {
                t.0 = text;
            }
        }
        (Some(text), Err(_)) => {
            let mut e = commands.spawn((
                RetreatBanner,
                crate::hud::TacticalHud,
                Name::new("retreat banner"),
                Text::new(text),
                match &ui.font_family {
                    Some(f) => TextFont { font: FontSource::Family(f.as_str().into()), font_size: FontSize::Px(22.0), ..default() },
                    None => TextFont::from_font_size(22.0),
                },
                TextColor(Color::srgb(1.0, 0.85, 0.4)),
                TextShadow { offset: Vec2::splat(1.0), color: Color::BLACK },
                Node { position_type: PositionType::Absolute, top: Val::Px(40.0), width: Val::Percent(100.0), justify_content: JustifyContent::Center, ..default() },
                TextLayout::justify(Justify::Center),
                GlobalZIndex(70),
            ));
            if let Some(cam) = ui.camera {
                e.insert(UiTargetCamera(cam));
            }
        }
        (None, Ok((e, _))) => commands.entity(e).despawn(),
        (None, Err(_)) => {}
    }
}

/// The Death Star countdown gauge at the top centre (`DS_Countdown_Timer`, texture `ds_countdown.tga`: red and green wings around a disc that
/// carries the number). Shown while a ship with a super laser lives; the number is the seconds until its super laser is ready.
#[derive(Component)]
pub(crate) struct DeathStarGauge;
/// The number inside the gauge.
#[derive(Component)]
struct DeathStarGaugeText;

/// Seconds until the first super laser of `ships` is usable: per ship `(has a super laser, [(seconds of effect left, seconds until usable)])`.
/// `None` without a living ship that has one.
fn death_star_seconds<'a>(ships: impl Iterator<Item = (&'a [bool], &'a [(f32, f32)])>) -> Option<f32> {
    ships
        .filter_map(|(is_laser, states)| {
            is_laser.iter().zip(states).filter(|(l, _)| **l).map(|(_, &(active, cooldown))| active.max(cooldown)).reduce(f32::min)
        })
        .reduce(f32::min)
}

#[allow(clippy::too_many_arguments)]
fn death_star_gauge(
    mut commands: Commands,
    ui: Res<eaw_ui::EawUiSettings>,
    rt: Res<BattleRuntime>,
    textures: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut text: Query<&mut Text, With<DeathStarGaugeText>>,
    gauge: Query<Entity, With<DeathStarGauge>>,
) {
    let lasers: Vec<Vec<bool>> = rt.cur.as_ref().map_or_else(Vec::new, |c| {
        c.ships.iter().map(|s| rt.catalog.ship(s.def).abilities.iter().map(|a| a.super_laser.is_some()).collect()).collect()
    });
    let seconds = rt.cur.as_ref().and_then(|c| death_star_seconds(c.ships.iter().zip(&lasers).map(|(s, l)| (l.as_slice(), s.abilities.as_slice()))));
    match (seconds, gauge.single()) {
        (Some(s), Ok(_)) => {
            let want = format!("{s:.2}");
            if let Ok(mut t) = text.single_mut() && t.0 != want {
                t.0 = want;
            }
        }
        (Some(s), Err(_)) => {
            let Some(mut textures) = textures else { return };
            let Some(image) = textures.get(&mut images, "ds_countdown.tga").map(|t| t.handle) else { return };
            let mut root = commands.spawn((
                DeathStarGauge,
                crate::hud::TacticalHud,
                Name::new("death star gauge"),
                ImageNode::new(image),
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Vh(0.5),
                    left: Val::Vw(50.0 - 7.8),
                    width: Val::Vw(15.6),
                    aspect_ratio: Some(220.0 / 97.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                GlobalZIndex(70),
            ));
            if let Some(cam) = ui.camera {
                root.insert(UiTargetCamera(cam));
            }
            root.with_child((
                DeathStarGaugeText,
                Text::new(format!("{s:.2}")),
                TextFont { font_size: FontSize::Vh(2.2), ..default() },
                TextColor(Color::srgb(0.55, 0.95, 0.5)),
                TextShadow { offset: Vec2::splat(1.0), color: Color::BLACK },
            ));
        }
        (None, Ok(e)) => commands.entity(e).despawn(),
        (None, Err(_)) => {}
    }
}

/// Title text shown in the middle of the screen when the battle ends (Remake/EaW victory banner).
#[derive(Component)]
struct VictoryBanner;

fn victory_banner(
    mut commands: Commands,
    ui: Res<eaw_ui::EawUiSettings>,
    settings: Res<BattleViewSettings>,
    status: Res<BattleStatus>,
    mut banner: Query<(Entity, &mut Text), With<VictoryBanner>>,
) {
    let want = if status.over {
        match status.winner {
            Some(w) if w == settings.player_side => Some("W E   A R E   V I C T O R I O U S !"),
            Some(_) => Some("D E F E A T E D"),
            None => Some("D R A W"),
        }
    } else {
        None
    };
    match (want, banner.single_mut()) {
        (Some(text), Ok((_, mut t))) => {
            if t.0 != text {
                t.0 = text.to_string();
            }
        }
        (Some(text), Err(_)) => {
            let mut e = commands.spawn((
                VictoryBanner,
                crate::hud::TacticalHud,
                Name::new("victory banner"),
                Text::new(text),
                game_font(&ui, 34.0),
                TextColor(Color::srgb(1.0, 0.85, 0.35)),
                TextShadow { offset: Vec2::splat(2.0), color: Color::BLACK },
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Percent(35.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                TextLayout::justify(Justify::Center),
                GlobalZIndex(80),
            ));
            if let Some(cam) = ui.camera {
                e.insert(UiTargetCamera(cam));
            }
        }
        (None, Ok((e, _))) => commands.entity(e).despawn(),
        (None, Err(_)) => {}
    }
}

/// Title banner shown at top of the screen when a tactical battle is paused.
#[derive(Component)]
struct PauseBanner;

fn pause_banner(
    mut commands: Commands,
    ui: Res<eaw_ui::EawUiSettings>,
    settings: Res<BattleViewSettings>,
    status: Res<BattleStatus>,
    mut banner: Query<(Entity, &mut Text), With<PauseBanner>>,
) {
    let want = if settings.paused && !status.over {
        Some("P A U S E D")
    } else {
        None
    };
    match (want, banner.single_mut()) {
        (Some(text), Ok((_, mut t))) => {
            if t.0 != text {
                t.0 = text.to_string();
            }
        }
        (Some(text), Err(_)) => {
            let mut e = commands.spawn((
                PauseBanner,
                crate::hud::TacticalHud,
                Name::new("pause banner"),
                Text::new(text),
                game_font(&ui, 26.0),
                TextColor(Color::srgb(1.0, 0.85, 0.35)),
                TextShadow { offset: Vec2::splat(1.5), color: Color::BLACK },
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(42.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                TextLayout::justify(Justify::Center),
                GlobalZIndex(80),
            ));
            if let Some(cam) = ui.camera {
                e.insert(UiTargetCamera(cam));
            }
        }
        (None, Ok((e, _))) => commands.entity(e).despawn(),
        (None, Err(_)) => {}
    }
}

/// Text font of the battle HUD notices: the game font of the UI settings (like the retreat banner), the default font without one.
fn game_font(ui: &eaw_ui::EawUiSettings, size: f32) -> TextFont {
    match &ui.font_family {
        Some(f) => TextFont { font: FontSource::Family(f.as_str().into()), font_size: FontSize::Px(size), ..default() },
        None => TextFont::from_font_size(size),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn death_star_countdown_is_the_soonest_super_laser() {
        let laser: &[bool] = &[false, true];
        let none: &[bool] = &[false];
        let a: &[(f32, f32)] = &[(0.0, 0.0), (0.0, 90.0)];
        let b: &[(f32, f32)] = &[(0.0, 0.0), (30.0, 10.0)];
        assert_eq!(death_star_seconds([(laser, a)].into_iter()), Some(90.0));
        assert_eq!(death_star_seconds([(laser, a), (laser, b)].into_iter()), Some(30.0), "charging counts as the larger of the two");
        assert_eq!(death_star_seconds([(none, &a[..1])].into_iter()), None);
    }

    #[test]
    fn button_ability_maps_to_group_columns() {
        let tac = TacticalData {
            active: true,
            focus: 0,
            selection: vec![
                eaw_ui::TacticalUnit {
                    unit_type: "Thrawn".into(),
                    abilities: vec![
                        eaw_ui::AbilityState {
                            def: eaw_ui::AbilityDef { kind: "POWER_TO_WEAPONS".into(), ..default() },
                            ..default()
                        },
                        eaw_ui::AbilityState {
                            def: eaw_ui::AbilityDef { kind: "TRACTOR_BEAM".into(), ..default() },
                            ..default()
                        },
                    ],
                    ..default()
                },
                eaw_ui::TacticalUnit {
                    unit_type: "Victory".into(),
                    abilities: vec![
                        eaw_ui::AbilityState {
                            def: eaw_ui::AbilityDef { kind: "FULL_SALVO".into(), ..default() },
                            ..default()
                        },
                    ],
                    ..default()
                },
            ],
            ..default()
        };
        let state = TacticalUiState {
            groups: vec![vec![101], vec![201, 202]],
            ..default()
        };

        // Col 0: Thrawn (special_button_00 and 01)
        assert_eq!(
            button_ability("special_button_00", &tac, &state),
            Some((vec![101], "POWER_TO_WEAPONS".into()))
        );
        assert_eq!(
            button_ability("special_button_01", &tac, &state),
            Some((vec![101], "TRACTOR_BEAM".into()))
        );

        // Col 1: Victory (special_button_02)
        assert_eq!(
            button_ability("special_button_02", &tac, &state),
            Some((vec![201, 202], "FULL_SALVO".into()))
        );
        assert_eq!(button_ability("special_button_03", &tac, &state), None);

        // Col 2: empty
        assert_eq!(button_ability("special_button_04", &tac, &state), None);
    }

    #[test]
    fn minimap_excludes_fake_backdrop_planets() {
        let props = vec![(Vec2::new(10.0, 20.0), 5.0)];
        let dots: Vec<eaw_ui::MinimapDot> = props.iter().map(|(pos, _)| eaw_ui::MinimapDot { pos: *pos, color: Color::srgb(0.55, 0.5, 0.45), selected: false }).collect();
        assert_eq!(dots.len(), 1);
        assert_eq!(dots[0].pos, Vec2::new(10.0, 20.0));
    }
}

/// Minimap dot colour by relation to the player (Remake: own green, enemies red).
fn relation_color(side: u8, player: u8) -> Color {
    if side == player { Color::srgb(0.25, 0.95, 0.3) } else { Color::srgb(0.95, 0.25, 0.2) }
}
