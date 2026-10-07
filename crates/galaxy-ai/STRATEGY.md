# Strategic layer (galaxy-ai `strategy.rs`)

The Remake goal functions rate a target by importance x priority (value, strategic importance, expected resistance). They ignore distance,
supply, nearby fleets and the enemy's main stack. This layer sits on top: it multiplies each offensive/defensive desire by a utility factor
(0.2..3) and changes how forces are sized. Deterministic, `Send`, no Bevy; a refresh plus scoring of every planet for 12 players costs ~1.3 ms
(`tests/strategy.rs`), ~1 ms per game week in a full match.

## Inputs (`Snapshot`, built every 4 s of game time)
Fleet strength per planet and player (`AiSim::force_strength`: square-law `sqrt(durability x damage rate)` from the unit combat stats, same
measure the auto-resolver uses) and fixed defenses of each planet (`AiSim::planet_defense`: star base, structures, garrison).

## Influence maps
`influence[p][planet] = sum over sources of strength x 0.5^hops` (3 hops, route graph). Hops to the nearest owned planet gives supply/distance.

## Utility (`Strategy::factor`, `exp(u)` clamped)
`u = 0.9 value x aggression - 0.7 caution x risk - 0.25 (hops-1)` plus: +0.2 on the frontier, neutral planets `+0.5 expansion x (1 - owned/(0.35 x map))`
(measured drive that fades), flank `+0.6 aggression` when the target is held at less than half of the owner's main stack and that stack is 2+
hops away, pocket `+0.4 aggression` when taking it isolates owner planets. `risk = (hostile strength at target + 0.5 x hostile influence) / (own
influence + 5% of own total)`. Own planets use the defend term `1 + 1.5 x value x threat-beyond-our-influence`. Value = income, structures used,
star base level, shipyard. Weights are estimates (not in the data).
`Phase::Consolidate` (planets lost in the last 2 min, or total strength below 35% of the strongest neighbour): offensive utility -0.7 except safe
adjacent targets, defend term x1.5.

## Honest information
Visible = own planets and fleets plus `vision_hops` (1) around them. Unseen planets use the last seen hostile strength, else the average of what the
faction has seen. `AiTier::full_map_knowledge` (Brutal) removes the fog. The Remake equations themselves still read the true state.

## Force sizing and pincers
`compose` sizes attack forces until `force_strength(wanted) >= believed defense x 1.25 x plan contrast scale (MinContrastScale..MaxContrastScale) x
tier contrast x sqrt(caution/aggression)` (1.25 is an estimate). Ships already on the free store are used first and the composition is a function
of (player, target, force), so a force assembled by an earlier plan is recognised as ready; released forces are reserved for their target for 5 min.
If the needed strength is 50-90% of everything the player owns, two plans attack the target together: each is sized for half, waits at its stage
(`Job::Rally`, max 90 s) and leaves so that all arrive together (`AiSim::eta`).

## Difficulty and personalities
`alamo_data::DifficultyTable` reads `Xml/DifficultyAdjustments.xml`. galaxy-sim applies per faction (`Sim::set_faction_difficulty`): credit multiplier
on positive income, build time, and damage/health/shield in statistical battles and (via `TacticalSide`, `BattleConfig::side_mults`) tactical ones.
The AI reads `Galactic_AI_Contrast_Multiplier` (force margin) and `Galactic_AI_Goal_Cycle_Sleep_Duration` (extra wait between evaluations as a fraction
of the interval; the unit is an estimate). Tiers (`galaxy_script::DifficultyTier`): Easy/Normal/Hard = mod data (`*_Default`, `*_Minor` for minor
factions); **Brutal** (estimate): Hard with credits x4 and full map knowledge; **Fair** (estimate): no multipliers at all, Normal contrast, fog.
Personalities (aggression, expansion, caution; default 1) per faction; defaults in `galaxy_script::default_personalities` are estimates.
