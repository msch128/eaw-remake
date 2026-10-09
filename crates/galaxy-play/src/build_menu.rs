//! Data model of a planet's build menu and production queues.
//!
//! [`BuildMenu::for_planet`] lists everything the faction could order at a planet, with cost,
//! build time, and whether the requirements are met right now. The UI only renders this model and
//! turns a click into [`build_command`].

use alamo_data::ShipCategory;
use galaxy_sim::{
    Command, CommandError, FactionId, Galaxy, Lane, PlanetId, ProductionKind, StarbaseTypeId, StructureTypeId, UnitRole, UnitTypeId, World,
};

use crate::catalog::PlayCatalog;

/// What a menu entry builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuildTarget {
    /// A ship, squadron or research item.
    Unit(UnitTypeId),
    /// A planetary or orbital structure.
    Structure(StructureTypeId),
    /// The next star base level (the id is the level that would be built).
    Starbase(StarbaseTypeId),
    /// The next tech level of the faction (index into the faction's tech upgrades).
    Tech(usize),
}

/// Menu section of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BuildCategory {
    /// Star base upgrade.
    Starbase,
    /// Tech level upgrade.
    Tech,
    /// Fighter and bomber squadrons.
    Squadrons,
    /// Corvettes.
    Corvettes,
    /// Frigates.
    Frigates,
    /// Capital and super capital ships.
    Capitals,
    /// Transports and other ships.
    Other,
    /// Research items that unlock units.
    Research,
    /// Orbital and ground structures.
    Structures,
}

impl BuildCategory {
    /// Section title.
    pub fn title(self) -> &'static str {
        match self {
            BuildCategory::Starbase => "Star base",
            BuildCategory::Tech => "Technology",
            BuildCategory::Squadrons => "Squadrons",
            BuildCategory::Corvettes => "Corvettes",
            BuildCategory::Frigates => "Frigates",
            BuildCategory::Capitals => "Capital ships",
            BuildCategory::Other => "Other ships",
            BuildCategory::Research => "Research",
            BuildCategory::Structures => "Structures",
        }
    }
}

/// Whether an entry can be ordered now.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Availability {
    /// Can be queued.
    Available,
    /// Requirements are met but the faction cannot pay yet.
    NeedCredits,
    /// A requirement is missing (text names it, e.g. a structure or star base level).
    Requires(String),
    /// Population cap reached.
    PopulationCap,
    /// Build limit reached.
    Limit,
    /// The queue or all slots are used.
    Full,
}

impl Availability {
    /// True if the order would be accepted.
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available)
    }

    /// Short explanation for greyed-out entries.
    pub fn reason(&self) -> String {
        match self {
            Availability::Available => String::new(),
            Availability::NeedCredits => "not enough credits".into(),
            Availability::Requires(r) => format!("requires {r}"),
            Availability::PopulationCap => "population cap reached".into(),
            Availability::Limit => "build limit reached".into(),
            Availability::Full => "no free slot".into(),
        }
    }
}

/// One orderable thing.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildOption {
    /// Menu section.
    pub category: BuildCategory,
    /// What it builds.
    pub target: BuildTarget,
    /// Display name.
    pub name: String,
    /// Credits.
    pub cost: f32,
    /// Galactic seconds the build takes (scaled by `build_time_scale`).
    pub build_seconds: f32,
    /// Population used by units (0 for the rest).
    pub population: i32,
    /// Whether it can be ordered now.
    pub availability: Availability,
}

/// One entry of a production queue.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueEntry {
    /// Lane the entry sits in.
    pub lane: Lane,
    /// What is built.
    pub target: BuildTarget,
    /// Display name.
    pub name: String,
    /// 0..=1 progress (only the first entry of a lane progresses).
    pub progress: f32,
    /// Galactic seconds left once it is its lane's turn.
    pub remaining_seconds: f32,
    /// True for the item currently being built.
    pub active: bool,
}

/// Everything shown for an owned planet.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildMenu {
    /// Planet.
    pub planet: PlanetId,
    /// Planet display name.
    pub planet_name: String,
    /// Weekly income of this planet.
    pub income: f32,
    /// Current star base level (0 = none) and its name.
    pub starbase_level: i32,
    /// Name of the current star base.
    pub starbase_name: Option<String>,
    /// Used and total orbital structure slots.
    pub space_slots: (i32, i32),
    /// Used and total ground structure slots.
    pub ground_slots: (i32, i32),
    /// Names of the built structures.
    pub structures: Vec<String>,
    /// Orderable entries, sorted by category then cost.
    pub options: Vec<BuildOption>,
    /// Queued items of all lanes (units, structures, base).
    pub queue: Vec<QueueEntry>,
}

fn category_of(world: &World, unit: UnitTypeId) -> BuildCategory {
    let ut = &world.units[unit as usize];
    if matches!(ut.role, UnitRole::Research(_)) {
        return BuildCategory::Research;
    }
    match ut.category {
        ShipCategory::Fighter | ShipCategory::Bomber => BuildCategory::Squadrons,
        ShipCategory::Corvette => BuildCategory::Corvettes,
        ShipCategory::Frigate => BuildCategory::Frigates,
        ShipCategory::Capital | ShipCategory::Super => BuildCategory::Capitals,
        _ => BuildCategory::Other,
    }
}

/// Maps the sim's refusal to a menu state; `None` hides the entry (not buildable here at all, or
/// only in the future such as a higher tech level).
fn availability(err: Result<(), CommandError>) -> Option<Availability> {
    match err {
        Ok(()) => Some(Availability::Available),
        Err(CommandError::InsufficientCredits) => Some(Availability::NeedCredits),
        Err(CommandError::PopulationCap) => Some(Availability::PopulationCap),
        Err(CommandError::BuildLimit) => Some(Availability::Limit),
        Err(CommandError::NoSlot) => Some(Availability::Full),
        Err(CommandError::RequirementsNotMet(r)) if r.starts_with("tech level") || r == "locked" || r == "planet" => None,
        Err(CommandError::RequirementsNotMet(r)) => Some(Availability::Requires(r)),
        Err(_) => None,
    }
}

impl BuildMenu {
    /// Build menu of `planet` for `faction`; `None` if the faction does not own the planet.
    pub fn for_planet(world: &World, galaxy: &Galaxy, names: &PlayCatalog, faction: FactionId, planet: PlanetId) -> Option<BuildMenu> {
        let ps = galaxy.planets.get(planet as usize)?;
        if ps.owner != Some(faction) {
            return None;
        }
        let info = &world.planets[planet as usize];
        let scale = world.config.build_time_scale;
        let mut options = Vec::new();

        // Star base: the next level, or the first one.
        if let Some(next) = world.next_starbase(galaxy, faction, planet)
            && let Some(a) = availability(world.check_upgrade_starbase(galaxy, faction, planet).map(|_| ()))
        {
            let sb = &world.starbases[next as usize];
            options.push(BuildOption {
                category: BuildCategory::Starbase,
                target: BuildTarget::Starbase(next),
                name: names.starbase(next).to_owned(),
                cost: sb.cost,
                build_seconds: sb.build_time * scale,
                population: 0,
                availability: a,
            });
        }
        // Tech upgrade.
        let fs = &galaxy.factions[faction as usize];
        let finfo = &world.factions[faction as usize];
        let tech_idx = (fs.tech - finfo.start_tech).max(0) as usize;
        if fs.tech < finfo.max_tech
            && let Some(up) = finfo.tech_upgrades.get(tech_idx)
        {
            let a = match world.check_upgrade_tech(galaxy, faction, planet) {
                Err(CommandError::RequirementsNotMet(r)) => Some(Availability::Requires(r)),
                other => availability(other.map(|_| ())),
            };
            if let Some(a) = a {
                options.push(BuildOption {
                    category: BuildCategory::Tech,
                    target: BuildTarget::Tech(tech_idx),
                    name: format!("Tech level {}", fs.tech + 1),
                    cost: up.cost,
                    build_seconds: up.time * scale,
                    population: 0,
                    availability: a,
                });
            }
        }
        // Units.
        for &u in &world.buildable_units[faction as usize] {
            let Some(a) = availability(world.check_build_unit(galaxy, faction, planet, u)) else { continue };
            let ut = &world.units[u as usize];
            options.push(BuildOption {
                category: category_of(world, u),
                target: BuildTarget::Unit(u),
                name: names.unit(u).to_owned(),
                cost: ut.cost,
                build_seconds: ut.build_time * scale,
                population: ut.population,
                availability: a,
            });
        }
        // Structures.
        for &s in &world.buildable_structures[faction as usize] {
            let Some(a) = availability(world.check_build_structure(galaxy, faction, planet, s)) else { continue };
            let st = &world.structures[s as usize];
            options.push(BuildOption {
                category: BuildCategory::Structures,
                target: BuildTarget::Structure(s),
                name: names.structure(s).to_owned(),
                cost: st.cost,
                build_seconds: st.build_time * scale,
                population: 0,
                availability: a,
            });
        }
        dedupe_options(world, &mut options);
        options.sort_by(|a, b| a.category.cmp(&b.category).then(a.cost.total_cmp(&b.cost)).then_with(|| a.name.cmp(&b.name)));

        // Queues.
        let mut queue = Vec::new();
        for (lane_idx, lane) in [Lane::Units, Lane::Structures, Lane::Base].into_iter().enumerate() {
            for (i, item) in ps.queues[lane_idx].iter().enumerate() {
                let (target, name) = match &item.kind {
                    ProductionKind::Unit(u) => (BuildTarget::Unit(*u), names.unit(*u).to_owned()),
                    ProductionKind::Structure(s) => (BuildTarget::Structure(*s), names.structure(*s).to_owned()),
                    ProductionKind::Starbase(b) => (BuildTarget::Starbase(*b), names.starbase(*b).to_owned()),
                    ProductionKind::Tech(t) => (BuildTarget::Tech(*t), format!("Tech level {}", finfo.start_tech + *t as i32 + 1)),
                };
                queue.push(QueueEntry {
                    lane,
                    target,
                    name,
                    progress: if item.total > 0.0 { (1.0 - item.remaining / item.total).clamp(0.0, 1.0) } else { 1.0 },
                    remaining_seconds: item.remaining.max(0.0),
                    active: i == 0,
                });
            }
        }

        let slots = |orbital: bool, total: i32| {
            let used = ps.structures.iter().filter(|&&s| world.structures[s as usize].orbital == orbital).count() as i32;
            (used, total)
        };
        let sb = ps.starbase;
        Some(BuildMenu {
            planet,
            planet_name: info.display_name.clone(),
            income: world.planet_income(galaxy, planet),
            starbase_level: sb.map_or(0, |b| world.starbases[b as usize].level),
            starbase_name: sb.map(|b| names.starbase(b).to_owned()),
            space_slots: slots(true, info.space_slots),
            ground_slots: slots(false, info.ground_slots),
            structures: ps.structures.iter().map(|&s| names.structure(s).to_owned()).collect(),
            options,
            queue,
        })
    }

    /// Entries of one section.
    pub fn section(&self, category: BuildCategory) -> impl Iterator<Item = &BuildOption> {
        self.options.iter().filter(move |o| o.category == category)
    }
}

/// Object name of the thing a target builds (for telling same-named entries apart).
fn object_name(world: &World, target: BuildTarget) -> String {
    match target {
        BuildTarget::Unit(u) => world.units[u as usize].name.clone(),
        BuildTarget::Structure(s) => world.structures[s as usize].name.clone(),
        BuildTarget::Starbase(b) => world.starbases[b as usize].name.clone(),
        BuildTarget::Tech(t) => format!("tech {t}"),
    }
}

/// Several game objects share one display name: terrain variants of a structure (`_Temperate` / `_Barren` / `_Urban` / `_Sector` / `_WW`),
/// `_E` / `_2` spawners, and a star base level that also exists as a structure. The list shows each thing once: entries with the same
/// name and cost collapse into one (an orderable variant wins over a blocked one, then the first), a structure named like a star base
/// level is dropped, and entries that still share a name but differ in cost get their object name appended.
fn dedupe_options(world: &World, options: &mut Vec<BuildOption>) {
    let base_names: Vec<String> = options.iter().filter(|o| o.category == BuildCategory::Starbase).map(|o| o.name.clone()).collect();
    options.retain(|o| !(o.category == BuildCategory::Structures && base_names.contains(&o.name)));
    let mut kept: Vec<BuildOption> = Vec::new();
    for o in options.drain(..) {
        match kept.iter_mut().find(|k| k.name == o.name && k.cost == o.cost && k.category == o.category) {
            Some(k) => {
                if !k.availability.is_available() && o.availability.is_available() {
                    *k = o;
                }
            }
            None => kept.push(o),
        }
    }
    let names: Vec<String> = kept.iter().map(|o| o.name.clone()).collect();
    for o in &mut kept {
        if names.iter().filter(|n| **n == o.name).count() > 1 {
            o.name = format!("{} ({})", o.name, object_name(world, o.target).replace('_', " "));
        }
    }
    *options = kept;
}

/// The sim command that orders `target` at `planet`.
pub fn build_command(faction: FactionId, planet: PlanetId, target: BuildTarget) -> Command {
    match target {
        BuildTarget::Unit(unit) => Command::BuildUnit { faction, planet, unit },
        BuildTarget::Structure(structure) => Command::BuildStructure { faction, planet, structure },
        BuildTarget::Starbase(_) => Command::UpgradeStarbase { faction, planet },
        BuildTarget::Tech(_) => Command::UpgradeTech { faction, planet },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_mapping() {
        assert_eq!(availability(Ok(())), Some(Availability::Available));
        assert_eq!(availability(Err(CommandError::InsufficientCredits)), Some(Availability::NeedCredits));
        assert_eq!(availability(Err(CommandError::RequirementsNotMet("tech level 3".into()))), None);
        assert_eq!(availability(Err(CommandError::RequirementsNotMet("locked".into()))), None);
        assert_eq!(
            availability(Err(CommandError::RequirementsNotMet("star base level 2".into()))),
            Some(Availability::Requires("star base level 2".into()))
        );
        assert_eq!(availability(Err(CommandError::NotAvailable)), None);
    }

    #[test]
    fn commands() {
        assert_eq!(build_command(1, 2, BuildTarget::Unit(3)), Command::BuildUnit { faction: 1, planet: 2, unit: 3 });
        assert_eq!(build_command(1, 2, BuildTarget::Starbase(9)), Command::UpgradeStarbase { faction: 1, planet: 2 });
        assert_eq!(build_command(1, 2, BuildTarget::Tech(0)), Command::UpgradeTech { faction: 1, planet: 2 });
    }
}
