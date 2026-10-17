//! The original EaW space tactical bar (`eaw-ui`) for a running battle: selection panel with
//! health and shield bars, ability buttons, retreat countdown and population, and the clicks coming back
//! as sim commands.
//!
//! Needs `eaw_ui::EawUiPlugin` (with `with_game_data`) next to `BattleViewPlugin`. Add [`TacticalUiPlugin`]
//! to get the bar; it spawns with each [`BattleRuntime`] and despawns with it. Not covered: reinforcement
//! buttons (the sim supports `Command::Reinforce`, the galaxy request has no reinforcement pool yet) and
//! eaw-ui's world-space ship overlays (the gizmo bars of `control::overlays` stay in use).
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

use std::collections::{BTreeMap, HashMap, HashSet};

use battle_sim::{Command, HpKind};
use bevy::prelude::*;
use eaw_ui::textures::UiTextures;
use eaw_ui::{
    AbilityState, CommandBarMode, CommandBarSpec, EawAbilityClicked, EawButtonClicked, EawCommandBar, EawHardpointClicked, EawMinimapClicked,
    EawSelectionClicked, EawUiAssets, EawUiCommandsExt, HardpointMarker, MemberBar, MemberBars, MinimapData, MinimapDot, MinimapSurface, PanelData, PanelSlot,
    RetreatState, ShellAnchor, ShellSpec, ShipOverlay, SquadIcon, SquadIcons, TacticalData, TacticalOverlays, TacticalUnit, UiFaction, minimap_to_world, spawn_shell,
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
        app.init_resource::<RetreatNotice>().init_resource::<InterdictNotice>().init_resource::<TacticalUiState>().init_resource::<CraftBarUnits>().init_resource::<ReinforceUi>().init_resource::<crate::hud::HudState>().insert_resource(OverlaysActive);
        app.init_resource::<crate::retreat_prompt::RetreatPrompt>().add_message::<crate::retreat_prompt::RetreatChoice>();
        crate::input_test::build(app);
        app.add_systems(Update, death_star_gauge.run_if(resource_exists::<BattleRuntime>));
        app.add_systems(
            Update,
            (
                (crate::retreat_prompt::open_prompt, crate::retreat_prompt::prompt_input, crate::retreat_prompt::order_retreat).chain().run_if(resource_exists::<BattleRuntime>),
                crate::retreat_prompt::close_prompt.run_if(resource_removed::<BattleRuntime>),
            ),
        );
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
        app.add_systems(Update, sync_member_bars.after(sync_overlays).run_if(resource_exists::<BattleRuntime>));
        crate::cursor::plugin(app);
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
    leftovers: Query<(), Or<(With<eaw_ui::tactical::ShipOverlayNode>, With<eaw_ui::tactical::HardpointNode>, With<eaw_ui::SquadIconNode>, With<crate::hud::TacticalHud>)>>,
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

fn remove_bar(mut commands: Commands, bars: Query<Entity, With<EawCommandBar>>, mut tac: ResMut<TacticalData>, mut overlays: ResMut<TacticalOverlays>, mut members: ResMut<MemberBars>, mut squad_icons: ResMut<SquadIcons>, mut craft_units: ResMut<CraftBarUnits>, mut fog: ResMut<eaw_ui::MinimapFog>, mut armed: ResMut<control::ArmedOrder>, mut action: ResMut<crate::camera::ActionCam>, mut sel: ResMut<Selection>, mut settings: ResMut<BattleViewSettings>) {
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
    members.items.clear();
    squad_icons.items.clear();
    craft_units.0.clear();
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

    let retreat = retreat_state(status.over, cur.retreat_jump_in.get(player as usize).copied().flatten(), status.time, rt.retreat_countdown);
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
    (mut interdict, mut prompt): (ResMut<InterdictNotice>, ResMut<crate::retreat_prompt::RetreatPrompt>),
) {
    let player = settings.player_side;
    for b in buttons.read() {
        let mine = || control::selected_mobile(&rt, &ships, &sel, player);
        match b.name.as_str() {
            "b_retreat" if matches!(retreat_press(tac.retreat), Command::CancelRetreat) => {
                // Pressed again while the hyperdrives charge: the retreat is called off (the sim's `RetreatCancelled` plays the line).
                rt.command(player, Command::CancelRetreat);
            }
            // The game asks first (`BattleViewSettings::retreat_prompt`): Retreat, Auto-Resolve or Cancel.
            "b_retreat" if settings.retreat_prompt && tac.retreat == RetreatState::Ready && !interdicted(&rt, player) => prompt.requested = true,
            "b_retreat" => {
                // The announcement follows the sim's `RetreatOrdered` event (`audio::battle_audio`), so a refused order is silent.
                // Before the retreat countdown ran out the sim refuses: the faction's `Space_Retreat_Not_Allowed_SFXEvent` instead.
                if rt.cur.as_ref().is_some_and(|c| c.time < rt.retreat_countdown)
                    && let Some(n) = rt.player_faction.as_deref().zip(audio.as_ref()).and_then(|(f, a)| a.constants.faction_sfx(f, "space_retreat_not_allowed_sfxevent"))
                {
                    sfx.write(alamo_audio::PlaySfx { event: n.to_string(), position: None, source: None });
                }
                // An enemy gravity well: the sim refuses the retreat; the game's own line explains why.
                if interdicted(&rt, player) {
                    interdict.0 = 4.0;
                    interdict.1 = "Retreat Impossible due to presence of Interdictor".to_string();
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
        // Toggle abilities: every ship of the slot takes the first ship's new state, so a squadron never ends half on, half off.
        let first_on = group
            .iter()
            .filter_map(|&id| rt.cur.as_ref().and_then(|c| c.ship(id)))
            .find_map(|s| {
                let i = rt.catalog.ship(s.def).abilities.iter().position(|d| d.kind.eq_ignore_ascii_case(&a.ability))?;
                s.abilities.get(i).map(|st| st.0 > 0.0)
            })
            .unwrap_or(false);
        for &id in group {
            let Some(s) = rt.cur.as_ref().and_then(|c| c.ship(id)) else { continue };
            let defs = &rt.catalog.ship(s.def).abilities;
            if let Some(i) = defs.iter().position(|d| d.kind.eq_ignore_ascii_case(&a.ability)) {
                // A ship already in the first ship's new state stays: the click flips the first ship, the others follow it.
                if defs[i].toggle && s.abilities.get(i).is_some_and(|st| (st.0 > 0.0) != first_on) {
                    continue;
                }
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
    /// Waves dropped on the map that have not arrived yet, oldest first.
    placed: Vec<Placed>,
}

/// A wave the player dropped on the map: the reserve count went down at the drop (also while the battle is paused, when the simulation
/// has not seen the command yet), and a ring marks where it will arrive until the simulation reports the arrival.
#[derive(Debug)]
struct Placed {
    /// Ship definition.
    ship: u32,
    /// Units.
    count: u32,
    /// The arrival marker.
    marker: Entity,
    /// The simulation accepted the call (`ReinforcementsCalled`).
    acked: bool,
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
    ui.placed.clear();
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
    // Population in play plus the waves dropped but not yet seen by the simulation (it does not step while paused).
    let unseen: i32 = ui.placed.iter().filter(|p| !p.acked).map(|p| rt.catalog.ship(p.ship).population * p.count as i32).sum();
    let pop = rt.cur.as_ref().and_then(|c| c.pop.get(settings.player_side as usize).copied()).unwrap_or(0) + unseen;
    for k in 0..12 {
        let name = format!("r_{:02}{:02}", k / 4, k % 4);
        match ui.rows.get(k) {
            Some((object, ship, left)) => {
                let slot = PanelSlot {
                    icon: game.as_deref().and_then(|g| eaw_ui::unit_icon(g, object)).unwrap_or_default(),
                    label: if *left > 1 { format!("x{left}") } else { String::new() },
                    // Greyed out: none left, or one more would not fit under the population cap (the rule of `place_check`).
                    disabled: *left == 0 || pop + rt.catalog.ship(*ship).population > rt.unit_cap,
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
    let text = format!("{pop}/{}", rt.unit_cap);
    if panel.texts.get("r_pop_text") != Some(&text) {
        panel.set_text("r_pop_text", text);
    }
}

/// A wave was dropped at `ground` (Bevy space): the reserve count goes down now, so the window tells the truth while the battle is paused
/// (the simulation only sees the command on its next step), and a ring marks the arrival spot until the wave arrives.
#[allow(clippy::too_many_arguments)]
fn reserve_wave(
    ui: &mut ReinforceUi,
    ship: u32,
    count: u32,
    ground: Vec3,
    ring: f32,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    if let Some(row) = ui.rows.iter_mut().find(|r| r.1 == ship) {
        row.2 = row.2.saturating_sub(count);
    }
    let marker = commands
        .spawn((
            crate::BattleEntity,
            Name::new("reinforcement arrival marker"),
            Mesh3d(meshes.add(Annulus::new(ring * 0.92, ring))),
            MeshMaterial3d(materials.add(marker_material())),
            Transform::from_translation(ground + Vec3::Y * 2.0).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ))
        .id();
    ui.placed.push(Placed { ship, count, marker, acked: false });
}

/// Material of the ring that marks where a placed wave will arrive.
pub(crate) fn marker_material() -> StandardMaterial {
    StandardMaterial { base_color: Color::srgba(0.35, 0.85, 1.0, 0.55), unlit: true, alpha_mode: AlphaMode::Add, double_sided: true, cull_mode: None, ..default() }
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
                // A wave dropped on the map was counted off the reserve at the drop; only a call that came another way counts off here.
                if let Some(p) = ui.placed.iter_mut().find(|p| !p.acked && p.ship == *ship) {
                    p.acked = true;
                } else if let Some(row) = ui.rows.iter_mut().find(|r| r.1 == *ship) {
                    row.2 = row.2.saturating_sub(*count);
                }
            }
            Event::ReinforcementsArrived { side, .. } if *side == settings.player_side => {
                if let Some(i) = ui.placed.iter().position(|p| p.acked) {
                    let p = ui.placed.remove(i);
                    commands.entity(p.marker).try_despawn();
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

/// Abilities that are aimed at a point: the reveal ping, the lightning storm (zone), the super laser and the weakening area.
fn is_targeted(d: &battle_sim::AbilityDef) -> bool {
    d.reveal_radius > 0.0 || d.zone.is_some() || d.super_laser.is_some() || d.weaken.is_some() || d.energy.is_some()
}

/// Radius within which the sim looks for a super laser target around the aimed point (`Battle::command`, `UseAbilityAt`).
const SUPER_LASER_AIM_RADIUS: f32 = 600.0;

/// Completes a targeted ability: the cursor shows its area (reveal ping radius, storm radius with the placing range around the ship,
/// super laser aim circle); the click sends `UseAbilityAt` for the first ship of the slot that has it ready. A position that the sim would
/// refuse (out of the storm's range, no enemy capital / frigate / structure near the super laser aim) shows the reason and keeps the mode.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ability_at(
    fonts: Res<eaw_ui::EawUiSettings>,
    mut aim: ResMut<crate::cursor::AbilityAim>,
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
        aim.0 = None;
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
                    crate::control::PointerTransparent,
                ))
                .id();
            state.ability_label = Some(e);
            e
        }
    };
    let (Some(ground), Some(cursor)) = (hover.ground.filter(|_| !block.0), pointer.cursor) else {
        aim.0 = None;
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
    if let Some(w) = &def.weaken {
        radius = w.radius;
        colour = Color::srgb(1.0, 0.75, 0.3);
    }
    if let Some(laser) = &def.super_laser {
        radius = SUPER_LASER_AIM_RADIUS;
        colour = Color::srgb(1.0, 0.5, 0.2);
        // The filter of `Battle::command`: the classes of the ability data (Eclipse: Capital, Frigate, SpaceStructure, Super; nothing
        // smaller than a frigate) and an aim circle that reaches the hull radius, so a click on the side of a big hull is not refused here.
        let target = cur.ships.iter().any(|t| {
            let d = rt.catalog.ship(t.def);
            t.side != settings.player_side && (laser.classes == 0 || laser.classes & d.class.bit() != 0) && t.pos.dist(pos) <= SUPER_LASER_AIM_RADIUS + d.radius
        });
        if !target {
            verdict = verdict.and(Err("No target"));
        }
    }
    if verdict.is_err() {
        colour = Color::srgb(1.0, 0.35, 0.3);
    }
    aim.0 = Some(verdict.is_ok());
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

/// The ability of the unit group behind `special_button_NN`: (ship slot group, kind of the ability). The bar entry of the button names
/// its card (`eaw_ui::ability_slots`).
fn button_ability(name: &str, bar: &eaw_ui::CommandBarData, state: &TacticalUiState) -> Option<(Vec<u32>, String)> {
    let k: usize = name.strip_prefix("special_button_")?.parse().ok()?;
    let entry = bar.abilities.get(k).filter(|a| !a.icon.is_empty())?;
    Some((state.groups.get(entry.card)?.clone(), entry.kind.clone()))
}

/// Right click on an ability button turns its autocast on or off for every ship of the slot (only abilities that support it).
#[allow(clippy::too_many_arguments)]
fn autocast_toggle(
    pointer: Res<control::Pointer>,
    bar: Res<eaw_ui::CommandBarData>,
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
    let Some((group, kind)) = button_ability(&name, &bar, &state) else { return };
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
    bar: Res<eaw_ui::CommandBarData>,
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
        let on = button_ability(&b.name, &bar, &state).is_some_and(|(group, kind)| {
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

/// Whether a unit's overlay carries the icon box: a squadron only while it is selected or hovered, any other unit when it shows
/// nothing else (`icon_only`) or is only a few pixels wide (`edge_px`, screen radius).
fn icon_box(is_squad: bool, picked: bool, icon_only: bool, edge_px: f32) -> bool {
    if is_squad { picked } else { icon_only || edge_px < 9.0 }
}

/// Marker id of a hardpoint: `<ship>:<hardpoint index>`.
fn hardpoint_id(ship: u32, hp: usize) -> String {
    format!("{ship}:{hp}")
}

/// Overlay key of a unit: a squadron's members share one (side and squadron), loose fighters and bombers share one per side, group and
/// type, any other ship has its own.
fn unit_key(s: &ShipRender, class: battle_sim::TargetClass) -> u64 {
    if s.squad != u32::MAX {
        (1 << 40) | (u64::from(s.side) << 32) | u64::from(s.squad)
    } else if matches!(class, battle_sim::TargetClass::Fighter | battle_sim::TargetClass::Bomber) {
        (2 << 40) | (u64::from(s.side) << 32) | (u64::from(s.group) << 16) | u64::from(s.def)
    } else {
        u64::from(s.id)
    }
}

/// Least distance in design px that the craft of a picked squadron keep from one another on screen for a bar and a flag each (estimate:
/// the shortest bar is 8 design px and its flag hangs about 10 beyond the bar's end). Closer together, their bars and flags would lie on top
/// of one another: the squadron then shows its one box with the average bar instead.
const CRAFT_BARS_MIN_GAP: f32 = 18.0;
/// Dead band round [`CRAFT_BARS_MIN_GAP`] (estimate): a squadron that shows craft bars keeps them until its craft come this share closer than the limit, one
/// that shows its one box bar needs this share more room. The weave and the flight of the craft move their spacing a few pixels a second; at the
/// limit itself the squadron would flip between the two every few frames.
const CRAFT_BARS_HYSTERESIS: f32 = 0.10;

/// The picked squadrons whose craft carry bars of their own this frame, by [`unit_key`]: decided by [`unit_overlays`] (which needs last frame's
/// answer for its dead band), read by [`sync_member_bars`].
#[derive(Resource, Default, Debug)]
pub(crate) struct CraftBarUnits(HashSet<u64>);

/// Whether the craft of one squadron, at these screen positions, stand far enough apart for a bar and a flag each: the typical craft (the
/// median of every craft's distance to its nearest neighbour) has `min_gap` pixels of room. A lone craft always does.
fn craft_bars_fit(points: &[Vec2], min_gap: f32) -> bool {
    if points.len() < 2 {
        return true;
    }
    let mut nearest: Vec<f32> = points
        .iter()
        .enumerate()
        .map(|(i, p)| points.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, q)| p.distance(*q)).fold(f32::MAX, f32::min))
        .collect();
    nearest.sort_by(f32::total_cmp);
    nearest[nearest.len() / 2] >= min_gap
}

/// Texture name of a unit type's icon (its `Icon_Name`), looked up once per type; squadrons without art get the generic fighter icon.
fn unit_icon_name(cache: &mut HashMap<u32, Option<String>>, assets: Option<&EawUiAssets>, def: &battle_sim::ShipDef, id: u32, is_squad: bool) -> Option<String> {
    cache
        .entry(id)
        .or_insert_with(|| assets.and_then(|a| a.game.as_deref()).and_then(|g| eaw_ui::unit_icon(g, &def.name)))
        .clone()
        .or_else(|| is_squad.then(|| "I_BUTTON_UNIT_GENERIC_FIGHTER.TGA".to_string()))
}

/// What a frame looks like from the camera: what [`unit_overlays`] needs to put units on the screen.
struct ScreenView<'a> {
    /// Window size in logical pixels.
    viewport: Vec2,
    /// The camera's right direction in the world.
    right: Vec3,
    /// Physical pixels per logical pixel: positions snap to whole physical pixels.
    phys: f32,
    /// UI scale: logical pixels per design pixel.
    ui_scale: f32,
    /// World point to logical window pixels, `None` behind the camera.
    project: &'a dyn Fn(Vec3) -> Option<Vec2>,
}

/// What [`unit_overlays`] reads of the battle.
struct UnitInputs<'a> {
    catalog: &'a battle_sim::Catalog,
    ships: &'a RenderedShips,
    sel: &'a Selection,
    hover: &'a Hover,
    /// The player's side.
    player: u8,
    assets: Option<&'a EawUiAssets>,
    /// Whether a retreat jump is counting down for that side.
    retreat_active: &'a dyn Fn(u8) -> bool,
}

/// The overlays of every visible unit and the icons of the squadrons nobody picks: a picked unit (selected or under the cursor) shows its
/// brackets and bars, a picked squadron its box and either its craft's bars ([`sync_member_bars`]) or its own average bar, an unpicked
/// squadron its one small icon and nothing else, any other unit nobody picks only its icon box.
fn unit_overlays(inp: &UnitInputs, icon_names: &mut HashMap<u32, Option<String>>, craft_bars: &mut HashSet<u64>, view: &ScreenView) -> (Vec<ShipOverlay>, Vec<SquadIcon>) {
    let (ships, sel, hover, player) = (inp.ships, inp.sel, inp.hover, inp.player);
    let (vp, project) = (view.viewport, view.project);
    let margin = 120.0;
    // Whole physical pixels: a bar that moves in sub-pixel steps is resampled every frame and its 2-3 px lines shimmer while a ship moves or turns.
    let snap = |v: f32| (v * view.phys).round() / view.phys;
    let (mut overlays, mut icons) = (Vec::new(), Vec::new());
    let mut next_bars: HashSet<u64> = HashSet::new();
    // One overlay per unit: a picked squadron is one unit, so its fighters share a single icon box (their bars are `sync_member_bars`).
    let mut groups: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
    for (&id, s) in ships.0.iter().filter(|(_, s)| s.visible) {
        groups.entry(unit_key(s, inp.catalog.ship(s.def).class)).or_default().push(id);
    }
    for (key, mut members) in groups {
        // The lowest id speaks for the unit: HashMap order would flip a mixed squadron between its types from frame to frame.
        members.sort_unstable();
        let selected = members.iter().any(|id| sel.ships.contains(id) || sel.enemy == Some(*id));
        let hovered = members.iter().any(|id| hover.ship == Some(*id));
        let first = &ships.0[&members[0]];
        let def = inp.catalog.ship(first.def);
        let is_squad = is_small_craft(def.class, first.squad);
        let enemy = first.side != player;
        // An unpicked squadron needs only where its craft are; every other unit also how large they look.
        let sized = !is_squad || selected || hovered;
        let pts: Vec<(Vec2, f32)> = members
            .iter()
            .filter_map(|id| {
                let s = &ships.0[id];
                let c = project(s.pos)?;
                let edge = if sized { project(s.pos + view.right * s.radius).map_or(8.0, |e| (e - c).length()) } else { 0.0 };
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
        // A squadron nobody picks: its one small icon, nothing else (no box, bar or flag; its craft are drawn at their model scale).
        if is_squad && !(selected || hovered) {
            if let Some(icon) = unit_icon_name(icon_names, inp.assets, def, first.def, true) {
                icons.push(SquadIcon { id: key, screen_pos: Vec2::new(snap(centre.x), snap(centre.y)), icon, enemy });
            }
            continue;
        }
        // A picked squadron shows its box, and its craft their bars when they stand far enough apart, else the box carries the average bar (with
        // a dead band round the limit, last frame's answer decides inside it).
        let own_bars = is_squad && {
            let room = CRAFT_BARS_MIN_GAP * view.ui_scale * if craft_bars.contains(&key) { 1.0 - CRAFT_BARS_HYSTERESIS } else { 1.0 + CRAFT_BARS_HYSTERESIS };
            craft_bars_fit(&pts.iter().map(|p| p.0).collect::<Vec<_>>(), room)
        };
        if own_bars {
            next_bars.insert(key);
        }
        // Units nobody selected or hovers show only their icon.
        let icon_only = if is_squad { own_bars } else { !selected && !hovered };
        let half = pts.iter().map(|(c, e)| (*c - centre).length() + e).fold(0.0f32, f32::max);
        let (centre, half) = (Vec2::new(snap(centre.x), snap(centre.y)), snap(half));
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
                .and_then(|k| inp.assets.and_then(|a| a.text.get(k)))
                .filter(|t| !t.is_empty())
                .map_or_else(|| def.name.clone(), str::to_string)
        });
        if let Some(n) = name.as_mut()
            && (inp.retreat_active)(first.side)
            && !live.iter().all(|s| s.can_retreat)
        {
            n.push_str(" [ENGINES DISABLED]");
        }
        // The box with the unit icon marks tiny units and every other unit nobody has selected or hovered (so that no unit is invisible),
        // and a picked squadron. An unpicked squadron has no box: its craft are drawn at their model scale, so it shows its one small icon
        // ([`SquadIcons`], above).
        let marked = icon_box(is_squad, selected || hovered, icon_only, edge_max);
        let icon = marked.then(|| unit_icon_name(icon_names, inp.assets, def, first.def, is_squad)).flatten();
        if icon_only && icon.is_none() {
            continue;
        }
        overlays.push(ShipOverlay {
            id: key,
            screen_pos: centre,
            half_size: if is_squad { 11.0 } else { half.clamp(11.0, 220.0) },
            health: live.iter().map(|s| s.hull).sum::<f32>() / n,
            shield: (def.shield > 0.0).then(|| live.iter().map(|s| s.shield).sum::<f32>() / n),
            selected,
            hovered,
            enemy,
            name,
            always: false,
            // Far zoom (units a few pixels wide): a small box with the unit icon marks the unit, as in the Remake (video b0700).
            // A picked squadron gets the box too; the bars and flags of its craft are `MemberBars`, not part of this overlay.
            icon,
            icon_only,
        });
    }
    *craft_bars = next_bars;
    (overlays, icons)
}

/// Feeds eaw-ui's world-space overlays: selection brackets and bars of the ships and structures that are hovered or selected, the
/// icon box of a picked squadron, the one small icon of every other squadron ([`SquadIcons`]) and the hardpoint reticles of the targeted
/// enemy. Bars and flags show only on picked units (selected or under the cursor); the bars and flags of the craft of a picked squadron
/// are [`sync_member_bars`].
#[allow(clippy::too_many_arguments)]
fn sync_overlays(
    rt: Res<BattleRuntime>,
    sel: Res<Selection>,
    hover: Res<Hover>,
    ships: Res<RenderedShips>,
    settings: Res<BattleViewSettings>,
    assets: Option<Res<EawUiAssets>>,
    ui_scale: Res<UiScale>,
    camera: Single<(&Camera, &GlobalTransform), With<BattleCamera>>,
    mut out: ResMut<TacticalOverlays>,
    mut squad_icons: ResMut<SquadIcons>,
    mut craft_units: ResMut<CraftBarUnits>,
    mut icon_names: Local<HashMap<u32, Option<String>>>,
) {
    let (cam, gt) = *camera;
    let Some(viewport) = cam.logical_viewport_size() else { return };
    let player = settings.player_side;
    let project = |p: Vec3| cam.world_to_viewport(gt, p).ok();
    let view = ScreenView { viewport, right: gt.right().into(), phys: cam.target_scaling_factor().unwrap_or(1.0).max(0.01), ui_scale: ui_scale.0, project: &project };
    let retreat_active = |side: u8| rt.cur.as_ref().and_then(|c| c.retreat_jump_in.get(side as usize).copied().flatten()).is_some();
    let inputs = UnitInputs { catalog: &rt.catalog, ships: &ships, sel: &sel, hover: &hover, player, assets: assets.as_deref(), retreat_active: &retreat_active };
    let (overlays, icons) = unit_overlays(&inputs, &mut icon_names, &mut craft_units.0, &view);
    let mut next = TacticalOverlays { ships: overlays, hardpoints: Vec::new() };
    let next_icons = SquadIcons { items: icons };
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
            let Some(p) = project(control::hardpoint_world(&rt, s, i)) else { continue };
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
    if *squad_icons != next_icons {
        *squad_icons = next_icons;
    }
}

/// Margin around the window inside which small craft still get a bar (logical px): a craft just off the edge keeps its bar sliding in.
const MEMBER_BAR_MARGIN: f32 = 120.0;

/// Whether a ship is one of the small craft that carry a bar and a flag of their own when picked: fighters, bombers and the members of any
/// squadron (a Skipray blast boat squadron is no fighter class but reads as a unit of small craft).
fn is_small_craft(class: battle_sim::TargetClass, squad: u32) -> bool {
    matches!(class, battle_sim::TargetClass::Fighter | battle_sim::TargetClass::Bomber) || squad != u32::MAX
}

/// The bars and flags of single craft, only for the craft of the units in `with_bars` (the picked squadrons whose craft stand far enough
/// apart, [`CraftBarUnits`]; `unit_of` names the unit of a small craft, `None` for every other ship): the member's own hull (and shield when
/// it has one), at the screen position and size of its drawn bounding sphere, like the Remake's (user's video b0703s, b0732s, b0850s,
/// screenshot 07_tactical_battle). Craft whose model is not built yet, that the player does not see (fog of war) or that are off screen get
/// none; dead ones are no longer in `ships`. `project` maps a world point to logical window pixels. The result is sorted by id so overlapping
/// bars keep their order. With no unit in `with_bars` there is nothing to do and no bar.
fn member_bars(
    ships: &[&ShipRender],
    unit_of: impl Fn(&ShipRender) -> Option<u64>,
    with_bars: &HashSet<u64>,
    has_shield: impl Fn(&ShipRender) -> bool,
    project: impl Fn(Vec3) -> Option<Vec2>,
    right: Vec3,
    viewport: Vec2,
    out: &mut Vec<MemberBar>,
) {
    out.clear();
    if with_bars.is_empty() {
        return;
    }
    let mut units: HashMap<u64, Vec<(&ShipRender, Vec2)>> = HashMap::new();
    for s in ships.iter().filter(|s| s.visible) {
        if let Some(key) = unit_of(s).filter(|k| with_bars.contains(k))
            && let Some(c) = project(s.pos)
        {
            units.entry(key).or_default().push((s, c));
        }
    }
    for members in units.values() {
        for &(s, c) in members.iter().filter(|(s, _)| s.drawn_radius > 0.0) {
            if c.x < -MEMBER_BAR_MARGIN || c.y < -MEMBER_BAR_MARGIN || c.x > viewport.x + MEMBER_BAR_MARGIN || c.y > viewport.y + MEMBER_BAR_MARGIN {
                continue;
            }
            let radius = project(s.pos + right * s.drawn_radius).map_or(8.0, |e| (e - c).length());
            out.push(MemberBar { id: s.id, pos: c, radius, health: s.hull, shield: has_shield(s).then_some(s.shield) });
        }
    }
    out.sort_unstable_by_key(|m| m.id);
}

/// Feeds eaw-ui's per-craft bars and flags ([`MemberBars`]) for the squadrons [`sync_overlays`] decided on ([`CraftBarUnits`]): the craft of selected
/// or hovered squadrons that stand far enough apart on screen, see [`member_bars`].
fn sync_member_bars(
    rt: Res<BattleRuntime>,
    ships: Res<RenderedShips>,
    units: Res<CraftBarUnits>,
    camera: Single<(&Camera, &GlobalTransform), With<BattleCamera>>,
    mut out: ResMut<MemberBars>,
) {
    let (cam, gt) = *camera;
    let Some(viewport) = cam.logical_viewport_size() else { return };
    out.scale_factor = cam.target_scaling_factor().unwrap_or(1.0).max(0.01);
    let catalog = &rt.catalog;
    let class_of = |s: &ShipRender| catalog.ship(s.def).class;
    let all: Vec<&ShipRender> = if units.0.is_empty() { Vec::new() } else { ships.0.values().collect() };
    member_bars(
        &all,
        |s| is_small_craft(class_of(s), s.squad).then(|| unit_key(s, class_of(s))),
        &units.0,
        |s| catalog.ship(s.def).shield > 0.0,
        |p| cam.world_to_viewport(gt, p).ok(),
        gt.right().into(),
        viewport,
        &mut out.items,
    );
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
                    crate::control::PointerTransparent,
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
                reserve_wave(&mut ui, row.1, count, crate::convert::sim_to_bevy(pos), 120.0, &mut commands, &mut meshes, &mut materials);
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
            // The count drops here, not at the simulation's report: a paused battle does not step, so it would show nothing (see `reserve_wave`).
            ctx.rt.command(player, Command::ReinforceAt { ship: row.1, count, pos: sim_pos, yaw_offset: ui.ghost_yaw_offset });
            reserve_wave(&mut ui, row.1, count, ground, ring, &mut commands, &mut meshes, &mut materials);
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

/// A short notice for an order the game refuses (seconds left, text), shown in the retreat banner: Retreat under an enemy interdictor
/// (`TEXT_SFX_*_RETREAT_INTERDICTOR_1`, the same English for all three factions) and a Move for a ship whose own ability pins it
/// (`Disable_Movement`: `TEXT_SFX_INT_CANNOT_MOVE_5` for the interdictor field, `_3` for the missile shield).
#[derive(Resource, Default)]
pub struct InterdictNotice(pub f32, pub String);

/// State of the Retreat button: unavailable once the battle is over and before the retreat countdown ran out (the original shows it dimmed and
/// without text, Remake video b0662), the jump countdown while the own retreat runs (pressed look and m:ss under it), else ready.
fn retreat_state(over: bool, jump_in: Option<f32>, time: f32, countdown: f32) -> RetreatState {
    if over {
        RetreatState::Unavailable
    } else if let Some(jump) = jump_in {
        RetreatState::Countdown(jump)
    } else if time >= countdown {
        RetreatState::Ready
    } else {
        RetreatState::Unavailable
    }
}

/// What a press on the Retreat button does: while the jump countdown runs it calls the retreat off (EaW), otherwise it orders the retreat.
fn retreat_press(state: RetreatState) -> Command {
    if matches!(state, RetreatState::Countdown(_)) { Command::CancelRetreat } else { Command::Retreat }
}

/// An enemy ship or station holds a gravity well against `side` (the rule of `Battle::interdicted`, read from the displayed snapshot).
fn interdicted(rt: &BattleRuntime, side: u8) -> bool {
    rt.cur.as_ref().is_some_and(|c| {
        c.ships.iter().any(|s| {
            let d = rt.catalog.ship(s.def);
            s.side != side && (d.interdictor || d.abilities.iter().zip(&s.abilities).any(|(a, st)| a.interdict && st.0 > 0.0))
        })
    })
}

fn retreat_banner(
    mut commands: Commands,
    ui: Res<eaw_ui::EawUiSettings>,
    settings: Res<BattleViewSettings>,
    time: Res<Time<Real>>,
    rt: Res<BattleRuntime>,
    status: Res<BattleStatus>,
    mut notice: ResMut<RetreatNotice>,
    mut interdict: ResMut<InterdictNotice>,
    tac: Res<TacticalData>,
    mut banner: Query<(Entity, &mut Text), With<RetreatBanner>>,
) {
    let mss = |s: f32| format!("{}:{:02}", s.ceil() as u32 / 60, s.ceil() as u32 % 60);
    notice.0 = (notice.0 - time.delta_secs()).max(0.0);
    interdict.0 = (interdict.0 - time.delta_secs()).max(0.0);
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
        _ if interdict.0 > 0.0 => Some(interdict.1.clone()),
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
    fn squadrons_carry_the_icon_box_only_while_selected_or_hovered() {
        // An unpicked squadron, own or enemy, at any zoom: no box (it has its one small icon instead, see `unit_overlays`).
        assert!(!icon_box(true, false, false, 3.0));
        assert!(!icon_box(true, false, true, 40.0));
        assert!(icon_box(true, true, false, 40.0));
        // Other units keep it when they show nothing else or are a few pixels wide.
        assert!(icon_box(false, false, true, 40.0));
        assert!(icon_box(false, false, false, 5.0));
        assert!(!icon_box(false, true, false, 40.0));
    }

    /// Class of a test craft by its definition: 0 is a fighter, anything else a capital ship.
    fn class_of(s: &ShipRender) -> battle_sim::TargetClass {
        if s.def == 0 { battle_sim::TargetClass::Fighter } else { battle_sim::TargetClass::Capital }
    }

    /// A craft as the renderer publishes it, `drawn_radius` 17 (the bounding sphere as drawn).
    fn ship(id: u32, def: u32, side: u8, squad: u32, pos: Vec3, hull: f32) -> ShipRender {
        ShipRender::test_craft(id, def, side, squad, pos, hull)
    }

    /// A member of own squadron 0 at screen x.
    fn craft(id: u32, x: f32, hull: f32) -> ShipRender {
        ship(id, 0, 0, 0, Vec3::new(x, 0.0, 100.0), hull)
    }

    /// Orthographic stand-in for the camera: one world unit is one pixel, x right, z down.
    fn flat(p: Vec3) -> Option<Vec2> {
        Some(Vec2::new(p.x, p.z))
    }

    const VIEWPORT: Vec2 = Vec2::new(1920.0, 1080.0);

    /// The bars of single craft of the units in `with_bars` (the picked squadrons that [`unit_overlays`] gave bars of their own).
    fn bars_for(ships: &[ShipRender], with_bars: &[u64], has_shield: impl Fn(&ShipRender) -> bool) -> Vec<MemberBar> {
        let all: Vec<&ShipRender> = ships.iter().collect();
        let with_bars: HashSet<u64> = with_bars.iter().copied().collect();
        let mut out = Vec::new();
        member_bars(&all, |s| is_small_craft(class_of(s), s.squad).then(|| unit_key(s, class_of(s))), &with_bars, has_shield, flat, Vec3::X, VIEWPORT, &mut out);
        out
    }

    /// Key of own squadron 0.
    fn squad0() -> u64 {
        unit_key(&craft(0, 0.0, 1.0), battle_sim::TargetClass::Fighter)
    }

    /// The bars of squadron 0, picked, with room for all of them.
    fn bars_of(ships: &[ShipRender], has_shield: impl Fn(&ShipRender) -> bool) -> Vec<MemberBar> {
        bars_for(ships, &[squad0()], has_shield)
    }

    #[test]
    fn every_member_of_a_picked_squadron_gets_its_own_bar() {
        let squad = [craft(7, 300.0, 1.0), craft(3, 200.0, 0.5), craft(5, 250.0, 0.2)];
        let bars = bars_of(&squad, |_| false);
        assert_eq!(bars.len(), 3, "one bar per member, not one per squadron");
        // At the member's own place, as large as its drawn sphere, sorted by id.
        assert_eq!(bars.iter().map(|b| b.id).collect::<Vec<_>>(), [3, 5, 7]);
        assert_eq!(bars.iter().map(|b| b.pos.x).collect::<Vec<_>>(), [200.0, 250.0, 300.0]);
        assert!(bars.iter().all(|b| b.radius == 17.0 && b.pos.y == 100.0));
    }

    #[test]
    fn nothing_picked_shows_no_bar_at_all() {
        // Three squadrons of 12 craft 40 px apart and an enemy capital ship: with nothing selected and nothing under the cursor no squadron has
        // craft bars, no craft has a bar and no overlay carries a health bar.
        let (ships, catalog) = field(40.0);
        let mut with_bars = HashSet::new();
        let (overlays, _) = overlays_state(&ships, &catalog, &Selection::default(), &Hover::default(), &mut with_bars);
        assert!(with_bars.is_empty(), "{with_bars:?}");
        assert!(overlays.iter().all(|o| o.icon_only), "{overlays:?}");
        assert!(bars_for(&ships, &[], |_| false).is_empty());
        // A selected or hovered capital ship adds no craft bars either: only squadrons have them.
        let sel = Selection { ships: vec![], enemy: Some(100) };
        let hover = Hover { ship: Some(100), ..Hover::default() };
        overlays_state(&ships, &catalog, &sel, &hover, &mut with_bars);
        assert!(with_bars.is_empty(), "{with_bars:?}");
    }

    #[test]
    fn only_the_craft_of_a_picked_squadron_get_bars() {
        let ships: Vec<ShipRender> = (0..24).map(|i| ship(i, 0, 0, i / 12, Vec3::new(100.0 + 600.0 * (i / 12) as f32 + 40.0 * (i % 12) as f32, 0.0, 100.0), 1.0)).collect();
        let key = |squad: u32| unit_key(&ship(0, 0, 0, squad, Vec3::ZERO, 1.0), battle_sim::TargetClass::Fighter);
        let ids = |picked: &[u64]| bars_for(&ships, picked, |_| false).iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(&[key(1)]), (12..24).collect::<Vec<_>>(), "squadron 1 only");
        assert_eq!(ids(&[key(0)]), (0..12).collect::<Vec<_>>(), "squadron 0 only");
        assert_eq!(ids(&[key(0), key(1)]).len(), 24);
    }

    #[test]
    fn selecting_or_hovering_a_craft_picks_its_whole_squadron() {
        // Own squadrons 0 and 1 are the craft 0-11 and 12-23, the enemy's squadron 0 the craft 24-35.
        let (ships, catalog) = field(40.0);
        let key = |i: usize| unit_key(&ships[i], battle_sim::TargetClass::Fighter);
        let (own0, own1, enemy0) = (key(0), key(12), key(24));
        assert_ne!(own0, enemy0, "squadron 0 of the other side is another unit");
        let picked = |sel: Selection, hover: Hover| {
            let mut with_bars = HashSet::new();
            overlays_state(&ships, &catalog, &sel, &hover, &mut with_bars);
            with_bars
        };
        assert!(picked(Selection::default(), Hover::default()).is_empty());
        assert_eq!(picked(Selection { ships: vec![5], enemy: None }, Hover::default()), HashSet::from([own0]), "one selected craft stands for its squadron");
        assert_eq!(picked(Selection::default(), Hover { ship: Some(7), ..Hover::default() }), HashSet::from([own0]), "so does the one under the cursor");
        assert_eq!(picked(Selection { ships: vec![], enemy: Some(30) }, Hover::default()), HashSet::from([enemy0]), "the enemy marked for targeting");
        assert_eq!(picked(Selection { ships: vec![0], enemy: None }, Hover { ship: Some(13), ..Hover::default() }), HashSet::from([own0, own1]));
    }

    #[test]
    fn a_members_bar_follows_its_own_health() {
        let mut squad = [craft(1, 100.0, 1.0), craft(2, 200.0, 0.75), craft(3, 300.0, 0.3)];
        let health = |bars: &[MemberBar]| bars.iter().map(|b| b.health).collect::<Vec<_>>();
        assert_eq!(health(&bars_of(&squad, |_| false)), [1.0, 0.75, 0.3]);
        // Only the member that took damage changes; the others keep theirs.
        squad[1].hull = 0.25;
        assert_eq!(health(&bars_of(&squad, |_| false)), [1.0, 0.25, 0.3]);
    }

    #[test]
    fn members_with_shields_carry_their_shield_and_the_others_none() {
        let mut squad = [craft(1, 100.0, 1.0), craft(2, 200.0, 1.0)];
        squad[0].shield = 0.6;
        let bars = bars_of(&squad, |s| s.id == 1);
        assert_eq!(bars[0].shield, Some(0.6));
        assert_eq!(bars[1].shield, None);
    }

    #[test]
    fn hidden_dead_unbuilt_and_off_screen_members_show_nothing() {
        let mut squad = [craft(1, 100.0, 1.0), craft(2, 200.0, 1.0), craft(3, 300.0, 1.0), craft(4, 400.0, 1.0), craft(5, 500.0, 1.0)];
        squad[1].visible = false; // fog of war
        squad[2].drawn_radius = 0.0; // model not built yet
        squad[3].pos.x = 5000.0; // far off the screen
        let ids = |ships: &[ShipRender]| bars_of(ships, |_| false).iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(&squad), [1, 5]);
        // A dead member is gone from the published ships altogether.
        assert_eq!(ids(&squad[..1]), [1]);
    }

    #[test]
    fn capital_ships_keep_their_own_overlay_and_get_no_member_bar() {
        use battle_sim::TargetClass as C;
        assert!(is_small_craft(C::Fighter, u32::MAX));
        assert!(is_small_craft(C::Bomber, u32::MAX));
        assert!(is_small_craft(C::Corvette, 4), "a squadron member of any class is a small craft");
        assert!(!is_small_craft(C::Capital, u32::MAX));
        assert!(!is_small_craft(C::Frigate, u32::MAX));
        let capital = ship(2, 1, 0, u32::MAX, Vec3::new(200.0, 0.0, 100.0), 1.0);
        let ships = [craft(1, 100.0, 1.0), capital];
        let key = unit_key(&ships[1], C::Capital);
        assert_eq!(bars_for(&ships, &[key, squad0()], |_| false).iter().map(|b| b.id).collect::<Vec<_>>(), [1], "even named, a capital ship has no craft bar");
    }

    #[test]
    fn craft_standing_close_together_do_not_pile_up_their_bars() {
        // One picked squadron of 12 on a grid: 9 px apart (a TIE squadron at camera 2000) the box carries one average bar and the craft none, 40 px
        // apart (a close zoom) each craft has its own bar and the box none.
        for (gap, own) in [(9.0, false), (40.0, true)] {
            let (ships, catalog) = field(gap);
            let key = unit_key(&ships[0], battle_sim::TargetClass::Fighter);
            let mut with_bars = HashSet::new();
            let (overlays, _) = overlays_state(&ships, &catalog, &Selection { ships: vec![0], enemy: None }, &Hover::default(), &mut with_bars);
            assert_eq!(with_bars.contains(&key), own, "{gap} px apart");
            assert_eq!(overlays.iter().find(|o| o.id == key).expect("the box").icon_only, own, "the box carries the bar only when the craft have none");
            assert_eq!(bars_for(&ships, &with_bars.iter().copied().collect::<Vec<_>>(), |_| false).len(), if own { 12 } else { 0 }, "{gap} px apart");
        }
        // The typical craft decides, not the one that strayed: a squadron spread out with one craft beside its neighbour still shows bars.
        let min_gap = CRAFT_BARS_MIN_GAP * 1.41;
        assert!(craft_bars_fit(&[Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(100.0, 0.0), Vec2::new(200.0, 0.0), Vec2::new(300.0, 0.0)], min_gap));
        assert!(!craft_bars_fit(&[Vec2::new(0.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(200.0, 0.0)], min_gap));
        assert!(craft_bars_fit(&[Vec2::new(3.0, 3.0)], min_gap), "a lone craft always has room");
        assert!(craft_bars_fit(&[], min_gap));
    }

    #[test]
    fn a_squadron_at_the_limit_does_not_flip_between_craft_bars_and_one_bar() {
        let limit = CRAFT_BARS_MIN_GAP * 1.41;
        let sel = Selection { ships: vec![0], enemy: None };
        let has_bars = |gap: f32, state: &mut HashSet<u64>| {
            let (ships, catalog) = field(gap);
            overlays_state(&ships, &catalog, &sel, &Hover::default(), state);
            state.contains(&unit_key(&ships[0], battle_sim::TargetClass::Fighter))
        };
        // The craft's spacing wobbles 6 % round the limit (the weave, the flight), frame after frame: nothing changes, whichever side it started on.
        for start in [0.5f32, 1.5] {
            let mut state = HashSet::new();
            let settled = has_bars(limit * start, &mut state);
            assert_eq!(settled, start > 1.0);
            for frame in 0..300 {
                let gap = limit * (1.0 + 0.06 * (frame as f32 * 0.9).sin());
                assert_eq!(has_bars(gap, &mut state), settled, "frame {frame}, spacing {:.2} of the limit", gap / limit);
            }
        }
        // A zoom from 0.5 to 1.5 times the limit and back, one percent a frame: one change each way, at 1.1 and at 0.9 of the limit.
        let mut state = HashSet::new();
        let (mut last, mut changes) = (false, Vec::new());
        for k in 0..=200 {
            let f = if k <= 100 { 0.5 + k as f32 / 100.0 } else { 1.5 - (k - 100) as f32 / 100.0 };
            let now = has_bars(limit * f, &mut state);
            if now != last {
                changes.push((now, f));
                last = now;
            }
        }
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(changes[0].0 && (1.1..1.12).contains(&changes[0].1), "craft bars come at 1.1 of the limit: {changes:?}");
        assert!(!changes[1].0 && (0.88..=0.9).contains(&changes[1].1), "and go at 0.9 of it: {changes:?}");
    }

    /// Visible units for `unit_overlays`: own squadrons 0 and 1 and enemy squadron 0 of twelve craft each, 40 px apart, and an enemy capital ship.
    fn field(gap: f32) -> (Vec<ShipRender>, battle_sim::Catalog) {
        let mut ships = Vec::new();
        for (side, squad, x0) in [(0u8, 0u32, 100.0f32), (0, 1, 700.0), (1, 0, 1300.0)] {
            for i in 0..12u32 {
                ships.push(ship(ships.len() as u32, 0, side, squad, Vec3::new(x0 + gap * (i % 4) as f32, 0.0, 300.0 + gap * (i / 4) as f32), 1.0));
            }
        }
        ships.push(ship(100, 1, 1, u32::MAX, Vec3::new(1500.0, 0.0, 800.0), 1.0));
        let def = |name: &str, class| battle_sim::ShipDef { name: name.into(), class, hull: 100.0, radius: 10.0, population: 1, ..battle_sim::ShipDef::default() };
        let catalog = battle_sim::Catalog::from_defs(vec![def("X-Wing", battle_sim::TargetClass::Fighter), def("Star_Destroyer", battle_sim::TargetClass::Capital)], vec![]);
        (ships, catalog)
    }

    fn overlays_of(ships: &[ShipRender], catalog: &battle_sim::Catalog, sel: &Selection, hover: &Hover) -> (Vec<ShipOverlay>, Vec<SquadIcon>) {
        overlays_state(ships, catalog, sel, hover, &mut HashSet::new())
    }

    /// [`unit_overlays`] with the answer of the last frame in `craft_bars` (the squadrons whose craft carry bars), which it replaces.
    fn overlays_state(ships: &[ShipRender], catalog: &battle_sim::Catalog, sel: &Selection, hover: &Hover, craft_bars: &mut HashSet<u64>) -> (Vec<ShipOverlay>, Vec<SquadIcon>) {
        let rendered = RenderedShips(ships.iter().cloned().map(|s| (s.id, s)).collect());
        let no_retreat = |_: u8| false;
        let inputs = UnitInputs { catalog, ships: &rendered, sel, hover, player: 0, assets: None, retreat_active: &no_retreat };
        let view = ScreenView { viewport: VIEWPORT, right: Vec3::X, phys: 1.0, ui_scale: 1.41, project: &flat };
        unit_overlays(&inputs, &mut HashMap::new(), craft_bars, &view)
    }

    #[test]
    fn every_unpicked_squadron_has_one_small_icon_and_no_box_or_bar() {
        let (ships, catalog) = field(40.0);
        let (overlays, icons) = overlays_of(&ships, &catalog, &Selection::default(), &Hover::default());
        assert_eq!(icons.len(), 3, "one icon per squadron, not one per craft (36 craft)");
        assert_eq!(icons.iter().filter(|i| i.enemy).count(), 1, "the enemy squadron has its icon too");
        let ids: HashSet<u64> = icons.iter().map(|i| i.id).collect();
        assert_eq!(ids.len(), 3, "each its own squadron");
        assert!(icons.iter().all(|i| !i.icon.is_empty()), "the unit's icon art (the generic fighter without game data)");
        // The icon sits in the middle of its squadron.
        let own0 = icons.iter().find(|i| !i.enemy && i.screen_pos.x < 400.0).expect("own squadron 0");
        assert_eq!(own0.screen_pos, Vec2::new(100.0 + 60.0, 300.0 + 40.0));
        // No squadron overlay (box, bars, flag), and no health bar anywhere with nothing picked (the enemy capital ship would show only its
        // icon box, which needs the game's icon art).
        assert!(overlays.iter().all(|o| o.id == 100 && o.icon_only), "nothing but the capital ship's icon box: {overlays:?}");
        // And no craft bar.
        assert!(bars_for(&ships, &[], |_| false).is_empty());
    }

    #[test]
    fn a_picked_squadron_keeps_its_box_and_gives_up_its_icon() {
        let (ships, catalog) = field(40.0);
        let own0 = unit_key(&ships[0], battle_sim::TargetClass::Fighter);
        for (sel, hover) in [(Selection { ships: vec![5], enemy: None }, Hover::default()), (Selection::default(), Hover { ship: Some(5), ..Hover::default() })] {
            let (overlays, icons) = overlays_of(&ships, &catalog, &sel, &hover);
            assert_eq!(icons.len(), 2, "the other two squadrons keep theirs");
            assert!(icons.iter().all(|i| i.id != own0));
            let boxed = overlays.iter().find(|o| o.id == own0).expect("the box of the picked squadron");
            assert!(boxed.icon.is_some() && (boxed.selected || boxed.hovered));
            assert_eq!(overlays.iter().filter(|o| o.id != 100).count(), 1, "one box, not one per craft");
            // 40 px apart there is room for a bar each: the box shows none of its own.
            assert!(boxed.icon_only, "its craft carry the bars");
        }
    }

    #[test]
    fn a_picked_squadron_standing_close_shows_one_average_bar_under_its_box() {
        let (mut ships, catalog) = field(6.0);
        ships[3].hull = 0.5;
        let (overlays, icons) = overlays_of(&ships, &catalog, &Selection { ships: vec![0], enemy: None }, &Hover::default());
        assert_eq!(icons.len(), 2);
        let boxed = overlays.iter().find(|o| o.id != 100 && o.icon.is_some()).expect("box");
        assert!(!boxed.icon_only, "the box carries the squadron's bar");
        assert!((boxed.health - (11.0 + 0.5) / 12.0).abs() < 1e-5, "the average of its craft: {}", boxed.health);
        // ... and the craft show none of their own.
        let key = unit_key(&ships[0], battle_sim::TargetClass::Fighter);
        let mut with_bars = HashSet::new();
        overlays_state(&ships, &catalog, &Selection { ships: vec![0], enemy: None }, &Hover::default(), &mut with_bars);
        assert!(!with_bars.contains(&key) && bars_for(&ships, &with_bars.iter().copied().collect::<Vec<_>>(), |_| false).is_empty());
    }

    #[test]
    fn squadrons_of_both_sides_and_numbers_are_separate_units() {
        let a = ship(1, 0, 0, 0, Vec3::ZERO, 1.0);
        let same = ship(2, 0, 0, 0, Vec3::X, 0.5);
        let other_squad = ship(3, 0, 0, 1, Vec3::ZERO, 1.0);
        let other_side = ship(4, 0, 1, 0, Vec3::ZERO, 1.0);
        let c = battle_sim::TargetClass::Fighter;
        assert_eq!(unit_key(&a, c), unit_key(&same, c));
        assert_ne!(unit_key(&a, c), unit_key(&other_squad, c));
        assert_ne!(unit_key(&a, c), unit_key(&other_side, c));
        let capital = ship(5, 1, 0, u32::MAX, Vec3::ZERO, 1.0);
        assert_eq!(unit_key(&capital, battle_sim::TargetClass::Capital), 5, "a capital ship is its own unit, keyed by its id");
    }

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
    fn button_ability_maps_to_the_card_of_its_bar_entry() {
        let ability = |kind: &str| eaw_ui::AbilityState { def: eaw_ui::AbilityDef { kind: kind.into(), icon: format!("I_SA_{kind}.TGA"), ..default() }, ..default() };
        let unit = |name: &str, kinds: &[&str]| eaw_ui::TacticalUnit { unit_type: name.into(), abilities: kinds.iter().map(|k| ability(k)).collect(), ..default() };
        let tac = TacticalData {
            active: true,
            selection: vec![unit("Thrawn", &["POWER_TO_WEAPONS", "TRACTOR_BEAM"]), unit("Victory", &["FULL_SALVO"]), unit("Corvette", &[]), unit("X-Wing", &["HUNT", "SPOILER_LOCK"])],
            ..default()
        };
        let bar = eaw_ui::CommandBarData { abilities: eaw_ui::ability_slots(&tac), ..default() };
        let state = TacticalUiState { groups: vec![vec![101], vec![201, 202], vec![301], vec![401, 402, 403]], ..default() };
        // The buttons fill in card order; the card of each button is in its bar entry, not in the button number.
        assert_eq!(button_ability("special_button_00", &bar, &state), Some((vec![101], "POWER_TO_WEAPONS".into())));
        assert_eq!(button_ability("special_button_01", &bar, &state), Some((vec![101], "TRACTOR_BEAM".into())));
        assert_eq!(button_ability("special_button_02", &bar, &state), Some((vec![201, 202], "FULL_SALVO".into())));
        assert_eq!(button_ability("special_button_03", &bar, &state), Some((vec![401, 402, 403], "HUNT".into())));
        assert_eq!(button_ability("special_button_04", &bar, &state), Some((vec![401, 402, 403], "SPOILER_LOCK".into())));
        assert_eq!(button_ability("special_button_05", &bar, &state), None);
        assert_eq!(button_ability("b_retreat", &bar, &state), None);
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

#[cfg(test)]
mod retreat_tests {
    use super::*;
    use battle_sim::{Battle, BattleConfig, BattleSetup, Catalog, ProjDef, ShipDef, SideSetup, TargetClass, UnitSetup};

    /// Two frigates of the player (side 0, no AI) against an idle station; the retreat is allowed at once and the jump comes 10 s after the order.
    fn battle() -> Battle {
        let runner = ShipDef { name: "runner".into(), class: TargetClass::Frigate, hull: 1000.0, max_speed: 100.0, accel: 100.0, turn_rate: 1.0, radius: 50.0, population: 1, ..ShipDef::default() };
        let idle = ShipDef { name: "idle".into(), class: TargetClass::Structure, is_static: true, hull: 1.0e6, radius: 100.0, ..ShipDef::default() };
        let cat = std::sync::Arc::new(Catalog::from_defs(vec![runner, idle], vec![ProjDef { name: "laser".into(), ..ProjDef::default() }]));
        let side = |n: &str, c: u32| SideSetup { name: n.into(), units: vec![UnitSetup { ship: cat.ship_id(n).unwrap(), count: c, tag: 1 }], structures: vec![], ai: false };
        let config = BattleConfig { retreat_countdown: 0.0, retreat_jump_seconds: 10.0, ..BattleConfig::default() };
        Battle::new(BattleSetup { sides: vec![side("runner", 2), side("idle", 1)], catalog: cat.clone(), config })
    }

    #[test]
    fn pressing_retreat_again_during_the_countdown_calls_it_off_and_the_button_holds_its_state() {
        // The real worker and snapshots, one step per frame: Retreat pressed after 1 s, pressed again 4 s later (EaW cancels the retreat).
        let sim = crate::sim::SimHandle::spawn(battle());
        let mut seen = Vec::new();
        let mut last = None;
        for tick in 0..30 * 25u64 {
            sim.request(tick + 1);
            let cur = sim.fetch(tick, true).0.expect("snapshot");
            let state = retreat_state(cur.over, cur.retreat_jump_in.first().copied().flatten(), cur.time, 0.0);
            if tick == 30 || tick == 150 {
                sim.command(0, retreat_press(state));
            }
            seen.push(state);
            last = Some(cur);
        }
        let countdown = |s: &RetreatState| matches!(s, RetreatState::Countdown(_));
        // The order reaches the next step: from then on the countdown shows on every frame until the second press.
        assert!(seen[..31].iter().all(|s| *s == RetreatState::Ready), "{:?}", &seen[..31]);
        assert!(seen[33..151].iter().all(countdown), "{:?}", &seen[33..151]);
        // Called off: ready again on every following frame, the countdown never comes back (no flicker between the two looks).
        assert!(seen[153..].iter().all(|s| *s == RetreatState::Ready), "{:?}", seen[153..].iter().find(|s| **s != RetreatState::Ready));
        let toggles = seen.windows(2).filter(|w| countdown(&w[0]) != countdown(&w[1])).count();
        assert_eq!(toggles, 2, "one change for the order, one for the cancel");
        // Well past the first jump time (11 s): both frigates are still in the battle.
        let last = last.unwrap();
        assert_eq!(last.alive.first().copied(), Some(2), "nobody jumped out");
        assert!(!last.over);
    }
}
