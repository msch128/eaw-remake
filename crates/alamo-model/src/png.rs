//! Minimal PNG decoder (8/16-bit gray, RGB, palette, gray+alpha and RGBA; not interlaced) with a
//! small inflate implementation, so textures stored as PNG can be read without extra crates.

use crate::error::{Error, Result};
use crate::image::Rgba8;

/// True if `d` starts with the PNG signature.
pub fn is_png(d: &[u8]) -> bool {
    d.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    bit: u32,
    nbits: u32,
}

impl<'a> Bits<'a> {
    fn need(&mut self, n: u32) -> Result<()> {
        while self.nbits < n {
            let b = *self.d.get(self.pos).ok_or(Error::Truncated("deflate stream"))?;
            self.pos += 1;
            self.bit |= (b as u32) << self.nbits;
            self.nbits += 8;
        }
        Ok(())
    }

    fn get(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = self.bit & ((1u32 << n) - 1);
        self.bit >>= n;
        self.nbits -= n;
        Ok(v)
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Huffman { counts, symbols }
    }

    fn decode(&self, b: &mut Bits) -> Result<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.get(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Error::Invalid("bad huffman code".into()))
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Inflates a raw deflate stream; `limit` bounds the output size.
fn inflate(d: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut b = Bits {
        d,
        pos: 0,
        bit: 0,
        nbits: 0,
    };
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = b.get(1)?;
        match b.get(2)? {
            0 => {
                b.bit = 0;
                b.nbits = 0;
                let h = d.get(b.pos..b.pos + 4).ok_or(Error::Truncated("stored block"))?;
                let len = u16::from_le_bytes([h[0], h[1]]) as usize;
                b.pos += 4;
                out.extend_from_slice(
                    d.get(b.pos..b.pos + len).ok_or(Error::Truncated("stored block"))?,
                );
                b.pos += len;
            }
            t @ (1 | 2) => {
                let (lit, dist) = if t == 1 {
                    let mut l = [0u8; 288];
                    for (i, v) in l.iter_mut().enumerate() {
                        *v = match i {
                            0..=143 => 8,
                            144..=255 => 9,
                            256..=279 => 7,
                            _ => 8,
                        };
                    }
                    (Huffman::new(&l), Huffman::new(&[5u8; 30]))
                } else {
                    let hlit = b.get(5)? as usize + 257;
                    let hdist = b.get(5)? as usize + 1;
                    let hclen = b.get(4)? as usize + 4;
                    const ORDER: [usize; 19] =
                        [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                    let mut cl = [0u8; 19];
                    for &o in ORDER.iter().take(hclen) {
                        cl[o] = b.get(3)? as u8;
                    }
                    let clh = Huffman::new(&cl);
                    let mut lengths = vec![0u8; hlit + hdist];
                    let mut i = 0;
                    while i < hlit + hdist {
                        let sym = clh.decode(&mut b)?;
                        match sym {
                            0..=15 => {
                                lengths[i] = sym as u8;
                                i += 1;
                            }
                            16 => {
                                let prev = *lengths
                                    .get(i.wrapping_sub(1))
                                    .ok_or_else(|| Error::Invalid("repeat without previous".into()))?;
                                for _ in 0..3 + b.get(2)? {
                                    *lengths.get_mut(i).ok_or_else(|| Error::Invalid("length overflow".into()))? = prev;
                                    i += 1;
                                }
                            }
                            17 | 18 => {
                                let n = if sym == 17 { 3 + b.get(3)? } else { 11 + b.get(7)? };
                                for _ in 0..n {
                                    *lengths.get_mut(i).ok_or_else(|| Error::Invalid("length overflow".into()))? = 0;
                                    i += 1;
                                }
                            }
                            _ => return Err(Error::Invalid("bad code length symbol".into())),
                        }
                    }
                    (Huffman::new(&lengths[..hlit]), Huffman::new(&lengths[hlit..]))
                };
                loop {
                    let sym = lit.decode(&mut b)? as usize;
                    match sym {
                        0..=255 => out.push(sym as u8),
                        256 => break,
                        257..=285 => {
                            let s = sym - 257;
                            let len = LEN_BASE[s] as usize + b.get(LEN_EXTRA[s] as u32)? as usize;
                            let ds = dist.decode(&mut b)? as usize;
                            if ds >= 30 {
                                return Err(Error::Invalid("bad distance symbol".into()));
                            }
                            let dd = DIST_BASE[ds] as usize + b.get(DIST_EXTRA[ds] as u32)? as usize;
                            if dd > out.len() {
                                return Err(Error::Invalid("distance too far back".into()));
                            }
                            for _ in 0..len {
                                out.push(out[out.len() - dd]);
                            }
                        }
                        _ => return Err(Error::Invalid("bad literal symbol".into())),
                    }
                    if out.len() > limit {
                        return Err(Error::Invalid("inflated data too large".into()));
                    }
                }
            }
            _ => return Err(Error::Invalid("bad deflate block type".into())),
        }
        if out.len() > limit {
            return Err(Error::Invalid("inflated data too large".into()));
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Decodes a PNG file to RGBA8.
pub fn decode_png(d: &[u8]) -> Result<Rgba8> {
    if !is_png(d) {
        return Err(Error::Invalid("not a PNG".into()));
    }
    let mut pos = 8;
    let (mut w, mut h, mut depth, mut ctype, mut interlace) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut idat = Vec::new();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    while pos + 8 <= d.len() {
        let len = u32::from_be_bytes([d[pos], d[pos + 1], d[pos + 2], d[pos + 3]]) as usize;
        let kind = &d[pos + 4..pos + 8];
        let body = d
            .get(pos + 8..pos + 8 + len)
            .ok_or(Error::Truncated("png chunk"))?;
        match kind {
            b"IHDR" if body.len() >= 13 => {
                w = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
                h = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
                depth = body[8];
                ctype = body[9];
                interlace = body[12];
            }
            b"PLTE" => palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err(Error::Invalid(format!("bad PNG size {w}x{h}")));
    }
    if interlace != 0 {
        return Err(Error::Invalid("interlaced PNG not supported".into()));
    }
    let channels = match ctype {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(Error::Invalid(format!("bad PNG color type {ctype}"))),
    };
    if !matches!(depth, 1 | 2 | 4 | 8 | 16) || (depth < 8 && ctype != 0 && ctype != 3) {
        return Err(Error::Invalid(format!("unsupported PNG depth {depth}")));
    }
    let bits_pp = channels * depth as usize;
    let stride = (w * bits_pp).div_ceil(8);
    let bpp = bits_pp.div_ceil(8);
    let raw = inflate(
        idat.get(2..).ok_or(Error::Truncated("zlib header"))?,
        (stride + 1) * h + 16,
    )?;
    if raw.len() < (stride + 1) * h {
        return Err(Error::Truncated("png pixel data"));
    }
    let mut img = vec![0u8; stride * h];
    for y in 0..h {
        let filter = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= bpp { img[y * stride + x - bpp] as i32 } else { 0 };
            let b = if y > 0 { img[(y - 1) * stride + x] as i32 } else { 0 };
            let c = if x >= bpp && y > 0 { img[(y - 1) * stride + x - bpp] as i32 } else { 0 };
            let v = src[x] as i32;
            let r = match filter {
                0 => v,
                1 => v + a,
                2 => v + b,
                3 => v + (a + b) / 2,
                4 => v + paeth(a, b, c),
                _ => return Err(Error::Invalid("bad PNG filter".into())),
            };
            img[y * stride + x] = r as u8;
        }
    }
    let sample = |row: &[u8], i: usize| -> u32 {
        // i-th sample in the row, scaled to 8 bits for gray; raw index for palette.
        match depth {
            8 => row[i] as u32,
            16 => row[i * 2] as u32,
            _ => {
                let per = 8 / depth as usize;
                let byte = row[i / per];
                let shift = 8 - depth as usize * (i % per + 1);
                ((byte >> shift) & ((1 << depth) - 1)) as u32
            }
        }
    };
    let mut out = vec![255u8; w * h * 4];
    for y in 0..h {
        let row = &img[y * stride..(y + 1) * stride];
        for x in 0..w {
            let o = (y * w + x) * 4;
            match ctype {
                0 => {
                    let mut g = sample(row, x);
                    if depth < 8 {
                        g = g * 255 / ((1 << depth) - 1);
                    }
                    out[o..o + 3].copy_from_slice(&[g as u8; 3]);
                }
                2 => out[o..o + 3].copy_from_slice(&[
                    sample(row, x * 3) as u8,
                    sample(row, x * 3 + 1) as u8,
                    sample(row, x * 3 + 2) as u8,
                ]),
                3 => {
                    let i = sample(row, x) as usize;
                    let p = palette.get(i).copied().unwrap_or([0, 0, 0]);
                    out[o..o + 3].copy_from_slice(&p);
                    out[o + 3] = trns.get(i).copied().unwrap_or(255);
                }
                4 => {
                    let g = sample(row, x * 2) as u8;
                    out[o..o + 3].copy_from_slice(&[g; 3]);
                    out[o + 3] = sample(row, x * 2 + 1) as u8;
                }
                _ => out[o..o + 4].copy_from_slice(&[
                    sample(row, x * 4) as u8,
                    sample(row, x * 4 + 1) as u8,
                    sample(row, x * 4 + 2) as u8,
                    sample(row, x * 4 + 3) as u8,
                ]),
            }
        }
    }
    Ok(Rgba8 {
        width: w,
        height: h,
        pixels: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{Rgba8, encode_png};

    #[test]
    fn roundtrips_our_own_png_encoder() {
        let img = Rgba8 {
            width: 3,
            height: 2,
            pixels: (0..24).map(|i| (i * 10) as u8).collect(),
        };
        let back = decode_png(&encode_png(&img)).unwrap();
        assert_eq!((back.width, back.height), (3, 2));
        assert_eq!(back.pixels, img.pixels);
    }

    #[test]
    fn inflates_fixed_huffman_block() {
        // zlib stream of "aaaaaaaaaa" produced with fixed Huffman codes.
        let z = [0x4b, 0x4c, 0x84, 0x01, 0x00];
        assert_eq!(inflate(&z, 100).unwrap(), b"aaaaaaaaaa");
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode_png(&[1, 2, 3]).is_err());
        let mut p = crate::image::encode_png(&Rgba8 {
            width: 1,
            height: 1,
            pixels: vec![0; 4],
        });
        p.truncate(p.len() - 20);
        assert!(decode_png(&p).is_err());
    }
}
