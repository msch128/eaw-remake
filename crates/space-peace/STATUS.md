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
  capillary 0.07; fixed world size, thinner on screen when zoomed out), the data colour (teal 13, 31, 38 for every relation; the data's
  Enemy look fades it out close up, which the map does not use, see the next item) x 3.6 in linear light (sRGB about 32, 62, 76 on
  black), a glint 1.6 times as bright runs along every lane
  (scrolling texture, replaces the red/green gizmo pulses). Orbit-line art (`Region_Lines`, the thin double curves) at 0.4 so it
  stays fainter than the lanes like in the Remake; grid off. galaxy_map.rs spawn_lanes/update_lanes.
- Every lane shows, whoever holds its planets (2026-10-17; user request: the connection between two planets must always be visible; a lane such as
  Alderaan - Brentaal was missing until a planet was conquered). Cause 1: the lane look was picked by the owners of
  its two planets (`Own` / `Enemy` / `Contested` / `Neutral`) and the Remake's Web_Way_Main and Web_Way_Inter `Enemy` look has
  `Color_Zoomed_In` alpha 0, so every main or inter lane between two planets of one other faction faded out with the zoom (alpha = zoom
  fraction: 0.25 at the opening zoom, 0.06 close up, gone below 0.01); in the Imperial start 141 of 145 planets belong to some faction
  (68 to Independent Forces), so 54 of the 189 routes were faded, 64 of 189 in the Rebel start, and each conquest lit the lanes at the
  new planet. Now `lane_line` gives every lane the look of an unclaimed route (the same teal as the other looks in the data), and
  nothing in the lane code reads an owner (`route_relation` is gone). Cause 2: the repaint of the vertex colours was keyed on zoom
  step and an owner hash held in a `Local` that outlived the galaxy, so a galaxy spawned again (back to the menu, the same campaign)
  whose owners and zoom matched the old one's stayed black; `LaneMesh::painted` now belongs to the mesh and a new mesh is always
  painted. At the first paint the log says `lanes: N routes in the campaign, N drawn, N at full brightness at zoom z (n use a hidden
  style)`: 189 / 189 / 189 in both Full Web Way campaigns (before: 189 drawn, 135 / 125 at full brightness). Tests (galaxy_map.rs):
  `every_drawn_lane_is_as_bright_as_the_next_at_every_zoom_whoever_holds_its_planets`,
  `a_galaxy_spawned_again_gets_its_lane_colours_at_the_same_zoom_and_owners`,
  `the_full_web_way_campaigns_draw_every_route_of_their_data_at_full_brightness` (real data, skips without the game; the campaign
  lists 189 routes, 189 are defined, 189 lie between planets of the map, 189 are drawn and bright at 8 zooms from 0 to 1). The look of
  a lane (width, colour, glint) is unchanged: `--zoom 0.15` / `0.06` renders at Alderaan differ from the old build only by the newly
  shown lanes.
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
  Every planet on screen shows its name (2026-10-17, G3 and G3b): names, pip rows and emblems are drawn below the command bar
  (`GlobalZIndex(-2)`; the bar's backing plate is -1), so like in the Remake (b0613s LORONAR, v0220 BYSS) a name behind the
  translucent build strip shows faintly through it and the opaque art covers the rest. The bar is no obstacle in `PanelRect` (its node
  is the bounding box of the faceplate, which hid every name below y 629 of 1080). A name that would sit under another HUD panel (the
  ticker line) or off the screen goes above its globe, with hysteresis (`PLACE_HYSTERESIS`: under its globe a name stays while its box
  is covered by up to 2 px, above it it stays until the place under the globe is clear by 4 px more), so a name grazing a panel edge
  while the camera moves does not jump (Ithor did, `--label-probe`). Zoomed out to zoom fraction 0.47 and beyond the map shows no names
  but the selected planet's (G3c, user decision 2026-10-17; the hover card still names the planet under the cursor), and they come
  back below 0.43 (`NAMES_HIDE_FROM`, `NAMES_SHOW_BELOW`: the gap is a hysteresis, so a zoom resting at the threshold never makes them
  blink); this replaced the crowding cull that thinned the names out from 0.45. At game start the camera opens on the capital at 0.25,
  so the names show. The Remake has no far zoom to compare with: its galaxy camera has one view (Gameconstants.xml
  `GMC_InitialPullbackDistance` 470 at `GMC_InitialPitchAngleDegrees` -50; `Max_Galactic_Zoom_Distance` is marked unused) and the
  wheel zooms into the planet view.
  Every galaxy frame of the user's videos is at that scale with names (b0613s: Byss to Corellia 9.3 px per map unit; ours shows them
  8.5 px per map unit apart at `--zoom 0.15`), and b0621s, the one without names, is the 1 s camera flight back from the Coruscant
  planet screen (Coruscant to Alderaan 776 px against 562 px in b0613s, so closer, not farther out).
  Offscreen runs log `labels: N of M on screen named, lower third n of m, toggles k` every 60 frames (smoke row `labels-lower-third`).
  `--label-probe` (offscreen, label_probe.rs) proves the lower part through the real input path: every planet whose globe or name is
  in the bottom 42 % is hovered (12 frames), clicked and gets a fleet of the start planet dropped on it, then the view holds still for
  120 frames and zooms out slowly for 120, logging per frame the names of the lower part with their positions. 2026-10-17 PASS at
  zoom 0.06, 0.15 and 0.30 in GC_Full_Web_Way_Imperial (The Coruscant System selected) and the default Rebel campaign (The Yavin
  System): 53 lower planets, all 37 with a globe point clear of the bar art hovered, selected and sent the fleet; no name moved in 720
  still frames or changed more than once in 720 zoom frames. A globe entirely behind the bar art cannot be hovered or clicked (the
  cursor is on the interface there): scroll the map. `SP_NO_PLANET_NAMES=1` renders the same view without names, to measure what the
  names add: behind the empty build strip our names stand 9-21 sRGB luma levels (median 12) above the strip, colour distance 17-40,
  against 20 and 34 for LORONAR in b0613s (a video frame, its dark levels are crushed); under the unit cards and the minimap column
  they are covered, as that art is opaque. Since G3c the probe also wants the hover card to name the hovered planet and the selected
  planet named in every frame, and zooms back in after the zoom out; the frame in which most names go or come at once (the threshold)
  may happen once each way. 2026-10-17 across the threshold (start 0.42, out to 0.50 and back, both views): the names went once and
  came back once, nothing else blinked, the selected name stayed; at 0.60 (names hidden) all 39 Rebel lower planets with a free globe
  point were hovered (card), selected and reached.
  Planets stay clickable under fleet icons (2026-10-17, G3d): from about zoom 0.4 the
  pick areas of the fleets parked around a small planet (20 px or more) covered its whole globe, so the click, the hover card and a
  drop there went to a fleet and the planet could not be selected (the old probe: Byss and Rendili at 0.42, Allanteen VI, Byss, The
  Corellia System and Fondor at 0.60). galaxy-play `input::pick_under` now gives a planet its visible globe (`GalaxyLayout::globe_radius`,
  0.54 of the bounding radius: 6.6-9.3 px at 0.42 in 1080p) against fleets whose icon is centred outside it (those icons stand 13-20 px,
  1.9-2.4 globe radii, from the centre); see galaxy-play STATUS. `--label-probe` tries every lower planet twice, on a free point and on
  a point of the visible globe under a fleet's pick area (the point 9/10 of the way to the nearest fleet icon first; the drop must be a
  plain move or a pad drop, not a merge into the fleet), then hovers every parked fleet in view at its icon centre and clicks the own
  ones at the centre and at the edge of their ring. It runs with the game clock paused: the dragged fleet leaves its planet with every
  drop and a departing icon drifts out of the globe the pointer was left on (1 to 4 of the drops went wrong in about half the runs
  with time running).
  Imperial start, 1080p, before / after: zoom 0.42 22 lower planets, 17 with a free globe point, 6 with a fleet over the visible globe,
  8 failures (Byss and Rendili on both points, Allanteen VI, Cato Neimoidia, Fondor and The Corellia System under a fleet) / none; zoom
  0.60 44 planets, 26 with a free globe point, 4 with a fleet over it, 8 failures (Allanteen VI, Byss, The Corellia System and Fondor
  on both points) / none. At 4K the same planets fail before (5 at 0.42, 4 at 0.60, all under a fleet) and pass after; Imperial zoom
  0.06, 0.15 and 0.30 pass. The Rebel start (38 and 53 lower planets, 20 and 39 with a free point) has no fleet over a lower globe and
  passes before and after. The 15 parked fleets in view stay hovered right at their icon centre and the 5 own ones stay selectable at
  the centre and at the ring edge. A click on the ring's edge towards the planet is the planet's now (the old aim of the windowed
  fleet-ring click, the ring's right edge, was the planet's for 4 of 5 own fleets at 0.42 and 2 of 5 at 0.60), so
  `input_test::ring_edge` aims at the edge away from the planet. Not run: the windowed `windowed-ui-pointer` row (the user was gaming).
  Rebel zoom 0.06 shows a name blink of Agamar (two shown/hidden changes on the zoom out), also in the build before G3d: not a pick matter.
- Selection: notched ring (mesh fill + gizmo outline) and the dashed sector ring in the colour of the relation
  (`territory_segments`); fleets stand on a faction-coloured base disc (galaxy-play visuals.rs). Round the dashed ring lies the Remake's
  sector shadow (2026-10-17, G9, `SectorShadow`): the map inside the ring is unchanged, right outside it shows at 0.44 of its
  brightness and is back to normal 0.62 ring radii out (measured on the user's video at the same map points with and without a
  selection: b0613s against v0240s south of Coruscant, v0220s against v0240s south of Fresia; same for every owner, colour-neutral).
  It is a multiplying annulus under the map plane, so planets, lanes and fleets stay as bright; the factor goes into linear light
  with the power 1.85, right for the map's typical brightness (a little darker than the Remake on the darkest space). Our render at
  `--zoom 0.15`: 0.49 / 0.54 on the nebula 0.1 / 0.23 ring radii out, 0.37 on dark space next to the ring. `SP_NO_SECTOR_SHADOW=1`
  leaves it out for comparison renders.
- Missing vs original: hero head portraits, faction emblem beside planets, lane/slot textures from the game.
  (`W_Planet_GC_Sector.alo` is not the dashed ring but the model of the "Sector" planets, the galaxy-shaped clusters like Senex-Juvex.)

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

## Galaxy camera at the Remake's 50 degrees (2026-10-17, C2)
- The galaxy view is tilted 50 degrees at every zoom, from `Gameconstants.xml` `GMC_InitialPitchAngleDegrees` (-50; `game::galaxy_camera` reads it into
  `GalaxyView::camera`, the camera takes it when a galaxy is framed, `camera::frame_galaxy`). Before, the tilt eased with the zoom from 50 degrees (fully out)
  to about 24 (closest, `ZOOM_FLATTEN` 0.45 rad, from `GMC_ZoomedPitchAngleDegrees` -20): 26 degrees at zoom 0.06, 28 at 0.15, 32 at 0.30, 35 at 0.42, 40 at 0.60.
  Decision for the closest zoom: it stays at 50. The Remake's galaxy camera has one tilt; its flatter -20 is the planet zoom, which is the planet view's dive here
  (`planet_view::camera_transition_pose` turns the camera from the galaxy pose into the scene's, about 13 degrees down); nothing else needs the flattening.
  Distances, `ZOOM_STEP` (12 % a notch), zoom range, start zoom (0.25) and all controls are untouched, so the wheel path to the planet view is the same (12.8
  notches from the start view to the opening distance of twice the closest zoom, with or without the tilt; the zoom is by distance).
- The point under the cursor now stays exactly put while zooming (it drifted by up to 8 % with the easing tilt). The middle button still tilts (limits 20 to 83
  degrees); a newly built galaxy starts at the data's tilt again.
- Measured against the user's video (1080p; `SP_PLANET_PX=1` logs the screen position of every planet at the capture, the video positions are read off b0613s): from
  Coruscant, Fresia is 328 px left and 105 px down in b0613s (down / left 0.32) and Alderaan 560 px right and 12 down; ours at zoom 0.10 (the scale of the frame) was 337 left,
  63 down (0.19) and 550 right, 6 down before, and is 334 left, 115 down (0.34) and 558 right, 12 down now. A circle on the map is 0.77 as tall as wide at every zoom (sin 50);
  it was 0.44 at the closest zoom (test `a_circle_on_the_map_is_as_flat_as_the_tilt_says_at_every_zoom`). The horizontal scale is the same as before at a given zoom, the
  vertical one grows with the tilt, so the zoom that matches the Remake frame is 0.10 (the G3c note above measured Byss - Corellia instead, 8.5 against 9.3 px per map unit at 0.15; Coruscant - Alderaan is 394 px at 0.15 and 560 px in b0613s, 558 px at 0.10).
- Core nebula coverage of the playfield (share of samples that are red-dominant above the command bar; `b0613s` 77 %, `v0240s` 80 %, `v0220s` 63 % with its selection
  shadow): ours 43 % -> 52 % (Coruscant, 0.10), 54 % -> 76 % (Byss, 0.10), 53 % -> 73 % (0.06), 36 % -> 39 % (0.15), 23 % -> 23 % (0.30). The tilt shows more of the
  plane and less sky, but the Remake's nebula still fills the whole screen around Coruscant (top row 65-91 % in b0613s, 1-34 % here): our core glow ends nearer the core.
- The planet view's dive now swings round the planet (`planet_view::swing`) instead of flying a straight line from the galaxy pose to the scene's: from the 50 degree tilt
  the straight line took the planet under the command bar from blend 0.35 on (test `the_planet_stays_in_view_above_the_command_bar_through_the_dive_from_every_tilt_of_the_galaxy_camera`
  fails with the old line at tilt 50, passes for 25 to 80). The path of the distance, the end pose and the scene are unchanged; `SP_PLANET_VIEW_BLEND=<0..0.78>` freezes a
  frame of the dive for a capture.
- `SP_PLANET_PX=1` (offscreen) logs the camera and the screen position and globe radius of every planet in view at the capture (`planet-px:` lines, `offscreen::log_planet_pixels`).
- Tests: `cargo test -p space-peace` (92), among them `camera::tests` (tilt at every zoom, circle flatness, ground point under the cursor, framing a galaxy at the data's tilt)
  and `game::camera_constant_tests`; the real-data `maps_real_campaign` checks 50. `--label-probe` at zoom 0.06 / 0.15 / 0.30 / 0.42 / 0.60 in both campaigns (Imperial with The
  Coruscant System, Rebel with The Yavin System), 1080p and 4K: all PASS, 0 still toggles. Three rules of the probe were wrong for the new geometry and are fixed: (1) only the
  visible globe is the planet's own, so a planet whose visible globe is behind the info panel and whose rest of the pick disc is under a parked fleet has nothing to prove (N'Zoth at
  Imperial 0.06 was flagged); (2) a name that goes above its globe when a panel meets its place and comes back tens of frames later is no blink: only a change of side again within
  `SIDE_FLIP_FRAMES` (30) is (Dathomir, Rebel 0.06); (3) `ZOOM_LAG` is 2, not 3 (measured: the camera frame whose zoom first reads 0.470 hides the names two frames later), so a
  name that shows for the single frame of the turnaround counts once out and once back; with 3 the old build flags such a planet at 4K 0.25 and 0.28 (Nanth'Ri, Rakata Prime) too.
  Left as it was: the shown / hidden rule allows one change per leg, so a name that hides near the bottom of the screen and returns later on the same leg (Taris at Rebel 4K,
  zoom 0.25: hidden at zoom frame 29, shown again at 117) is still flagged; that zoom is not in the usual set.
