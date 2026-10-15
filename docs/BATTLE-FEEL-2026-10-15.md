# Space battle feel vs the Remake, 2026-10-15

What still feels different, ranked by how much a player notices. Sources: Remake frames in `space-peace-ref\yt` (n93 f0011/f0013/f0023,
CS9 f0027/f0033, Bqc5 f0031/f0053/f0057, ref 07_tactical_battle) against our frames: real window runs through
`tools/playtest/driver.ps1` (Hoth battle) and `battle_demo` close-ups (`SP_DEMO_FIGHT`, `SP_DEMO_DOGFIGHT`, Bestine, 1920x1080, t=40-50 s).
Limits: stills only (no motion), our demo fleets are not the Remake's, and the demo draws no HUD overlays. Items 1-3 are comparisons of
looks, not measurements; every number below is an estimate until measured on a matching frame.

## Ranked list

1. **Explosions are small; the Remake's are big and leave smoke.** Remake: capital deaths are large orange volumetric fireballs with black debris and
   round black smoke puffs that stay on screen (CS9 f0033, n93 f0011, f0013 "black puff on the desert"). Ours (`fight.png`): small orange
   puffs a few ship-widths across, no lingering smoke. `ships.rs::update_dying` already plays the data's `Death_Explosions` particle every 1.5 s and
   a flash at the end, so the gap is particle size/density and the missing smoke layer, not the trigger.
   **Re-measured (battlemap, same day):** the effect data is already played in full. `pd_medium_3_750` has 77 emitters: fire up to 750, smoke
   (`smoke_background` 19.5 s, 375->1500; `smoke_foreground_puff` 7 s, 375->1650), debris, shockwave, lens flares; the object lives 1500 frames. A frigate
   death in `battle_demo` (`SP_DEMO_WRECK`, t=84 s) and a forced capital effect (`SP_DEMO_BOOM=Large_Explosion_Space_Empire_pd_medium_3_750@30`) show a
   fireball about a ship wide with dark smoke that stays for 20 s, the same look as CS9 f0033 right side. Smoke tone is a little greyer than the Remake's black.
   Left as is. Note: `ship_events` picks the effect by ship class, while the units' own `Death_Explosions` is mostly `New_Space_Explosion_Medium_Heat`
   (150-500 wide, smaller) for corvettes and frigates; following the data would make those smaller, so the class table stays.
2. **Missile and torpedo trails.** Remake: thick, curved white arcs/spirals that swing onto the target (n93 f0023). Ours: very long, thin,
   straight white lines (both demo frames) that look like laser beams, not trails. Check trail width, lifetime and the missile's steering.
3. **Engine glow on capital ships.** Remake: ISD engines are big white discs with bloom (Bqc5 f0031, f0057), readable from far. Ours: no comparable
   glow in the frames I could frame; check `engine glow tied to speed` (open item in `STATUS.md`).
4. **Planet backdrop colour.** Remake Bestine reads as a dark blue ocean with soft clouds and a thin bright limb (Bqc5 f0020, 07). Ours in the demo is a pale,
   washed blue with a wide bright band below the limb. Owned by the shader tuning of d50793b; verify after it settles.
5. **Health bars and unit boxes.** Remake: wide thin double bars above every ship, even tiny ones, white flag; boxes only on fighter groups and heroes.
   Ours matches the idea (verified in the real window: capital ships selected show green corner brackets and bars, fighters show boxes with
   flags). Differences left: ours are shorter bars and the box is larger.
6. **Hero portraits.** Remake shows round portrait rings top-left/top-right during battle (n93 f0011/f0013, CS9 f0027). Not seen in our Hoth window run (no hero in the fleet).
7. **Camera motion.** Not comparable from stills. Start framing is fixed (fleet core, 700-2000); the Remake's camera drift/shake on explosions is unmeasured.
8. **Superweapon gauge and Death Star dish.** Gauge exists in our Brentaal run (QA round 2); the green lattice on the dish (Bqc5 f0053) is not compared.

## Checked and fine

- Pitch 60, fov 60 (data), free zoom, Home/PgUp/End/C/Space/P with real keys (see below).
- Selection feedback in the real window: brackets on capital ships, box + bar + flag on fighters, ability icons above the unit cards.
- Unit card bar: grouped cards with xN counts and health lines, minimap trapezoid, order grid.

## Real-key check (windowed, Hoth battle)

Home, End, PgUp, C, Space and P all act. The earlier "dead" Home/PgUp/End were a driver artifact: `keybd_event` without extended scan codes delivers
the numpad keys. `tools/playtest/driver.ps1` now sends the navigation block by scan code.

## Added after QA round 3 and the lead's notes

- **Brentaal (and 10 other planets) is a terrain map by design.** `_SPACE_PLANET_GMOUNTAIN.ted` (Felucia, Mandalore, Brentaal ...) contains one prop,
  `Prop_AAA_Terrain_GrassyMountainsHuge`, and its markers sit at z -42.9 up to +1052: the map is a low pass over mountains, not a black-space
  map with a backdrop planet. The Remake video shows the same kind of view on Byss (video b0700: green terrain fills the picture, ships over it).
  Other terrain maps: `_space_planet_gmountain_night_neb`, `_space_planet_gvulcano`, `_space_planet_polus_01`, `_space_planet_sullust_01`
  (moon terrain), `_space_planet_skies_island`. Kept as is; the clamp keeps the terrain at least 500 below the plane.
- **Retreat flash** was an opaque white sphere: a unit sphere of radius `r` AND a scale of `r`, drawn without blending. Now a unit sphere with additive blending
  at 0.6 r, like the arrival flash (`ships.rs`). The game data has no tactical hyperspace effect (only the `Unit_Ship_Hyperspace_*` sounds), so the stretch
  streak of `update_departures` plus a soft additive flash is the faithful form.
- **Squadron models under icon boxes:** the size floor for fighters now also covers every squadron member, and rises to 3.6 % of the screen height, so the
  model is larger than the 24 px icon box. The Remake's boxes also sit over ships (n93 f0011), so a box over a small ship is normal.
- **Scroll feel (4.0 data).** `camera.rs` already uses the 4.0 `Gameconstants.xml` values: `Tactical_Min/Max_Scroll_Speed` 1000/4000, `Push_Scroll_Speed_Modifier` 2.0,
  `Tactical_Edge_Scroll_Region` 2. `Hypergameconstants.XML` (`TacticalMaxScrollSpeed` 2000) is a separate test constants file, not the 4.0 game values.
  Not used: `Scroll_Acceleration_Factor` 0.08 and `Scroll_Deceleration_Factor` 1.5 (the file says "the higher the number, the less acceleration will
  be evident"). Our pan is an exponential smoothing of the target (rate 12/s). Reading the factors as time constants (0.08 s ramp-up, 1.5 s coast) would
  make the camera drift for about a second and a half after the key is released; that is a feel change, so it is left undone until you decide.
