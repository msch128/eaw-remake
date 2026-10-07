//! Alamo animation (`.ala`) parsing.
//!
//! An animation has a frame count, a frame rate and, per bone, translation / rotation / scale
//! tracks plus an optional visibility track. Bones are matched to the model skeleton **by name**
//! (case-insensitive); the index stored in the file counts animated bones only.
//!
//! Two on-disk encodings exist and both decode to the same [`Animation`]:
//!
//! * *per-bone*: each bone chunk (`0x1002`) carries its own key chunks `0x1004` (translation,
//!   3 x u16 per frame), `0x1006` (rotation, 4 x i16 per frame) and `0x1007` / `0x1008`
//!   (bit fields, see [`BoneTrack::visibility`]). A key chunk holding a single key (rotation: 8 bytes)
//!   means the value is constant.
//! * *pooled*: the bone only references channels (`0x0e` translation, `0x10` rotation, `0x0f` scale
//!   mini-chunks) of the shared pools `0x100a` / `0x1009` / `0x100b`, which are laid out
//!   `[frame][channel]` as u16. The header gives the channel counts (`0x0b` rotation,
//!   `0x0c` translation, `0x0d` scale). A channel index of `0xffff` means constant.
//!
//! Dequantization: translation `= base + q * step` (u16 `q`), scale likewise, rotation is a
//! quaternion `(x, y, z, w) = i16 / 32767`. Bone transforms are *local* (relative to the parent
//! bone) and replace the bone's transform from the model skeleton, in the same Alamo space.

use crate::alo::minis;
use crate::chunk::{Chunk, Chunks, cstr};
use crate::error::{Error, Result};

/// Chunk ids used by `.ala` files.
pub mod ids {
    /// Root container.
    pub const ANIMATION: u32 = 0x1000;
    /// Header (frames, fps, bone count, pool channel counts).
    pub const HEADER: u32 = 0x1001;
    /// One animated bone (container).
    pub const BONE: u32 = 0x1002;
    /// Bone description mini-chunks.
    pub const BONE_INFO: u32 = 0x1003;
    /// Per-bone translation keys.
    pub const TRANSLATION_KEYS: u32 = 0x1004;
    /// Per-bone rotation keys.
    pub const ROTATION_KEYS: u32 = 0x1006;
    /// Per-bone visibility bit field.
    pub const VISIBILITY: u32 = 0x1007;
    /// Per-bone second bit field (meaning unknown, kept raw).
    pub const UNKNOWN_BITS: u32 = 0x1008;
    /// Shared rotation pool.
    pub const ROTATION_POOL: u32 = 0x1009;
    /// Shared translation pool.
    pub const TRANSLATION_POOL: u32 = 0x100a;
    /// Shared scale pool.
    pub const SCALE_POOL: u32 = 0x100b;
}

/// A value over time: either constant or one key per frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Track<T> {
    /// The same value on every frame.
    Constant(T),
    /// One value per frame (`frames` entries).
    Keys(Vec<T>),
}

impl<T: Copy> Track<T> {
    /// Value at `frame` (clamped to the last key).
    pub fn at(&self, frame: usize) -> T {
        match self {
            Track::Constant(v) => *v,
            Track::Keys(k) => k[frame.min(k.len() - 1)],
        }
    }

    /// True if the track has one key per frame.
    pub fn is_animated(&self) -> bool {
        matches!(self, Track::Keys(_))
    }
}

/// Animation of one bone.
#[derive(Debug, Clone)]
pub struct BoneTrack {
    /// Bone name, matches a [`crate::alo::Bone::name`].
    pub name: String,
    /// Static visibility flag stored with the bone (mini-chunk `0x0a`).
    pub visible: bool,
    /// Local translation.
    pub translation: Track<[f32; 3]>,
    /// Local rotation quaternion `(x, y, z, w)`.
    pub rotation: Track<[f32; 4]>,
    /// Local scale.
    pub scale: Track<[f32; 3]>,
    /// Per-frame visibility (bit `i` of chunk `0x1007`), if the bone has a visibility track.
    pub visibility: Option<Vec<bool>>,
    /// Raw bytes of chunk `0x1008` (meaning unknown), if present.
    pub unknown_bits: Option<Vec<u8>>,
}

/// A parsed `.ala` animation.
#[derive(Debug, Clone)]
pub struct Animation {
    /// Number of frames.
    pub frames: u32,
    /// Frames per second.
    pub fps: f32,
    /// Animated bones.
    pub bones: Vec<BoneTrack>,
}

impl Animation {
    /// Duration in seconds (`frames / fps`; one frame fewer when counting intervals).
    pub fn duration(&self) -> f32 {
        if self.fps > 0.0 {
            self.frames.saturating_sub(1) as f32 / self.fps
        } else {
            0.0
        }
    }

    /// Track for the bone called `name` (case-insensitive).
    pub fn track(&self, name: &str) -> Option<&BoneTrack> {
        self.bones
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(name))
    }

    /// Parses a `.ala` file.
    pub fn parse(data: &[u8]) -> Result<Animation> {
        let root = Chunks::new(data)
            .find_map(|c| match c {
                Ok(c) if c.id == ids::ANIMATION => Some(Ok(c)),
                Err(e) => Some(Err(e)),
                _ => None,
            })
            .ok_or_else(|| Error::Invalid("no animation chunk".into()))??;

        let (mut frames, mut fps, mut bone_count) = (None, 30.0f32, None);
        let (mut rot_ch, mut tr_ch, mut sc_ch) = (0usize, 0usize, 0usize);
        let mut pools: [&[u8]; 3] = [&[]; 3]; // rotation, translation, scale
        let mut bone_chunks = Vec::new();
        for c in root.children() {
            let c = c?;
            match c.id {
                ids::HEADER => {
                    for (id, p) in minis(c.data) {
                        let u =
                            u32::from_le_bytes(p.get(..4).unwrap_or(&[0; 4]).try_into().unwrap());
                        match id {
                            1 => frames = Some(u),
                            2 => fps = f32::from_bits(u),
                            3 => bone_count = Some(u as usize),
                            0x0b => rot_ch = u as usize,
                            0x0c => tr_ch = u as usize,
                            0x0d => sc_ch = u as usize,
                            _ => {}
                        }
                    }
                }
                ids::ROTATION_POOL => pools[0] = c.data,
                ids::TRANSLATION_POOL => pools[1] = c.data,
                ids::SCALE_POOL => pools[2] = c.data,
                ids::BONE => bone_chunks.push(c),
                _ => {}
            }
        }
        let frames = frames.ok_or_else(|| Error::Invalid("animation without header".into()))?;
        if frames == 0 || frames > 1_000_000 {
            return Err(Error::Invalid(format!("implausible frame count {frames}")));
        }
        if let Some(n) = bone_count
            && n != bone_chunks.len()
        {
            return Err(Error::Invalid(format!(
                "header declares {n} bones, found {}",
                bone_chunks.len()
            )));
        }
        let nf = frames as usize;
        let pool = |data: &[u8], channels: usize, what: &str| -> Result<Vec<u16>> {
            let want = channels
                .checked_mul(nf)
                .and_then(|n| n.checked_mul(2))
                .ok_or_else(|| Error::Invalid(format!("{what} pool size overflow")))?;
            if data.len() != want {
                return Err(Error::Invalid(format!(
                    "{what} pool is {} bytes, expected {want}",
                    data.len()
                )));
            }
            Ok(data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes(*b))
                .collect())
        };
        let rot_pool = pool(pools[0], rot_ch, "rotation")?;
        let tr_pool = pool(pools[1], tr_ch, "translation")?;
        let sc_pool = pool(pools[2], sc_ch, "scale")?;
        let ctx = Ctx {
            frames: nf,
            rot: (&rot_pool, rot_ch),
            tr: (&tr_pool, tr_ch),
            sc: (&sc_pool, sc_ch),
        };
        let bones = bone_chunks
            .iter()
            .map(|c| parse_bone(c, &ctx))
            .collect::<Result<_>>()?;
        Ok(Animation { frames, fps, bones })
    }
}

struct Ctx<'a> {
    frames: usize,
    rot: (&'a [u16], usize),
    tr: (&'a [u16], usize),
    sc: (&'a [u16], usize),
}

fn f3(p: &[u8]) -> Option<[f32; 3]> {
    let f = |i: usize| {
        Some(f32::from_le_bytes(
            p.get(i * 4..i * 4 + 4)?.try_into().ok()?,
        ))
    };
    Some([f(0)?, f(1)?, f(2)?])
}

fn quat(q: [i16; 4]) -> [f32; 4] {
    q.map(|v| (v as f32 / 32767.0).clamp(-1.0, 1.0))
}

fn u16s(d: &[u8]) -> impl Iterator<Item = u16> + '_ {
    d.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b))
}

fn i16s4(d: &[u8]) -> impl Iterator<Item = [i16; 4]> + '_ {
    d.as_chunks::<8>().0.iter().map(|b| {
        let v = |i: usize| i16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);
        [v(0), v(1), v(2), v(3)]
    })
}

fn dequant(q: [u16; 3], base: [f32; 3], step: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| base[i] + q[i] as f32 * step[i])
}

fn parse_bone(c: &Chunk, ctx: &Ctx) -> Result<BoneTrack> {
    let nf = ctx.frames;
    let mut name = String::new();
    let mut visible = true;
    let (mut t_base, mut t_step) = ([0.0; 3], [0.0; 3]);
    let (mut s_base, mut s_step) = ([1.0; 3], [0.0; 3]);
    let (mut t_idx, mut s_idx, mut r_idx) = (0xffffu16, 0xffffu16, 0xffffu16);
    let mut r_default = [0i16, 0, 0, 32767];
    let (mut t_keys, mut r_keys, mut vis, mut unk) = (None, None, None, None);
    for ch in c.children() {
        let ch = ch?;
        match ch.id {
            ids::BONE_INFO => {
                for (id, p) in minis(ch.data) {
                    let u16v = || p.get(..2).map(|b| u16::from_le_bytes([b[0], b[1]]));
                    match id {
                        4 => name = cstr(p),
                        0x0a => visible = p.first().is_some_and(|&b| b != 0),
                        6 => t_base = f3(p).unwrap_or(t_base),
                        7 => t_step = f3(p).unwrap_or(t_step),
                        8 => s_base = f3(p).unwrap_or(s_base),
                        9 => s_step = f3(p).unwrap_or(s_step),
                        0x0e => t_idx = u16v().unwrap_or(0xffff),
                        0x0f => s_idx = u16v().unwrap_or(0xffff),
                        0x10 => r_idx = u16v().unwrap_or(0xffff),
                        0x11 if p.len() >= 8 => r_default = i16s4(p).next().unwrap_or(r_default),
                        _ => {}
                    }
                }
            }
            ids::TRANSLATION_KEYS => t_keys = Some(ch.data),
            ids::ROTATION_KEYS => r_keys = Some(ch.data),
            ids::VISIBILITY => vis = Some(ch.data),
            ids::UNKNOWN_BITS => unk = Some(ch.data.to_vec()),
            _ => {}
        }
    }
    let bad = |what: &str, got: usize| {
        Error::Invalid(format!(
            "bone {name:?}: {what} data has bad size {got} for {nf} frames"
        ))
    };

    let translation = if let Some(d) = t_keys {
        match d.len() {
            6 => Track::Constant(dequant(
                [
                    u16s(d).next().unwrap_or(0),
                    u16s(d).nth(1).unwrap_or(0),
                    u16s(d).nth(2).unwrap_or(0),
                ],
                t_base,
                t_step,
            )),
            n if n == nf * 6 => {
                let v: Vec<u16> = u16s(d).collect();
                Track::Keys(
                    v.as_chunks::<3>()
                        .0
                        .iter()
                        .map(|q| dequant(*q, t_base, t_step))
                        .collect(),
                )
            }
            n => return Err(bad("translation", n)),
        }
    } else if t_idx != 0xffff {
        let i = t_idx as usize;
        if i + 3 > ctx.tr.1 {
            return Err(Error::Invalid(format!(
                "bone {name:?}: translation channel {i} out of range"
            )));
        }
        Track::Keys(
            (0..nf)
                .map(|f| {
                    let o = f * ctx.tr.1 + i;
                    dequant(
                        [ctx.tr.0[o], ctx.tr.0[o + 1], ctx.tr.0[o + 2]],
                        t_base,
                        t_step,
                    )
                })
                .collect(),
        )
    } else {
        Track::Constant(t_base)
    };

    let rotation = if let Some(d) = r_keys {
        match d.len() {
            8 => Track::Constant(quat(i16s4(d).next().unwrap_or(r_default))),
            n if n == nf * 8 => Track::Keys(i16s4(d).map(quat).collect()),
            n => return Err(bad("rotation", n)),
        }
    } else if r_idx != 0xffff {
        let i = r_idx as usize;
        if i + 4 > ctx.rot.1 {
            return Err(Error::Invalid(format!(
                "bone {name:?}: rotation channel {i} out of range"
            )));
        }
        Track::Keys(
            (0..nf)
                .map(|f| {
                    let o = f * ctx.rot.1 + i;
                    quat([0, 1, 2, 3].map(|k| ctx.rot.0[o + k] as i16))
                })
                .collect(),
        )
    } else {
        Track::Constant(quat(r_default))
    };

    let scale = if s_idx != 0xffff {
        let i = s_idx as usize;
        if i + 3 > ctx.sc.1 {
            return Err(Error::Invalid(format!(
                "bone {name:?}: scale channel {i} out of range"
            )));
        }
        Track::Keys(
            (0..nf)
                .map(|f| {
                    let o = f * ctx.sc.1 + i;
                    dequant(
                        [ctx.sc.0[o], ctx.sc.0[o + 1], ctx.sc.0[o + 2]],
                        s_base,
                        s_step,
                    )
                })
                .collect(),
        )
    } else {
        Track::Constant(s_base)
    };

    let visibility = vis.map(|d| {
        (0..nf)
            .map(|f| d.get(f / 8).is_some_and(|b| b >> (f % 8) & 1 != 0))
            .collect()
    });
    Ok(BoneTrack {
        name,
        visible,
        translation,
        rotation,
        scale,
        visibility,
        unknown_bits: unk,
    })
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

    fn f32s(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn header(frames: u32, bones: u32, extra: &[(u8, u32)]) -> Vec<u8> {
        let mut h = mini(1, &frames.to_le_bytes());
        h.extend(mini(2, &30.0f32.to_le_bytes()));
        h.extend(mini(3, &bones.to_le_bytes()));
        for (id, v) in extra {
            h.extend(mini(*id, &v.to_le_bytes()));
        }
        chunk(ids::HEADER, false, &h)
    }

    fn info(name: &str, extra: &[u8]) -> Vec<u8> {
        let mut i = mini(4, &[name.as_bytes(), &[0]].concat());
        i.extend(mini(5, &1u32.to_le_bytes()));
        i.extend(mini(0x0a, &1u32.to_le_bytes()));
        i.extend(mini(6, &f32s(&[1.0, 2.0, 3.0])));
        i.extend(mini(7, &f32s(&[0.5, 0.5, 0.5])));
        i.extend(mini(8, &f32s(&[1.0, 1.0, 1.0])));
        i.extend(mini(9, &f32s(&[0.0, 0.0, 0.0])));
        i.extend_from_slice(extra);
        chunk(ids::BONE_INFO, false, &i)
    }

    #[test]
    fn per_bone_keys() {
        let mut bone = info("Arm", &[]);
        // 2 frames of translation, rotation (identity then 90 deg about z), visibility bits 0b01
        bone.extend(chunk(
            ids::TRANSLATION_KEYS,
            false,
            &[0, 0, 2, 0, 4, 0, 2, 0, 2, 0, 2, 0],
        ));
        let mut rot = Vec::new();
        for q in [[0i16, 0, 0, 32767], [0, 0, 23170, 23170]] {
            rot.extend(q.iter().flat_map(|v| v.to_le_bytes()));
        }
        bone.extend(chunk(ids::ROTATION_KEYS, false, &rot));
        bone.extend(chunk(ids::VISIBILITY, false, &[1]));
        let mut root = header(2, 1, &[]);
        root.extend(chunk(ids::BONE, true, &bone));
        let a = Animation::parse(&chunk(ids::ANIMATION, true, &root)).unwrap();
        assert_eq!((a.frames, a.fps), (2, 30.0));
        let t = a.track("arm").unwrap();
        assert_eq!(t.translation.at(0), [1.0, 3.0, 5.0]);
        assert_eq!(t.translation.at(1), [2.0, 3.0, 4.0]);
        assert!((t.rotation.at(1)[2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        assert_eq!(t.visibility.as_deref(), Some(&[true, false][..]));
        assert!(t.scale == Track::Constant([1.0; 3]));
        assert!((a.duration() - 1.0 / 30.0).abs() < 1e-6);
    }

    #[test]
    fn pooled_rotation_and_translation() {
        let mut extra = mini(0x0e, &0u16.to_le_bytes());
        extra.extend(mini(0x0f, &0xffffu16.to_le_bytes()));
        extra.extend(mini(0x10, &0u16.to_le_bytes()));
        extra.extend(mini(0x11, &[0, 0, 0, 0, 0, 0, 0xff, 0x7f]));
        let bone = info("Pelvis", &extra);
        let mut root = header(2, 1, &[(0x0b, 4), (0x0c, 3), (0x0d, 0)]);
        root.extend(chunk(ids::BONE, true, &bone));
        let rot: Vec<u8> = [[0i16, 0, 0, 32767], [0, 0, 32767, 0]]
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        root.extend(chunk(ids::ROTATION_POOL, false, &rot));
        let tr: Vec<u8> = [0u16, 0, 0, 2, 4, 6]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        root.extend(chunk(ids::TRANSLATION_POOL, false, &tr));
        let a = Animation::parse(&chunk(ids::ANIMATION, true, &root)).unwrap();
        let t = &a.bones[0];
        assert_eq!(t.rotation.at(0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(t.rotation.at(1), [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(t.translation.at(1), [2.0, 4.0, 6.0]);
    }

    #[test]
    fn bad_sizes_are_errors() {
        let mut bone = info("Arm", &[]);
        bone.extend(chunk(ids::ROTATION_KEYS, false, &[0; 12]));
        let mut root = header(2, 1, &[]);
        root.extend(chunk(ids::BONE, true, &bone));
        let data = chunk(ids::ANIMATION, true, &root);
        assert!(Animation::parse(&data).is_err());
        assert!(Animation::parse(&data[..data.len() - 3]).is_err());
        assert!(Animation::parse(&[]).is_err());
    }
}
