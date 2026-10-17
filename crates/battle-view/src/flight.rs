//! Lively flight, view only: ships bank into their turns and pitch with their climbs and dives, both from the simulated motion, and
//! small craft weave and bob a little around their simulated place, so a squadron at rest is no longer a rigid grid.
//!
//! Everything is a deterministic function of the snapshots, the ship id and the battle clock: bank and pitch advance once per
//! simulation step (not per rendered frame), the weave reads the battle time. Nothing is random.
//!
//! Reference, the Remake in YouTube video wkiF9UTI3xo (frames at 10-15 per second) and the user's own video:
//! * TIE fighters in a turning dogfight bank up to about 35 degrees, X-wings 45-60 degrees (15.3 s).
//! * A Venator (`Bank_Turn_Angle` 20, `Max_Rate_Of_Roll` 0.05) rolls 6-8 degrees in a turn at its full rate and is level within 1.5
//!   degrees once the turn is over (513-527 s; the line through its two main engines against a level horizon, static camera).
//! * Squadrons on the move keep rigid rows: 23 tracked TIEs of 4-abreast rows (532 s, static camera, up to 10 s each) move by under
//!   0.8 px RMS against their row neighbours at 20-27 px spacing, the noise of the measurement.
//! * At rest the Remake's fighters never stand still (`Min_Speed` 1): idle squadrons are loose clusters whose fighters face and bank
//!   every way and shift between frames (user video b0703s, b0727s; neighbours 36-51 px apart in b0703s).

use std::f32::consts::TAU;

use battle_sim::TargetClass;
use bevy::prelude::*;

use crate::convert::ship_rotation;

/// Steepest bank of a small craft, in a turn at its full turn rate and top speed (estimate: the steepest bank measured in wkiF9UTI3xo,
/// X-wings at 15.3 s). The data's `Bank_Turn_Angle` is 360 for most fighters, which reads as no limit, and 90 in `Spacecraft_Base`.
const SMALL_CRAFT_MAX_BANK: f32 = 60.0 * std::f32::consts::PI / 180.0;
/// Share of `Bank_Turn_Angle` a bigger ship banks in a turn at its full `Max_Rate_Of_Turn` and top speed (estimate: the Venator,
/// `Bank_Turn_Angle` 20, rolls 6-8 degrees in wkiF9UTI3xo 513-521 s).
const BIG_SHIP_BANK_SHARE: f32 = 0.35;
/// Steepest climb or dive pitch of a ship that is no small craft (estimate: the simulation lets every mobile ship climb at half its top
/// speed; a capital ship with its nose 27 degrees up would look wrong, a few degrees show the climb).
const BIG_SHIP_MAX_PITCH: f32 = 4.0 * std::f32::consts::PI / 180.0;
/// Steepest pitch of a small craft without `Max_Lift` (estimate, between the TIE bomber's 32 and the Y-wing's 30 degrees).
const SMALL_CRAFT_MAX_PITCH: f32 = 30.0 * std::f32::consts::PI / 180.0;
/// `Max_Rate_Of_Roll` (radians per second) where a unit has none (estimates: the fighters' 1.56-4, the frigates' 0.05-0.15).
const SMALL_CRAFT_ROLL_RATE: f32 = 2.0;
const BIG_SHIP_ROLL_RATE: f32 = 0.05;

/// The flight tags of a unit's XML (0 where the data has none).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FlightXml {
    /// `Bank_Turn_Angle`, degrees.
    pub bank_turn_angle: f32,
    /// `Max_Rate_Of_Roll`, radians per second.
    pub max_rate_of_roll: f32,
    /// `Max_Lift`, per frame like `Max_Speed`.
    pub max_lift: f32,
    /// `Max_Speed`, per frame.
    pub max_speed: f32,
}

/// How a ship type leans in flight, from its unit XML and its simulation definition.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FlightLook {
    /// Steepest bank in radians, in a turn at `turn_rate` and `max_speed`.
    pub max_bank: f32,
    /// `Max_Rate_Of_Roll`: how fast the bank changes, radians per second.
    pub roll_rate: f32,
    /// Steepest nose-up or nose-down pitch in radians.
    pub max_pitch: f32,
    /// Turn rate of the simulation (`Max_Rate_Of_Turn`, radians per second); the pitch changes at most this fast too (estimate: the data
    /// has no pitch rate).
    pub turn_rate: f32,
    /// Top speed of the simulation, units per second.
    pub max_speed: f32,
    /// Fighters, bombers and squadron members: they bank steeply and weave.
    pub small: bool,
}

impl FlightLook {
    /// The look of a ship type: its class, whether it flies in squadrons, its XML flight tags and the simulation's turn rate (radians per
    /// second) and top speed (units per second).
    pub fn new(class: TargetClass, squadron_member: bool, xml: FlightXml, turn_rate: f32, max_speed: f32) -> Self {
        let small = squadron_member || matches!(class, TargetClass::Fighter | TargetClass::Bomber);
        let bank_data = xml.bank_turn_angle.max(0.0).to_radians();
        let (max_bank, max_pitch, fallback_roll) = if small {
            // Inferred: `Max_Lift` is the climb speed in the units of `Max_Speed` (TIE fighter 2.8 of 4, about 35 degrees).
            let pitch = if xml.max_lift > 0.0 && xml.max_speed > 0.0 { xml.max_lift.atan2(xml.max_speed) } else { SMALL_CRAFT_MAX_PITCH };
            (bank_data.min(SMALL_CRAFT_MAX_BANK), pitch, SMALL_CRAFT_ROLL_RATE)
        } else {
            (bank_data * BIG_SHIP_BANK_SHARE, BIG_SHIP_MAX_PITCH, BIG_SHIP_ROLL_RATE)
        };
        let mobile = turn_rate > 0.0 && max_speed > 0.0 && !matches!(class, TargetClass::Structure);
        Self {
            max_bank: if mobile { max_bank } else { 0.0 },
            roll_rate: if xml.max_rate_of_roll > 0.0 { xml.max_rate_of_roll } else { fallback_roll },
            max_pitch: if mobile { max_pitch } else { 0.0 },
            turn_rate: turn_rate.max(0.0),
            max_speed: max_speed.max(0.0),
            small,
        }
    }

    /// Bank the motion asks for (radians, positive: right wing down): into the turn, in proportion to the turn rate and to the speed
    /// (estimate of the engine's rule): the full `max_bank` at the full turn rate and top speed, none flying straight or turning on the spot.
    pub fn bank_for(&self, m: Motion) -> f32 {
        if self.turn_rate <= 0.0 || self.max_speed <= 0.0 {
            return 0.0;
        }
        let turn = (m.yaw_rate / self.turn_rate).clamp(-1.0, 1.0);
        let pace = (m.speed / self.max_speed).clamp(0.0, 1.0);
        // The sim's yaw grows to the left: a left turn lowers the left wing.
        -self.max_bank * turn * pace
    }

    /// Pitch the motion asks for (radians, positive: nose up): the climb angle of the flight path within `max_pitch`, the speed taken as
    /// at least half the top speed (a craft that rises on the spot does not stand on its tail).
    pub fn pitch_for(&self, m: Motion) -> f32 {
        if self.max_speed <= 0.0 {
            return 0.0;
        }
        m.climb.atan2(m.speed.max(self.max_speed * 0.5)).clamp(-self.max_pitch, self.max_pitch)
    }

    /// How still a craft flies at `speed`: 1 at rest, 0 at top speed (the weave keeps `WEAVE_IN_MOTION` of its drift then).
    pub fn calm(&self, speed: f32) -> f32 {
        if self.max_speed <= 0.0 { 1.0 } else { 1.0 - (speed / self.max_speed).clamp(0.0, 1.0) }
    }
}

/// What a ship did over one simulation step.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Motion {
    /// Turn rate in radians per second (positive: to the left, like the simulation's yaw).
    pub yaw_rate: f32,
    /// Speed in the battle plane, units per second.
    pub speed: f32,
    /// Climb speed, units per second (negative: diving).
    pub climb: f32,
}

impl Motion {
    /// The motion from one snapshot to the next, `dt` seconds later (simulation space: z up).
    pub fn between(from: battle_sim::Vec3, from_yaw: f32, to: battle_sim::Vec3, to_yaw: f32, dt: f32) -> Self {
        let dt = dt.max(1e-3);
        let turn = (to_yaw - from_yaw + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
        Self { yaw_rate: turn / dt, speed: (to.x - from.x).hypot(to.y - from.y) / dt, climb: (to.z - from.z) / dt }
    }
}

/// Bank and pitch with the speeds they change at.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Lean {
    bank: f32,
    bank_rate: f32,
    pitch: f32,
    pitch_rate: f32,
}

impl Lean {
    /// One simulation step towards `(bank, pitch)`.
    fn step(self, look: &FlightLook, target: (f32, f32), dt: f32) -> Self {
        let (bank, bank_rate) = ease(self.bank, self.bank_rate, target.0, look.roll_rate, look.max_bank, dt);
        let (pitch, pitch_rate) = ease(self.pitch, self.pitch_rate, target.1, look.turn_rate, look.max_pitch, dt);
        Self { bank, bank_rate, pitch, pitch_rate }
    }
}

/// One step of a critically damped approach to `target` (eased at both ends, no overshoot from rest), its speed held to `max_rate` and its
/// value to `limit`. The stiffness lets the full sweep take about the time `max_rate` allows (estimate, clamped so a 30 Hz step stays stable).
fn ease(x: f32, v: f32, target: f32, max_rate: f32, limit: f32, dt: f32) -> (f32, f32) {
    if limit <= 0.0 || max_rate <= 0.0 {
        return (0.0, 0.0);
    }
    let w = (4.0 * max_rate / limit).clamp(0.5, 12.0);
    let a = w * w * (target - x) - 2.0 * w * v;
    let v = (v + a * dt).clamp(-max_rate, max_rate);
    ((x + v * dt).clamp(-limit, limit), v)
}

/// How far a ship has caught up after a clock jump: steps beyond this are dropped (the lean has long settled by then).
const CATCH_UP_STEPS: u64 = 60;

/// Bank and pitch of one ship. They advance once per simulation step towards what the step's motion asks for, so the result does not
/// depend on the frame rate; a frame shows the value between the step and the next.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Attitude {
    /// Simulation step of `now`.
    tick: u64,
    /// The lean at `tick` and one step later.
    now: Lean,
    ahead: Lean,
    /// What the latest known motion asks for: (bank, pitch).
    target: (f32, f32),
    /// Speed of the latest known motion (units per second).
    speed: f32,
}

impl Attitude {
    /// A ship that appears level at step `tick`.
    pub fn new(tick: u64) -> Self {
        Self { tick, ..Self::default() }
    }

    /// Moves to step `tick`, whose motion to the next step is `motion` (`None` while the next snapshot is not there). Steps the clock
    /// skipped (several in one frame) repeat the last known motion.
    pub fn update(&mut self, tick: u64, look: &FlightLook, motion: Option<Motion>, dt: f32) {
        if tick > self.tick {
            let mut lean = self.ahead;
            for _ in 1..(tick - self.tick).min(CATCH_UP_STEPS) {
                lean = lean.step(look, self.target, dt);
            }
            self.now = lean;
        } else if tick < self.tick {
            self.now = Lean::default();
        }
        self.tick = tick;
        if let Some(m) = motion {
            self.target = (look.bank_for(m), look.pitch_for(m));
            self.speed = m.speed;
        }
        self.ahead = self.now.step(look, self.target, dt);
    }

    /// Speed of the latest known motion: a frame without the next snapshot keeps it (the weave must not jump).
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// `(bank, pitch)` in radians at the fraction `alpha` of the way to the next step (bank positive: right wing down; pitch positive:
    /// nose up).
    pub fn at(&self, alpha: f32) -> (f32, f32) {
        let a = alpha.clamp(0.0, 1.0);
        (self.now.bank + (self.ahead.bank - self.now.bank) * a, self.now.pitch + (self.ahead.pitch - self.now.pitch) * a)
    }
}

/// Rotation of a ship heading `yaw` (simulation radians) with its nose `pitch` up and banked `bank` (positive: right wing down).
pub fn attitude_rotation(yaw: f32, pitch: f32, bank: f32) -> Quat {
    // The model's bow is +Z, its left +X and its top +Y; a turn about +X lowers the bow.
    ship_rotation(yaw) * Quat::from_rotation_x(-pitch) * Quat::from_rotation_z(bank)
}

/// Drift of a small craft at rest to either side, world units (estimate: a slow float about the craft's own size; squadrons whose neighbours
/// have less room than this drift less, see [`weave_scale`]).
const WEAVE_SIDE: f32 = 6.0;
/// Up and down drift (bob) at rest, world units (estimate, same size).
const WEAVE_UP: f32 = 5.0;
/// Fore and aft drift at rest, world units (estimate, same size).
const WEAVE_AHEAD: f32 = 2.0;
/// Mean periods of the three drifts in seconds; each craft takes 0.8-1.2 times them, from its id (estimates: a slow float, the Remake's
/// idle fighters loop round at `Min_Speed` 1, 30 units per second).
const WEAVE_SIDE_PERIOD: f32 = 4.2;
const WEAVE_UP_PERIOD: f32 = 3.4;
const WEAVE_AHEAD_PERIOD: f32 = 5.6;
/// Attitude flying the drift needs at rest, radians (estimates): nose turned into the side drift, bank into its pull, nose up rising.
const WEAVE_YAW: f32 = 10.0 * std::f32::consts::PI / 180.0;
const WEAVE_BANK: f32 = 14.0 * std::f32::consts::PI / 180.0;
const WEAVE_PITCH: f32 = 7.0 * std::f32::consts::PI / 180.0;
/// Share of the drift and of its attitude a craft keeps at top speed (estimates: squadrons on the move keep rigid rows in wkiF9UTI3xo,
/// within 3 % of their spacing, the measurement's noise).
const WEAVE_IN_MOTION: f32 = 0.2;
const WEAVE_TURN_IN_MOTION: f32 = 0.35;
/// Seconds over which a craft's weave grows in after it appears (launch from a hangar, arrival): it leaves the bay on its exact path.
const WEAVE_FADE_IN: f32 = 3.0;
/// Share of the free room between two squadron neighbours that their drifts may use together, both drifting towards each other at once
/// (estimate): they never touch, whatever the camera distance (the craft and the gap between them shrink alike on screen).
const WEAVE_ROOM_SHARE: f32 = 0.6;

/// Drift of a small craft around its simulated place and the attitude that flies it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Weave {
    /// Offset in the craft's heading frame: x to its left, y up, z ahead (world units).
    pub offset: Vec3,
    /// Nose turned towards the side drift (radians, positive: to the left, like the simulation's yaw).
    pub yaw: f32,
    /// Bank into the side pull (radians, positive: right wing down).
    pub bank: f32,
    /// Nose up while rising (radians).
    pub pitch: f32,
}

impl Weave {
    /// The bank and pitch from the motion with the weave's added, within the craft's limits (the weave's own swing always fits).
    pub fn lean(&self, look: &FlightLook, bank: f32, pitch: f32) -> (f32, f32) {
        let bank_limit = look.max_bank.max(WEAVE_BANK);
        let pitch_limit = look.max_pitch.max(WEAVE_PITCH);
        ((bank + self.bank).clamp(-bank_limit, bank_limit), (pitch + self.pitch).clamp(-pitch_limit, pitch_limit))
    }
}

/// 0..1 from a craft's id: the same id always gives the same value; `salt` picks one of several.
fn unit_hash(id: u32, salt: u32) -> f32 {
    let mut x = id.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B).rotate_left(13);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// The weave of craft `id` at battle time `t`. `calm` (0..1) is how still it flies ([`FlightLook::calm`]), `age` the seconds since it
/// appeared and `scale` (0..1) how much of the drift the room between squadron neighbours allows ([`weave_scale`]). The attitude stays whole
/// (it does not move the craft).
pub fn weave(id: u32, t: f32, calm: f32, age: f32, scale: f32) -> Weave {
    let grown = smoothstep((age / WEAVE_FADE_IN).clamp(0.0, 1.0));
    let calm = calm.clamp(0.0, 1.0);
    let drift = grown * scale.clamp(0.0, 1.0) * (WEAVE_IN_MOTION + (1.0 - WEAVE_IN_MOTION) * calm);
    let turn = grown * (WEAVE_TURN_IN_MOTION + (1.0 - WEAVE_TURN_IN_MOTION) * calm);
    let phase = |salt: u32, period: f32| TAU * unit_hash(id, salt) + TAU * t / (period * (0.8 + 0.4 * unit_hash(id, salt + 1)));
    let side = phase(1, WEAVE_SIDE_PERIOD);
    let up = phase(3, WEAVE_UP_PERIOD);
    let ahead = phase(5, WEAVE_AHEAD_PERIOD);
    Weave {
        offset: Vec3::new(WEAVE_SIDE * side.sin(), WEAVE_UP * up.sin(), WEAVE_AHEAD * ahead.sin()) * drift,
        // Drifting left (the offset growing) turns the nose left; at the far left the pull to the right banks it right.
        yaw: WEAVE_YAW * side.cos() * turn,
        bank: WEAVE_BANK * side.sin() * turn,
        pitch: WEAVE_PITCH * up.cos() * turn,
    }
}

/// Share of the drift a squadron member of bounding radius `radius` keeps when its closest neighbour's place is `gap` apart (`Squadron_Offsets`,
/// `None`: no formation, the craft has room all round). The free room between the two spheres is `gap` less their diameters; both drifting
/// towards each other at once use at most [`WEAVE_ROOM_SHARE`] of it, so neighbours never touch, however far the camera is: craft and gap
/// shrink alike on screen.
pub fn weave_scale(gap: Option<f32>, radius: f32) -> f32 {
    let Some(gap) = gap else { return 1.0 };
    let room = (gap - 2.0 * radius.max(0.0)).max(0.0);
    (WEAVE_ROOM_SHARE * room / (2.0 * weave_reach())).clamp(0.0, 1.0)
}

/// The largest distance a craft's weave moves it from its simulated place (the bound the overlap budget is built on).
pub fn weave_reach() -> f32 {
    WEAVE_SIDE.hypot(WEAVE_AHEAD).hypot(WEAVE_UP)
}

fn smoothstep(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 30.0;
    fn tie() -> FlightLook {
        // TIE_Fighter: Bank_Turn_Angle 360, Max_Rate_Of_Roll 2.56, Max_Lift 2.8, Max_Speed 4 (x30 = 120 units/s), Max_Rate_Of_Turn 2.1.
        let xml = FlightXml { bank_turn_angle: 360.0, max_rate_of_roll: 2.56, max_lift: 2.8, max_speed: 4.0 };
        FlightLook::new(TargetClass::Fighter, true, xml, 2.1, 120.0)
    }

    fn isd() -> FlightLook {
        // Star_Destroyer: Bank_Turn_Angle 20, Max_Rate_Of_Roll 0.0375, Max_Speed 3 (x30 = 90 units/s), Max_Rate_Of_Turn 0.055.
        let xml = FlightXml { bank_turn_angle: 20.0, max_rate_of_roll: 0.0375, max_lift: 0.0, max_speed: 3.0 };
        FlightLook::new(TargetClass::Capital, false, xml, 0.055, 90.0)
    }

    /// Runs `steps` simulation steps of the same motion and returns the attitude.
    fn fly(look: &FlightLook, motion: Motion, steps: u64) -> Attitude {
        let mut a = Attitude::new(0);
        for tick in 0..=steps {
            a.update(tick, look, Some(motion), DT);
        }
        a
    }

    #[test]
    fn data_sets_the_bank_limits() {
        let tie = tie();
        assert!((tie.max_bank.to_degrees() - 60.0).abs() < 1e-3, "360 in the data means no limit, the measured 60 degrees hold");
        assert!((tie.max_pitch.to_degrees() - 35.0).abs() < 0.1, "Max_Lift 2.8 of Max_Speed 4");
        assert!((isd().max_bank.to_degrees() - 7.0).abs() < 1e-3, "35 % of the Star Destroyer's 20 degrees");
        let xml = FlightXml { bank_turn_angle: 20.0, max_rate_of_roll: 0.1, ..FlightXml::default() };
        let station = FlightLook::new(TargetClass::Structure, false, xml, 0.0, 0.0);
        assert_eq!((station.max_bank, station.max_pitch), (0.0, 0.0));
        // Spacecraft_Base's 90 degrees and a fighter without Max_Lift.
        let bomber = FlightLook::new(TargetClass::Bomber, true, FlightXml { bank_turn_angle: 90.0, ..FlightXml::default() }, 1.65, 105.0);
        assert!((bomber.max_bank.to_degrees() - 60.0).abs() < 1e-3 && (bomber.max_pitch.to_degrees() - 30.0).abs() < 1e-3);
        assert_eq!(bomber.roll_rate, SMALL_CRAFT_ROLL_RATE);
    }

    #[test]
    fn a_capital_ship_flying_straight_or_standing_does_not_roll() {
        let isd = isd();
        for motion in [
            Motion { yaw_rate: 0.0, speed: 90.0, climb: 0.0 },
            Motion::default(),
            // Turning on the spot.
            Motion { yaw_rate: 0.055, speed: 0.0, climb: 0.0 },
        ] {
            for steps in [1, 30, 900] {
                let (bank, pitch) = fly(&isd, motion, steps).at(0.5);
                assert_eq!((bank, pitch), (0.0, 0.0), "{motion:?} after {steps} steps");
            }
        }
        // Out of a turn it levels again.
        let mut a = fly(&isd, Motion { yaw_rate: 0.055, speed: 90.0, climb: 0.0 }, 600);
        assert!(a.at(0.0).0 < -0.1);
        for tick in 601..1500 {
            a.update(tick, &isd, Some(Motion { yaw_rate: 0.0, speed: 90.0, climb: 0.0 }), DT);
        }
        assert!(a.at(0.0).0.abs() < 1e-4, "{}", a.at(0.0).0);
    }

    #[test]
    fn ships_bank_into_their_turn_and_the_left_wing_goes_down_in_a_left_turn() {
        let look = tie();
        // The sim's yaw grows to the left: a left turn at full rate and speed.
        let a = fly(&look, Motion { yaw_rate: 2.1, speed: 120.0, climb: 0.0 }, 60);
        let (bank, _) = a.at(0.0);
        assert!((bank + look.max_bank).abs() < 0.02, "settled at the full bank, left wing down: {bank}");
        let rot = attitude_rotation(0.0, 0.0, bank);
        // The model's +X is its left side; at yaw 0 the left is sim +Y, Bevy -Z.
        let (left, right) = (rot * Vec3::X, rot * Vec3::NEG_X);
        assert!(left.z < 0.0, "+X is the left wing, towards the turn: {left:?}");
        assert!(left.y < right.y - 1.0, "left wing below the right one: {left:?} {right:?}");
        let (bank_right, _) = fly(&look, Motion { yaw_rate: -2.1, speed: 120.0, climb: 0.0 }, 60).at(0.0);
        assert!(bank_right > 0.9 * look.max_bank, "and the other way round: {bank_right}");
    }

    #[test]
    fn a_capital_ship_rolls_slightly_and_slowly() {
        let isd = isd();
        let turn = Motion { yaw_rate: 0.055, speed: 90.0, climb: 0.0 };
        // Max_Rate_Of_Roll 0.0375 rad/s: after one second it is still far from its 7 degrees, after ten it is there.
        let (one, _) = fly(&isd, turn, 30).at(0.0);
        assert!(one.abs() < 0.0375 * 1.01 && one < 0.0, "{one}");
        let (ten, _) = fly(&isd, turn, 300).at(0.0);
        assert!((ten.to_degrees() + 7.0).abs() < 0.2, "{}", ten.to_degrees());
    }

    #[test]
    fn bank_and_pitch_stay_within_the_limits_and_move_at_most_at_the_roll_rate() {
        for look in [tie(), isd()] {
            let mut a = Attitude::new(0);
            let mut last = (0.0, 0.0);
            for tick in 0..2000u64 {
                // Hard alternating turns and climbs, beyond the limits.
                let k = (tick as f32 * 0.07).sin().signum();
                let m = Motion { yaw_rate: 9.0 * k, speed: look.max_speed * 3.0, climb: -look.max_speed * 5.0 * k };
                a.update(tick, &look, Some(m), DT);
                let (bank, pitch) = a.at(0.0);
                assert!(bank.abs() <= look.max_bank + 1e-5 && pitch.abs() <= look.max_pitch + 1e-5, "{tick}: {bank} {pitch}");
                assert!((bank - last.0).abs() <= look.roll_rate * DT + 1e-5, "{tick}: roll rate");
                assert!((pitch - last.1).abs() <= look.turn_rate * DT + 1e-5, "{tick}: pitch rate");
                last = (bank, pitch);
            }
        }
    }

    #[test]
    fn the_attitude_does_not_depend_on_the_frame_rate() {
        let look = tie();
        let m = |tick: u64| Motion { yaw_rate: if tick % 50 < 25 { 2.1 } else { -1.0 }, speed: 100.0, climb: 30.0 };
        // 30 frames per step against one frame per step: the same values at every step.
        let (mut slow, mut fast) = (Attitude::new(0), Attitude::new(0));
        for tick in 0..200u64 {
            for _ in 0..30 {
                slow.update(tick, &look, Some(m(tick)), DT);
            }
            fast.update(tick, &look, Some(m(tick)), DT);
            assert_eq!(slow.at(0.3), fast.at(0.3), "step {tick}");
        }
    }

    #[test]
    fn climbs_and_dives_pitch_the_nose() {
        let look = tie();
        let (_, up) = fly(&look, Motion { yaw_rate: 0.0, speed: 120.0, climb: 60.0 }, 60).at(0.0);
        assert!((up - 0.5f32.atan()).abs() < 0.01, "the flight path's 26.6 degrees: {}", up.to_degrees());
        let (_, down) = fly(&look, Motion { yaw_rate: 0.0, speed: 0.0, climb: -200.0 }, 60).at(0.0);
        assert!((down + look.max_pitch).abs() < 0.01, "a steep drop on the spot stops at the limit: {}", down.to_degrees());
        let rot = attitude_rotation(0.0, up, 0.0);
        assert!((rot * Vec3::Z).y > 0.4, "nose up: {:?}", rot * Vec3::Z);
    }

    #[test]
    fn the_weave_is_bounded_deterministic_and_differs_per_craft() {
        let reach = weave_reach();
        let mut seen = Vec::new();
        for id in 0..64u32 {
            for k in 0..600 {
                let t = k as f32 * 0.1;
                let w = weave(id, t, 1.0, 100.0, 1.0);
                assert_eq!(w, weave(id, t, 1.0, 100.0, 1.0), "same input, same weave");
                assert!(w.offset.length() <= reach + 1e-4, "{id} {t}: {:?}", w.offset);
                assert!(w.offset.x.abs() <= WEAVE_SIDE + 1e-4 && w.offset.y.abs() <= WEAVE_UP + 1e-4 && w.offset.z.abs() <= WEAVE_AHEAD + 1e-4);
                assert!(w.yaw.abs() <= WEAVE_YAW + 1e-5 && w.bank.abs() <= WEAVE_BANK + 1e-5 && w.pitch.abs() <= WEAVE_PITCH + 1e-5);
            }
            seen.push(weave(id, 7.0, 1.0, 100.0, 1.0).offset);
        }
        // Neighbours in a squadron (consecutive ids) do not move in step: no rigid grid.
        let distinct = seen.windows(2).filter(|p| p[0].distance(p[1]) > 1.0).count();
        assert!(distinct >= 56, "{distinct} of 63 neighbour pairs differ");
        // The drift really moves: over a period every craft covers most of its side range.
        for id in 0..8u32 {
            let xs: Vec<f32> = (0..60).map(|k| weave(id, k as f32 * 0.1, 1.0, 100.0, 1.0).offset.x).collect();
            let span = xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min);
            assert!(span > WEAVE_SIDE, "{id}: {span}");
        }
    }

    #[test]
    fn the_weave_adds_to_the_turn_within_the_limits() {
        let look = tie();
        let w = Weave { bank: WEAVE_BANK, pitch: -WEAVE_PITCH, ..Weave::default() };
        assert_eq!(w.lean(&look, 0.1, 0.2), (0.1 + WEAVE_BANK, 0.2 - WEAVE_PITCH));
        assert_eq!(w.lean(&look, look.max_bank, -look.max_pitch), (look.max_bank, -look.max_pitch));
        // A craft that does not bank in turns still sways.
        let flat = FlightLook { max_bank: 0.0, max_pitch: 0.0, ..look };
        assert_eq!(w.lean(&flat, 0.0, 0.0), (WEAVE_BANK, -WEAVE_PITCH));
    }

    #[test]
    fn the_weave_fades_with_speed_age_and_room() {
        let full = weave_reach();
        let max_offset = |calm: f32, age: f32, room: f32| (0..400).map(|k| weave(3, k as f32 * 0.05, calm, age, room).offset.length()).fold(0.0f32, f32::max);
        assert!(max_offset(1.0, 100.0, 1.0) > 0.8 * full);
        assert!(max_offset(0.0, 100.0, 1.0) <= WEAVE_IN_MOTION * full + 1e-4, "at top speed the rows stay rigid");
        assert_eq!(max_offset(1.0, 0.0, 1.0), 0.0, "a craft leaving the hangar is on its exact path");
        assert!((max_offset(1.0, 100.0, 0.5) - 0.5 * max_offset(1.0, 100.0, 1.0)).abs() < 1e-3, "the room scales the whole drift");
        assert_eq!(max_offset(1.0, 100.0, 0.0), 0.0, "no room, no drift");
    }


    /// The closest two places of a squadron of the Remake's data and the bounding radius of its craft (TIE Fighter: three rows of four, 20
    /// apart side by side; TIE Interceptor: pairs stacked 6 apart; A-wing 28; X-wing 50).
    const SQUADRONS: [(f32, f32); 4] = [(20.0, 2.6), (6.0, 2.6), (28.3, 2.4), (50.0, 4.1)];

    #[test]
    fn squadron_neighbours_never_touch_however_the_camera_stands() {
        for (gap, radius) in SQUADRONS {
            let scale = weave_scale(Some(gap), radius);
            // Both drifting towards each other as far as the weave allows, the two spheres keep 40 % of their free room.
            let room = gap - 2.0 * radius;
            assert!(2.0 * weave_reach() * scale <= WEAVE_ROOM_SHARE * room.max(0.0) + 1e-4, "gap {gap}, radius {radius}: scale {scale}");
            // Sampled: two neighbours in one heading frame, next to each other along each of its axes, drift for two minutes.
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                let mut closest = f32::MAX;
                for id in 0..24u32 {
                    for k in 0..2400 {
                        let t = k as f32 * 0.05;
                        let (a, b) = (weave(id, t, 1.0, 100.0, scale).offset, weave(id + 1, t, 1.0, 100.0, scale).offset);
                        closest = closest.min((axis * gap + b - a).length());
                    }
                }
                assert!(closest >= 2.0 * radius, "gap {gap}, radius {radius}, axis {axis}: spheres come {closest:.2} apart");
            }
        }
    }

    #[test]
    fn a_cramped_squadron_drifts_less_and_a_roomy_one_keeps_the_whole_weave() {
        let tie = weave_scale(Some(20.0), 2.6);
        assert!(tie > 0.4 && tie < 0.7, "the TIE fighter's 20 units leave 14.8 free: {tie}");
        assert_eq!(weave_scale(Some(50.0), 4.1), 1.0, "the X-wing's 50 units leave room for all of it");
        assert!(weave_scale(Some(6.0), 2.6) < 0.05, "stacked TIE interceptors hardly move");
        assert_eq!(weave_scale(Some(5.0), 2.6), 0.0, "craft that already touch stay put");
        assert_eq!(weave_scale(None, 4.0), 1.0, "a craft without a formation has room all round");
        assert_eq!(weave_scale(Some(20.0), 0.0), weave_scale(Some(20.0), -1.0), "an unknown radius counts as none");
    }
}
