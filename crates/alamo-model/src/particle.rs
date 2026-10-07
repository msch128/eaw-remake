//! Particle system (`.alo` files with root chunk `0x900`) parsing.
//!
//! The format is not publicly documented; everything below was reverse engineered from the 1,505
//! particle files of the Remake. Fields whose meaning is *inferred* from value statistics and
//! effect names (rather than known) are marked as such. Unrecognized emitter properties are kept
//! raw in [`Emitter::props`].
//!
//! # Layout
//!
//! ```text
//! 0x900 system
//!   0x0   name (C string)
//!   0x1   system flags (mini-chunks without payload)
//!   0x800 emitter list
//!     0x700 emitter (one per emitter)
//!       0x2   properties (mini-chunks: u8 id, u8 size, payload)
//!       0x3   texture path   0x45 second texture path   0x16 emitter name
//!       0x29  three 0x1100 containers, each with a 0x1101 shape block (position, velocity, extra)
//!       0x1   seven tracks, each a 0x0 header (start, end, mode) followed by a 0x1 key list
//!       0x36  links to other emitters (mini-chunks 0x37, 0x39; 0xffffffff = none)
//! ```
//!
//! Tracks, in order: red, green, blue, alpha (byte values, `0..=255`), size, atlas frame index
//! (start and end frame, so it animates as a flipbook when they differ; mode 2) and a rotation
//! speed. A track runs from `start` at birth to `end` at
//! the end of the particle's life; color keys `(value, time)` are knots in between, with `time` as
//! a fraction of the life.
//!
//! Positions and velocities are in the emitter's local space, which is **Y-up** (not the Z-up of
//! models): when an emitter sits on a model proxy, particle `+Y` lines up with the proxy bone's
//! `+Z` axis (engine exhaust leaving a Star Destroyer's stern, damage fire leaving the hull).

use std::collections::BTreeMap;

use crate::alo::minis;
use crate::chunk::{Chunk, Chunks, cstr};
use crate::error::{Error, Result};

/// How particles are blended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    /// Additive (emitter type 1). Inferred.
    Additive,
    /// Type 4: dynamic light emitters (`light_*`, `muzzle_flash` light halos) whose "size" is a light
    /// radius of hundreds to thousands of units; not sprites. The track colors are the light color
    /// over its life. Inferred.
    Light,
    /// Alpha blended (type 2 and 5). Inferred.
    Alpha,
    /// Frame-animation emitters (type 0, e.g. `frames_240`); treated as additive. Inferred.
    Frames,
    /// Type 11: mesh debris / asteroid fields; not drawn as sprites. Inferred.
    Mesh,
    /// Unknown type value.
    Other(u32),
}

/// A position, velocity or extra distribution block (`0x1101`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// Shape type: 0 none/box, 1 axis range, 3 box. Inferred.
    pub kind: u32,
    /// Lower corner.
    pub min: [f32; 3],
    /// Upper corner.
    pub max: [f32; 3],
    /// The remaining nine floats of the block (meaning unknown).
    pub params: [f32; 9],
}

/// A color channel curve over a particle's life, normalized to `0..=1`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColorTrack {
    /// Value at birth.
    pub start: f32,
    /// Value at the end of life.
    pub end: f32,
    /// `(time fraction, value)` knots, sorted by time.
    pub keys: Vec<(f32, f32)>,
}

impl ColorTrack {
    /// Value at life fraction `t` (0..=1).
    pub fn at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let (mut pt, mut pv) = (0.0, self.start);
        for &(kt, kv) in &self.keys {
            if t <= kt {
                let span = (kt - pt).max(1e-6);
                return pv + (kv - pv) * ((t - pt) / span);
            }
            (pt, pv) = (kt, kv);
        }
        let span = (1.0 - pt).max(1e-6);
        pv + (self.end - pv) * ((t - pt) / span)
    }
}

/// A float curve (size, spin) between a start and end value.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FloatTrack {
    /// Value at birth.
    pub start: f32,
    /// Value at the end of life.
    pub end: f32,
    /// Mode word of the track header (0, 1 or 2; meaning unknown).
    pub mode: u32,
}

impl FloatTrack {
    /// Linear value at life fraction `t`.
    pub fn at(&self, t: f32) -> f32 {
        self.start + (self.end - self.start) * t.clamp(0.0, 1.0)
    }
}

/// One particle emitter.
#[derive(Debug, Clone)]
pub struct Emitter {
    /// Emitter name, e.g. `smoke`.
    pub name: String,
    /// Texture file name without directory, e.g. `p_particle_master.tga`.
    pub texture: String,
    /// Second texture (chunk `0x45`), usually identical to `texture`.
    pub texture2: Option<String>,
    /// Blend mode.
    pub blend: BlendMode,
    /// Particles live in world space once spawned (property `0x07`). Inferred.
    pub world_space: bool,
    /// Sprites keep the emitter's orientation instead of facing the camera (property `0x08`). Inferred.
    pub oriented: bool,
    /// Particle lifetime in seconds (`0x0f`). Inferred.
    pub lifetime: f32,
    /// Texture atlas is `grid x grid` cells, `grid = sqrt(0x10)`. Inferred.
    pub atlas_grid: u32,
    /// Index of the emitter within its system (`0x14`).
    pub index: u32,
    /// Properties `0x12` and `0x13` (two fractions of unknown meaning, kept for reference).
    pub unknown_fractions: (f32, f32),
    /// Seconds before the emitter starts (`0x24`). Inferred.
    pub initial_delay: f32,
    /// Particles per second in continuous mode (`0x2a`). Inferred.
    pub spawn_rate: f32,
    /// Seconds between spawns (`0x25`, about `1 / spawn_rate`). Inferred.
    pub spawn_interval: f32,
    /// Maximum number of live particles (`0x26`). Inferred.
    pub max_particles: u32,
    /// `0x27`: 0 = continuous, otherwise burst mode with this many repeats (`u32::MAX` = forever). Inferred.
    pub loop_count: u32,
    /// Speed multiplier (`0x09`). Inferred.
    pub speed: f32,
    /// Constant acceleration (`0x0a`). Inferred.
    pub acceleration: [f32; 3],
    /// Spawn position distribution.
    pub position: Shape,
    /// Initial velocity distribution.
    pub velocity: Shape,
    /// Third distribution (random velocity / force); meaning inferred.
    pub extra: Shape,
    /// Red, green, blue and alpha curves.
    pub color: [ColorTrack; 4],
    /// Particle size curve (world units).
    pub size: FloatTrack,
    /// Atlas cell (row-major from the top left) in `start`; `end` is a placeholder (usually 2 or 0,
    /// the track is in mode 2), so the sprite is fixed per emitter. Track 5.
    pub frame: FloatTrack,
    /// Rotation speed, probably degrees per second (track 6). Inferred.
    pub spin: FloatTrack,
    /// Links to other emitters (`0x37`, `0x39`); `u32::MAX` means none.
    pub links: [u32; 2],
    /// All properties of chunk `0x2` by id, raw payloads.
    pub props: BTreeMap<u8, Vec<u8>>,
}

impl Emitter {
    /// Raw `u32` property.
    pub fn prop_u32(&self, id: u8) -> Option<u32> {
        let p = self.props.get(&id)?;
        Some(u32::from_le_bytes(p.get(..4)?.try_into().ok()?))
    }

    /// Raw `f32` property.
    pub fn prop_f32(&self, id: u8) -> Option<f32> {
        self.prop_u32(id).map(f32::from_bits)
    }

    /// Raw flag property.
    pub fn prop_flag(&self, id: u8) -> bool {
        self.props
            .get(&id)
            .and_then(|p| p.first())
            .is_some_and(|&b| b != 0)
    }
}

/// A particle system: a named list of emitters.
#[derive(Debug, Clone, Default)]
pub struct ParticleSystem {
    /// System name.
    pub name: String,
    /// Emitters in file order.
    pub emitters: Vec<Emitter>,
}

fn f32_at(d: &[u8], i: usize) -> f32 {
    d.get(i * 4..i * 4 + 4)
        .map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn parse_shape(c: &Chunk) -> Shape {
    let d = c.child(0x1101).map_or(&[][..], |s| s.data);
    let kind = d
        .get(..4)
        .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let mut params = [0.0; 9];
    for (i, p) in params.iter_mut().enumerate() {
        *p = f32_at(d, 7 + i);
    }
    Shape {
        kind,
        min: [f32_at(d, 1), f32_at(d, 2), f32_at(d, 3)],
        max: [f32_at(d, 4), f32_at(d, 5), f32_at(d, 6)],
        params,
    }
}

/// Reads a track header; returns `(is_byte, start, end, mode)`.
fn track_header(d: &[u8]) -> (bool, f32, f32, u32) {
    let (mut is_byte, mut start, mut end, mut mode) = (false, 0.0, 0.0, 0);
    for (id, p) in minis(d) {
        let val = |p: &[u8]| match p.len() {
            1 => (true, p[0] as f32 / 255.0),
            4 => (false, f32::from_le_bytes([p[0], p[1], p[2], p[3]])),
            _ => (false, 0.0),
        };
        match id {
            2 => (is_byte, start) = val(p),
            3 => end = val(p).1,
            4 => {
                mode = p
                    .get(..4)
                    .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            _ => {}
        }
    }
    (is_byte, start, end, mode)
}

fn parse_tracks(c: &Chunk) -> Result<([ColorTrack; 4], [FloatTrack; 3])> {
    let mut color: [ColorTrack; 4] = Default::default();
    let mut floats = [FloatTrack::default(); 3];
    let mut index = 0usize;
    let mut header: Option<(bool, f32, f32, u32)> = None;
    for ch in c.children() {
        let ch = ch?;
        match ch.id {
            0x0 => header = Some(track_header(ch.data)),
            0x1 => {
                let (is_byte, start, end, mode) = header.take().unwrap_or_default();
                if index < 4 {
                    // Color tracks store bytes; keys are `(u32 value 0..255, f32 time)`.
                    let mut keys: Vec<(f32, f32)> = minis(ch.data)
                        .filter(|(id, p)| *id == 5 && p.len() == 8)
                        .map(|(_, p)| {
                            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as f32 / 255.0;
                            let t = f32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                            (t, v)
                        })
                        .collect();
                    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let scale = if is_byte { 1.0 } else { 1.0 / 255.0 };
                    color[index] = ColorTrack {
                        start: start * scale,
                        end: end * scale,
                        keys,
                    };
                } else if index < 7 {
                    floats[index - 4] = FloatTrack { start, end, mode };
                }
                index += 1;
            }
            _ => {}
        }
    }
    Ok((color, floats))
}

fn base_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

fn parse_emitter(c: &Chunk) -> Result<Emitter> {
    let mut props = BTreeMap::new();
    let mut texture = String::new();
    let mut texture2 = None;
    let mut name = String::new();
    let mut shapes = Vec::new();
    let mut tracks = None;
    let mut links = [u32::MAX; 2];
    for ch in c.children() {
        let ch = ch?;
        match ch.id {
            0x2 => {
                for (id, p) in minis(ch.data) {
                    props.insert(id, p.to_vec());
                }
            }
            0x3 => texture = base_name(&cstr(ch.data)),
            0x45 => texture2 = Some(base_name(&cstr(ch.data))),
            0x16 => name = cstr(ch.data),
            0x29 => {
                for s in ch.children() {
                    shapes.push(parse_shape(&s?));
                }
            }
            0x1 => tracks = Some(parse_tracks(&ch)?),
            0x36 => {
                for (id, p) in minis(ch.data) {
                    let v = p
                        .get(..4)
                        .map_or(u32::MAX, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    match id {
                        0x37 => links[0] = v,
                        0x39 => links[1] = v,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if props.is_empty() {
        return Err(Error::Invalid(format!(
            "emitter {name:?} without properties"
        )));
    }
    let (color, floats) =
        tracks.ok_or_else(|| Error::Invalid(format!("emitter {name:?} without tracks")))?;
    shapes.resize(
        3,
        Shape {
            kind: 0,
            min: [0.0; 3],
            max: [0.0; 3],
            params: [0.0; 9],
        },
    );
    let mut e = Emitter {
        name,
        texture,
        texture2,
        blend: BlendMode::Other(0),
        world_space: false,
        oriented: false,
        lifetime: 1.0,
        atlas_grid: 1,
        index: 0,
        unknown_fractions: (0.0, 0.0),
        initial_delay: 0.0,
        spawn_rate: 0.0,
        spawn_interval: 0.0,
        max_particles: 0,
        loop_count: 0,
        speed: 1.0,
        acceleration: [0.0; 3],
        position: shapes[0],
        velocity: shapes[1],
        extra: shapes[2],
        color,
        size: floats[0],
        frame: floats[1],
        spin: floats[2],
        links,
        props,
    };
    e.blend = match e.prop_u32(0x04).unwrap_or(0) {
        1 => BlendMode::Additive,
        4 => BlendMode::Light,
        2 | 5 => BlendMode::Alpha,
        0 => BlendMode::Frames,
        11 => BlendMode::Mesh,
        o => BlendMode::Other(o),
    };
    e.world_space = e.prop_flag(0x07);
    e.oriented = e.prop_flag(0x08);
    e.lifetime = e.prop_f32(0x0f).unwrap_or(1.0).max(0.001);
    let cells = e.prop_u32(0x10).unwrap_or(1).max(1);
    e.atlas_grid = (cells as f32).sqrt().round().max(1.0) as u32;
    e.index = e.prop_u32(0x14).unwrap_or(0);
    e.unknown_fractions = (
        e.prop_f32(0x12).unwrap_or(0.0),
        e.prop_f32(0x13).unwrap_or(0.0),
    );
    e.initial_delay = e.prop_f32(0x24).unwrap_or(0.0).max(0.0);
    e.spawn_interval = e.prop_f32(0x25).unwrap_or(0.0).max(0.0);
    e.max_particles = e.prop_u32(0x26).unwrap_or(0);
    e.loop_count = e.prop_u32(0x27).unwrap_or(0);
    e.spawn_rate = e.prop_u32(0x2a).unwrap_or(0) as f32;
    e.speed = e.prop_f32(0x09).filter(|v| v.is_finite()).unwrap_or(0.0);
    if let Some(p) = e.props.get(&0x0a)
        && p.len() >= 12
    {
        e.acceleration = [f32_at(p, 0), f32_at(p, 1), f32_at(p, 2)];
    }
    Ok(e)
}

impl ParticleSystem {
    /// Parses a particle `.alo`. Files that are models yield [`Error::NotAModel`]-like
    /// [`Error::Invalid`].
    pub fn parse(data: &[u8]) -> Result<ParticleSystem> {
        let root = Chunks::new(data)
            .find_map(|c| match c {
                Ok(c) if c.id == 0x900 => Some(Ok(c)),
                Err(e) => Some(Err(e)),
                _ => None,
            })
            .ok_or_else(|| Error::Invalid("not a particle system".into()))??;
        let mut sys = ParticleSystem::default();
        for ch in root.children() {
            let ch = ch?;
            match ch.id {
                0x0 => sys.name = cstr(ch.data),
                0x800 => {
                    for e in ch.children() {
                        let e = e?;
                        if e.id == 0x700 {
                            sys.emitters.push(parse_emitter(&e)?);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(sys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u32, container: bool, payload: &[u8]) -> Vec<u8> {
        let mut v = id.to_le_bytes().to_vec();
        let size = payload.len() as u32 | if container { 0x8000_0000 } else { 0 };
        v.extend(size.to_le_bytes());
        v.extend(payload);
        v
    }

    fn mini(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![id, payload.len() as u8];
        v.extend(payload);
        v
    }

    fn sample() -> Vec<u8> {
        let mut props = mini(0x04, &2u32.to_le_bytes());
        props.extend(mini(0x0f, &2.5f32.to_le_bytes()));
        props.extend(mini(0x10, &64u32.to_le_bytes()));
        props.extend(mini(0x07, &[1]));
        let mut tracks = Vec::new();
        for (start, end, key) in [
            (255u8, 65u8, Some((100u32, 0.5f32))),
            (0, 0, None),
            (0, 0, None),
            (0, 0, None),
        ] {
            let mut h = mini(2, &[start]);
            h.extend(mini(3, &[end]));
            h.extend(mini(4, &0u32.to_le_bytes()));
            tracks.extend(chunk(0, false, &h));
            let mut k = Vec::new();
            if let Some((v, t)) = key {
                let mut p = v.to_le_bytes().to_vec();
                p.extend(t.to_le_bytes());
                k.extend(mini(5, &p));
            }
            tracks.extend(chunk(1, false, &k));
        }
        for (a, b) in [(10.0f32, 20.0f32), (0.0, 0.0), (0.0, 0.0)] {
            let mut h = mini(2, &a.to_le_bytes());
            h.extend(mini(3, &b.to_le_bytes()));
            h.extend(mini(4, &0u32.to_le_bytes()));
            tracks.extend(chunk(0, false, &h));
            tracks.extend(chunk(1, false, &[]));
        }
        let mut shapes = Vec::new();
        for _ in 0..3 {
            let mut s = 3u32.to_le_bytes().to_vec();
            s.extend([0u8; 60]);
            shapes.extend(chunk(0x1100, true, &chunk(0x1101, false, &s)));
        }
        let mut emitter = chunk(2, false, &props);
        emitter.extend(chunk(3, false, b"C:\\Art\\Textures\\Game\\tex.tga\0"));
        emitter.extend(chunk(0x16, false, b"smoke\0"));
        emitter.extend(chunk(0x29, true, &shapes));
        emitter.extend(chunk(1, true, &tracks));
        let mut sys = chunk(0, false, b"test\0");
        sys.extend(chunk(0x800, true, &chunk(0x700, true, &emitter)));
        chunk(0x900, true, &sys)
    }

    #[test]
    fn parses_synthetic_system() {
        let s = ParticleSystem::parse(&sample()).unwrap();
        assert_eq!(s.name, "test");
        let e = &s.emitters[0];
        assert_eq!((e.name.as_str(), e.texture.as_str()), ("smoke", "tex.tga"));
        assert_eq!(e.blend, BlendMode::Alpha);
        assert!(e.world_space);
        assert_eq!((e.lifetime, e.atlas_grid), (2.5, 8));
        assert_eq!(e.size.at(0.5), 15.0);
        let r = &e.color[0];
        assert_eq!(r.at(0.0), 1.0);
        assert!((r.at(0.5) - 100.0 / 255.0).abs() < 1e-6);
        assert!((r.at(1.0) - 65.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn truncation_is_an_error_not_a_panic() {
        let data = sample();
        for cut in [3, 40, data.len() / 2, data.len() - 1] {
            let _ = ParticleSystem::parse(&data[..cut]);
        }
        assert!(ParticleSystem::parse(&data[..data.len() - 1]).is_err());
        assert!(ParticleSystem::parse(&[]).is_err());
    }
}
