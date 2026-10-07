---
name: remake-data
description: How to find, read and trust Empire at War Remake 4.0 mod data (Workshop 1770851727) in this repo - install/mod location, XML game objects and Variant_Of_Existing_Type inheritance, Gameconstants, campaigns/planets/trade routes, Lua scripts, string table - and how to verify gameplay values (unit stats, speeds, shields, damage, income) against the real data instead of guessing. Use whenever a task involves unit stats, balance, a game object's name or field, campaign/galaxy content, Lua story/AI scripts, or "how does the original game do X".
---

# Remake 4.0 data

The game is never shipped. Everything is read at runtime from the user's Steam install of
*Star Wars Empire at War: Gold Pack* (app 32470) plus Workshop mod **1770851727** (Empire at War Remake).
The Steam root is in the registry (`HKCU\Software\Valve\Steam\SteamPath`); the game can be in any library
folder. Override with env `EAW_GAME_DIR` (game root) or the app flag `--game-dir`.

**Rule: never invent gameplay numbers.** Look the value up in the data, cite the XML file and tag in code
comments or RULES.md, and mark values the engine hides (closed source) as an **estimate**.

## Getting at the data

```powershell
# search all XML (mod over FoC over EaW), case-insensitive substring
cargo run -p alamo-render --example xml_grep -- Star_Destroyer 30 20
# one game object, fully resolved (inheritance applied), typed space view
cargo run -p alamo-data --example object_dump -- 1770851727 Star_Destroyer
cargo run -p alamo-data --example object_dump -- 1770851727 raw:Star_Destroyer   # every tag
cargo run -p alamo-data --example object_dump -- 1770851727 ?Corvette            # name search
# raw file out of the VFS
cargo run -p alamo-fs --example megtool -- vfs-cat 1770851727 Xml/Gameconstants.xml
# campaigns
cargo run -p space-peace -- --list-campaigns
```

Use a separate `CARGO_TARGET_DIR` for data tools (e.g. `target/tmp`), so you don't block other builds.
Tests against real data (`tests/real_data.rs`, ...) **skip silently** when the game is missing, so a green
test run does not prove real data was checked. Look for the skip message.

## Data model (alamo-data)

- `GameData` / `load_galaxy(&dyn FileSource)`: campaigns, planets, trade routes, factions;
  `campaign_planet_owners` gives the exact starting ownership.
- `objects.rs`: all object XML merged. **`Variant_Of_Existing_Type` inheritance only applies through `resolve()`**.
  Reading raw fields of a variant without `resolve()` is a classic bug.
- `space.rs`: typed units, squadrons, hardpoints, projectiles, abilities (`Mod_Multiplier`).
- `combat.rs`: `CombatTable` (`Damage_To_Armor_Mod`, 673 entries) and the Gameconstants key/value map.
- Unknown tags stay in `fields` maps. Energy fields and `Depleted_Shield_Regen_Cap` are untyped (raw map only).
- English string table only (`MasterTextFile_ENGLISH.dat`). No localization.

## Interpreting values (established rules, see crate RULES.md)

- XML `Max_Speed` (ships and projectiles) is **per frame** and is multiplied by 30 (`Catalog::speed_scale`).
  `*_Seconds` fields are real seconds. `Max_Rate_Of_Turn` is used as rad/s.
- Shields: `Shield_Refresh_Rate` per `ShieldRechargeIntervalInSecs` (2 s). A shield drained to zero is offline for
  `Depleted_Shield_Disable_Time` (4 s).
- `Fire_Category_Restrictions` = classes the weapon must **not** fire at.
- GC week = 60 s. Planet capture without ground combat is a rule inferred from the AI scripts.
- Battle rules: `crates/battle-sim/RULES.md`. GC rules: `crates/galaxy-sim/RULES.md`.
  Update the matching RULES.md when you change or discover a rule.

## Lua scripts

709 scripts (mod + base game). Inventory of the 446 engine API functions and entry points: `crates/alamo-lua/API.md`
(regenerate with `cargo run -p alamo-lua --example lua_survey -- 1770851727 --md crates/alamo-lua/API.md`).
Story plots: `run_story [workshop_id] [plots.xml] [seconds]`. GC with scripts: `galaxy-script --example scripted_match`.
API gaps between scripts and the sim: `crates/galaxy-script/NEEDS.md` -> `crates/galaxy-sim/NEEDS-DONE.md`.

## Scope
Galactic Conquest and space battles only. Ground combat, story campaigns and movies are out of scope.
Their data is parsed but not used.
