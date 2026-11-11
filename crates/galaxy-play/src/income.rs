//! Where a faction's money comes from and goes to: the sources and sinks the simulation adds up for its income figure
//! (`galaxy_sim::World::planet_income`, `refresh_economy`), as lines that sum to it.

use galaxy_sim::{FactionId, FleetLocation, Galaxy, PlanetId, World};

/// One source or sink of a month's income (credits per fiscal cycle; negative for upkeep and siphons).
#[derive(Debug, Clone, PartialEq)]
pub struct IncomeLine {
    pub label: String,
    pub credits: f64,
}

/// The parts of one planet's income: `base * (1 + pct) + abs`, split by where the percentages and absolute amounts come from.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct PlanetParts {
    base: f64,
    /// `base` times the percentages of buildings and the star base.
    building_pct: f64,
    /// `base` times the market swings (a trade hub's or a gambling hero's).
    swing_pct: f64,
    /// `base` times the percentages of the owner's ships in orbit.
    ship_pct: f64,
    /// Absolute income of buildings and the star base, and their upkeep.
    building_income: f64,
    building_upkeep: f64,
}

impl PlanetParts {
    fn total(&self) -> f64 {
        self.base + self.building_pct + self.swing_pct + self.ship_pct + self.building_income + self.building_upkeep
    }
}

/// The same sums as `World::planet_income`, kept apart.
fn planet_parts(world: &World, galaxy: &Galaxy, planet: PlanetId) -> PlanetParts {
    let ps = &galaxy.planets[planet as usize];
    let info = &world.planets[planet as usize];
    let base = f64::from(info.credit_value);
    let mut p = PlanetParts { base, ..Default::default() };
    let mut abs = |v: f32| {
        if v >= 0.0 {
            p.building_income += f64::from(v);
        } else {
            p.building_upkeep += f64::from(v);
        }
    };
    let mut building_pct = 0.0f32;
    for &s in &ps.structures {
        let st = &world.structures[s as usize];
        building_pct += st.income_pct;
        abs(st.income_abs);
        if info.mining_colony {
            building_pct += st.income_pct_mining;
            abs(st.income_abs_mining);
        }
    }
    if let Some(sb) = ps.starbase {
        let sb = &world.starbases[sb as usize];
        building_pct += sb.income_pct;
        abs(sb.income_abs);
    }
    let mut ship_pct = 0.0f32;
    if let Some(owner) = ps.owner {
        for f in galaxy.fleets.values() {
            if f.owner == owner && f.location == FleetLocation::AtPlanet(planet) {
                for (&u, &c) in &f.units {
                    ship_pct += world.units[u as usize].income_pct * c as f32;
                }
            }
        }
    }
    p.building_pct = base * f64::from(building_pct);
    p.swing_pct = base * f64::from(ps.gamble + ps.hero_gamble);
    p.ship_pct = base * f64::from(ship_pct);
    p
}

/// Adds `credits` under `label` unless it is nothing.
fn push(lines: &mut Vec<IncomeLine>, label: impl Into<String>, credits: f64) {
    if credits.abs() > 1e-9 {
        lines.push(IncomeLine { label: label.into(), credits });
    }
}

fn planet_lines(p: &PlanetParts, lines: &mut Vec<IncomeLine>, base_label: &str) {
    push(lines, base_label, p.base);
    push(lines, "Building and star base bonuses", p.building_pct);
    push(lines, "Market swings", p.swing_pct);
    push(lines, "Ship bonuses in orbit", p.ship_pct);
    push(lines, "Building income", p.building_income);
    push(lines, "Building upkeep", p.building_upkeep);
}

/// What one planet yields its owner per month, line by line (sums to `World::planet_income`).
pub fn planet_income_lines(world: &World, galaxy: &Galaxy, planet: PlanetId) -> Vec<IncomeLine> {
    let mut lines = Vec::new();
    planet_lines(&planet_parts(world, galaxy, planet), &mut lines, "Planet value");
    lines
}

/// A faction's income per month, line by line: what its planets yield (split as in [`planet_income_lines`]), the share enemy agents
/// siphon off or its own agents siphon in, ship upkeep and ship income, and the faction's `Credits_Accumulation_Factor` on all of it.
/// The lines sum to the faction's `income_per_week` (the same sums as `refresh_economy`).
pub fn faction_income_lines(world: &World, galaxy: &Galaxy, faction: FactionId) -> Vec<IncomeLine> {
    let n = world.factions.len();
    let mut sum = PlanetParts::default();
    let mut planets = 0;
    let (mut siphoned_off, mut siphoned_in) = (0.0f64, 0.0f64);
    for (i, ps) in galaxy.planets.iter().enumerate() {
        let Some(owner) = ps.owner else { continue };
        let parts = (owner == faction || ps.siphon.is_some_and(|s| s.by == faction)).then(|| planet_parts(world, galaxy, i as PlanetId));
        // The sim adds the planet's income as it computes it in f32 (`planet_income`) and takes the siphon from that.
        let income = parts.map(|_| f64::from(world.planet_income(galaxy, i as PlanetId)));
        if owner == faction {
            let p = parts.unwrap_or_default();
            planets += 1;
            sum.base += p.base;
            sum.building_pct += p.building_pct;
            sum.swing_pct += p.swing_pct;
            sum.ship_pct += p.ship_pct;
            sum.building_income += p.building_income;
            sum.building_upkeep += p.building_upkeep;
            // The f32 rounding of the planet's figure, so the lines add up to what the sim adds.
            sum.base += income.unwrap_or(0.0) - p.total();
        }
        if let (Some(s), Some(income)) = (ps.siphon.filter(|s| (s.by as usize) < n), income)
            && income > 0.0
        {
            let taken = income * f64::from(s.share);
            if owner == faction {
                siphoned_off -= taken;
            }
            if s.by == faction {
                siphoned_in += taken;
            }
        }
    }
    let (mut ship_upkeep, mut ship_income) = (0.0f64, 0.0f64);
    for f in galaxy.fleets.values().filter(|f| f.owner == faction) {
        for (&u, &c) in &f.units {
            let v = f64::from(world.units[u as usize].upkeep * c as f32);
            if v < 0.0 {
                ship_upkeep += v;
            } else {
                ship_income += v;
            }
        }
    }
    let mut lines = Vec::new();
    planet_lines(&sum, &mut lines, &format!("Planet values ({planets} planets)"));
    push(&mut lines, "Siphoned off by enemy agents", siphoned_off);
    push(&mut lines, "Siphoned from enemy planets", siphoned_in);
    push(&mut lines, "Ship upkeep", ship_upkeep);
    push(&mut lines, "Ship income", ship_income);
    let factor = f64::from(world.factions.get(faction as usize).map_or(1.0, |f| f.credits_factor));
    if (factor - 1.0).abs() > 1e-9 {
        let subtotal: f64 = lines.iter().map(|l| l.credits).sum();
        push(&mut lines, format!("Faction factor x{factor:.2}"), subtotal * (factor - 1.0));
    }
    lines
}

/// Whole credits for each line that add up to `total` exactly: each line rounded down, the credits left over handed to the lines
/// that lost the most (largest remainder), so the hover card's lines sum to the figure the header shows.
pub fn whole_credits(lines: &[IncomeLine], total: i64) -> Vec<i64> {
    let mut out: Vec<i64> = lines.iter().map(|l| l.credits.floor() as i64).collect();
    let mut left = total - out.iter().sum::<i64>();
    let mut by_rest: Vec<usize> = (0..lines.len()).collect();
    by_rest.sort_by(|&a, &b| (lines[b].credits - lines[b].credits.floor()).total_cmp(&(lines[a].credits - lines[a].credits.floor())));
    let mut k = 0;
    while left != 0 && !lines.is_empty() {
        let i = by_rest[k % by_rest.len()];
        let step = left.signum();
        out[i] += step;
        left -= step;
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_credits_add_up_to_the_shown_figure() {
        let l = |c: f64| IncomeLine { label: String::new(), credits: c };
        let lines = [l(1000.4), l(250.4), l(-300.3), l(15.5)];
        // 965.999...: the header truncates to 965.
        for total in [965, 966, -3, 0] {
            assert_eq!(whole_credits(&lines, total).iter().sum::<i64>(), total);
        }
        assert_eq!(whole_credits(&lines, 966), vec![1000, 250, -300, 16]);
        assert!(whole_credits(&[], 0).is_empty());
    }
}
