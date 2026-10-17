# Audio check 2026-10-15 (objective, no listening)

Method: `SP_AUDIO_LEDGER` + `SP_AUDIO_LOG` runs of the offscreen scenarios `battle-fight-orders` (speed 30), `battle-reinforce`,
`build-and-move` and a 150 s fight at speed 1 (`--battle-speed 1 --battle-abort-after 150`), debug build, `--volume 0`, scratch APPDATA.
Event definitions come from `SFXEvent*.xml` through `cargo run -p alamo-audio --example sfx_dump -- <event>...` (new: prints priority,
Max_Instances, volume, saturation distance and the real sample lengths). A ledger entry is written when a voice *starts*, so counts include
voices that the plugin cuts a moment later; in ledger mode inaudible sounds also start, so the numbers are an upper bound.

## Ranked findings

| # | Finding | Evidence | Status |
|---|---|---|---|
| 1 | `Unit_Ship_Hyperspace_Enter` fired for every launched fighter (all hyperspace sounds in a fight were squadron launches, none reinforcements) | 254 starts in 150 s, up to 24 in one second, in bursts at each carrier/station launch; the event is priority 5, volume 100, 2.2 s long | fixed: no arrival sound for ids in `SquadronLaunched`; now 0 in the fight, a real reinforcement wave still plays it (1 in `battle-reinforce`). The visual arrival flash for launched fighters is a separate question (`ships.rs`, not changed) |
| 2 | Cutting a voice (Max_Instances, voice limit, unit-response replacement, Kills_Previous) despawned it at full amplitude: a click, and at battle fire rates it happens dozens of times a second | weapon events have `Max_Instances` 3 to 4 and samples of 1.3 to 2.6 s (reverb tails); `Unit_Laser_Fire_Empire_Fighter_1` starts up to 48 times per second, so each voice lived about 80 ms and ended abruptly | fixed: a cut voice fades out over 60 ms (`CUT_FADE_SECONDS`, `cut_voice`), then despawns |
| 3 | A far, inaudible request still cut the oldest *audible* voice of its event before being discarded as inaudible | order of checks in `process_sfx_requests` | fixed: audibility is decided first |
| 4 | What `Max_Instances` does at the limit is not stated in the data. We cut the oldest (with finding 2's fade); the other plausible engine behaviour is to ignore the new sound so the long tails finish. In a dense fight the two sound very different (many short chirps vs. a few full tails) | `SFXEventPresets.xml`: 90 of 94 presets carry `Max_Instances`; no tag says more | open, needs ears: `SP_AUDIO_MAXINST=drop` switches to "drop the new one" for an A/B listen; default unchanged. 2026-10-17: a far one-shot no longer cuts a near one, the quietest instance goes and a request more than 3 dB quieter than it is dropped (`SP_AUDIO_MAXINST=oldest` is the old rule), see `alamo-audio/STATUS.md` "Max_Instances: the nearest sound wins" |
| 5 | Dropped requests were invisible in the ledger, so "expected but never heard" could not be told from "never requested" | new `Dropped` outcome with the rule | added. Seen: `RHD_Choose_Reinforcements` dropped by `Overlap_Test` group `Unit_RHD` (it was asked for while `RHD_Reinforcements_Available` played), `RHD_Reinforcements_En_Route`, `RHD_Unit_Destroyed` and the chatter lines `Fighters_Empire_Won/Winning_1` dropped by their `Probability` |
| 6 | Expected events that never fired in the scripted fights | see list below | all but one explained by the data, see below |
| 7 | Mix: battle SFX are quiet next to music and speech | per-voice effective gain in the log: weapon shots 0.02 to 0.07 (72% of 5114 starts below 0.05, 116 below the 0.01 floor), deaths 0.6 to 1.0, speech lines 0.9, music 0.6 (bus 0.6 x event 100%); `audio balance` lines show 22 to 46 SFX voices with an average of 0.03 to 0.06 | not changed: the data defines no bus volumes (`Audio.xml` has none), the falloff curve and bus defaults are estimates, only a listen can judge it |
| 8 | Music | `Galactic -> BattleLoading (loop, 0.05 s) -> Space ambient (2 s fade-in) -> Space battle (0.5 s fade-in, ~33 s into the fight, at the first hostile shot) -> result / Galactic (2 s)`. Outgoing tracks fade over the new event's `Fade_Out_Previous_Seconds`; only result stingers use a fast 0.35 s fade (by design). The ambient/battle switch has the 15 s peace delay from `Audio.xml` | no defect found; crossfades are covered by `tests/plugin.rs`, `pass2.rs`, `music_data.rs` |

## What the data limits and what we do with it

| Data field | Used? | Note |
|---|---|---|
| `Max_Instances` | yes | policy question, finding 4; HUD / unit-response voices drop the new request (never cut) |
| `Min/Max_Volume`, `Min/Max_Pitch` | yes | random per start |
| `Priority` | yes | voice limit (48): the least important / oldest voice is stolen. Busiest observed: ~46 SFX + music + speech, so the cap is close in big fights |
| `Probability` | yes | now visible as `Dropped` |
| `Play_Sequentially` | yes | |
| `Overlap_Test` | yes for non-unit-response events | unit responses instead cut each other (engine behaviour described in `STATUS.md`) |
| `Kills_Previous_Object_SFX` | only with a source entity | battle sends fire / death sounds without a source, so the 19 events with this flag (e.g. `Unit_TIE_Fighter_Death_SFX`) never cut anything; the effect would be to stop other sounds of the dying ship, which has no source tracking today |
| `Volume_Saturation_Distance` x `Audio_Space_3D_Saturation_Distance_Mod` 1.5, rolloff mod 1.0 | yes | curve `(sat/d)^rolloff` is our estimate. Notable data: fighter weapons 100 to 150, capital turbolasers 350 to 600, death sounds of corvettes and Star Destroyers 450000 to 500000 (audible over the whole map at full volume) |
| `Pre/Post_Samples`, `Chained_SFXEvent`, `Min/Max_Predelay`, `Loop_Fade_*`, `Play_Count` | yes | |
| `Kill_Lowest_Priority`, min/max distance tags | not in this data | none of the SFXEvent files contain them |

## Expected events that did not fire in the scripted fights

- `Unit_Move_*`, `Unit_Attack_*`, `Unit_Guard_*`: the scripted fight only selects and retreats (`--battle-orders`); these are covered by unit tests and the earlier windowed audio scenarios, not by an offscreen run.
- `Unit_Under_Attack`, `Health_Low/Critical`, `Unit_Lost`, `Hardpoint_Destroyed` lines: the player's units in the scenario (Assault Frigate, Nebulon, Gunboat, Corvette) define none of them in the data (checked in `Units_Space_Rebel_Assault_Frigate.xml`, which names select, move, stop, guard, attack, fleet-move and hardpoint lines only).
- `EnemyUnitDestroyed` advisor line: no `*HD_Enemy_Unit_Destroyed` event exists (known data gap).
- Impact sounds on shields: the Remake event is blank.
- Everything else asked for fired: weapon fire per hardpoint (14 distinct `*_Fire_*` events), hull detonations, deaths of small and large ships, engine loops, advisor battle pending / destroyed / end, music switches at battle start, ambient return and the win stinger.

## Still not verifiable without ears

Loudness balance (finding 7), the Max_Instances policy (finding 4), whether the 60 ms fade is long enough to remove the click on every sample, and the feel of the death sounds that are audible map-wide by data.
