# galaxy-play status

Bevy plugin that makes the galactic map playable: `galaxy-sim` runs on a worker thread, fleets are drawn on the map,
the player selects and orders them, builds at owned planets, controls game speed and gets notifications and battle prompts.

## What works
- `driver.rs`: threaded `SimDriver` (commands, advance, battle resolution, snapshots), headless-testable.
- `GalaxyPlayPlugin::deferred()` + `start_play(world, PlayInit)` / `stop_play(world)`: the host starts a game when a campaign
  opens (the `space-peace` app does this on entering the map) and stops it when leaving. `GalaxyPlayPlugin::new(init)` still
  starts one at plugin build (used by the demo example).
- Fleets on the map (generic fleet model or flagship), orbit slots, lane travel, selection, box select, drag-and-drop and
  right-click move orders with route/ETA hint, merge/split/retreat, blockade-aware route planning.
- A new fleet model spawns hidden and shows once `fit_fleet_models` has scaled it (2026-10-17): alamo-render spawns the parts in
  PostUpdate, after the fit, and a hero Star Destroyer at its own size flashed the whole map for a frame (user report on recruiting
  Thrawn). Test: `visuals::tests::a_new_fleet_model_never_shows_at_its_own_size`.
- Carrying (carry.rs): the dragged fleet's model centre sits on this frame's cursor ray, 2.2 pads above the map or nearer the camera in
  front of a planet globe the ray would enter, so it hangs exactly under the pointer at every zoom and tilt with no follow lag and is
  not hidden by the planet it is held over; other carried fleets hang beside it along the screen's horizontal, and the white rings and
  the tether end on the map point under the cursor. A carried fleet is 1.5x its size at every zoom (2026-10-17, G4: the zoom-dependent
  1.5-9x scale of 28d1013 / 60903b2 is gone, nothing in the world changes size with the zoom; with the lift above the pad it put the
  model 31-59 px off the cursor).
- Fleet pads as drop targets (2026-10-17, G5): while fleets are carried over an own or allied planet (`input::pads_open`), its three
  pads are outlined and the one under the cursor is lit (white and green band, pulsing like a selection rim; the carried fleet's white
  rings give way to it). A drop there sends `PlayAction::MoveToSlot`, so the fleet takes that slot at once at its own planet (group 1 to
  group 3) or on arrival elsewhere, and merges into the fleet that holds it. The pad wins over a parked fleet whose pick area reaches
  over it (zoomed out the pads are a few pixels apart). The route hint names the slot ("fleet slot 2", "merge into fleet slot 1").
  Enemy and neutral planets have no pad targets: a drop sends the fleet there (`MoveSelected`). Zoomed out a pad is a pixel or two, so
  there the hint is what tells the slot.
- `BuildMenu` (units, structures, star base, tech, research) with availability reasons; `PlayAction::Build` queues it, credits go
  through galaxy-sim. Production queue model.
- `GameClock`: pause + speed levels, `hold` flag for the host (pause menu, battle screen); time is held while a human battle waits.
- Notifications (captured/lost planets, battle pending/resolved, construction, income, refused orders).
- `PlayUiModel` (plain data) is the UI seam: `GalaxyPlayUiPlugin` is a simple built-in UI; the app uses `eaw-ui` instead.
- `PlayCursorBlocked` lets the host disable mouse handling (over its UI, menus, other screens). `PlayCursorOverUi` tells the plugin that
  the block only comes from the host's interface under the cursor: a fleet drag in progress then keeps going (no target, no order on a
  release there); only a menu / other screen / loading galaxy, Esc or a right click end it early (Esc / right click give no order).
- Battles: with `PlayConfig::tactical_hook` "Fight" asks the worker for the tactical setup (`PlayEvent::TacticalReady(TacticalRequest)`); the host plays it and answers `PlayAction::ApplyTactical` (result applied by galaxy-sim) or `PlayAction::CancelTactical` (auto-resolve).
  `PlayAction::ResolveBattle` (currently always auto-resolved).

## Run
`cargo run -p galaxy-play --example galaxy_play_demo -- --offscreen out.png [--weeks N] [--human] ...` (no window).
Integration test path: `space-peace --offscreen out.png --campaign GC_Full_Web_Way_Rebel --select "The Yavin System" --weeks 8 --build Squadron --move Hoth`.

## Gaps
- No order to cancel a queue entry (no galaxy-sim command).
- Fleet picking/dragging was only verified in offscreen scenarios and unit tests, never with a real mouse.
- Orders to fleets in hyperspace are refused (retreat first).

## Scripted backend, difficulty, save/load (new)
The worker can run `galaxy_script::ScriptedSim` (Remake GC story scripts, Remake AI + strategy, difficulty, snapshot save/load) instead of plain galaxy-sim. Saves are state snapshots (load time does not depend on game length, see galaxy-script STATUS);
the journaled methods, so a save is possible at any moment. `PlayInit::from_data` / `SimDriver::spawn` still give the old plain backend (no scripts, no save).
- `PlayConfig::difficulty: Option<DifficultyTier>` (default `Some(Normal)`) and `PlayConfig::classic_ai: bool` (default false = Remake AI; true = galaxy-sim's built-in AI, no tiers).
- `PlayInit::scripted(data: Arc<GameData>, fs: Arc<dyn FileSource>, &campaign, config)`: player = the campaign's active player, every other faction AI (`human_is_ai` = all AI).
- `PlayInit::from_save(path, data, fs, config)`: starts a game by replaying a save (campaign, seed, difficulty, AI choice come from the file).
- `PlayAction::Save(path)` -> `PlayEvent::SaveFinished { path, result }`; `PlayAction::Load(path)` -> `PlayEvent::LoadFinished { path, result: Ok(week) | Err(reason) }` (same campaign and player only;
  otherwise restart with `from_save`). Both also put a ticker notification. `PlayEvent::Script(ScriptEvent)` forwards story dialogs/messages of the scripts.
- Placeholder: with a scripted init `start_play` returns at once; `SimState::current.loading` is true (starting galaxy) until the worker finished the setup (about 1 s) or the replay of a save.
  The game clock holds while `loading` or while a replay runs. `SimState::driver.load_progress() -> Option<LoadProgressView { done, total, week }>` (`fraction()` 0..1) feeds a loading bar
  (also during an in-game `Load`); `None` when no replay runs.
- Cost (tests/scripted.rs, release, 12 factions): 20 game weeks at 60 fps and the fastest speed (16x) take ~210 ms of worker time per game week (AI, scripts, sim, snapshot clone), the worst single batch was 41 ms,
  the worst `poll()` on the render thread 27 us. The worker is a separate thread and `drive_clock` already limits the backlog (3 advances in flight), so the AI cannot stall rendering; at 16x the worker uses ~6% of a core.
  Replay (load) speed: ~0.2 s per game week plus ~1 s setup; a 200-week save takes ~40 s: show the loading bar. No keyframes yet (a state-hash checkpoint every N weeks would only verify, not speed up).
- Tests: `tests/scripted.rs` (render-loop responsiveness, save -> continue -> load -> identical continuation, `from_save` with monotone progress, classic-AI save/load, plain backend refusing to save).

### Steps for the app (integrator, `space-peace`)
1. Settings/menu data: add to the campaign start screen a difficulty selector (Easy / Normal / Hard from the mod, plus "Brutal" and "Fair" labelled as custom) and a checkbox "Classic AI"
   (disables the selector). Store them in `PlayConfig { difficulty, classic_ai, .. }`.
2. New game: `let fs: Arc<dyn FileSource> = game_fs.clone(); let data: Arc<GameData> = ...; start_play(world, PlayInit::scripted(data, fs, &campaign, cfg)?)` (replaces `PlayInit::from_data`).
   Keep `cfg.pick_planets = false` as before. Show a "Preparing galaxy..." overlay while `SimState.current.loading`.
3. Save: a button/hotkey sends `PlayAction::Save(path)`; slots live in e.g. `%APPDATA%\SpacePeace\saves\<slot>.json`. Read `PlayEvent::SaveFinished` for success/failure.
   A slot list can use `galaxy_script::SaveFile::read(path)` (cheap): `checkpoint.week`, `checkpoint.factions`, `header.campaign`, `header.config.difficulty`, `header.config.remake_ai`.
4. Load from the main menu: `start_play(world, PlayInit::from_save(path, data, fs, PlayConfig::default())?)`, show a loading bar from `SimState.driver.load_progress()` (fraction + week) while it is `Some`
   or `current.loading`. Load from inside a game of the same campaign/player: `PlayAction::Load(path)` with the same bar, then handle `PlayEvent::LoadFinished` (on `Err` show the text).
5. Battles keep working as before (`tactical_hook`, `TacticalReady`, `ApplyTactical`); a save made while a battle is pending restores it (the tactical set-up is rebuilt from the state). For tactical resolution of AI battles
   the catalog/arenas are runtime-only (see `galaxy-script/STATUS.md`); the worker uses the statistical resolver unless a catalog is installed through `ScriptedSim::with_sim_runtime`.
6. Do not call `Sim` mutators from the app thread; everything goes through `PlayAction`.

## Soak test (example `soak` + ignored tests)
`cargo run --release -p galaxy-play --example soak -- --campaign GC_Full_Web_Way_Imperial --human Empire --seed 2 --weeks 120 --tier brutal [--battles tactical] [--ai-tactical [--ai-tactical-max N]] [--classic] [--twice]`
plays a scripted game headless with a simple player bot (builds, merges, attacks weak neighbours) and checks every week: credits negative/exploding, script errors (`Snapshot::diagnostics`), absurd fleet sizes,
fleets frozen in hyperspace, stalled game time, a dead worker; every 25 weeks save size per journal step and process memory; at the end factions that never acted and (with `--twice`) determinism. Exit code 1 on findings.
`cargo test --release -p galaxy-play --test soak -- --ignored` runs 2 x 100 weeks (determinism) and a small classic-AI campaign.
Matrix run on 2026-10 (all clean after the fixes below): Full GC Rebel/Normal 100 weeks, Full GC Empire/Brutal 120, Mini Rebel/Normal 120, Standard Empire/Hard 120 with tactical player battles, Standard Underworld classic AI 100,
Mini Empire/Fair 60 twice (identical final state), Full GC Rebel 110 with tactical player battles and AI battles. 150-210 ms worker time per game week (worst batch 260 ms), memory flat at 340-410 MB, journal ~7 bytes per step.
Findings fixed: Lua VM leak (below), tech never advanced, AI task force bookkeeping leaks. Open: the AI hoards credits (Empire 2.4M at week 100 with population far below the cap),
classic AI snowballs (879 captures in 100 weeks), `BattleResolver::Tactical` for AI battles costs ~0.7 s per battle (12 s per game week with 18 battles) so it is only for the player's battles
(`SimConfig::tactical_resolver_max_ships`, default 16, limits it).
