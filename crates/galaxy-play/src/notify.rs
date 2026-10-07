//! Turns simulation events into player notifications.

use bevy::prelude::Resource;
use galaxy_sim::{CommandError, Event, FactionId, Galaxy, PlanetId, ProductionKind, World};

use crate::catalog::PlayCatalog;

/// Category of a notification (the UI picks icons and colors from it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// A production item finished.
    Construction,
    /// A battle involving the player waits for a decision.
    BattlePending,
    /// A battle involving the player was settled.
    BattleResolved,
    /// The player took a planet.
    PlanetCaptured,
    /// The player lost a planet.
    PlanetLost,
    /// A fleet reached its destination.
    FleetArrived,
    /// Weekly income report.
    Income,
    /// A faction was wiped out or the game ended.
    Campaign,
    /// A refused order.
    Error,
}

/// A line in the notification ticker.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// Unique id (for dismissing).
    pub id: u64,
    /// Category.
    pub kind: NotificationKind,
    /// English text.
    pub text: String,
    /// Planet the camera could jump to.
    pub planet: Option<PlanetId>,
    /// Real time (seconds since app start) it was created at.
    pub created: f64,
}

/// The live notification list, newest last.
#[derive(Resource, Debug, Default)]
pub struct Notifications {
    /// Current entries.
    pub list: Vec<Notification>,
    next_id: u64,
}

impl Notifications {
    /// Appends a notification.
    pub fn push(&mut self, kind: NotificationKind, text: String, planet: Option<PlanetId>, now: f64) {
        self.next_id += 1;
        self.list.push(Notification { id: self.next_id, kind, text, planet, created: now });
        if self.list.len() > 64 {
            self.list.remove(0);
        }
    }

    /// Drops entries older than `max_age` real seconds (battle prompts are kept by the dialog).
    pub fn expire(&mut self, now: f64, max_age: f64) {
        self.list.retain(|n| now - n.created < max_age);
    }
}

/// Wording of a refused order.
pub fn describe_error(e: &CommandError) -> String {
    match e {
        CommandError::Invalid => "Invalid order.".into(),
        CommandError::NotOwner => "You do not own this planet.".into(),
        CommandError::Eliminated => "Your faction has been eliminated.".into(),
        CommandError::InsufficientCredits => "Not enough credits.".into(),
        CommandError::RequirementsNotMet(r) => format!("Requirement not met: {r}."),
        CommandError::PopulationCap => "Population cap reached.".into(),
        CommandError::BuildLimit => "Build limit reached.".into(),
        CommandError::NoSlot => "No free slot or queue position.".into(),
        CommandError::NotAvailable => "Not available to your faction.".into(),
        CommandError::NoPath => "No hyperspace route to that planet.".into(),
        CommandError::WrongState => "The fleet cannot do that right now (in hyperspace or locked in battle).".into(),
    }
}

/// What a simulation event means for the player, if anything.
pub fn describe_event(
    world: &World,
    names: &PlayCatalog,
    galaxy: &Galaxy,
    human: Option<FactionId>,
    event: &Event,
) -> Option<(NotificationKind, String, Option<PlanetId>)> {
    let planet = |p: PlanetId| world.planets[p as usize].display_name.clone();
    let me = |f: FactionId| human == Some(f);
    match event {
        Event::ConstructionComplete { faction, planet: p, kind } if me(*faction) => {
            let what = match kind {
                ProductionKind::Unit(u) => names.unit(*u).to_owned(),
                ProductionKind::Structure(s) => names.structure(*s).to_owned(),
                ProductionKind::Starbase(b) => names.starbase(*b).to_owned(),
                ProductionKind::Tech(t) => format!("Tech level {}", world.factions[*faction as usize].start_tech + *t as i32 + 1),
            };
            Some((NotificationKind::Construction, format!("{what} completed at {}.", planet(*p)), Some(*p)))
        }
        Event::BattlePending(b) if human.is_some_and(|h| b.factions.contains(&h)) => {
            Some((NotificationKind::BattlePending, format!("Battle at {}!", planet(b.planet)), Some(b.planet)))
        }
        Event::BattleResolved { planet: p, winner, losses, .. } if human.is_some_and(|h| losses.iter().any(|(f, _)| *f == h) || *winner == Some(h)) => {
            let lost = losses.iter().find(|(f, _)| Some(*f) == human).map_or(0, |&(_, n)| n);
            let verdict = match winner {
                Some(w) if me(*w) => "Victory".to_owned(),
                Some(w) => format!("Defeat by {}", names.faction(*w)),
                None => "Draw".to_owned(),
            };
            Some((NotificationKind::BattleResolved, format!("{verdict} at {} ({lost} ships lost).", planet(*p)), Some(*p)))
        }
        Event::PlanetCaptured { planet: p, old_owner, new_owner } => {
            if me(*new_owner) {
                Some((NotificationKind::PlanetCaptured, format!("{} captured.", planet(*p)), Some(*p)))
            } else if old_owner.is_some_and(me) {
                Some((NotificationKind::PlanetLost, format!("{} was captured by {}!", planet(*p), names.faction(*new_owner)), Some(*p)))
            } else {
                None
            }
        }
        Event::FleetArrived { fleet, planet: p } => {
            let f = galaxy.fleets.get(fleet)?;
            me(f.owner).then(|| (NotificationKind::FleetArrived, format!("Fleet arrived at {}.", planet(*p)), Some(*p)))
        }
        Event::WeekEnded { week } => {
            let h = human?;
            let fs = &galaxy.factions[h as usize];
            Some((
                NotificationKind::Income,
                format!("Week {week}: income {:+.0} credits/week, treasury {:.0}.", fs.income_per_week, fs.credits),
                None,
            ))
        }
        Event::FactionEliminated { faction } => Some((NotificationKind::Campaign, format!("{} has been eliminated.", names.faction(*faction)), None)),
        Event::Victory { faction } => Some((NotificationKind::Campaign, format!("{} wins the campaign.", names.faction(*faction)), None)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_is_bounded_and_expires() {
        let mut n = Notifications::default();
        for i in 0..100 {
            n.push(NotificationKind::Income, format!("n{i}"), None, i as f64);
        }
        assert_eq!(n.list.len(), 64);
        n.expire(99.0, 5.0);
        assert!(n.list.iter().all(|x| x.created > 94.0));
    }
}
