//! Parser for Alamo engine (Star Wars: Empire at War) `.ted` map files, focused on **space** maps.
//!
//! # File layout (as reverse engineered from the shipped maps)
//!
//! A `.ted` is a chunk stream (`u32 id, u32 size`, top bit of size = container, see [`chunk`]).
//! Leaf chunks hold *mini-chunks* (`u8 id, u8 size, data`). All integers are little-endian.
//!
//! ```text
//! 0x00  header (mini): version, names (UTF-16), map size
//! 0x13  preview image (opaque, kept raw)
//! 0x02  list of 0x0b "defender slot" records
//! 0x01  map body
//!   0x100  environment
//!     0x04/0x06  lighting, ambient, sky, nebula, ... (mini)
//!     0x02/0x00  fog (mini)
//!   0x102/0x01  objects: a list of 0x44c entries
//!     0x44c/0x454  two u32 ids
//!     0x44c/0x459/0x4b0  object properties (mini): id, type crc, position, rotation, ...
//!     0x44c/0x459/0x4b7  optional attachment
//!     0x44c/0x459/0x4ba  77 constant "NONE" slots (ignored)
//!   0x103  bounds (mini)
//!   0x105  camera / misc (mini, kept raw)
//! ```
//!
//! Object types are stored as the **CRC-32 of the upper-cased object name** (the XML name of a game
//! object, e.g. `ASTEROID_LARGE`). Call [`SpaceMap::resolve_names`] with the names of the game objects
//! to turn the hashes into names.
//!
//! # Coordinates
//!
//! Z is up (same as `.alo` models): the map plane is XY (bounds roughly +-9500 in X/Y, +-5000 in Z),
//! units are the same as in models. Rotations are Euler angles in **degrees**; markers only use the Z
//! (yaw) component, asteroids tumble around all axes (the Euler order is not verified). Yaw is measured
//! counter-clockwise from +X towards +Y: on every map the attackers sit at -Y with yaw ~83-90 (facing +Y)
//! and the defenders at +Y with yaw 270 (facing -Y), so the facing vector is (cos yaw, sin yaw, 0).
//!
//! Fields whose meaning was inferred rather than proven are marked **(guess)**; everything that is not
//! decoded stays available through [`Minis`] (`raw` fields) or [`SpaceMap::unknown`].

pub mod chunk;

use chunk::{Chunk, Minis, ParseError, chunks};

/// Parsed space tactical map.
#[derive(Debug, Clone, Default)]
pub struct SpaceMap {
    /// Map header.
    pub header: Header,
    /// Playable area and related bounds.
    pub bounds: Bounds,
    /// Lighting, sky and nebula.
    pub environment: Environment,
    /// All placed objects in file order.
    pub objects: Vec<MapObject>,
    /// Records of top-level chunk `0x02` (one per defender start object, see [`DefenderSlot`]).
    pub defender_slots: Vec<DefenderSlot>,
    /// Chunk `0x105` (camera / misc settings), undecoded.
    pub camera_misc: Minis,
    /// Preview image chunk `0x13` (opaque: 256 KiB D3DX-saved bitmap), if present.
    pub preview: Option<Vec<u8>>,
    /// Chunks that were not understood, with their chunk id path from the root.
    pub unknown: Vec<UnknownChunk>,
}

/// Header chunk `0x00`.
#[derive(Debug, Clone, Default)]
pub struct Header {
    /// Format version (always `0x0201` in the shipped maps).
    pub version: u32,
    /// Text key (`TEXT_NONE` for most maps).
    pub text_key: String,
    /// Map name as shown in the editor.
    pub name: String,
    /// Full width / height of the map in game units (equal to the width of [`Bounds::min`]..[`Bounds::max`]).
    pub size: [f32; 2],
    /// All header fields, undecoded.
    pub raw: Minis,
}

/// Chunk `0x103`.
#[derive(Debug, Clone, Default)]
pub struct Bounds {
    /// Lower corner of the map volume (e.g. `-9500, -9500, -5000`).
    pub min: [f32; 3],
    /// Upper corner of the map volume.
    pub max: [f32; 3],
    /// Lower corner of an inner volume, about 96% of the map **(guess: units are kept inside this area)**.
    pub inner_min: [f32; 3],
    /// Upper corner of the inner volume.
    pub inner_max: [f32; 3],
}

/// One of the three scene lights.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Light {
    /// Diffuse colour (linear RGB, 0..1).
    pub color: [f32; 3],
    /// Azimuth in radians **(guess)**.
    pub yaw: f32,
    /// Elevation in radians; positive = above the horizon **(guess: sunset maps have small positive values for the main light)**.
    pub pitch: f32,
    /// Per-light scalar stored next to the colours (probably an intensity) **(guess)**.
    pub scalar: f32,
}

impl Light {
    /// Unit vector pointing from the scene *towards* the light, Z-up, assuming `yaw` is measured from +X
    /// towards +Y **(guess)**.
    pub fn direction_to_light(&self) -> [f32; 3] {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        [cp * cy, cp * sy, sp]
    }
}

/// Distance fog (chunk `0x100/0x02/0x00`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Fog {
    /// Fog colour.
    pub color: [f32; 3],
    /// Whether fog is enabled.
    pub enabled: bool,
    /// Distance where fog starts.
    pub start: f32,
    /// Distance where fog is opaque.
    pub end: f32,
}

/// Lighting, sky and nebula settings (mini-chunk list `0x100/0x04/0x06`).
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// Environment name, normally `Environment 1`.
    pub name: String,
    /// Main light (the sun), then the two fill lights.
    pub lights: [Light; 3],
    /// Specular colour of the sun **(guess)**.
    pub sun_specular: [f32; 3],
    /// Ambient colour.
    pub ambient: [f32; 3],
    /// Fog.
    pub fog: Fog,
    /// Skydome model name without extension (e.g. `AAA_STARS`, `AAA_BESPIN`); empty if none.
    pub skydome: String,
    /// Nebula model name (e.g. `NEBULAE_CLOUD_3_DARK`); empty if none.
    pub nebula: String,
    /// Skydome rotation as Euler degrees **(guess)**.
    pub skydome_rotation_deg: [f32; 3],
    /// Nebula rotation in degrees **(guess)**.
    pub nebula_rotation_deg: [f32; 2],
    /// Cloud texture name (`W_Clouds00.tga`).
    pub cloud_texture: String,
    /// All fields of the chunk, undecoded (ids 21, 23, 27, 28, 35, 36, 40, 43..46 are still unnamed).
    pub raw: Minis,
}

/// A placed object (chunk `0x44c`).
#[derive(Debug, Clone, Default)]
pub struct MapObject {
    /// Unique object id inside the map.
    pub id: u32,
    /// The two ids of the `0x454` chunk (probably editor GUID halves).
    pub guid: [u32; 2],
    /// CRC-32 of the upper-cased game object name.
    pub type_hash: u32,
    /// Game object name, filled by [`SpaceMap::resolve_names`].
    pub type_name: Option<String>,
    /// World position (Z up).
    pub position: [f32; 3],
    /// Euler rotation in degrees.
    pub rotation_deg: [f32; 3],
    /// Uniform scale (always 1.0 in the shipped maps).
    pub scale: f32,
    /// Position as projected on the map plane, used by the editor / minimap **(guess)**. Equal to
    /// `position` for ordinary objects; differs for far-away backdrop planets.
    pub map_position: [f32; 3],
    /// Byte `34`: `0xFF` for most objects, `1` for some entry/defender markers. Possibly a player slot
    /// **(guess)**; `None` when it is `0xFF`.
    pub player_slot: Option<u8>,
    /// Field `2` (3 for almost all objects).
    pub kind: u32,
    /// Field `9` (1 when [`attachment`](Self::attachment) is present).
    pub linked: u32,
    /// Chunk `0x4b8` inside `0x4b7`: four ids (`[hash, 0, 0, id]`), purpose unknown.
    pub attachment: Option<[u32; 4]>,
}

/// Record of top-level chunk `0x02/0x0b`.
///
/// Every record refers to a *Defending Forces Position* object and carries two coordinates in 0.05..0.95
/// (probably the normalized position on the minimap / start screen) **(guess)**.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DefenderSlot {
    /// [`MapObject::id`] of the referenced object.
    pub object_id: u32,
    /// A constant hash (`0x4BACBCFA` in all shipped maps).
    pub hash: u32,
    /// Normalized coordinates.
    pub norm_pos: [f32; 2],
}

/// A chunk the parser did not interpret.
#[derive(Debug, Clone)]
pub struct UnknownChunk {
    /// Chunk ids from the root to this chunk (inclusive).
    pub path: Vec<u32>,
    /// Payload bytes (for containers: the raw child chunk stream).
    pub data: Vec<u8>,
}

/// CRC-32 (IEEE) used by the engine for object type names; the name must be upper-cased first.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Hash of an object name as stored in [`MapObject::type_hash`].
pub fn type_hash(name: &str) -> u32 {
    crc32(name.to_ascii_uppercase().as_bytes())
}

fn err(msg: impl Into<String>) -> ParseError {
    ParseError(msg.into())
}

impl SpaceMap {
    /// Parses a `.ted` file. Works for land maps too (the object list and environment share the layout),
    /// but only space maps were verified.
    pub fn parse(data: &[u8]) -> Result<SpaceMap, ParseError> {
        let mut map = SpaceMap::default();
        let mut saw_header = false;
        let mut saw_body = false;
        for chunk in chunks(data) {
            let chunk = chunk?;
            match chunk.id {
                0 if !chunk.is_container => {
                    map.header = parse_header(&chunk)?;
                    saw_header = true;
                }
                0x13 if !chunk.is_container => map.preview = Some(chunk.data.to_vec()),
                2 if chunk.is_container => {
                    for rec in chunk.children() {
                        let rec = rec?;
                        if rec.id == 0xb && !rec.is_container {
                            map.defender_slots.push(parse_defender_slot(&rec)?);
                        } else {
                            map.unknown.push(unknown(&[2, rec.id], &rec));
                        }
                    }
                }
                1 if chunk.is_container => {
                    parse_body(&chunk, &mut map)?;
                    saw_body = true;
                }
                _ => map.unknown.push(unknown(&[chunk.id], &chunk)),
            }
        }
        if !saw_header || !saw_body {
            return Err(err("not a .ted map: header or body chunk missing"));
        }
        Ok(map)
    }

    /// Fills [`MapObject::type_name`] by hashing every candidate name; names that match no object stay
    /// unresolved. Returns how many objects are resolved afterwards.
    pub fn resolve_names<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) -> usize {
        let table: std::collections::HashMap<u32, &str> =
            names.into_iter().map(|n| (type_hash(n), n)).collect();
        let mut resolved = 0;
        for o in &mut self.objects {
            if o.type_name.is_none() {
                o.type_name = table.get(&o.type_hash).map(|s| (*s).to_string());
            }
            resolved += usize::from(o.type_name.is_some());
        }
        resolved
    }

    /// Objects whose resolved name equals `name` (case-insensitive).
    pub fn objects_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a MapObject> + 'a {
        self.objects.iter().filter(move |o| {
            o.type_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    }
}

impl MapObject {
    /// Coarse classification from the resolved name (see [`ObjectClass`]).
    pub fn class(&self) -> ObjectClass {
        let Some(name) = self.type_name.as_deref() else {
            return ObjectClass::Unknown;
        };
        let u = name.to_ascii_uppercase();
        if u.ends_with("POSITION") || u.ends_with("MARKER") {
            ObjectClass::Marker
        } else if u.starts_with("AAA_PLANET") {
            ObjectClass::BackdropPlanet
        } else if u.contains("ASTEROID") {
            ObjectClass::Asteroid
        } else {
            ObjectClass::Prop
        }
    }
}

/// Coarse object category derived from the name. **(heuristic)**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectClass {
    /// Entry / start / structure placement marker (`Attacker Entry Position`, `Shipyard 1 Marker`, ...).
    Marker,
    /// Far-away decorative planet (`AAA_Planet_*`).
    BackdropPlanet,
    /// Asteroid or asteroid field.
    Asteroid,
    /// Any other prop, structure or ship.
    Prop,
    /// Name not resolved.
    Unknown,
}

fn unknown(path: &[u32], c: &Chunk) -> UnknownChunk {
    UnknownChunk {
        path: path.to_vec(),
        data: c.data.to_vec(),
    }
}

fn parse_header(c: &Chunk) -> Result<Header, ParseError> {
    let m = Minis::parse(c.data)?;
    Ok(Header {
        version: m.u32(0).unwrap_or(0),
        text_key: m.wstr(8).unwrap_or_default(),
        name: m.wstr(9).unwrap_or_default(),
        size: [m.f32(16).unwrap_or(0.0), m.f32(17).unwrap_or(0.0)],
        raw: m,
    })
}

fn parse_defender_slot(c: &Chunk) -> Result<DefenderSlot, ParseError> {
    let m = Minis::parse(c.data)?;
    Ok(DefenderSlot {
        object_id: m.u32(12).ok_or_else(|| err("defender slot without id"))?,
        hash: m.u32(13).unwrap_or(0),
        norm_pos: [m.f32(14).unwrap_or(0.0), m.f32(15).unwrap_or(0.0)],
    })
}

fn parse_body(body: &Chunk, map: &mut SpaceMap) -> Result<(), ParseError> {
    for c in body.children() {
        let c = c?;
        match c.id {
            0x100 if c.is_container => {
                parse_environment(&c, &mut map.environment, &mut map.unknown)?
            }
            0x102 if c.is_container => parse_objects(&c, map)?,
            0x103 if !c.is_container => {
                let m = Minis::parse(c.data)?;
                map.bounds = Bounds {
                    min: m.vec3(4).unwrap_or_default(),
                    max: m.vec3(5).unwrap_or_default(),
                    inner_min: m.vec3(7).unwrap_or_default(),
                    inner_max: m.vec3(8).unwrap_or_default(),
                };
            }
            0x105 if !c.is_container => map.camera_misc = Minis::parse(c.data)?,
            _ => map.unknown.push(unknown(&[1, c.id], &c)),
        }
    }
    Ok(())
}

fn parse_environment(
    env: &Chunk,
    out: &mut Environment,
    unk: &mut Vec<UnknownChunk>,
) -> Result<(), ParseError> {
    let mut fog_chunk: Option<Fog> = None;
    for c in env.children() {
        let c = c?;
        match c.id {
            4 if c.is_container => {
                for l in c.children() {
                    let l = l?;
                    if l.id == 6 && !l.is_container {
                        decode_lighting(&Minis::parse(l.data)?, out);
                    } else if l.id != 5 {
                        unk.push(unknown(&[1, 0x100, 4, l.id], &l));
                    }
                }
            }
            2 if c.is_container => {
                for f in c.children() {
                    let f = f?;
                    if f.id == 0 && !f.is_container {
                        fog_chunk = Some(decode_fog(&Minis::parse(f.data)?, 0, 1, 2, 3));
                    } else {
                        unk.push(unknown(&[1, 0x100, 2, f.id], &f));
                    }
                }
            }
            8 => {}
            _ => unk.push(unknown(&[1, 0x100, c.id], &c)),
        }
    }
    if let Some(f) = fog_chunk {
        out.fog = f;
    }
    Ok(())
}

fn decode_fog(m: &Minis, color: u8, enabled: u8, start: u8, end: u8) -> Fog {
    Fog {
        color: m.vec3(color).unwrap_or_default(),
        enabled: m.u8(enabled).unwrap_or(0) != 0,
        start: m.f32(start).unwrap_or(0.0),
        end: m.f32(end).unwrap_or(0.0),
    }
}

fn decode_lighting(m: &Minis, out: &mut Environment) {
    out.name = m.str(20).unwrap_or_default();
    for i in 0..3u8 {
        out.lights[i as usize] = Light {
            color: m.vec3(i).unwrap_or_default(),
            yaw: m.f32(8 + i).unwrap_or(0.0),
            pitch: m.f32(11 + i).unwrap_or(0.0),
            scalar: m.f32(5 + i).unwrap_or(0.0),
        };
    }
    out.sun_specular = m.vec3(3).unwrap_or_default();
    out.ambient = m.vec3(4).unwrap_or_default();
    out.fog = decode_fog(m, 15, 16, 17, 18);
    out.skydome = m.str(25).unwrap_or_default();
    out.nebula = m.str(26).unwrap_or_default();
    out.skydome_rotation_deg = [29, 30, 31].map(|i| m.f32(i).unwrap_or(0.0));
    out.nebula_rotation_deg = [32, 33].map(|i| m.f32(i).unwrap_or(0.0));
    out.cloud_texture = m.str(47).unwrap_or_default();
    out.raw = m.clone();
}

fn parse_objects(c: &Chunk, map: &mut SpaceMap) -> Result<(), ParseError> {
    for child in c.children() {
        let child = child?;
        match child.id {
            0 | 8 => {}
            1 if child.is_container => {
                for o in child.children() {
                    let o = o?;
                    if o.id == 0x44c && o.is_container {
                        map.objects.push(parse_object(&o)?);
                    } else {
                        map.unknown.push(unknown(&[1, 0x102, 1, o.id], &o));
                    }
                }
            }
            _ => map.unknown.push(unknown(&[1, 0x102, child.id], &child)),
        }
    }
    Ok(())
}

fn parse_object(c: &Chunk) -> Result<MapObject, ParseError> {
    let mut obj = MapObject {
        scale: 1.0,
        ..MapObject::default()
    };
    let mut have_props = false;
    for part in c.children() {
        let part = part?;
        match part.id {
            0x454 => {
                let m = Minis::parse(part.data)?;
                obj.guid = [m.u32(0).unwrap_or(0), m.u32(1).unwrap_or(0)];
            }
            0x459 if part.is_container => {
                for p in part.children() {
                    let p = p?;
                    match p.id {
                        0x4b0 if !p.is_container => {
                            decode_props(&Minis::parse(p.data)?, &mut obj)?;
                            have_props = true;
                        }
                        0x4b7 if p.is_container => {
                            for a in p.children() {
                                let a = a?;
                                if a.id == 0x4b8 {
                                    let m = Minis::parse(a.data)?;
                                    obj.attachment =
                                        Some([0, 1, 2, 3].map(|i| m.u32(i).unwrap_or(0)));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if have_props {
        Ok(obj)
    } else {
        Err(err("object without property chunk 0x4b0"))
    }
}

fn decode_props(m: &Minis, o: &mut MapObject) -> Result<(), ParseError> {
    o.id = m.u32(0).ok_or_else(|| err("object without id"))?;
    o.type_hash = m.u32(1).ok_or_else(|| err("object without type hash"))?;
    o.kind = m.u32(2).unwrap_or(0);
    o.scale = m.f32(3).unwrap_or(1.0);
    o.position = m.vec3(4).ok_or_else(|| err("object without position"))?;
    o.rotation_deg = m.vec3(5).unwrap_or_default();
    o.linked = m.u32(9).unwrap_or(0);
    o.map_position = m.vec3(21).unwrap_or(o.position);
    o.player_slot = m.u8(34).filter(|&s| s != 0xFF);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(type_hash("Asteroid_Large"), 0x8EB6_541F);
    }

    #[test]
    fn rejects_garbage() {
        assert!(SpaceMap::parse(&[1, 2, 3]).is_err());
        assert!(SpaceMap::parse(&[]).is_err());
    }
}
