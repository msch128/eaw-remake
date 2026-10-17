# space-peace (app crate) - status

Bevy 0.19.1 shell for "Space Peace: Not at War": Galactic Conquest only (no story, no movies).

## What works
- Screens (`flow.rs`, `AppScreen`): Loading -> MainMenu -> CampaignSelect -> Galaxy <-> SpaceBattle (seam).
- Loading (`loading.rs`): game data parsed on a worker thread, galaxy backdrop + hidden planet-model warm-up, progress bar.
- Menus (`menu.rs`, `options.rs`): main menu (Galactic Conquest / Options / Quit), campaign list grouped by size with
  Rebel/Empire/Underworld, pause menu on Esc, options overlay. UI is built from plain `MenuSpec` data via `spawn_menu`
  (single replacement point for the future `eaw-ui` look).
- Settings (`settings.rs`): window mode, resolution, vsync, FPS cap, UI scale, volumes, edge scroll, camera speed;
  stored in `%APPDATA%\SpacePeace\settings.toml`; CLI flags override per run.
- Galaxy map (`galaxy_map.rs`, `planet_visual.rs`, `camera.rs`, `ui.rs`): real `.alo` planets via `alamo-render`,
  galaxy disc/stars from `w_galaxy00`, route lines from TRADEROUTELINES.XML, declutter labels, owner rings, selection,
  info panel (ability, structure slots, starting forces), faction bar, tooltip, click/hover SFX.
- Audio: `alamo-audio` (music director: MainMenu / Galactic(faction), GUI SFX). UI sends `UiSound`, played only when audio is active.
- Game data mapping (`game.rs`): campaigns, planets (dummies filtered), exact ownership, routes, props, light.
- Offscreen testing: `--offscreen out.png` renders 3840x2160 headless (no window, no audio) and logs frame times (~4 ms/frame).

## CLI flags
`--game-dir`, `--mod <id>`, `--campaign <name>`, `--list-campaigns`, `--demo`, `--volume`, `--no-vsync`, `--fullscreen`,
`--offscreen <png> [--frames N] [--screen menu|campaigns|galaxy|pause|options|loading] [--select <planet>] [--zoom 0..1]`,
`--loading-delay <s>`. No flags: start on the menu; `--campaign`/`--demo` skip to the map.
`--burst <n>` (offscreen, 2026-10-17): also saves the n frames from the scripted orders on (`--grant-unit`, `--build`, `--move`,
`--weeks`) as `<out>-000.png` ..., for single-frame flicker. Example (the hero-recruit flash, G7): `--campaign
GC_Full_Web_Way_Imperial --select "The Coruscant System" --zoom 0.09 --grant-unit Strikefast --at "The Coruscant System" --burst 40`.
A `--build` that matches nothing lists what the planet can build.

## New galaxy models stay hidden until fitted (2026-10-17, G7)
User: the galaxy map flashes white for a moment when a hero is recruited (example Thrawn). alamo-render spawns a model's parts in
PostUpdate, after the Update systems that scale fleet and station models to their icon size, so a new model showed at its own
size for one frame: Thrawn's Strikefast (radius about 800) filled the whole map with grey hull (burst frame 2: 827,000 of 2 million
pixels changed for one frame). Fleet models (galaxy-play `spawn_model` / `fit_fleet_models`) and station models (`station_model` /
`fit_stations`) now spawn hidden and show once fitted; the same Thrawn burst changes 109 pixels in that frame. Tests run the fit
over several frames with a stand-in that finishes the model in PostUpdate: `a_new_fleet_model_never_shows_at_its_own_size`
(galaxy-play, also shows the old spawn flashing), `a_new_station_model_never_shows_at_its_own_size`.

## SpaceBattle seam (`battle_seam.rs`, plan in the module doc)
galaxy-play `PlayEvent::OpenTacticalBattle(PendingBattle)` -> `OpenBattle` -> `AppScreen::SpaceBattle` (galaxy scene kept hidden) ->
host builds `battle_sim::BattleSetup` + `BattleSession` for battle-view -> `BattleResult` -> `BattleFinished` -> back to Galaxy.
F9 on the map opens the placeholder screen. Needed: galaxy-sim API to apply a tactical `BattleResult` to a pending battle
(today only headless `resolve_battle_tactical` / AutoResolve); battle-view must let the galaxy camera be switched off.

## Playable galaxy (play.rs, play_ui.rs)
- Entering the map with a real campaign builds a `galaxy-sim` game (~0.5 s) and starts `galaxy-play` (`PlayGluePlugin`); leaving to the menus stops it.
- Map selection feeds the sim; owners and credits flow back into `GalaxyView` (rings, labels, faction bar). Real planet radii go to the fleet layout.
- Original `eaw-ui` galactic command bar (faction skin): credits, pop, tech, day, planet name/owner/income, production queue with icons, build pads (space tab = ships/star base/tech, planet tab = structures), pause and fast-forward buttons. Queue entries cannot be cancelled (no sim command). The minimap shows planets by owner, fleets and the camera rectangle; click or drag pans.
- HUD: notification ticker, speed/paused/battle-pending read-out, selected fleet list, route hint. Planet info panel moves to the right.
- Battle: a pending battle with the player opens an `eaw-ui` dialog (Fight / Auto-resolve). Fight -> sim worker `tactical_setup` -> `PlayEvent::TacticalReady` -> `PendingTactical` + SpaceBattle screen (`battle_play.rs`): galaxy camera, sky dome, sun, HUD and fleets are suspended, `BattleSession::from_request` runs `battle-view` with the `eaw-ui` tactical bar; `battle_view::BattleFinished` -> `PlayAction::ApplyTactical` (events reach the ticker) -> map restored (command bar respawned: the tactical UI removes every bar). F10 abandons the fight (`CancelTactical`: sim auto-resolve). Auto-resolve -> sim auto-resolve.
- Retreat question (2026-10-17): the battle's Retreat button and the Esc menu's Retreat open a banner in the look of the battle-start banner (`battle_view::retreat_prompt`, title "<PLANET>: RETREAT FROM BATTLE", buttons from `TEXT_BUTTON_RETREAT` / `TEXT_BUTTON_AUTORESOLVE` / `TEXT_BUTTON_CANCEL`). Retreat orders the jump countdown, Auto-Resolve gives the fight up like F10 (the sim auto-resolves from the units that are left, with their hull and shield), Cancel or Esc closes it; the battle clock stands still while it is up. `SP_TEST_RETREAT` still orders the retreat directly.
- UI scale: eaw-ui owns `UiScale` (height/768 x settings scale) so bar and HUD match; labels compensate.
- Battle scenarios: `--battle-choice fight|auto`, `--battle-auto` (AI plays both sides), `--battle-speed X`, `--battle-seconds S` (mid-battle capture `<out>_battle.png`, camera framed on all ships); the final capture waits for the return to the map.
- Offscreen scenarios: `--battle-choice fight|auto` clicks the battle dialog button when it opens (verifies the seam headless). `--select P --weeks N --build TEXT --move PLANET` (orders at the selected planet, then N weeks, stops early on a pending battle; capture waits for it).

## Gaps
- Real window input (clicks on pads, fleet drag, Esc) never run live; only offscreen renders and unit tests.
- Pad and queue tooltips show name, cost, build time and the unavailability reason (`BuildPadEntry::tooltip`, eaw-ui).
- Menu/HUD look is a plain placeholder; live window input (clicks, Esc, options) only checked via unit tests/offscreen renders.
- alamo-render: emissive-only materials render flat (Stars dome patched locally), no `Nebula.fx` shader, black-hole TGA type 78 textures fail.
- Starting-force names are prettified ids (no object text ids in alamo-data); Camera_Shift_*/Camera_Distance unused.

## Next steps
Connect the real battle-view hook and galaxy-sim result injection, switch menus to `eaw-ui`, run the app in a real window (input, settings apply,
fullscreen, FPS cap).

## Galaxy map visuals (2026-10-08)
- Lanes (2026-10-17, measured on the user's Remake video v0520/v0540 and CS9 f0007/f0016 at full HD): one additive ribbon per route
  from planet centre to planet centre (no overshoot), flat band, world width = data `Width` x 0.14 map units (main 0.84, inter 0.35,
  capillary 0.07; fixed world size, thinner on screen when zoomed out), the data colour (teal 13, 31, 38 for every relation; enemy
  alpha fades it close up) x 3.6 in linear light (sRGB about 32, 62, 76 on black), a glint 1.6 times as bright runs along every lane
  (scrolling texture, replaces the red/green gizmo pulses). Orbit-line art (`Region_Lines`, the thin double curves) at 0.4 so it
  stays fainter than the lanes like in the Remake; grid off. galaxy_map.rs spawn_lanes/update_lanes.
- Lanes add in display (sRGB) values (2026-10-17, G1b), like the original's gamma-space blend: `LaneMaterial` (lane.wgsl) reads what
  lies under the ribbon from Bevy's view transmission texture and writes linear(srgb(under) + lane). The disc art (core glow, orbit
  lines) draws before that copy with alamo-render's `FLAG_UNDERLAY`. Over the core nebula next to Byss the Byss - Coruscant lane now
  adds sRGB (+29, +58, +69) instead of (+4, +20, +28) (Remake v0240: about +57, +72, +72, a wider lane, video chroma pulls it toward
  white); over black unchanged (32, 60, 75). Where two lanes cross, the later one shows, not the sum. Other galaxy nebulae (Utegetu
  etc.) are still drawn after the lanes and add in linear light over them.
- Core nebula (2026-10-17, G6): the galaxy art `w_galaxy00.alo` (object `Galaxy_Core_Art_Model_Web_Way`, scale 1.5) draws the pink
  glow over the middle of the map with its additive `core` disc (`W_GAL_gradient.tga`, scrolling). It runs at 0.37 (`CORE_GLOW`,
  `SP_GALAXY_CORE=<value>` for comparison renders), measured against v0240 and b0613s at the same map points; it was 0.14 (a maroon
  haze, the area round Coruscant read black/dark red). Not drawn yet: the core star cluster, the model's `p_galaxycore` proxy (three
  emitters of thousands of 0.2-0.4 unit sparks). The particle plugin starts `p_` proxies switched off, and switched on its sparks
  come out at 1-2 px soft dots that the sub-pixel cull removes and that spread evenly instead of the Remake's dense crisp cluster,
  so it needs work in the particle renderer.
- Labels: caps in faction colour (own = green via relation_color), two pip rows (ground, space) from Max_Ground_Base/Max_Space_Base.
  Every planet on screen shows its name unless the planet itself sits under opaque interface (2026-10-17, G3): the obstacles in
  `PanelRect` are the command bar's opaque art (`eaw_ui::ShellOpaque::rects`, not its bounding box, which hid every name below
  y 629 of 1080), the bar backing, the ticker, the hero frame and open panels. A name whose place under the globe is covered or off
  the bottom goes above the globe instead (the Remake lets its translucent bar cover it, v0240 LORONAR, v0220 BYSS). Crowding hides
  names only when zoomed far out (zoom fraction >= 0.45, farther than any Remake frame: those are about 8.5 px per map unit, our
  `--zoom 0.06`). Offscreen runs log `labels: N of M on screen named, lower third n of m, toggles k` every 60 frames (smoke row
  `labels-lower-third`).
- Selection: notched ring (mesh fill + gizmo outline); fleets stand on a faction-coloured base disc (galaxy-play visuals.rs).
- Missing vs original: hero head portraits, faction emblem beside planets, dashed sector ring (W_Planet_GC_Sector), lane/slot textures from the game.

## Real pointer test (galaxy screens)
`SP_UI_TEST=exit SP_SAVES_DIR=<scratch> space-peace --hidden --exit-after 8000 --volume 0 --campaign <c> --select <planet> --weeks 8 --move <planet>`
injects window events (cursor, mouse buttons, Esc) and checks the battle banner click, the pause menu, Save Game with a slot
click, Options, Load Game with a slot click and Main Menu (10 checks, log line `ui-test: ... PASS`). `tools\smoke.ps1`
runs it as `windowed-ui-pointer`. Source: `src/input_test.rs`.

Offscreen fleet drag (safe while the user plays: the window exists only as data, offscreen runs have no winit):
`--select <planet> --drag-pointer <planet>[:<pad>] [--drag-hold] [--drag-cancel esc|right]` presses on the first own fleet of the
selected planet, moves to the target in 48 steps, rests 24 frames (nothing may toggle, the carried model's centre within 3 px of the
cursor), releases (or cancels), and logs `drag-pointer: ... PASS|FAIL` with the frame the carry was lost, the icon offset, the carried
scale, the orders written and the fleet's state; a magenta square marks the cursor in the capture. With `:<pad>` the drop must go into
that slot on an own or allied planet and must only send the fleet on an enemy or neutral one. Smoke rows `drag-lower-right`,
`drag-hold-closeup`, `drag-cancel-right`, `drag-cancel-esc`, `drag-pad-same-planet`, `drag-pad-own-planet`,
`drag-pad-enemy-planet`. Source: `src/drag_demo.rs`.
Fixed 2026-10-17 (G2): the command bar's root node is the bounding box of its faceplate, the whole width of the bottom 42 % of the
screen; `cursor_blocked` took all of it as "over the bar", so a drag towards a planet in the lower right was dropped the moment the
cursor crossed that line (and those planets could not be hovered or clicked). Now only the opaque art (`eaw_ui::ShellOpaque`, alpha
mask of the faceplate in 4 design px cells) and the bar backing count, and a drag only ends on release, Esc or a right click.

## Galaxy UI round (2026-10-09, QA U6-U12)
- Build pad / queue / fleet-unit tooltips use `Encyclopedia_Text` (text ids, `TEXT_LINE` = line break; fallback `Tooltip_Text`) via `play_ui::lore`, plus cost / build time / population / upkeep rows and the availability reason. The tech upgrade pad lists the ships, structures and star bases that require the next tech level (`tech_unlocks`, from the sim catalog). Hovering the small red/green marker explains upkeep / income (`marker_tooltip`).
- Planet and fleet hover text goes through `eaw_ui::WorldTooltip` (same panel and droid anchor); the old `FleetTip` entity is gone. `--tooltip <button>` shows a bar tooltip in offscreen renders.
- Queue: `BarState::queue` has 10 slots (`None` = empty): 0..5 space lane (ships, star base, tech), 5..10 ground lane (structures).
- Objectives window: both boxes scroll (`ObjectivesScroll(0|1)`, wheel on the hovered box, thumb follows).
- `b_option_g` (the "Main Menu" button of the left column) opens the pause menu like Esc.
- `SP_UI_TEST` phases 140-143 (pad tooltip lore + droid anchor, marker tooltip, tech tooltip, objectives wheel scroll, main menu button) and the ground-lane check in phase 121.
- U20: `BarClass::of(category, hero)` maps every build entry to exactly one class filter; the stations / freighters filter holds the star base, structures, tech, research, transports and all heroes (test: `every_entry_of_a_real_space_menu_belongs_to_exactly_one_class_filter`).
