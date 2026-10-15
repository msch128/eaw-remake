# Remake Reforged 5.2 probe (2026-10-15)

Question: how far does our engine get when pointed at Remake Reforged 5.2 (workshop 3372762649, formerly "5.1 - The
Final Update") instead of Remake 4.0? Read-only probe. No 5.2 content choice, no loader change, nothing committed.
Scope set by the user: 4.0 is finished first; 5.2 only is probed (5.0 was dropped).

## Short answer

5.2 loads and runs today with `--mod 3372762649` and no code change: the galaxy, the story and AI scripts, five
playable factions. The engine does not yet implement the new 5.2 rules (per-class tech upgrades, population,
orbital structures, ground units), so it would run but play like a thin version of the mod. A space battle on 5.2
was not reproduced yet.

## How the mod is selected

- `--mod <workshop id>` (`crates/space-peace/src/cli.rs`). `alamo_fs::GameFs::for_mods` stacks the workshop folder
  on FoC and EaW data. Any id works. `Content` in `crates/space-peace/src/game.rs` only knows `Remake` (4.0) and
  `CloneWars` (`fft`); an unknown id is treated as plain Remake.
- 5.2 is loose XML plus MEG archives with the same layout as 4.0 (`Config.meg`, `Maps.meg`, `Maps_RE.meg`,
  `Shaders.meg`, `Patch.meg`, `Patch2.meg`, `64Patch.meg` at the `Data` root). The VFS needs no change.

## Method

Release exe of the shared tree, scratch APPDATA, `--volume 0`, offscreen only. Plus a headless scripted-sim run of
30 weeks per version (a local copy of `galaxy-script/examples/scripted_match.rs` with the mod id read from an env
var; not kept). 4.0 ran as the control.

## 4.0 vs 5.2

| Area | 4.0 (control) | 5.2 | Verdict |
|---|---|---|---|
| Load | 12 campaigns (Web Way), 0 warnings | 59,697 files in 113 ms, about 60 campaigns (`GalacticConquest_*`, `IndustriesofWar_*`, `EconomyUpheaval_*`, `TechnologicalUpheaval_*`, `OldWorld_Complete_*`), 0 warnings | works |
| Galaxy | 187 planets, 1,591 unit types, 802 structure types, 12 factions | GalacticConquest_Standard_Rebel: 115 planets, 154 routes, 2,409 unit types, 3,636 structure types, 13 factions | works |
| Playable | Rebel, Empire, Underworld | Rebel, Empire, Underworld, Republic and CIS start (checked `GalacticConquest_Standard_{Rebel,Empire,Underworld}`, `IndustriesofWar_Standard_CIS`, `IndustriesofWar_Standard_Republic`, `OldWorld_Complete_Rebel`); one extra free planet (Trieron) for the human | works |
| Factions | `Empire`, `Underworld` | the data holds both `Empire` and `GalacticEmpire`, and `CriminalUnderworld` besides `Underworld` | breaks: string lookups by faction name need an alias check |
| Story scripts | 25 alive, 0 errors, 119 story events | 13 alive, 0 start errors, 19 story events | works |
| AI scripts | 0 errors, 0 unimplemented API calls | 9 errors in `GC_*_CONQUER_UNRESTRICTED_SPACE` (they concatenate `TaskForce.Get_Goal_Type_Name()` into a debug string; our host does not implement it) | breaks, one-line fix |
| Story rewards | `BUILDABLE_UNIT`, `LOCK_UNIT`, `SET_TECH_LEVEL`, `PLANET_FACTION` applied | `INCREMENT_FLAG` ignored (65 times); no ships spawned by scripts, only structures | missing |
| Space battle | works | fleet granted and sent to Hapes Cluster and Korriban arrived, no battle; Mandalore System and Corporate Sector do not exist under those names | not verified |
| Campaign list | 12 entries | about 60 entries across 5 sets and 3 sizes, no filter on the screen | breaks (UI) |
| Rules | covered | per-class tech upgrades, population, orbital structures and selling, ground companies not modelled | missing |

## Data differences that matter for the rules code

Element-name inventory of all XML: 1,504 element names in 5.2 do not exist in 4.0. Most new names are props and
story reward parameters. The rule-relevant ones:

1. Per-class tech upgrades: `Combat_Bonus_Ability` entries such as `Corvette_Armor_Lvl0..5_Upgrade`, `_Engines_`,
   `_Hull_`, `_Shield_` for about 12 classes. Our battle and galaxy sims do not apply them.
2. Population: `Downgrade_Population_Lvl1..14` dummy `SpaceUnit`s. Not modelled.
3. Orbital structures: `OrbitalStructure`, `StagStructure`, `HomeWorldObject`, `*_ORBITAL_DUMMY` units and
   `Is_Sellable`. The sim treats them as plain structures, with no sell action.
4. Ground: `DCUnit` and `Companies` replace the 4.0 `*_Regiments`. We have no ground combat (scope), so this is
   harmless today.
5. New AI goal names (`GC_RA_*`, `IW_*`, `OW_*`, `EU_*`, `CB_*` = Reforged, Industries of War, Old World, Economy
   Upheaval, Conflict Begins). They run through the existing Lua host. The per-structure `Build_*` AI scripts of
   4.0 are gone, replaced by `Filter*` scripts.
6. Removed 4.0 names: `Potential_Indigenous_Power`, `*_Regiments`, `AotR_*`. None are read by our rules.

## Effort for a menu content choice later (not started)

About 6 to 9 h for the choice itself: `Content::Reforged` (key, label, install check, restart, save key), the faction
alias, a campaign-set filter, the `Get_Goal_Type_Name` stub, `INCREMENT_FLAG` handling. Full rule parity is another
30 to 50 h (tech upgrades, population, orbitals, Industries of War economy, Old World map rules).

## Recommendation

1. Finish 4.0 first (the user's decision). Nothing in this probe blocks it.
2. Treat 5.2 as a later content choice after the Clone Wars one is solid; label it experimental until tech upgrades
   exist.
3. Next probe step when it matters: grant a unit, find a defended enemy planet in 5.2 and run one battle.

## Findings useful for 4.0 now

What 5.2 changed that also fits our 4.0 game without 5.2 data. These are tuning ideas read from the data diff; none
was tried in game. I found no evidence of UI changes (for example hiding empty ability slots): the 5.2
`COMMANDBARCOMPONENTS.XML` differs from 4.0 only by dropping `Disabled_Darken`, `Disabled_Offset` and
`Mouse_Over_Offset`, so UI behaviour in 5.2 sits in art or Lua and is not visible in the data.

1. Camera scroll feel. 5.2 `Gameconstants.xml`: `Scroll_Acceleration_Factor` 0.65 (4.0: 0.08),
   `Scroll_Deceleration_Factor` 1.15 (1.5), `Tactical_Edge_Scroll_Region` 10 px (2), `Tactical_Min_Scroll_Speed`
   900 (1000); `Hypergameconstants.XML` `TacticalMaxScrollSpeed` 4000 (2000). Go to
   `crates/battle-view/src/camera.rs` (line 97 reads only the edge region today) and the galaxy camera in
   `crates/space-peace/src/camera.rs`: try the 5.2 values as a faster, snappier scroll with a 10 px edge zone. 4.0 data: same tags exist with the old values, so this is a number change, not a data gap.
2. Hard and Brutal without stat cheats. 5.2 `DIFFICULTYADJUSTMENTS.XML` sets `Damage/Health/Shield_Multiplier` to 1
   on the hard levels (4.0: 1.05 and 1.1) and lowers the AI `Credit_Multiplier` (3.0 to 2.0, 2.0 to 1.5). That is
   an economy-only difficulty, which matches our `fair` option. Go to `crates/alamo-data/src/difficulty.rs` (where
   the multipliers are parsed) and wherever `--difficulty` is applied: offer it as the default for Hard so battles stay honest. 4.0 data: same tags exist (1.05 / 1.1 values), the choice is ours.
3. Slower AI thinking cycle. 5.2 doubles `Galactic_AI_Goal_Cycle_Sleep_Duration` (hard 0.0 to 2.0, normal 2.0 to
   4.0) and raises `Galactic_AI_Contrast_Multiplier` on all levels. Our AI costs about 240 ms per game week
   (`crates/galaxy-script/STATUS.md`). Go to `crates/galaxy-script/src/ai.rs` (idle backoff): a longer sleep on
   Hard cuts that cost with little visible loss. 4.0 data: same tag exists (0.0 on hard); 4.0 itself has no stronger version, this is untested.

## Not verified

- A space battle on 5.2, audio (`SP_AUDIO_LOG`), save/load on 5.2, the real window.
