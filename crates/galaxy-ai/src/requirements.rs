//! Parsing of the task force requirement strings of plan scripts.
//!
//! ```lua
//! TaskForce = {
//!   { "SpaceForce",
//!     "MinimumTotalSize = 3",
//!     "DenyHeroAttach",
//!     "Frigate|Capital|Super|Corvette|Bomber|Fighter|Gunship = 100%",
//!     "-Star_Destroyer_R" } }
//! ```
//! A group is `Type | Type | ... = spec` where each type is a size category (`Frigate`) or an exact object
//! name, and `spec` is `N` (exactly N), `min,max`, or `P%` (share of the force). `-Name` excludes a type.

use std::collections::HashSet;

/// One `A | B = spec` line.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Alternatives (categories or exact names).
    pub types: Vec<String>,
    /// Minimum count.
    pub min: u32,
    /// Maximum count (`u32::MAX` for percentage groups).
    pub max: u32,
    /// Share of the force in percent, for `P%` groups.
    pub percent: Option<f64>,
}

/// Parsed requirements of one task force.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Requirements {
    /// `MinimumTotalSize`.
    pub min_total: u32,
    /// `MaximumTotalSize`.
    pub max_total: Option<u32>,
    /// `DenyHeroAttach`.
    pub deny_hero: bool,
    /// `AllowFreeStoreUnits = No` disables using existing ships.
    pub allow_free_store: bool,
    /// Groups.
    pub groups: Vec<Group>,
    /// Excluded types (lower case).
    pub excluded: HashSet<String>,
    /// Flags and options not understood.
    pub other: Vec<String>,
}

/// Parses the strings after the task force name.
pub fn parse(strings: &[String]) -> Requirements {
    let mut r = Requirements { allow_free_store: true, ..Requirements::default() };
    for raw in strings {
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        if let Some(ex) = s.strip_prefix('-') {
            r.excluded.insert(ex.trim().to_ascii_lowercase());
            continue;
        }
        let Some((lhs, rhs)) = s.split_once('=') else {
            if s.eq_ignore_ascii_case("DenyHeroAttach") {
                r.deny_hero = true;
            } else {
                r.other.push(s.to_string());
            }
            continue;
        };
        let (lhs, rhs) = (lhs.trim(), rhs.trim());
        match lhs.to_ascii_lowercase().as_str() {
            "minimumtotalsize" => r.min_total = rhs.parse().unwrap_or(0),
            "maximumtotalsize" => r.max_total = rhs.parse().ok(),
            "allowfreestoreunits" => r.allow_free_store = !rhs.eq_ignore_ascii_case("no") && rhs != "0" && !rhs.eq_ignore_ascii_case("false"),
            _ => {
                let types: Vec<String> = lhs.split('|').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
                if types.is_empty() {
                    r.other.push(s.to_string());
                    continue;
                }
                let g = if let Some(p) = rhs.strip_suffix('%') {
                    Group { types, min: 0, max: u32::MAX, percent: p.trim().parse().ok() }
                } else if let Some((a, b)) = rhs.split_once(',') {
                    let (a, b) = (a.trim().parse().unwrap_or(0), b.trim().parse().unwrap_or(1));
                    Group { types, min: a, max: b, percent: None }
                } else {
                    let n = rhs.parse().unwrap_or(1);
                    Group { types, min: n, max: n, percent: None }
                };
                r.groups.push(g);
            }
        }
    }
    r
}

/// True for the size categories used in requirement groups.
pub fn is_category(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "fighter" | "bomber" | "gunship" | "corvette" | "frigate" | "capital" | "super" | "transport" | "spacestructure" | "regiment"
            | "infantry" | "vehicle" | "air" | "structure" | "antifighter" | "anticorvette" | "antifrigate" | "anticapital"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_groups() {
        let r = parse(&s(&[
            "MinimumTotalSize = 3",
            "DenyHeroAttach",
            "Frigate|Capital|Super = 100%",
            "Corvette = 0,1",
            "E_Shipyard_2 | R_Shipyard_2 = 1",
            "-Star_Destroyer_R",
            "AllowFreeStoreUnits = No",
        ]));
        assert_eq!(r.min_total, 3);
        assert!(r.deny_hero && !r.allow_free_store);
        assert_eq!(r.groups.len(), 3);
        assert_eq!(r.groups[0].percent, Some(100.0));
        assert_eq!((r.groups[1].min, r.groups[1].max), (0, 1));
        assert_eq!((r.groups[2].min, r.groups[2].max), (1, 1));
        assert!(r.excluded.contains("star_destroyer_r"));
    }
}
