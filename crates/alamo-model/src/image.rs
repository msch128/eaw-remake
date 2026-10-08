//! Minimal texture decoding (DDS DXT1/3/5 and uncompressed, TGA) and PNG encoding, enough to
//! export models for inspection. Only the top mip level is decoded.

use crate::error::{Error, Result};

/// 8-bit RGBA image, row-major from the top.
#[derive(Debug, Clone)]
pub struct Rgba8 {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// Decodes a DDS, PNG or TGA file, detected by content.
pub fn decode_texture(data: &[u8]) -> Result<Rgba8> {
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        decode_jpeg(data)
    } else if crate::png::is_png(data) {
        crate::png::decode_png(data)
    } else if data.starts_with(b"DDS ") {
        decode_dds(data)
    } else {
        decode_tga(data)
    }
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(Error::Truncated("texture header"))
}

fn expand565(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [
        ((r << 3) | (r >> 2)) as u8,
        ((g << 2) | (g >> 4)) as u8,
        ((b << 3) | (b >> 2)) as u8,
    ]
}

/// Decodes an 8-byte DXT color block into 16 RGBA pixels. `force4` selects the 4-color mode
/// (used by DXT3/5, which carry alpha separately).
fn dxt_color_block(b: &[u8], force4: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let (p0, p1) = (expand565(c0), expand565(c1));
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    let mix = |a: u8, b: u8, wa: u32, wb: u32, d: u32| ((a as u32 * wa + b as u32 * wb) / d) as u8;
    if c0 > c1 || force4 {
        for k in 0..3 {
            pal[2][k] = mix(p0[k], p1[k], 2, 1, 3);
            pal[3][k] = mix(p0[k], p1[k], 1, 2, 3);
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for k in 0..3 {
            pal[2][k] = mix(p0[k], p1[k], 1, 1, 2);
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let mut out = [[0u8; 4]; 16];
    for (i, px) in out.iter_mut().enumerate() {
        *px = pal[((bits >> (2 * i)) & 3) as usize];
    }
    out
}

fn decode_dds(d: &[u8]) -> Result<Rgba8> {
    let height = u32_at(d, 12)? as usize;
    let width = u32_at(d, 16)? as usize;
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(Error::Invalid(format!("bad DDS size {width}x{height}")));
    }
    let pf_flags = u32_at(d, 80)?;
    let fourcc = d.get(84..88).ok_or(Error::Truncated("dds"))?;
    let mut data_off = 128;
    if fourcc == b"DX10" {
        data_off += 20;
    }
    let body = d.get(data_off..).ok_or(Error::Truncated("dds data"))?;
    let mut img = vec![0u8; width * height * 4];
    if pf_flags & 4 != 0 {
        let block = match fourcc {
            b"DXT1" => 8,
            b"DXT3" | b"DXT5" => 16,
            other => {
                return Err(Error::Invalid(format!("unsupported DDS fourcc {other:?}")));
            }
        };
        let (bw, bh) = (width.div_ceil(4), height.div_ceil(4));
        if body.len() < bw * bh * block {
            return Err(Error::Truncated("dds blocks"));
        }
        for by in 0..bh {
            for bx in 0..bw {
                let b = &body[(by * bw + bx) * block..][..block];
                let mut px = dxt_color_block(&b[block - 8..], block == 16);
                match fourcc {
                    b"DXT3" => {
                        for (i, p) in px.iter_mut().enumerate() {
                            let n = (b[i / 2] >> (4 * (i % 2))) & 15;
                            p[3] = n * 17;
                        }
                    }
                    b"DXT5" => {
                        let (a0, a1) = (b[0] as u32, b[1] as u32);
                        let mut pal = [0u32; 8];
                        pal[0] = a0;
                        pal[1] = a1;
                        for k in 1..7u32 {
                            pal[k as usize + 1] = if a0 > a1 {
                                ((7 - k) * a0 + k * a1) / 7
                            } else if k < 5 {
                                ((5 - k) * a0 + k * a1) / 5
                            } else {
                                0
                            };
                        }
                        if a0 <= a1 {
                            pal[7] = 255;
                        }
                        let mut bits = 0u64;
                        for (i, &byte) in b[2..8].iter().enumerate() {
                            bits |= (byte as u64) << (8 * i);
                        }
                        for (i, p) in px.iter_mut().enumerate() {
                            p[3] = pal[((bits >> (3 * i)) & 7) as usize] as u8;
                        }
                    }
                    _ => {}
                }
                for (i, p) in px.iter().enumerate() {
                    let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                    if x < width && y < height {
                        img[(y * width + x) * 4..][..4].copy_from_slice(p);
                    }
                }
            }
        }
    } else {
        let bpp = u32_at(d, 88)? as usize;
        if bpp != 32 && bpp != 24 {
            return Err(Error::Invalid(format!("unsupported DDS bit count {bpp}")));
        }
        let bytes = bpp / 8;
        if body.len() < width * height * bytes {
            return Err(Error::Truncated("dds pixels"));
        }
        let (rm, gm, bm) = (u32_at(d, 92)?, u32_at(d, 96)?, u32_at(d, 100)?);
        let shift = |m: u32| if m == 0 { 0 } else { m.trailing_zeros() };
        let (rs, gs, bs) = (shift(rm), shift(gm), shift(bm));
        for i in 0..width * height {
            let mut v = 0u32;
            for k in 0..bytes {
                v |= (body[i * bytes + k] as u32) << (8 * k);
            }
            img[i * 4] = ((v & rm) >> rs) as u8;
            img[i * 4 + 1] = ((v & gm) >> gs) as u8;
            img[i * 4 + 2] = ((v & bm) >> bs) as u8;
            img[i * 4 + 3] = if bytes == 4 { (v >> 24) as u8 } else { 255 };
        }
    }
    Ok(Rgba8 {
        width,
        height,
        pixels: img,
    })
}

fn decode_tga(d: &[u8]) -> Result<Rgba8> {
    if d.len() < 18 {
        return Err(Error::Truncated("tga header"));
    }
    let id_len = d[0] as usize;
    let cmap_type = d[1];
    let img_type = d[2];
    let width = u16::from_le_bytes([d[12], d[13]]) as usize;
    let height = u16::from_le_bytes([d[14], d[15]]) as usize;
    let bpp = d[16] as usize;
    let desc = d[17];
    if cmap_type != 0 || !matches!(img_type, 2 | 3 | 10 | 11) || ![8, 24, 32].contains(&bpp) {
        return Err(Error::Invalid(format!(
            "unsupported TGA type {img_type} bpp {bpp}"
        )));
    }
    if width == 0 || height == 0 {
        return Err(Error::Invalid("empty TGA".into()));
    }
    let bytes = bpp / 8;
    let mut pos = 18 + id_len;
    let total = width * height * bytes;
    let mut raw = Vec::with_capacity(total);
    if img_type < 10 {
        raw.extend_from_slice(
            d.get(pos..pos + total)
                .ok_or(Error::Truncated("tga pixels"))?,
        );
    } else {
        while raw.len() < total {
            let h = *d.get(pos).ok_or(Error::Truncated("tga rle"))?;
            pos += 1;
            let n = (h & 127) as usize + 1;
            if h & 128 != 0 {
                let px = d.get(pos..pos + bytes).ok_or(Error::Truncated("tga rle"))?;
                pos += bytes;
                for _ in 0..n {
                    raw.extend_from_slice(px);
                }
            } else {
                raw.extend_from_slice(
                    d.get(pos..pos + n * bytes)
                        .ok_or(Error::Truncated("tga rle"))?,
                );
                pos += n * bytes;
            }
        }
        raw.truncate(total);
    }
    let mut img = vec![0u8; width * height * 4];
    for y in 0..height {
        let sy = if desc & 0x20 != 0 { y } else { height - 1 - y };
        for x in 0..width {
            let s = &raw[(sy * width + x) * bytes..][..bytes];
            let o = (y * width + x) * 4;
            let px = match bytes {
                1 => [s[0], s[0], s[0], 255],
                3 => [s[2], s[1], s[0], 255],
                _ => [s[2], s[1], s[0], s[3]],
            };
            img[o..o + 4].copy_from_slice(&px);
        }
    }
    Ok(Rgba8 {
        width,
        height,
        pixels: img,
    })
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Encodes an image as an uncompressed (stored deflate) PNG.
pub fn encode_png(img: &Rgba8) -> Vec<u8> {
    let mut raw = Vec::with_capacity((img.width * 4 + 1) * img.height);
    for row in img.pixels.chunks_exact(img.width * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        z.extend((c.len() as u16).to_le_bytes());
        z.extend((!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend(adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut put = |kind: &[u8; 4], body: &[u8]| {
        out.extend((body.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(body);
        out.extend(&c);
        out.extend(crc32(&c).to_be_bytes());
    };
    let mut ihdr = (img.width as u32).to_be_bytes().to_vec();
    ihdr.extend((img.height as u32).to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    put(b"IHDR", &ihdr);
    put(b"IDAT", &z);
    put(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_known_values() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn dxt1_solid_block() {
        let mut d = vec![0u8; 128];
        d[..4].copy_from_slice(b"DDS ");
        d[12..16].copy_from_slice(&4u32.to_le_bytes());
        d[16..20].copy_from_slice(&4u32.to_le_bytes());
        d[80..84].copy_from_slice(&4u32.to_le_bytes());
        d[84..88].copy_from_slice(b"DXT1");
        d.extend([0x00, 0xf8, 0x00, 0xf8, 0, 0, 0, 0]); // pure red, all indices 0
        let img = decode_texture(&d).unwrap();
        assert_eq!(&img.pixels[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn tga_uncompressed_bottom_up() {
        let mut d = vec![0u8; 18];
        d[2] = 2;
        d[12] = 1;
        d[14] = 2;
        d[16] = 24;
        d.extend([0, 0, 255, 0, 255, 0]); // bottom row red (BGR), then top row green
        let img = decode_texture(&d).unwrap();
        assert_eq!(&img.pixels[..4], &[0, 255, 0, 255]);
        assert_eq!(&img.pixels[4..8], &[255, 0, 0, 255]);
    }

    #[test]
    fn png_has_signature() {
        let img = Rgba8 {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 4],
        };
        let png = encode_png(&img);
        assert_eq!(&png[1..4], b"PNG");
        assert!(png.ends_with(&[0xae, 0x42, 0x60, 0x82]));
    }
}

/// Decodes a JPEG (the Remake ships a few skydome textures as `.jpg`).
fn decode_jpeg(data: &[u8]) -> Result<Rgba8> {
    let mut dec = jpeg_decoder::Decoder::new(data);
    let px = dec.decode().map_err(|e| Error::Invalid(format!("jpeg: {e}")))?;
    let info = dec.info().ok_or(Error::Truncated("jpeg header"))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let mut pixels = Vec::with_capacity(w * h * 4);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => px.chunks_exact(3).for_each(|c| pixels.extend([c[0], c[1], c[2], 255])),
        jpeg_decoder::PixelFormat::L8 => px.iter().for_each(|&l| pixels.extend([l, l, l, 255])),
        jpeg_decoder::PixelFormat::L16 => px.chunks_exact(2).for_each(|c| pixels.extend([c[0], c[0], c[0], 255])),
        jpeg_decoder::PixelFormat::CMYK32 => return Err(Error::Invalid("jpeg: CMYK not supported".into())),
    }
    Ok(Rgba8 { width: w, height: h, pixels })
}
