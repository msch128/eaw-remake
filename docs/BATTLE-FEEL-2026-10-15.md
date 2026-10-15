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
