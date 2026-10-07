# alamo-data status

Typed, lenient reader for the Alamo XML/text data of EaW FoC + Remake 4.0. Read-only; game files are never modified.

## What works
- Lenient XML parser (`xml.rs`), English string table (`text.rs`, MasterTextFile_ENGLISH.dat only).
- Galaxy model (`galaxy.rs`): campaigns, planets, trade routes, factions; `load_galaxy(&dyn FileSource)`.
- Game objects (`objects.rs`): all object XML merged, `Variant_Of_Existing_Type` inheritance via `resolve()`.
- Typed space views (`space.rs`): units, squadrons, hardpoints, projectiles, abilities
  (`UnitAbility.mod_multipliers` from `Mod_Multiplier`), direct fighter weapons.
- `CombatTable` (`combat.rs`): `Damage_To_Armor_Mod` matrix; Gameconstants key/value map.
- `GameData::campaign_planet_owners`: exact starting ownership.
- Examples: `galaxy_dump`, `object_dump` (use `raw:Name` for non-space objects). Tests in `tests/real_data.rs` skip when the game is absent.

## Key decisions
- English only, no localization. GC and space battles only; ground and story data is parsed but unused.
- Unknown tags are kept in `fields` maps rather than rejected.

## Gaps / next steps
- Energy fields (`Energy_Capacity`, `Energy_Refresh_Rate`) are not typed.
- Ground-unit and story-mission data are only parsed generically.
- `Depleted_Shield_Regen_Cap` and other Gameconstants are only reachable through the raw map.
