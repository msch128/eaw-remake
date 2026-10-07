//! Projectile and particle object definitions read from the game's XML.
//!
//! Only what rendering needs is extracted, with a deliberately small tag scanner instead of a full
//! XML parser: `<Projectile Name="...">` and `<Particle Name="...">` blocks with simple
//! `<Tag>value</Tag>` children. `Variant_Of_Existing_Type` inheritance is resolved.

use std::collections::HashMap;

use alamo_fs::{FileSource, GameFs};
use bevy::prelude::*;

/// Ticks per second used by the engine to convert per-frame XML values.
pub const FRAMES_PER_SECOND: f32 = 30.0;

/// Visual (and impact effect) fields of a `<Projectile>`.
#[derive(Debug, Clone, Default)]
pub struct ProjectileDef {
    /// Object name, e.g. `Proj_Ship_Turbolaser_Green`.
    pub name: String,
    /// `Space_Model_Name`: the bolt model (`w_turbolaser_g.alo`), drawn as batched geometry.
    pub model: Option<String>,
    /// `Scale_Factor` applied to the bolt model.
    pub scale: f32,
    /// `Max_Speed` in world units per frame (30 frames per second).
    pub max_speed: f32,
    /// `Projectile_Max_Flight_Distance`.
    pub max_distance: f32,
    /// `Projectile_Category` (`LASER`, `MISSILE`, ...).
    pub category: String,
    /// `Projectile_Custom_Render`: nonzero for engine-drawn bolts without a model.
    pub custom_render: u32,
    /// `Projectile_Width` of custom-rendered bolts.
    pub width: f32,
    /// `Projectile_Length` of custom-rendered bolts.
    pub length: f32,
    /// `Projectile_Texture_Slot` (column, row) of custom-rendered bolts.
    pub texture_slot: (u32, u32),
    /// `Projectile_Laser_Color` as RGBA bytes.
    pub laser_color: Option<[u8; 4]>,
    /// `Projectile_Object_Detonation_Particle`: hit on an unarmored hull.
    pub object_detonation: Option<String>,
    /// `Projectile_Object_Armor_Reduced_Detonation_Particle`: hit on armor.
    pub armor_detonation: Option<String>,
    /// `Projectile_Lifetime_Detonation_Particle`: end of flight.
    pub lifetime_detonation: Option<String>,
    /// `Projectile_Ground_Detonation_Particle`.
    pub ground_detonation: Option<String>,
    /// `Projectile_Absorbed_By_Shields_Particle`: shield impact.
    pub shield_absorb: Option<String>,
}

/// A `<Particle>` object: a particle `.alo` plus scale and lifetime.
#[derive(Debug, Clone, Default)]
pub struct ParticleDef {
    /// Object name.
    pub name: String,
    /// `Space_Model_Name` (the particle system file).
    pub model: String,
    /// `Scale_Factor`.
    pub scale: f32,
    /// `Particle_Lifetime_Frames`, if any.
    pub lifetime_frames: Option<u32>,
}

impl ParticleDef {
    /// Lifetime in seconds (default 10).
    pub fn lifetime_secs(&self) -> f32 {
        self.lifetime_frames
            .map_or(10.0, |f| f as f32 / FRAMES_PER_SECOND)
    }
}

/// Space unit fields needed for death sequences.
#[derive(Debug, Clone, Default)]
pub struct UnitDef {
    /// Object name.
    pub name: String,
    /// `Space_Model_Name`.
    pub model: Option<String>,
    /// `Death_Clone`: the unit to swap in when this one dies (the damage-type prefix removed).
    pub death_clone: Option<String>,
    /// `Death_Explosions`: particle objects played while dying.
    pub death_explosions: Vec<String>,
    /// `HardPoints`: names of the hardpoints of this unit.
    pub hardpoints: Vec<String>,
}

/// A `<HardPoint>`: a weapon, shield generator or art piece attached to a ship bone.
#[derive(Debug, Clone, Default)]
pub struct HardpointDef {
    /// Object name, e.g. `HP_Allegiance_TL_01`.
    pub name: String,
    /// `Type` (`HARD_POINT_WEAPON_LASER`, ...).
    pub kind: String,
    /// `Model_To_Attach`: turret or art model placed on the bone (often empty).
    pub model: Option<String>,
    /// `Attachment_Bone` of the ship the model is attached to.
    pub bone: String,
    /// `Is_Destroyable`.
    pub destroyable: bool,
    /// `Health`.
    pub health: f32,
    /// `Death_Explosion_Particles`: particle object played when destroyed.
    pub death_particles: Option<String>,
    /// `Damage_Particles`: bone (with fire/smoke proxies) or particle object enabled on destruction.
    pub damage_particles: Option<String>,
    /// `Fire_Bone_A` and `Fire_Bone_B`: muzzle bones.
    pub fire_bones: [Option<String>; 2],
    /// `Is_Turret`.
    pub is_turret: bool,
    /// `Turret_Bone_Name`: the bone that yaws (default `Turret`).
    pub turret_bone: Option<String>,
    /// `Barrel_Bone_Name`: the bone that pitches (default `Barrel`).
    pub barrel_bone: Option<String>,
    /// `Turret_Rotate_Speed`: turn speed (radians per second as far as the data suggests).
    pub rotate_speed: f32,
    /// `Turret_Rotate_Extent_Degrees`: total yaw range; 360 means a full circle.
    pub rotate_extent_deg: f32,
    /// `Turret_Elevate_Extent_Degrees`: total pitch range above the horizontal.
    pub elevate_extent_deg: f32,
}

/// All projectile and particle definitions.
#[derive(Resource, Default, Debug)]
pub struct XmlDefs {
    /// Projectiles by lowercase name.
    pub projectiles: HashMap<String, ProjectileDef>,
    /// Particle objects by lowercase name.
    pub particles: HashMap<String, ParticleDef>,
    /// Space units by lowercase name.
    pub units: HashMap<String, UnitDef>,
    /// Hardpoints by lowercase name.
    pub hardpoints: HashMap<String, HardpointDef>,
}

impl XmlDefs {
    /// Projectile by name (case-insensitive).
    pub fn projectile(&self, name: &str) -> Option<&ProjectileDef> {
        self.projectiles.get(&name.trim().to_ascii_lowercase())
    }

    /// Particle object by name (case-insensitive).
    pub fn particle(&self, name: &str) -> Option<&ParticleDef> {
        self.particles.get(&name.trim().to_ascii_lowercase())
    }

    /// Space unit by name (case-insensitive).
    pub fn unit(&self, name: &str) -> Option<&UnitDef> {
        self.units.get(&name.trim().to_ascii_lowercase())
    }

    /// Hardpoint by name (case-insensitive).
    pub fn hardpoint(&self, name: &str) -> Option<&HardpointDef> {
        self.hardpoints.get(&name.trim().to_ascii_lowercase())
    }

    /// Scans every XML file below `XML/` of `fs`.
    pub fn load(fs: &GameFs) -> XmlDefs {
        let mut raw_proj: Vec<RawBlock> = Vec::new();
        let mut raw_units: Vec<RawBlock> = Vec::new();
        let mut raw_hp: Vec<RawBlock> = Vec::new();
        let mut defs = XmlDefs::default();
        for path in fs.list("XML") {
            if !path.ends_with(".XML") {
                continue;
            }
            let Ok(bytes) = fs.read(&path) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            if !text.contains("<Projectile ")
                && !text.contains("<Particle ")
                && !text.contains("<SpaceUnit ")
                && !text.contains("<UniqueUnit ")
                && !text.contains("<HardPoint ")
            {
                continue;
            }
            let text = strip_comments(&text);
            raw_hp.extend(blocks(&text, "HardPoint"));
            raw_units.extend(blocks(&text, "SpaceUnit"));
            raw_units.extend(blocks(&text, "UniqueUnit"));
            raw_proj.extend(blocks(&text, "Projectile"));
            for b in blocks(&text, "Particle") {
                let def = ParticleDef {
                    name: b.name.clone(),
                    model: b
                        .get("Space_Model_Name")
                        .or_else(|| b.get("Land_Model_Name"))
                        .unwrap_or_default(),
                    scale: b
                        .get("Scale_Factor")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1.0),
                    lifetime_frames: b
                        .get("Particle_Lifetime_Frames")
                        .and_then(|v| v.parse().ok()),
                };
                if !def.model.is_empty() {
                    defs.particles.insert(b.name.to_ascii_lowercase(), def);
                }
            }
        }
        // Resolve variants (a few levels deep).
        let by_name: HashMap<String, usize> = raw_proj
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.to_ascii_lowercase(), i))
            .collect();
        for b in &raw_proj {
            let mut chain = vec![b];
            let mut cur = b;
            for _ in 0..8 {
                let Some(base) = cur.get("Variant_Of_Existing_Type") else {
                    break;
                };
                let Some(&i) = by_name.get(&base.to_ascii_lowercase()) else {
                    break;
                };
                cur = &raw_proj[i];
                chain.push(cur);
            }
            let get = |tag: &str| chain.iter().find_map(|c| c.get_raw(tag));
            let text = |tag: &str| get(tag).filter(|v| !v.is_empty());
            let num = |tag: &str, d: f32| get(tag).and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
            let slot = text("Projectile_Texture_Slot")
                .and_then(|v| {
                    let mut it = v.split(',').filter_map(|p| p.trim().parse().ok());
                    Some((it.next()?, it.next()?))
                })
                .unwrap_or((0, 0));
            let color = text("Projectile_Laser_Color").and_then(|v| {
                let c: Vec<u8> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                (c.len() == 4).then(|| [c[0], c[1], c[2], c[3]])
            });
            defs.projectiles.insert(
                b.name.to_ascii_lowercase(),
                ProjectileDef {
                    name: b.name.clone(),
                    model: text("Space_Model_Name"),
                    scale: num("Scale_Factor", 1.0),
                    max_speed: num("Max_Speed", 0.0),
                    max_distance: num("Projectile_Max_Flight_Distance", 0.0),
                    category: text("Projectile_Category").unwrap_or_default(),
                    custom_render: num("Projectile_Custom_Render", 0.0) as u32,
                    width: num("Projectile_Width", 0.0),
                    length: num("Projectile_Length", 0.0),
                    texture_slot: slot,
                    laser_color: color,
                    object_detonation: text("Projectile_Object_Detonation_Particle"),
                    armor_detonation: text("Projectile_Object_Armor_Reduced_Detonation_Particle"),
                    lifetime_detonation: text("Projectile_Lifetime_Detonation_Particle"),
                    ground_detonation: text("Projectile_Ground_Detonation_Particle"),
                    shield_absorb: text("Projectile_Absorbed_By_Shields_Particle"),
                },
            );
        }
        let unit_by_name: HashMap<String, usize> = raw_units
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.to_ascii_lowercase(), i))
            .collect();
        for b in &raw_units {
            let mut chain = vec![b];
            let mut cur = b;
            for _ in 0..8 {
                let Some(base) = cur.get("Variant_Of_Existing_Type") else {
                    break;
                };
                let Some(&i) = unit_by_name.get(&base.to_ascii_lowercase()) else {
                    break;
                };
                cur = &raw_units[i];
                chain.push(cur);
            }
            let get = |tag: &str| chain.iter().find_map(|c| c.get(tag));
            let death_clone =
                get("Death_Clone").map(|v| v.rsplit(',').next().unwrap_or("").trim().to_string());
            defs.units.insert(
                b.name.to_ascii_lowercase(),
                UnitDef {
                    name: b.name.clone(),
                    model: get("Space_Model_Name"),
                    death_clone: death_clone.filter(|c| !c.is_empty()),
                    hardpoints: get("HardPoints")
                        .map(|v| {
                            v.split(|c: char| c == ',' || c.is_whitespace())
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    death_explosions: get("Death_Explosions")
                        .map(|v| {
                            v.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default(),
                },
            );
        }
        let hp_by_name: HashMap<String, usize> = raw_hp
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.to_ascii_lowercase(), i))
            .collect();
        for b in &raw_hp {
            let mut chain = vec![b];
            let mut cur = b;
            for _ in 0..8 {
                let Some(base) = cur.get("Variant_Of_Existing_Type") else {
                    break;
                };
                let Some(&i) = hp_by_name.get(&base.to_ascii_lowercase()) else {
                    break;
                };
                cur = &raw_hp[i];
                chain.push(cur);
            }
            let get = |tag: &str| chain.iter().find_map(|c| c.get(tag));
            let yes = |tag: &str| {
                get(tag).is_some_and(|v| {
                    v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true")
                })
            };
            defs.hardpoints.insert(
                b.name.to_ascii_lowercase(),
                HardpointDef {
                    name: b.name.clone(),
                    kind: get("Type").unwrap_or_default(),
                    model: get("Model_To_Attach"),
                    bone: get("Attachment_Bone").unwrap_or_default(),
                    destroyable: yes("Is_Destroyable"),
                    health: get("Health").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                    death_particles: get("Death_Explosion_Particles"),
                    damage_particles: get("Damage_Particles"),
                    fire_bones: [get("Fire_Bone_A"), get("Fire_Bone_B")],
                    is_turret: yes("Is_Turret"),
                    turret_bone: get("Turret_Bone_Name"),
                    barrel_bone: get("Barrel_Bone_Name"),
                    rotate_speed: get("Turret_Rotate_Speed")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0.0),
                    rotate_extent_deg: get("Turret_Rotate_Extent_Degrees")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(360.0),
                    elevate_extent_deg: get("Turret_Elevate_Extent_Degrees")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(90.0),
                },
            );
        }
        defs
    }
}

struct RawBlock {
    name: String,
    tags: Vec<(String, String)>,
}

impl RawBlock {
    fn get_raw(&self, tag: &str) -> Option<String> {
        self.tags
            .iter()
            .find(|(t, _)| t.eq_ignore_ascii_case(tag))
            .map(|(_, v)| v.clone())
    }

    fn get(&self, tag: &str) -> Option<String> {
        self.get_raw(tag).filter(|v| !v.is_empty())
    }
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        match rest[i..].find("-->") {
            Some(j) => rest = &rest[i + j + 3..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Finds `<kind Name="X"> ... </kind>` blocks and their simple child tags.
fn blocks(text: &str, kind: &str) -> Vec<RawBlock> {
    let open = format!("<{kind} ");
    let close = format!("</{kind}>");
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(i) = text[pos..].find(&open) {
        let start = pos + i;
        let Some(head_end) = text[start..].find('>') else {
            break;
        };
        let head = &text[start..start + head_end];
        let name = head
            .split("Name=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .unwrap_or("")
            .to_string();
        let body_start = start + head_end + 1;
        let end = text[body_start..]
            .find(&close)
            .map_or(text.len(), |e| body_start + e);
        let body = &text[body_start..end];
        out.push(RawBlock {
            name,
            tags: tags(body),
        });
        pos = end.max(body_start);
    }
    out
}

/// Parses `<Tag>value</Tag>` and `<Tag />` children (first level, no nesting).
fn tags(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find('<') {
        let after = &rest[i + 1..];
        let Some(gt) = after.find('>') else { break };
        let tag = &after[..gt];
        if tag.starts_with('/') || tag.starts_with('!') || tag.starts_with('?') {
            rest = &after[gt + 1..];
            continue;
        }
        if let Some(name) = tag.strip_suffix('/') {
            out.push((name.trim().to_string(), String::new()));
            rest = &after[gt + 1..];
            continue;
        }
        let name = tag.split_whitespace().next().unwrap_or("").to_string();
        let value_start = gt + 1;
        let closing = format!("</{name}>");
        match after[value_start..].find(&closing) {
            Some(e) => {
                out.push((name, after[value_start..value_start + e].trim().to_string()));
                rest = &after[value_start + e + closing.len()..];
            }
            None => rest = &after[value_start..],
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
<Root>
<!-- <Projectile Name="Hidden"><Max_Speed>1</Max_Speed></Projectile> -->
<Projectile Name="Proj_Base">
  <Space_Model_Name>w_laser_g.alo</Space_Model_Name>
  <Scale_Factor>0.5</Scale_Factor>
  <Max_Speed>45.0</Max_Speed>
  <Projectile_Absorbed_By_Shields_Particle>Shield_Hit</Projectile_Absorbed_By_Shields_Particle>
  <Projectile_Object_Detonation_Particle />
</Projectile>
<Projectile Name="Proj_Variant">
  <Variant_Of_Existing_Type>Proj_Base</Variant_Of_Existing_Type>
  <Max_Speed>10</Max_Speed>
  <Projectile_Laser_Color>1,2,3,255</Projectile_Laser_Color>
</Projectile>
<Particle Name="Shield_Hit">
  <Space_Model_Name>pd_shield.alo</Space_Model_Name>
  <Scale_Factor>2.0</Scale_Factor>
  <Particle_Lifetime_Frames>90</Particle_Lifetime_Frames>
</Particle>
</Root>"#;

    fn defs() -> XmlDefs {
        let text = strip_comments(SAMPLE);
        let mut d = XmlDefs::default();
        for b in blocks(&text, "Particle") {
            d.particles.insert(
                b.name.to_lowercase(),
                ParticleDef {
                    name: b.name.clone(),
                    model: b.get("Space_Model_Name").unwrap(),
                    scale: b.get("Scale_Factor").unwrap().parse().unwrap(),
                    lifetime_frames: b
                        .get("Particle_Lifetime_Frames")
                        .and_then(|v| v.parse().ok()),
                },
            );
        }
        assert_eq!(
            blocks(&text, "Projectile").len(),
            2,
            "comment must be ignored"
        );
        d
    }

    #[test]
    fn scans_blocks_and_tags() {
        let text = strip_comments(SAMPLE);
        let p = blocks(&text, "Projectile");
        assert_eq!(p[0].name, "Proj_Base");
        assert_eq!(p[0].get("Max_Speed").as_deref(), Some("45.0"));
        assert_eq!(p[0].get("Projectile_Object_Detonation_Particle"), None);
        assert_eq!(
            p[1].get("Projectile_Laser_Color").as_deref(),
            Some("1,2,3,255")
        );
    }

    #[test]
    fn particle_lifetime() {
        let d = defs();
        let p = d.particle("SHIELD_HIT").unwrap();
        assert_eq!(p.lifetime_secs(), 3.0);
        assert_eq!(p.scale, 2.0);
    }
}
