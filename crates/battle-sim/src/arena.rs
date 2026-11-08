//! Battle arena: playable bounds, spawn markers and obstacles, usually read from a space `.ted` map.

use alamo_data::GameData;
use alamo_fs::FileSource;
use alamo_map::{ObjectClass, SpaceMap};

use crate::math::Vec3;

/// A round obstacle (asteroid) that blocks projectiles and pushes ships away.
#[derive(Debug, Clone, PartialEq)]
pub struct Obstacle {
    /// Centre.
    pub pos: Vec3,
    /// Collision radius.
    pub radius: f32,
    /// Object name.
    pub name: String,
}

/// A named marker object of the map (`Shipyard 1 Marker`, `Golan All-in-One Marker`, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    /// Marker object name.
    pub name: String,
    /// Position.
    pub pos: Vec3,
    /// Heading in radians (counter-clockwise from +X).
    pub yaw: f32,
}

/// Arena description. Coordinates are the map's: X/Y plane, Z up; attackers sit at -Y facing +Y.
#[derive(Debug, Clone, Default)]
pub struct Arena {
    /// Lower XY corner of the playable area.
    pub min: [f32; 2],
    /// Upper XY corner.
    pub max: [f32; 2],
    /// `Attacker Entry Position` markers.
    pub attacker_entries: Vec<(Vec3, f32)>,
    /// `Defending Forces Position` markers.
    pub defender_positions: Vec<(Vec3, f32)>,
    /// `Space Station Position` marker (where the star base is placed).
    pub station: Option<(Vec3, f32)>,
    /// All other markers (structure placement spots).
    pub markers: Vec<Marker>,
    /// Solid rocks (`Is_Impassable_Asteroid` with `Space_Obstacle_Radius`).
    pub obstacles: Vec<Obstacle>,
    /// Asteroid damage fields (`Is_Asteroid_Field`). Not collision.
    pub asteroid_fields: Vec<Obstacle>,
    /// Nebula zones (`Is_Nebula`). Not collision.
    pub nebulae: Vec<Obstacle>,
    /// Ion-storm zones (`Is_Ion_Storm`). Not collision.
    pub ion_storms: Vec<Obstacle>,
}

/// Radii a resolved prop contributes. Absent means the prop is not that kind of zone.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PropZones {
    /// Solid-rock radius.
    pub rock: Option<f32>,
    /// Asteroid-field radius.
    pub field: Option<f32>,
    /// Nebula radius.
    pub nebula: Option<f32>,
    /// Ion-storm radius.
    pub ion: Option<f32>,
    /// `Space_Obstacle_Offset` in the prop's local axes.
    pub offset: [f32; 3],
}

/// Zones of one game object. A missing object contributes nothing.
pub fn prop_zones(data: &GameData, name: &str) -> PropZones {
    data.objects.resolve(name).map(|r| zones_of(&r)).unwrap_or_default()
}

fn zones_of(r: &alamo_data::Resolved<'_>) -> PropZones {
    let radius = r.num::<f32>("space_obstacle_radius").unwrap_or(0.0);
    let off = r.floats("space_obstacle_offset");
    let offset = [
        off.first().copied().unwrap_or(0.0),
        off.get(1).copied().unwrap_or(0.0),
        off.get(2).copied().unwrap_or(0.0),
    ];
    // Only a `SPACE_OBSTACLE` object is a rock or a zone, in any `Behavior` / `SpaceBehavior` copy (`Asteroid_Huge` lists
    // `Behavior` twice, the first copy has it). `Nebula_Ultra_Huge` (radius 40000, `In_Background`) has `IDLE` only: a backdrop.
    let obstacle = ["behavior", "spacebehavior"]
        .iter()
        .flat_map(|t| r.get_all(t))
        .any(|v| v.split([',', ' ', '\t', '\n', '\r']).any(|t| t.eq_ignore_ascii_case("space_obstacle")));
    let live = radius > 0.0 && obstacle;
    let field = r.flag("is_asteroid_field") == Some(true);
    let rock = r.flag("is_impassable_asteroid") == Some(true) && !field && live;
    PropZones {
        rock: rock.then_some(radius),
        field: (field && live).then_some(radius),
        nebula: (r.flag("is_nebula") == Some(true) && live).then_some(radius),
        ion: (r.flag("is_ion_storm") == Some(true) && live).then_some(radius),
        offset,
    }
}

/// Map position plus the offset rotated by yaw.
/// shortcut: yaw only, full Euler of a tumbling prop is unverified.
fn zone_pos(pos: Vec3, yaw: f32, offset: [f32; 3]) -> Vec3 {
    pos + Vec3::from_array(offset).rotate_yaw(yaw)
}

impl Arena {
    /// Builds an arena from a parsed map whose object names were resolved
    /// (see [`SpaceMap::resolve_names`]).
    pub fn from_space_map(data: &GameData, map: &SpaceMap) -> Arena {
        let mut a = Arena {
            min: [map.bounds.min[0], map.bounds.min[1]],
            max: [map.bounds.max[0], map.bounds.max[1]],
            ..Arena::default()
        };
        if a.max[0] <= a.min[0] || a.max[1] <= a.min[1] {
            a.min = [-9500.0, -9500.0];
            a.max = [9500.0, 9500.0];
        }
        for o in &map.objects {
            let Some(name) = o.type_name.as_deref() else { continue };
            let pos = Vec3::from_array(o.position);
            let yaw = o.rotation_deg[2].to_radians();
            match o.class() {
                ObjectClass::Marker => {
                    let lower = name.to_ascii_lowercase();
                    if lower == "attacker entry position" {
                        a.attacker_entries.push((pos, yaw));
                    } else if lower == "defending forces position" {
                        a.defender_positions.push((pos, yaw));
                    } else if lower == "space station position" {
                        a.station = Some((pos, yaw));
                    } else {
                        a.markers.push(Marker { name: name.to_string(), pos, yaw });
                    }
                }
                _ => {
                    let z = prop_zones(data, name);
                    let center = zone_pos(pos, yaw, z.offset);
                    let push = |radius| Obstacle { pos: center, radius, name: name.to_string() };
                    if let Some(radius) = z.rock {
                        a.obstacles.push(push(radius));
                    }
                    if let Some(radius) = z.field {
                        a.asteroid_fields.push(push(radius));
                    }
                    if let Some(radius) = z.nebula {
                        a.nebulae.push(push(radius));
                    }
                    if let Some(radius) = z.ion {
                        a.ion_storms.push(push(radius));
                    }
                }
            }
        }
        a
    }

    /// Loads `Art/Maps/<map_name>` through `fs` and builds the arena. Object names are resolved
    /// against all objects of `data`.
    pub fn load(data: &GameData, fs: &dyn FileSource, map_name: &str) -> Option<Arena> {
        let bytes = fs.read(&format!("Art/Maps/{map_name}")).ok()?;
        let mut map = SpaceMap::parse(&bytes).ok()?;
        map.resolve_names(data.objects.iter().map(|o| o.name.as_str()));
        Some(Arena::from_space_map(data, &map))
    }

    /// Largest absolute coordinate of the playable area.
    pub fn half_size(&self) -> f32 {
        self.min.iter().chain(self.max.iter()).fold(0.0f32, |m, v| m.max(v.abs()))
    }

    /// Marker whose name matches a structure name (shipyard level, Golan, research institute, ...).
    pub fn marker_for_structure(&self, structure: &str) -> Option<&Marker> {
        let s = structure.to_ascii_lowercase();
        let wanted: &[&str] = if let Some(i) = s.find("shipyard_") {
            let digit = s[i + 9..].chars().next().unwrap_or('1');
            return self.markers.iter().find(|m| m.name.to_ascii_lowercase() == format!("shipyard {digit} marker"));
        } else if s.contains("golan") {
            &["golan all-in-one marker"]
        } else if s.contains("research") {
            &["research institute marker"]
        } else if s.contains("trade_hub") || s.contains("trade hub") {
            &["trade hub marker"]
        } else if s.contains("satellite") || s.contains("sat_") {
            &["sat grid marker"]
        } else if s.contains("skystrike") {
            &["skystrike marker"]
        } else if s.contains("admin") {
            &["admin offices marker"]
        } else if s.contains("uplink") {
            &["uplink command station marker"]
        } else if s.contains("refinery") || s.contains("depot") {
            &["gas refinery marker"]
        } else {
            return None;
        };
        self.markers.iter().find(|m| wanted.contains(&m.name.to_ascii_lowercase().as_str()))
    }
}
