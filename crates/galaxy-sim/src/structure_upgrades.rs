//! Structure level swaps from the Remake GameObject scripts (`Upgrade_Golan.lua` and the other `Upgrade_*.lua`).
//!
//! Each non-spawner branch is `Upgrade_Structure(planet, player, from, to)` or `Downgrade_Structure(...)`. The build item is that
//! object when it is a space unit, or the `GroundCompany` whose `Company_Units` is the ground dummy the script runs on. Prices and
//! requirements stay on the object; this module only reads the from/to pair out of the script.

use crate::state::*;
use crate::world::*;

/// One swap named by a script: the object the script runs on, the structure it removes, the structure it spawns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptSwap {
    /// `Object.Get_Type().Get_Name()` of the branch (the dummy, not the spawner).
    pub actor: String,
    /// Structure removed.
    pub from: String,
    /// Structure spawned.
    pub to: String,
    /// `Downgrade_Structure` (the refund differs from an upgrade).
    pub downgrade: bool,
}

/// Every upgrade and downgrade branch in `scripts` (file text, comments already irrelevant). Spawners are skipped: the catalog builds
/// those as the structure itself.
pub(crate) fn parse_swaps(scripts: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<ScriptSwap> {
    let mut out = Vec::new();
    for src in scripts {
        let mut actor: Option<String> = None;
        for line in src.as_ref().lines() {
            let line = strip_comment(line);
            if line.contains("Get_Name()") && line.contains("==")
                && let Some(name) = quoted(&line).into_iter().next()
            {
                actor = Some(name);
            }
            let downgrade = line.contains("Downgrade_Structure(");
            let upgrade = line.contains("Upgrade_Structure(");
            if !downgrade && !upgrade {
                continue;
            }
            let Some(name) = actor.clone() else { continue };
            let lower = name.to_ascii_lowercase();
            if lower.contains("spawner") || (!lower.contains("upgrade") && !lower.contains("downgrade")) {
                continue;
            }
            let q = quoted(&line);
            if q.len() >= 2 {
                out.push(ScriptSwap { actor: name, from: q[0].clone(), to: q[1].clone(), downgrade });
            }
        }
    }
    out
}

/// One spawner branch named by a script: the spawner itself, the base structure it places,
/// and higher level structures that abort it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpawnerBranch {
    pub spawner: String,
    pub base: String,
    pub higher: Vec<String>,
}

/// Parses spawner branches from `Upgrade_*.lua` scripts.
pub(crate) fn parse_spawners(scripts: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<SpawnerBranch> {
    let mut out = Vec::new();
    for src in scripts {
        let mut spawner: Option<String> = None;
        let mut base: Option<String> = None;
        let mut higher: Vec<String> = Vec::new();

        let finish = |spawner: &mut Option<String>, base: &mut Option<String>, higher: &mut Vec<String>, out: &mut Vec<SpawnerBranch>| {
            if let Some(sp) = spawner.take() {
                if let Some(b) = base.take() {
                    let mut h = std::mem::take(higher);
                    h.retain(|name| name != &b);
                    out.push(SpawnerBranch { spawner: sp, base: b, higher: h });
                }
            }
            higher.clear();
            *base = None;
        };

        for line in src.as_ref().lines() {
            let line = strip_comment(line);
            if line.contains("Get_Name()") && line.contains("==") {
                let q = quoted(&line);
                if let Some(name) = q.first() {
                    if name.to_ascii_uppercase().contains("SPAWNER") {
                        finish(&mut spawner, &mut base, &mut higher, &mut out);
                        spawner = Some(name.clone());
                    } else if spawner.is_some() {
                        finish(&mut spawner, &mut base, &mut higher, &mut out);
                    }
                }
            } else if spawner.is_some() {
                if line.contains("Check_For_Higher_Level(") {
                    let q = quoted(&line);
                    if let Some(name) = q.first() {
                        if !higher.contains(name) {
                            higher.push(name.clone());
                        }
                    }
                }
                if line.contains("Set_Structure(") {
                    let q = quoted(&line);
                    if let Some(name) = q.first() {
                        base = Some(name.clone());
                    }
                }
            }
        }
        finish(&mut spawner, &mut base, &mut higher, &mut out);
    }
    out
}

/// One `Upgrade_Unit` branch: the object the script runs on, the ship it removes, the ship it spawns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnitSwap {
    /// `Object.Get_Type().Get_Name()`.
    pub actor: String,
    /// Ship removed.
    pub from: String,
    /// Ship spawned.
    pub to: String,
}

/// Every `Upgrade_Unit` branch in `Upgrade_Units.lua`. The phase repairs are the actors whose name starts with `REPAIR_PHASE_`.
pub(crate) fn parse_unit_swaps(script: &str) -> Vec<UnitSwap> {
    let mut out = Vec::new();
    let mut actor: Option<String> = None;
    for line in script.lines() {
        let line = strip_comment(line);
        let q = quoted(&line);
        if line.contains("Get_Name()") && line.contains("==") && let Some(name) = q.first() {
            actor = Some(name.clone());
        }
        if !line.contains("Upgrade_Unit(") {
            continue;
        }
        let Some(name) = actor.clone() else { continue };
        if q.len() >= 2 {
            out.push(UnitSwap { actor: name, from: q[0].clone(), to: q[1].clone() });
        }
    }
    out
}

fn strip_comment(line: &str) -> String {
    let b = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'"' {
            in_str = !in_str;
        }
        if !in_str && b[i] == b'-' && b[i + 1] == b'-' {
            return line[..i].to_string();
        }
        i += 1;
    }
    line.to_string()
}

fn quoted(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find('"') {
        rest = &rest[i + 1..];
        let Some(j) = rest.find('"') else { break };
        out.push(rest[..j].to_string());
        rest = &rest[j + 1..];
    }
    out
}

/// Replaces `from` with `to` the way `Upgrade_Structure` / `Downgrade_Structure` do, and pays the script's refund.
///
/// An exact `from` on the planet is replaced in place (the slot stays taken; the script despawns and spawns, so nothing else is
/// copied). Our build menu places the spawner variant instead of `from`; that variant is replaced when no exact `from` is present.
/// Structures have no per-instance health here, and the script does not copy health either.
///
/// Upgrade with nothing to replace refunds `(to_cost - from_cost) * 0.8` once. A downgrade refunds `(from_cost - to_cost) * 0.5 + 1`
/// per structure replaced, or 1 credit when none was.
pub(crate) fn apply(g: &mut Galaxy, w: &World, owner: FactionId, pi: usize, from: StructureTypeId, to: StructureTypeId, downgrade: bool) {
    let ps = &mut g.planets[pi];
    let mut replaced = 0u32;
    for s in ps.structures.iter_mut() {
        if *s == from {
            *s = to;
            replaced += 1;
        }
    }
    if replaced == 0 {
        for s in ps.suspended_structures.iter_mut() {
            if *s == from {
                *s = to;
                replaced += 1;
            }
        }
    }
    if replaced == 0
        && let Some(i) = ps.structures.iter().position(|&s| w.structures[s as usize].ancestors.contains(&from))
    {
        ps.structures[i] = to;
        replaced = 1;
    }
    if replaced == 0
        && let Some(i) = ps.suspended_structures.iter().position(|&s| w.structures[s as usize].ancestors.contains(&from))
    {
        ps.suspended_structures[i] = to;
        replaced = 1;
    }
    let from_cost = f64::from(w.structures[from as usize].cost);
    let to_cost = f64::from(w.structures[to as usize].cost);
    let refund = if downgrade {
        if replaced == 0 { 1.0 } else { f64::from(replaced) * ((from_cost - to_cost) * 0.5 + 1.0) }
    } else if replaced == 0 {
        (to_cost - from_cost) * 0.8
    } else {
        0.0
    };
    if refund != 0.0 {
        g.factions[owner as usize].credits += refund;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spawner_branch_is_not_an_item_and_a_dummy_branch_is() {
        let src = r#"
            if Object.Get_Type().Get_Name() == "E_GOLAN_1_SPAWNER" then
                Upgrade_Structure(planet, player, "E_Golan_1", "E_Golan_2")
            elseif Object.Get_Type().Get_Name() == "UPGRADE_E_GOLAN_2_DUMMY" then
                Upgrade_Structure(planet, player, "E_Golan_1", "E_Golan_2")
            elseif Object.Get_Type().Get_Name() == "DOWNGRADE_E_GOLAN_2_TO_1_DUMMY" then
                Downgrade_Structure(planet, player, "E_Golan_2", "E_Golan_1")
            end
        "#;
        let swaps = parse_swaps([src]);
        assert_eq!(swaps.len(), 2);
        assert_eq!(swaps[0].actor, "UPGRADE_E_GOLAN_2_DUMMY");
        assert!(!swaps[0].downgrade);
        assert_eq!((swaps[0].from.as_str(), swaps[0].to.as_str()), ("E_Golan_1", "E_Golan_2"));
        assert!(swaps[1].downgrade);
        assert_eq!((swaps[1].from.as_str(), swaps[1].to.as_str()), ("E_Golan_2", "E_Golan_1"));
    }
}
