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
    /// Asteroids.
    pub obstacles: Vec<Obstacle>,
}

fn asteroid_radius(name: &str) -> f32 {
    let n = name.to_ascii_lowercase();
    if n.contains("huge") {
        600.0
    } else if n.contains("large") {
        350.0
    } else if n.contains("medium") {
        180.0
    } else if n.contains("small") {
        80.0
    } else {
        200.0
    }
}

impl Arena {
    /// Builds an arena from a parsed map whose object names were resolved
    /// (see [`SpaceMap::resolve_names`]).
    pub fn from_space_map(map: &SpaceMap) -> Arena {
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
                ObjectClass::Asteroid => a.obstacles.push(Obstacle { pos, radius: asteroid_radius(name), name: name.to_string() }),
                _ => {}
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
        Some(Arena::from_space_map(&map))
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
