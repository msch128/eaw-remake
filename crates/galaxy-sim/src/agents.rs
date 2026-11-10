//! Non-combat agents of the galactic map and the `System_Spy_Ability` data (docs/RULES.md, "Agents" and "Galaxy fog of war").
//!
//! Agents are `HeroCompany` objects of `CategoryMask` `NonCombatHero` on the heroes build tab (`Build_Tab_Heroes`) whose company units carry a
//! `Siphon_Credits_Ability` or a `System_Spy_Ability`: in the Remake the Rebel `Rebel_Smuggler_Team`, the Underworld `Underboss_Team`
//! and the Rebel `Rebel_Spy_Team` (`Units/Ground/Non-Combat_Heroes.xml`). They are recruited like ships, travel in fleets at the
//! speed of their `Company_Transport_Unit`, take no part in battles and use their ability at a hostile planet:
//!
//! * siphon (`Activation_Style` `Ground_Activated`): ordered with [`Command::ActivateAgent`]; for `Siphon_Duration_In_Cycles` fiscal
//!   cycles (`Fiscal_Cycle_Time_In_Secs`) a share between `Min_Siphon_Percentage` and `Max_Siphon_Percentage` of the planet's income
//!   goes to the agent's faction instead of the owner;
//! * spy (`Galactic_Automatic`): on arrival at a hostile planet the planet is revealed for `Duration_In_Secs` with the `See_*` flags.
//!
//! Both have `Causes_Despawn`: the agent is used up.

use std::collections::HashMap;

use alamo_data::{GameData, SubAbility, xml::parse_bool};

use crate::catalog::{name_list, parse_requirement};
use crate::sim::Sim;
use crate::state::*;
use crate::world::*;

/// `See_*` flags of a `System_Spy_Ability`: which details of a planet a spy reveals. A missing flag is `No`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SpyFlags {
    /// `See_Base_Levels`: star base and ground base level.
    pub base_levels: bool,
    /// `See_Num_Fleets`: the fleets in orbit, without their contents.
    pub num_fleets: bool,
    /// `See_Fleet_Contents`: what the fleets in orbit hold.
    pub fleet_contents: bool,
    /// `See_Credit_Income`: the planet's income.
    pub credit_income: bool,
    /// `See_Special_Structures`: the structures built.
    pub special_structures: bool,
    /// `See_Current_Production`: what the planet is building.
    pub current_production: bool,
    /// `See_Minor_Stealth_Heroes`: stealthy minor heroes (the agents).
    pub minor_stealth: bool,
    /// `See_Major_Stealth_Heroes`: stealthy named heroes.
    pub major_stealth: bool,
}

impl SpyFlags {
    /// Everything (a faction's own and allied planets).
    pub const ALL: SpyFlags = SpyFlags {
        base_levels: true,
        num_fleets: true,
        fleet_contents: true,
        credit_income: true,
        special_structures: true,
        current_production: true,
        minor_stealth: true,
        major_stealth: true,
    };

    /// Flags set in either.
    pub fn union(self, o: SpyFlags) -> SpyFlags {
        SpyFlags {
            base_levels: self.base_levels || o.base_levels,
            num_fleets: self.num_fleets || o.num_fleets,
            fleet_contents: self.fleet_contents || o.fleet_contents,
            credit_income: self.credit_income || o.credit_income,
            special_structures: self.special_structures || o.special_structures,
            current_production: self.current_production || o.current_production,
            minor_stealth: self.minor_stealth || o.minor_stealth,
            major_stealth: self.major_stealth || o.major_stealth,
        }
    }

    /// True if any flag is set.
    pub fn any(self) -> bool {
        self != SpyFlags::default()
    }
}

/// A `System_Spy_Ability` of a unit or structure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpyAbility {
    /// What it reveals.
    pub see: SpyFlags,
    /// `Duration_In_Secs`; negative = for as long as the holder is there.
    pub duration: f32,
    /// `Causes_Despawn`: using it removes the holder (the field agent).
    pub despawn: bool,
    /// `Galactic_Influence_Range` (galaxy map units, of the ability or the object): planets this close to the holder's planet are spied too.
    pub range: f32,
    /// `Hyperspace_Fleet_Reveal_Range`: fleets in hyperspace this close to the holder's planet are revealed.
    pub fleet_reveal_range: f32,
}

/// The `Siphon_Credits_Ability` of an agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SiphonAbility {
    /// `Min_Siphon_Percentage` (fraction of the planet's income).
    pub min: f32,
    /// `Max_Siphon_Percentage`.
    pub max: f32,
    /// `Siphon_Duration_In_Cycles` (fiscal cycles, [`World::fiscal_cycle_seconds`]).
    pub cycles: u32,
}

/// A credit siphon running at a planet.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ActiveSiphon {
    /// Faction that receives the credits.
    pub by: FactionId,
    /// Agent type that started it.
    pub agent: UnitTypeId,
    /// Share of the planet's income taken during the current cycle.
    pub share: f32,
    /// Cycles left, the current one included.
    pub cycles_left: u32,
    /// Game time at which the current cycle ends.
    pub cycle_end: f64,
}

/// A timed spy report: `faction` sees `planet` with `see` until `until` (a field agent's `Duration_In_Secs`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpyReport {
    /// Faction that receives the report.
    pub faction: FactionId,
    /// Spied planet.
    pub planet: PlanetId,
    /// Game time the report ends.
    pub until: f64,
    /// What it reveals.
    pub see: SpyFlags,
}

fn num(fields: &HashMap<String, String>, k: &str) -> Option<f32> {
    fields.get(k).and_then(|v| v.trim().parse().ok())
}

/// The `System_Spy_Ability` entries of an object's abilities, merged (flags of any, the longest ranges); `fields` are the object's own tags,
/// which carry `Galactic_Influence_Range` and `Hyperspace_Fleet_Reveal_Range` when the ability does not (`Spy_Center`, Dodonna, the MTC).
pub(crate) fn spy_ability(subs: &[SubAbility], fields: &HashMap<String, String>) -> Option<SpyAbility> {
    let mut out: Option<SpyAbility> = None;
    for a in subs.iter().filter(|a| a.kind.eq_ignore_ascii_case("system_spy_ability")) {
        let yes = |k: &str| a.fields.get(k).is_some_and(|v| parse_bool(v));
        let see = SpyFlags {
            base_levels: yes("see_base_levels"),
            num_fleets: yes("see_num_fleets"),
            fleet_contents: yes("see_fleet_contents"),
            credit_income: yes("see_credit_income"),
            special_structures: yes("see_special_structures"),
            current_production: yes("see_current_production"),
            minor_stealth: yes("see_minor_stealth_heroes"),
            major_stealth: yes("see_major_stealth_heroes"),
        };
        let range = |k: &str| num(&a.fields, k).or_else(|| num(fields, k)).unwrap_or(0.0).max(0.0);
        let s = SpyAbility {
            see,
            duration: num(&a.fields, "duration_in_secs").unwrap_or(-1.0),
            despawn: yes("causes_despawn"),
            range: range("galactic_influence_range"),
            fleet_reveal_range: range("hyperspace_fleet_reveal_range"),
        };
        out = Some(match out {
            None => s,
            Some(o) => SpyAbility {
                see: o.see.union(s.see),
                duration: if o.duration < 0.0 || s.duration < 0.0 { -1.0 } else { o.duration.max(s.duration) },
                despawn: o.despawn && s.despawn,
                range: o.range.max(s.range),
                fleet_reveal_range: o.fleet_reveal_range.max(s.fleet_reveal_range),
            },
        });
    }
    out
}

fn siphon_ability(subs: &[SubAbility]) -> Option<SiphonAbility> {
    let a = subs.iter().find(|a| a.kind.eq_ignore_ascii_case("siphon_credits_ability"))?;
    let min = num(&a.fields, "min_siphon_percentage").unwrap_or(0.0).max(0.0);
    Some(SiphonAbility {
        min,
        max: num(&a.fields, "max_siphon_percentage").unwrap_or(min).max(min),
        cycles: num(&a.fields, "siphon_duration_in_cycles").unwrap_or(0.0).max(0.0) as u32,
    })
}

impl World {
    /// Adds the agents (`HeroCompany` objects on the heroes tab whose `Company_Units` siphon credits or spy) to the unit catalog. Cost, time,
    /// tech, population, requirements, locks and limits are the company's; abilities and stealth come from its company units; the hyperspace speed
    /// is the `Company_Transport_Unit`'s.
    pub(crate) fn add_agents(&mut self, data: &GameData) {
        let names: Vec<String> = data.objects.of_kind("herocompany").map(|o| o.name.clone()).collect();
        for name in names {
            if self.unit_index.contains_key(&name.to_ascii_lowercase()) {
                continue;
            }
            let Some(r) = data.objects.resolve(&name) else { continue };
            // Combat hero teams (Boba Fett, IG-88 ... with their own spy abilities) are `Tech_Level` 99 story objects, not agents.
            let non_combat = r.list("categorymask").iter().any(|c| c.trim().eq_ignore_ascii_case("noncombathero"));
            if !r.flag("build_tab_heroes").unwrap_or(false) || !non_combat {
                continue;
            }
            let members = r.list("company_units");
            let subs: Vec<SubAbility> = members.iter().flat_map(|m| data.objects.sub_abilities(m)).collect();
            let (siphon, spy) = (siphon_ability(&subs), spy_ability(&subs, &HashMap::new()));
            if siphon.is_none() && spy.is_none() {
                continue;
            }
            let affiliation: Vec<String> = r.list("affiliation").iter().map(|a| a.trim().to_ascii_lowercase()).filter(|a| !a.is_empty()).collect();
            let speed = r.string("company_transport_unit").and_then(|t| data.objects.space_unit(&t)).map_or(0.0, |t| t.hyperspace_speed);
            let i32_of = |k: &str| r.num::<i32>(k).unwrap_or(0);
            let cost = r.num::<f32>("build_cost_credits").unwrap_or(0.0);
            let (groups, groups_ok) = self.resolve_requirement(&parse_requirement(r.get("required_special_structures").unwrap_or("")));
            let (planets, planets_ok) = self.resolve_planets(&name_list(r.get("required_planets").unwrap_or("")));
            let population = i32_of("population_value");
            let unit = UnitType {
                name: r.name().to_string(),
                text_id: r.string("text_id"),
                role: UnitRole::Agent,
                category: alamo_data::ShipCategory::Other,
                category_mask: crate::world::category_bits(&r.list("categorymask")),
                buildable: cost > 0.0 && !affiliation.is_empty() && groups_ok && planets_ok,
                affiliation,
                combat: CombatStats::default(),
                hyperspace_speed: speed,
                cost,
                build_time: r.num::<f32>("build_time_seconds").unwrap_or(0.0),
                tech: i32_of("tech_level"),
                gui_row: r.num::<i32>("gui_row").unwrap_or(1),
                population,
                space_population: population,
                req_starbase: i32_of("required_star_base_level"),
                req_structures: groups,
                req_planets: planets,
                locked: r.flag("build_initially_locked").unwrap_or(false),
                limit_current: r.num("build_limit_current_per_player"),
                limit_lifetime: r.num("build_limit_lifetime_per_player"),
                hero: false,
                prod_mods: Vec::new(),
                planet_destroyer: false,
                galactic_stealth: subs.iter().any(|a| a.kind.eq_ignore_ascii_case("galactic_stealth_ability")),
                system_spy: spy.is_some(),
                spy,
                siphon,
                bonuses: Vec::new(),
                req_ground_base: i32_of("required_ground_base_level"),
                upkeep: 0.0,
                income_pct: 0.0,
                gamble: None,
                attrition_credits: 0,
                ai_power: 0.0,
                carries: Vec::new(),
                members: Vec::new(),
            };
            self.unit_index.insert(name.to_ascii_lowercase(), self.units.len() as UnitTypeId);
            self.units.push(unit);
        }
    }

    /// True for an agent (see [`UnitRole::Agent`]).
    pub fn is_agent(&self, unit: UnitTypeId) -> bool {
        self.units.get(unit as usize).is_some_and(|u| u.role == UnitRole::Agent)
    }

    /// True if the fleet holds anything but agents: only such fleets fight, blockade, contest and capture planets.
    pub fn is_combat_fleet(&self, fleet: &Fleet) -> bool {
        fleet.units.keys().any(|&u| !self.is_agent(u))
    }

    /// The planet's ground base level. `GroundBases.xml`: ground bases are no longer built, the level is kept in step with the number of
    /// special structures built; so it is the number of ground (non-orbital) structures, capped at [`World::ground_base_max`].
    pub fn ground_base_level(&self, galaxy: &Galaxy, planet: PlanetId) -> i32 {
        let Some(ps) = galaxy.planets.get(planet as usize) else { return 0 };
        let n = ps.structures.iter().filter(|&&s| self.structures.get(s as usize).is_some_and(|st| !st.orbital)).count() as i32;
        n.min(self.ground_base_max)
    }

    /// The siphon agent of `fleet` that could start now at its planet: the fleet orbits a planet owned by a faction hostile to its owner, no
    /// siphon runs there yet, and the fleet holds an agent with a `Siphon_Credits_Ability`.
    pub fn siphon_ready(&self, galaxy: &Galaxy, fleet: FleetId) -> Result<UnitTypeId, CommandError> {
        let f = galaxy.fleets.get(&fleet).ok_or(CommandError::Invalid)?;
        let agent = f.units.iter().find(|&(&u, &n)| n > 0 && self.units[u as usize].siphon.is_some()).map(|(&u, _)| u).ok_or(CommandError::NotAvailable)?;
        let FleetLocation::AtPlanet(p) = f.location else { return Err(CommandError::WrongState) };
        let ps = &galaxy.planets[p as usize];
        if !ps.owner.is_some_and(|o| galaxy.hostile(self, f.owner, o)) {
            return Err(CommandError::RequirementsNotMet("an enemy planet".into()));
        }
        // The Remake AI's `Suitable_Smuggler_Target` (OFFENSIVEGALATICEQUATIONS.XML): "Can't have multiple smugglers on a target".
        if ps.siphon.is_some() {
            return Err(CommandError::RequirementsNotMet("no other siphon at the planet".into()));
        }
        Ok(agent)
    }
}

/// Removes one unit of `unit` from `fleet`; an empty fleet disappears.
fn use_up(galaxy: &mut Galaxy, fleet: FleetId, unit: UnitTypeId) {
    let Some(f) = galaxy.fleets.get_mut(&fleet) else { return };
    if let Some(n) = f.units.get_mut(&unit) {
        *n -= 1;
        if *n == 0 {
            f.units.remove(&unit);
        }
    }
    if f.units.is_empty() {
        galaxy.fleets.remove(&fleet);
        galaxy.hyperspace.remove(&fleet);
        galaxy.dest_slots.remove(&fleet);
    }
    galaxy.sync_ships();
}

impl Sim {
    /// [`Command::ActivateAgent`]: the fleet's siphon agent starts siphoning the planet it orbits and is used up.
    pub(crate) fn activate_siphon(&mut self, faction: FactionId, fleet: FleetId) -> Result<CommandOk, CommandError> {
        let world = self.world.clone();
        let g = &mut self.galaxy;
        if g.fleets.get(&fleet).is_none_or(|f| f.owner != faction) {
            return Err(CommandError::Invalid);
        }
        let agent = world.siphon_ready(g, fleet)?;
        let p = g.fleets[&fleet].planet().expect("checked in orbit");
        let s = world.units[agent as usize].siphon.expect("siphon agent");
        if s.cycles == 0 {
            return Err(CommandError::NotAvailable);
        }
        let share = s.min + (s.max - s.min) * g.rng.unit();
        g.planets[p as usize].siphon = Some(ActiveSiphon { by: faction, agent, share, cycles_left: s.cycles, cycle_end: g.time + f64::from(world.fiscal_cycle_seconds) });
        use_up(g, fleet, agent);
        world.refresh_economy(g);
        self.queued_events.push(Event::AgentUsed { faction, planet: p, unit: agent });
        Ok(CommandOk::Done)
    }

    /// Runs the agents once per tick: siphon cycles (a new share each cycle, the end after the last), field agents that reached a hostile
    /// planet, and the end of spy reports.
    pub(crate) fn run_agents(&mut self, events: &mut Vec<Event>) {
        let world = self.world.clone();
        let w = &*world;
        let g = &mut self.galaxy;
        let now = g.time;
        for p in 0..g.planets.len() {
            let Some(mut s) = g.planets[p].siphon else { continue };
            // The siphon stops when the planet is no longer an enemy's (captured, allied, destroyed or unowned).
            let target = g.planets[p].owner.filter(|&o| !g.planets[p].destroyed && g.hostile(w, s.by, o));
            if target.is_none() {
                g.planets[p].siphon = None;
                continue;
            }
            while now >= s.cycle_end && s.cycles_left > 0 {
                s.cycles_left -= 1;
                s.cycle_end += f64::from(w.fiscal_cycle_seconds);
                if s.cycles_left > 0 {
                    let a = w.units[s.agent as usize].siphon.unwrap_or(SiphonAbility { min: s.share, max: s.share, cycles: 0 });
                    s.share = a.min + (a.max - a.min) * g.rng.unit();
                }
            }
            g.planets[p].siphon = (s.cycles_left > 0).then_some(s);
        }

        // Field agents (a spy that is used up) report on the hostile planet they orbit.
        let mut used: Vec<(FleetId, UnitTypeId, PlanetId, FactionId, SpyAbility)> = Vec::new();
        for f in g.fleets.values() {
            let FleetLocation::AtPlanet(p) = f.location else { continue };
            if !g.planets[p as usize].owner.is_some_and(|o| g.hostile(w, f.owner, o)) {
                continue;
            }
            let reported = |faction: FactionId| g.spy_reports.iter().any(|r| r.faction == faction && r.planet == p) || used.iter().any(|u| u.3 == faction && u.2 == p);
            if reported(f.owner) {
                continue;
            }
            if let Some((&u, spy)) = f.units.iter().filter(|&(_, &n)| n > 0).find_map(|(u, _)| w.units[*u as usize].spy.filter(|s| s.despawn && w.is_agent(*u)).map(|s| (u, s))) {
                used.push((f.id, u, p, f.owner, spy));
            }
        }
        for (fleet, unit, planet, faction, spy) in used {
            let until = if spy.duration < 0.0 { f64::MAX } else { now + f64::from(spy.duration) };
            g.spy_reports.push(SpyReport { faction, planet, until, see: spy.see });
            use_up(g, fleet, unit);
            events.push(Event::AgentUsed { faction, planet, unit });
        }
        g.spy_reports.retain(|r| r.until > now);
    }
}
