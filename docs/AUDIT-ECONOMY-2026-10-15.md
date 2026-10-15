# Audit: start income per week against the Remake 4.0 data, 2026-10-15

Question: does the weekly income of every main faction at the start of every Remake 4.0 campaign equal what the XML says (planet value, structure and star base income abilities, ship upkeep, faction factor)?

## Method (all headless)

* `crates/galaxy-sim/examples/audit_economy.rs [mod id] [campaign substring]` builds each campaign with the default `SimConfig` (no Ground Up cut) and recomputes every main faction's income **straight from the XML** with its own code (it does not use the sim's catalog): `Planet_Credit_Value` of each owned planet, every `Planet_Income_Bonus_Ability` of the star base, the structures and the ships (absolute, percentage, `Additional_Multiplier_On_Mining_Colonies`), the market gamble, times `Credits_Accumulation_Factor`. It reads the abilities under two readings of a variant that redefines `<Abilities SubObjectList>`: replace the parent's list (what `alamo-data` does) and merge by ability name.
* It also lists every starting object with an income effect (`Starting_Forces`) that the sim does not hold, and every starting credit value that differs from `Starting_Credits`.
* `crates/galaxy-sim/tests/economy.rs`: over one week the Rebel and Empire gain their weekly income in credits (within 2 percent).

## Result

| Check | Result |
|---|---|
| Weekly income, sim against the data, 15 base Remake campaigns x 3 main factions | **identical** (0 deviations) |
| Clone Wars stack (`fft`), 52 campaigns x its main factions | **identical**, every start income is positive (lowest 499) |
| Replace against merge reading of `Abilities` | **no difference** for any starting object (so the open question of how the engine merges variant ability lists does not change a start) |
| Starting objects with an income effect missing in the sim | **none** |
| Starting credits | **equal** to `Starting_Credits` everywhere |
| Credits per week actually paid by the sim over a week | equal to the weekly income (test) |

Start income per week (sim, equals the data), base campaigns:

| Campaign | Rebel | Empire | Underworld |
|---|---|---|---|
| Mini Rebel / Imperial / Underworld | 2385 (3 planets) / 2434 / 2434 | 4729 / 3993 / 4631 | 3724 / 3724 / 3619 |
| Standard Rebel / Imperial / Underworld | 2385 / 2434 / 2434 | 6832 / 3454 / 6637 | 3910 / 3910 / 3806 |
| Full Rebel / Imperial / Underworld | 2385 / 2434 / 2434 | 7529 / 3939 / 7334 | 3910 / 3910 / 3616 |
| Standard Casual (1 planet each) | 1727 / 1623 / 1623 | 2722 / 2216 / 2722 | 778 / 778 / 1458 |

## Things that look odd but are the data

* The Empire earns about half in the `_Imperial` campaigns (3454 to 3993) of what it earns when an AI plays it (6637 to 7529): the campaign files give it different starting structures; the sim equals the data in both cases.
* Rebel start income is 6 to 7 times its planets' base value (2385 against 371): structures pay (mines +300, admin +150 ...) and ships cost little.
* The Clone Wars CIS start carries a ship with **positive** upkeep (+1500, `ship upkeep 1500` in the tool output): a ship whose `Planet_Income_Bonus_Ability` pays; it is in the data.

## Where the economy deviates from the data on purpose (known and documented in `galaxy-sim/RULES.md`)

1. One week is 60 s (`SimConfig::week_seconds`, an **estimate**; the data has no weeks). Income accrues every tick.
2. Not modelled: `Political_Income_Curve` and `Income_Redistribution` (political control is treated as 1), `Income_Stream_Ability` (its own comment says it only exists in tactical modes), `Credit_Cap_Per_Planet` (off by default: user rule), trade route `Credit_Gain_Factor` (0 on the Web Way lanes anyway), black market and corruption income.
3. Credits never go below 0: a faction with negative income stays at 0 (the engine's rule is unknown).
4. **Ground Up starts** (`ground_up.rs`) remove start structures the build rules refuse (listed in RULES.md "Dropped in the base Remake campaigns") and cut the start to 1 or 3 planets: income there is lower than the data's full start on purpose. In the Clone Wars stack that makes Underworld at Ryloth -5 per week in the Mini to Full campaigns and -195 in two others (documented, valid layout).
5. Market gamble (Underworld cantinas, `Planet_Income_Gambling_Ability`): the data gives min, max, chance and interval but not the exact rule; the sim applies the modifier like a percentage until the next evaluation (**estimate**).

No code change needed from this pass.
