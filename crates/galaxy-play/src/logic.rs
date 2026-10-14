//! Game logic systems: snapshot pump, game clock, player actions and the UI model.

use std::collections::BTreeMap;

use bevy::prelude::*;
use galaxy_sim::{Command, FleetId, FleetLocation, PlanetId};

use crate::build_menu::{BuildMenu, build_command};
use crate::catalog::PlayCatalog;
use crate::driver::ResolveMode;
use crate::geometry::{eta_seconds, flagship, split_half};
use crate::notify::{NotificationKind, Notifications, describe_error, describe_event};
use crate::state::*;

/// True while a battle involving the player waits for a decision that the plugin owns.
pub fn battle_holds_game(state: &SimState) -> bool {
    !state.current.human_battles.is_empty()
}

/// Tracks fleets ordered to merge into another fleet upon arrival.
#[derive(Resource, Default, Debug)]
pub struct PendingFleetMerges(pub std::collections::HashMap<FleetId, FleetId>);

/// Receives snapshots from the worker, publishes events and notifications.
pub fn pump_snapshots(
    mut state: ResMut<SimState>,
    time: Res<Time>,
    catalog: Res<PlayCatalog>,
    player: Res<PlayerFaction>,
    mut notes: ResMut<Notifications>,
    mut out: MessageWriter<PlayEvent>,
    cfg: Res<PlayConfig>,
    mut pending_merges: Option<ResMut<PendingFleetMerges>>,
) {
    let now = time.elapsed_secs_f64();
    notes.expire(now, cfg.notification_seconds);
    let snaps = state.driver.poll();
    let Some(last) = snaps.last().cloned() else { return };
    for s in &snaps {
        for e in &s.script_events {
            // A hero coming back (the Remake's respawn timers) is news for its owner.
            if let galaxy_script::ScriptEvent::UnitSpawned { unit, planet, faction } = e
                && player.0.is_some_and(|h| state.world.factions[h as usize].name.eq_ignore_ascii_case(faction))
            {
                notes.push(NotificationKind::Campaign, format!("{} has returned at {}.", crate::catalog::prettify(unit), planet), None, now);
            }
            out.write(PlayEvent::Script(e.clone()));
        }
        for r in &s.saves {
            let text = match &r.result {
                Ok(()) => format!("Game saved to {}", r.path.display()),
                Err(e) => format!("Saving failed: {e}"),
            };
            notes.push(if r.result.is_ok() { NotificationKind::Campaign } else { NotificationKind::Error }, text, None, now);
            out.write(PlayEvent::SaveFinished { path: r.path.clone(), result: r.result.clone() });
        }
        for r in &s.loads {
            let text = match &r.result {
                Ok(w) => format!("Game loaded (week {w})"),
                Err(e) => format!("Loading failed: {e}"),
            };
            notes.push(if r.result.is_ok() { NotificationKind::Campaign } else { NotificationKind::Error }, text, None, now);
            out.write(PlayEvent::LoadFinished { path: r.path.clone(), result: r.result.clone() });
        }
        for r in &s.results {
            if let Err(e) = &r.result {
                notes.push(NotificationKind::Error, describe_error(e), None, now);
                out.write(PlayEvent::CommandRejected { command: r.command.clone(), error: e.clone() });
            }
        }
        for ev in &s.events {
            if let galaxy_sim::Event::FleetArrived { fleet, planet } = ev {
                if let Some(ref mut pm) = pending_merges {
                    if let Some(into) = pm.0.remove(fleet) {
                        if let Some(h) = player.0 {
                            let into_fleet = s.galaxy.fleets.get(&into);
                            if into_fleet.is_some_and(|f| f.owner == h && f.planet() == Some(*planet)) {
                                state.driver.send_command(Command::MergeFleets { faction: h, into, from: *fleet });
                            }
                        }
                    }
                }
            }
            if let Some((kind, text, planet)) = describe_event(&state.world, &catalog, &s.galaxy, player.0, ev) {
                notes.push(kind, text, planet, now);
            }
            out.write(PlayEvent::Sim(ev.clone()));
        }
    }
    state.previous = Some(if snaps.len() >= 2 { snaps[snaps.len() - 2].clone() } else { state.current.clone() });
    state.current = last;
    state.received_at = now;
    for s in &snaps {
        for r in &s.tactical {
            out.write(PlayEvent::TacticalReady(Box::new(r.clone())));
        }
    }
    let alive: Vec<u32> = state.current.human_battles.iter().map(|b| b.id).collect();
    state.awaiting_external.retain(|id| alive.contains(id));
}

/// Sends elapsed game time to the worker, limited by how far it lags behind.
pub fn drive_clock(time: Res<Time>, mut clock: ResMut<GameClock>, mut state: ResMut<SimState>) {
    let held = battle_holds_game(&state) || state.current.loading || state.driver.load_progress().is_some();
    let rate = if held || state.current.galaxy.winner.is_some() { 0.0 } else { clock.rate() };
    let dt = time.delta_secs().min(0.25);
    clock.pending += dt * rate;
    if rate <= 0.0 {
        clock.pending = 0.0;
        return;
    }
    // At most one second of backlog (a stalled worker must not queue up an avalanche).
    clock.pending = clock.pending.min(rate.max(1.0));
    if clock.pending > 0.0 && state.driver.advances_in_flight(&state.current) < 3 {
        let secs = std::mem::take(&mut clock.pending);
        state.driver.advance(secs);
    }
}

/// Executes [`PlayAction`]s.
#[allow(clippy::too_many_arguments)]
pub fn handle_actions(
    mut actions: MessageReader<PlayAction>,
    mut state: ResMut<SimState>,
    mut clock: ResMut<GameClock>,
    mut selection: ResMut<PlayerSelection>,
    mut notes: ResMut<Notifications>,
    player: Res<PlayerFaction>,
    cfg: Res<PlayConfig>,
    time: Res<Time>,
    mut pending_merges: Option<ResMut<PendingFleetMerges>>,
) {
    let now = time.elapsed_secs_f64();
    for action in actions.read() {
        match action {
            PlayAction::SetSpeedLevel(l) => {
                clock.override_multiplier = None;
                clock.level = (*l).min(clock.speeds.len());
                if clock.level > 0 {
                    clock.resume_level = clock.level;
                }
            }
            PlayAction::SetSpeedMultiplier(m) => clock.override_multiplier = Some(m.max(0.0)),
            PlayAction::TogglePause => {
                if clock.paused() {
                    clock.override_multiplier = None;
                    clock.level = clock.resume_level;
                } else {
                    clock.override_multiplier = None;
                    clock.resume_level = clock.level.max(1);
                    clock.level = 0;
                }
            }
            PlayAction::CancelProduction { planet, lane, index } => {
                if let Some(h) = player.0 {
                    state.driver.send_command(Command::CancelProduction { faction: h, planet: *planet, lane: *lane, index: *index });
                }
            }
            PlayAction::DebugGrant { unit_name, planet, count } => {
                if let (Some(h), Some(unit)) = (player.0, state.world.unit_id(unit_name)) {
                    state.driver.send_command(Command::DebugGrant { faction: h, planet: *planet, unit, count: *count });
                } else {
                    warn!("debug grant: unknown unit name `{unit_name}` (or no player faction); nothing granted");
                }
            }
            PlayAction::SellStructure { planet, structure } => {
                if let Some(h) = player.0 {
                    state.driver.send_command(Command::SellStructure { faction: h, planet: *planet, structure: *structure });
                }
            }
            PlayAction::Cheat(cheat) => {
                if let Some(h) = player.0 {
                    state.driver.send_command(Command::Cheat { faction: h, cheat: cheat.clone() });
                }
            }
            PlayAction::DestroyPlanet(planet) => {
                if let Some(h) = player.0 {
                    state.driver.send_command(Command::DestroyPlanet { faction: h, planet: *planet });
                }
            }
            PlayAction::Save(path) => state.driver.save(path.clone()),
            PlayAction::Load(path) => state.driver.load(path.clone()),
            PlayAction::SelectPlanet(p) => selection.planet = *p,
            PlayAction::SelectFleets(f) => selection.fleets = f.iter().copied().collect(),
            PlayAction::DismissNotification(id) => notes.list.retain(|n| n.id != *id),
            PlayAction::Build { planet, target } => {
                if let Some(h) = player.0 {
                    state.driver.send_command(build_command(h, *planet, *target));
                }
            }
            PlayAction::MoveSelected(to) => {
                let Some(h) = player.0 else { continue };
                let mut sent = 0;
                for &fid in &selection.fleets {
                    let Some(f) = state.current.galaxy.fleets.get(&fid).filter(|f| f.owner == h) else { continue };
                    match f.location {
                        FleetLocation::AtPlanet(p) if p == *to => {}
                        FleetLocation::AtPlanet(_) => {
                            state.driver.send_command(Command::MoveFleet { faction: h, fleet: fid, to: *to });
                            sent += 1;
                        }
                        FleetLocation::Moving { .. } => {
                            notes.push(NotificationKind::Error, "Fleets in hyperspace cannot be redirected; order a retreat first.".into(), None, now);
                        }
                    }
                }
                let _ = sent;
            }
            PlayAction::MergeFleets { into, from } => {
                let Some(h) = player.0 else { continue };
                state.driver.send_command(Command::MergeFleets { faction: h, into: *into, from: *from });
                selection.fleets.remove(from);
                selection.fleets.insert(*into);
            }
            PlayAction::MoveAndMerge { fleets, into, to } => {
                let Some(h) = player.0 else { continue };
                for &fid in fleets {
                    let Some(f) = state.current.galaxy.fleets.get(&fid).filter(|f| f.owner == h) else { continue };
                    match f.location {
                        FleetLocation::AtPlanet(p) if p == *to => {
                            if fid != *into {
                                state.driver.send_command(Command::MergeFleets { faction: h, into: *into, from: fid });
                                selection.fleets.remove(&fid);
                                selection.fleets.insert(*into);
                            }
                        }
                        FleetLocation::AtPlanet(_) => {
                            if let Some(ref mut pm) = pending_merges {
                                pm.0.insert(fid, *into);
                            }
                            state.driver.send_command(Command::MoveFleet { faction: h, fleet: fid, to: *to });
                        }
                        FleetLocation::Moving { .. } => {
                            notes.push(NotificationKind::Error, "Fleets in hyperspace cannot be redirected; order a retreat first.".into(), None, now);
                        }
                    }
                }
            }
            PlayAction::MergeSelected => {
                let Some(h) = player.0 else { continue };
                let mut by_planet: BTreeMap<PlanetId, Vec<FleetId>> = BTreeMap::new();
                for &fid in &selection.fleets {
                    if let Some(f) = state.current.galaxy.fleets.get(&fid).filter(|f| f.owner == h)
                        && let FleetLocation::AtPlanet(p) = f.location
                    {
                        by_planet.entry(p).or_default().push(fid);
                    }
                }
                for ids in by_planet.values().filter(|v| v.len() > 1) {
                    for &from in &ids[1..] {
                        state.driver.send_command(Command::MergeFleets { faction: h, into: ids[0], from });
                        selection.fleets.remove(&from);
                    }
                }
            }
            PlayAction::SplitSelected => {
                let Some(h) = player.0 else { continue };
                if selection.fleets.len() != 1 {
                    continue;
                }
                let fid = *selection.fleets.iter().next().expect("one selected");
                if let Some(f) = state.current.galaxy.fleets.get(&fid).filter(|f| f.owner == h) {
                    let units = split_half(f);
                    if !units.is_empty() {
                        state.driver.send_command(Command::SplitFleet { faction: h, fleet: fid, units });
                    }
                }
            }
            PlayAction::RetreatSelected => {
                let Some(h) = player.0 else { continue };
                for &fid in &selection.fleets {
                    if state.current.galaxy.fleets.get(&fid).is_some_and(|f| f.owner == h) {
                        state.driver.send_command(Command::Retreat { faction: h, fleet: fid });
                    }
                }
            }
            PlayAction::ApplyTactical { id, result } => state.driver.apply_tactical(*id, (**result).clone()),
            PlayAction::AbandonTactical { id, partial } => state.driver.abandon_tactical(*id, (**partial).clone()),
            PlayAction::CancelTactical(id) => state.driver.cancel_tactical(*id),
            PlayAction::ResolveBattle { id, choice } => {
                let Some(b) = state.current.human_battles.iter().find(|b| b.id == *id).cloned() else { continue };
                match choice {
                    BattleChoice::Fight if cfg.tactical_hook && !state.awaiting_external.contains(id) => {
                        state.awaiting_external.insert(*id);
                        state.driver.tactical_setup(b.id);
                    }
                    BattleChoice::Fight if !cfg.tactical_hook => state.driver.resolve_battle(*id, ResolveMode::Headless),
                    // Host finished (or declined) the tactical fight and reports back.
                    BattleChoice::Fight | BattleChoice::AutoResolve => state.driver.resolve_battle(*id, ResolveMode::Auto),
                }
            }
        }
    }
}

/// Drops fleets from the selection that no longer exist or changed hands.
pub fn prune_selection(state: Res<SimState>, player: Res<PlayerFaction>, mut selection: ResMut<PlayerSelection>) {
    if !state.is_changed() {
        return;
    }
    let g = state.galaxy();
    let before = selection.fleets.len();
    let mine = |fid: &FleetId| g.fleets.get(fid).is_some_and(|f| player.0 == Some(f.owner));
    let keep: Vec<FleetId> = selection.fleets.iter().copied().filter(mine).collect();
    if keep.len() != before {
        selection.fleets = keep.into_iter().collect();
    }
}

/// Rebuilds [`PlayUiModel`] a few times per second and whenever the selection changes.
#[allow(clippy::too_many_arguments)]
pub fn update_ui_model(
    state: Res<SimState>,
    clock: Res<GameClock>,
    selection: Res<PlayerSelection>,
    catalog: Res<PlayCatalog>,
    player: Res<PlayerFaction>,
    notes: Res<Notifications>,
    cfg: Res<PlayConfig>,
    time: Res<Time>,
    mut model: ResMut<PlayUiModel>,
    mut since: Local<f32>,
    mut last_sel: Local<PlayerSelection>,
    mut last_snapshot: Local<u64>,
) {
    *since += time.delta_secs();
    let dirty = selection.is_changed() && *last_sel != *selection;
    let clock_dirty = clock.is_changed();
    if !dirty && !clock_dirty && (*since < cfg.ui_interval || *last_snapshot == state.current.seq) && model.version != 0 {
        // Keep ticker and speed fresh without recomputing the menus.
        if notes.is_changed() && model.notifications != notes.list {
            model.notifications = notes.list.clone();
        }
        return;
    }
    *since = 0.0;
    *last_sel = selection.clone();
    *last_snapshot = state.current.seq;

    let world = &state.world;
    let g = state.galaxy();
    let human = player.0;
    let mut next = PlayUiModel { version: model.version + 1, route_hint: model.route_hint.clone(), ..default() };

    if let Some(h) = human {
        let fs = &g.factions[h as usize];
        next.top = TopBarModel {
            faction: catalog.faction(h).to_owned(),
            color: world.factions[h as usize].color.map(|c| c as f32 / 255.0)[..3].try_into().unwrap_or([1.0; 3]),
            credits: fs.credits,
            income_per_week: fs.income_per_week,
            population_used: fs.population_used,
            population_cap: fs.population_cap,
            planets: fs.planets_owned,
            week: state.current.week,
            time: state.current.time,
            winner: g.winner.map(|w| catalog.faction(w).to_owned()),
        };
    } else {
        next.top = TopBarModel { faction: "Spectator".into(), week: state.current.week, time: state.current.time, winner: g.winner.map(|w| catalog.faction(w).to_owned()), ..default() };
    }
    if let Some(h) = human {
        let mut heroes: std::collections::BTreeSet<u32> = Default::default();
        for f in g.fleets.values().filter(|f| f.owner == h) {
            heroes.extend(f.units.keys().copied().filter(|&u| world.units[u as usize].hero));
        }
        next.heroes = heroes.into_iter().map(|u| world.units[u as usize].name.clone()).collect();
    }
    next.speed = SpeedModel { level: clock.level, speeds: clock.speeds.clone(), rate: clock.rate(), held_by_battle: !state.current.human_battles.is_empty() };

    if let Some(p) = selection.planet {
        match human.and_then(|h| BuildMenu::for_planet(world, g, &catalog, h, p)) {
            Some(menu) => next.planet = Some(menu),
            None => {
                let ps = &g.planets[p as usize];
                next.other_planet = Some((world.planets[p as usize].display_name.clone(), ps.owner.map(|o| catalog.faction(o).to_owned())));
            }
        }
    }

    for &fid in &selection.fleets {
        let Some(f) = g.fleets.get(&fid) else { continue };
        let name = |p: PlanetId| world.planets[p as usize].display_name.clone();
        let (at, heading) = match &f.location {
            FleetLocation::AtPlanet(p) => (Some(name(*p)), None),
            FleetLocation::Moving { to, rest, .. } => {
                let dest = rest.last().copied().unwrap_or(*to);
                (None, Some((name(dest), eta_seconds(world, f).unwrap_or(0.0))))
            }
        };
        next.fleets.push(FleetRow {
            id: fid,
            owner: f.owner,
            mine: human == Some(f.owner),
            flagship: flagship(world, f).map_or_else(String::new, |u| catalog.unit(u).to_owned()),
            ships: f.units.iter().map(|(&u, &c)| (catalog.unit(u).to_owned(), c)).collect(),
            count: f.unit_count(),
            at,
            heading,
        });
    }

    next.notifications = notes.list.clone();
    if let Some(b) = state.current.human_battles.iter().find(|b| !state.awaiting_external.contains(&b.id)) {
        let sides = b
            .factions
            .iter()
            .map(|&fac| BattleSide {
                faction: catalog.faction(fac).to_owned(),
                color: [world.factions[fac as usize].color[0] as f32 / 255.0, world.factions[fac as usize].color[1] as f32 / 255.0, world.factions[fac as usize].color[2] as f32 / 255.0],
                ships: b.fleets.iter().filter_map(|id| g.fleets.get(id)).filter(|f| f.owner == fac).map(|f| f.unit_count()).sum(),
                flagship: b
                    .fleets
                    .iter()
                    .filter_map(|id| g.fleets.get(id))
                    .filter(|f| f.owner == fac)
                    .max_by_key(|f| f.unit_count())
                    .and_then(|f| flagship(world, f))
                    .map(|u| world.units[u as usize].name.clone())
                    // A defender without a fleet is represented by the star base of the planet.
                    .or_else(|| (b.defender == Some(fac)).then(|| g.planets[b.planet as usize].starbase).flatten().map(|sb| world.starbases[sb as usize].name.clone()))
                    .unwrap_or_default(),
                defender: b.defender == Some(fac),
                mine: human == Some(fac),
            })
            .collect();
        next.battle = Some(BattleDialog { id: b.id, planet: b.planet, planet_name: world.planets[b.planet as usize].display_name.clone(), sides });
    }
    *model = next;
}

/// Space = pause, 1..9 = speed levels, M/S/R = merge/split/retreat.
pub fn hotkeys(keys: Res<ButtonInput<KeyCode>>, hidden: Res<crate::PlayMapHidden>, mut actions: MessageWriter<PlayAction>) {
    if hidden.0 {
        return;
    }
    if keys.just_pressed(KeyCode::Space) {
        actions.write(PlayAction::TogglePause);
    }
    for (i, k) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5].into_iter().enumerate() {
        if keys.just_pressed(k) {
            actions.write(PlayAction::SetSpeedLevel(i + 1));
        }
    }
    if keys.just_pressed(KeyCode::KeyM) {
        actions.write(PlayAction::MergeSelected);
    }
    if keys.just_pressed(KeyCode::KeyV) {
        actions.write(PlayAction::SplitSelected);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        actions.write(PlayAction::RetreatSelected);
    }
}

