//! RIFF/WAVE parsing and decoding to 16-bit PCM, including IMA and Microsoft ADPCM which neither
//! `hound` nor rodio's WAV decoder handle.

use std::fmt;

/// Error while parsing or decoding audio data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioError(pub String);

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AudioError {}

fn err<T>(msg: impl Into<String>) -> Result<T, AudioError> {
    Err(AudioError(msg.into()))
}

/// The `fmt ` chunk of a WAV file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavFormat {
    /// Format tag: 1 = PCM, 2 = MS ADPCM, 3 = float, 0x11 = IMA ADPCM, 0x55 = MP3, 0xFFFE = extensible.
    pub tag: u16,
    /// Channel count.
    pub channels: u16,
    /// Samples per second.
    pub sample_rate: u32,
    /// Bytes per block (ADPCM block size).
    pub block_align: u16,
    /// Bits per sample.
    pub bits: u16,
}

/// Decoded 16-bit PCM audio, channels interleaved.
#[derive(Debug, Clone, Default)]
pub struct Pcm {
    /// Channel count.
    pub channels: u16,
    /// Samples per second.
    pub sample_rate: u32,
    /// Interleaved samples.
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        if self.channels == 0 || self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / f32::from(self.channels) / self.sample_rate as f32
    }

    /// Serializes as a canonical PCM16 WAV file (what Bevy's `wav` decoder accepts).
    pub fn to_wav_bytes(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend(b"RIFF");
        out.extend((36 + data_len).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(self.channels.to_le_bytes());
        out.extend(self.sample_rate.to_le_bytes());
        out.extend((self.sample_rate * u32::from(self.channels) * 2).to_le_bytes());
        out.extend((self.channels * 2).to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(data_len.to_le_bytes());
        for s in &self.samples {
            out.extend(s.to_le_bytes());
        }
        out
    }
}

/// A parsed WAV container: format plus the sample data slice.
#[derive(Debug, Clone, Copy)]
pub struct Wav<'a> {
    /// Format description.
    pub format: WavFormat,
    /// Raw `data` chunk.
    pub data: &'a [u8],
}

fn u16_at(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}

fn u32_at(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

impl<'a> Wav<'a> {
    /// Parses the RIFF structure (tolerates odd chunk padding and truncated `data`).
    pub fn parse(bytes: &'a [u8]) -> Result<Self, AudioError> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return err("not a RIFF/WAVE file");
        }
        let mut pos = 12;
        let mut format = None;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32_at(bytes, pos + 4) as usize;
            let body = pos + 8;
            let end = (body + size).min(bytes.len());
            match id {
                b"fmt " if end - body >= 16 => {
                    let mut tag = u16_at(bytes, body);
                    // WAVE_FORMAT_EXTENSIBLE: the real tag is the first two bytes of the sub-format GUID.
                    if tag == 0xFFFE && end - body >= 26 {
                        tag = u16_at(bytes, body + 24);
                    }
                    format = Some(WavFormat {
                        tag,
                        channels: u16_at(bytes, body + 2),
                        sample_rate: u32_at(bytes, body + 4),
                        block_align: u16_at(bytes, body + 12),
                        bits: u16_at(bytes, body + 14),
                    });
                }
                b"data" => {
                    let format =
                        format.ok_or_else(|| AudioError("data chunk before fmt chunk".into()))?;
                    return Ok(Wav {
                        format,
                        data: &bytes[body..end],
                    });
                }
                _ => {}
            }
            pos = body + size + (size & 1);
        }
        err("no data chunk")
    }

    /// Decodes to 16-bit PCM. Supports PCM 8/16/24/32 bit, float, IMA ADPCM and MS ADPCM.
    pub fn decode(&self) -> Result<Pcm, AudioError> {
        let f = self.format;
        if f.channels == 0 || f.channels > 8 {
            return err(format!("unsupported channel count {}", f.channels));
        }
        let samples = match (f.tag, f.bits) {
            (1, 8) => self
                .data
                .iter()
                .map(|&b| (i16::from(b) - 128) << 8)
                .collect(),
            (1, 16) => self
                .data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect(),
            (1, 24) => self
                .data
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes([c[1], c[2]]))
                .collect(),
            (1, 32) => self
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes([c[2], c[3]]))
                .collect(),
            (3, 32) => self
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| {
                    (f32::from_le_bytes([c[0], c[1], c[2], c[3]]).clamp(-1.0, 1.0) * 32767.0) as i16
                })
                .collect(),
            (0x11, 4) => decode_ima(self.data, f)?,
            (2, 4) => decode_ms(self.data, f)?,
            (tag, bits) => return err(format!("unsupported WAV format tag {tag:#x} / {bits} bit")),
        };
        Ok(Pcm {
            channels: f.channels,
            sample_rate: f.sample_rate,
            samples,
        })
    }
}

const IMA_STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const IMA_INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

#[derive(Clone, Copy, Default)]
struct ImaState {
    pred: i32,
    index: i32,
}

impl ImaState {
    fn step(&mut self, nibble: u8) -> i16 {
        let step = IMA_STEPS[self.index as usize];
        let mut diff = step >> 3;
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 4 != 0 {
            diff += step;
        }
        self.pred = if nibble & 8 != 0 {
            self.pred - diff
        } else {
            self.pred + diff
        }
        .clamp(-32768, 32767);
        self.index = (self.index + IMA_INDEX[nibble as usize]).clamp(0, 88);
        self.pred as i16
    }
}

/// IMA/DVI ADPCM as used in WAV (format 0x11): every block starts with a 4-byte header per channel
/// (predictor, step index, reserved) followed by groups of 4 bytes (8 nibbles) per channel.
fn decode_ima(data: &[u8], f: WavFormat) -> Result<Vec<i16>, AudioError> {
    let ch = usize::from(f.channels);
    let block = usize::from(f.block_align);
    if block < 4 * ch {
        return err("IMA ADPCM block too small");
    }
    let mut out = Vec::with_capacity(data.len() * 2);
    for blk in data.chunks(block) {
        if blk.len() < 4 * ch {
            break;
        }
        let mut states = Vec::with_capacity(ch);
        for c in 0..ch {
            let h = &blk[c * 4..c * 4 + 4];
            let st = ImaState {
                pred: i32::from(i16::from_le_bytes([h[0], h[1]])),
                index: i32::from(h[2]).min(88),
            };
            states.push(st);
        }
        let groups = (blk.len() - 4 * ch) / (4 * ch);
        let frames = 1 + groups * 8;
        let base = out.len();
        out.resize(base + frames * ch, 0);
        for (c, st) in states.iter().enumerate() {
            out[base + c] = st.pred as i16;
        }
        for g in 0..groups {
            for (c, st) in states.iter_mut().enumerate() {
                let src = &blk[4 * ch + (g * ch + c) * 4..][..4];
                for (i, byte) in src.iter().enumerate() {
                    let f0 = 1 + g * 8 + i * 2;
                    out[base + f0 * ch + c] = st.step(byte & 0xF);
                    out[base + (f0 + 1) * ch + c] = st.step(byte >> 4);
                }
            }
        }
    }
    Ok(out)
}

const MS_ADAPT: [i32; 16] = [
    230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
];
const MS_COEF: [(i32, i32); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];

#[derive(Clone, Copy, Default)]
struct MsChannel {
    coef: (i32, i32),
    delta: i32,
    s1: i32,
    s2: i32,
}

impl MsChannel {
    fn step(&mut self, nibble: u8) -> i16 {
        let pred = (self.s1 * self.coef.0 + self.s2 * self.coef.1) >> 8;
        let signed = if nibble & 8 != 0 {
            i32::from(nibble) - 16
        } else {
            i32::from(nibble)
        };
        let s = (pred + signed * self.delta).clamp(-32768, 32767);
        self.s2 = self.s1;
        self.s1 = s;
        self.delta = ((MS_ADAPT[nibble as usize] * self.delta) >> 8).max(16);
        s as i16
    }
}

/// Microsoft ADPCM (format 2) with the standard 7 predictor coefficient pairs.
fn decode_ms(data: &[u8], f: WavFormat) -> Result<Vec<i16>, AudioError> {
    let ch = usize::from(f.channels);
    let block = usize::from(f.block_align);
    if ch > 2 || block < 7 * ch {
        return err("unsupported MS ADPCM layout");
    }
    let mut out = Vec::with_capacity(data.len() * 4);
    for blk in data.chunks(block) {
        if blk.len() < 7 * ch {
            break;
        }
        let mut st = [MsChannel::default(); 2];
        let mut p = 0;
        for s in st.iter_mut().take(ch) {
            let idx = usize::from(blk[p]);
            s.coef = *MS_COEF
                .get(idx)
                .ok_or_else(|| AudioError("bad MS ADPCM predictor".into()))?;
            p += 1;
        }
        for s in st.iter_mut().take(ch) {
            s.delta = i32::from(i16::from_le_bytes([blk[p], blk[p + 1]]));
            p += 2;
        }
        for s in st.iter_mut().take(ch) {
            s.s1 = i32::from(i16::from_le_bytes([blk[p], blk[p + 1]]));
            p += 2;
        }
        for s in st.iter_mut().take(ch) {
            s.s2 = i32::from(i16::from_le_bytes([blk[p], blk[p + 1]]));
            p += 2;
        }
        // Oldest sample first: s2 then s1 for every channel.
        for s in st.iter().take(ch) {
            out.push(s.s2 as i16);
        }
        for s in st.iter().take(ch) {
            out.push(s.s1 as i16);
        }
        let mut c = 0;
        for byte in &blk[p..] {
            for nib in [byte >> 4, byte & 0xF] {
                out.push(st[c].step(nib));
                c = (c + 1) % ch;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(tag: u16, ch: u16, rate: u32, align: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(b"RIFF");
        v.extend((36 + data.len() as u32).to_le_bytes());
        v.extend(b"WAVEfmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(tag.to_le_bytes());
        v.extend(ch.to_le_bytes());
        v.extend(rate.to_le_bytes());
        v.extend((rate * 2).to_le_bytes());
        v.extend(align.to_le_bytes());
        v.extend(bits.to_le_bytes());
        v.extend(b"data");
        v.extend((data.len() as u32).to_le_bytes());
        v.extend(data);
        v
    }

    #[test]
    fn pcm_roundtrip() {
        let pcm = Pcm {
            channels: 1,
            sample_rate: 22050,
            samples: vec![0, 1000, -1000, 32767],
        };
        let bytes = pcm.to_wav_bytes();
        let back = Wav::parse(&bytes).unwrap().decode().unwrap();
        assert_eq!(back.samples, pcm.samples);
        assert_eq!(back.sample_rate, 22050);
    }

    #[test]
    fn ima_block_header_and_steps() {
        // predictor 100, index 0, then one group of 8 zero nibbles: each step adds step>>3 (=0 for 7) ...
        let mut blk = vec![100, 0, 0, 0];
        blk.extend([0x00, 0x00, 0x00, 0x00]);
        let w = wav(0x11, 1, 8000, 8, 4, &blk);
        let pcm = Wav::parse(&w).unwrap().decode().unwrap();
        assert_eq!(pcm.samples.len(), 9);
        assert_eq!(pcm.samples[0], 100);
        // nibble 0 -> diff = 7>>3 = 0, index decreases (clamped at 0)
        assert!(pcm.samples.iter().all(|&s| s == 100));
        // nibble 7 -> diff = 0 + 1 + 3 + 7 = 11
        let mut blk2 = vec![0, 0, 0, 0, 0x07, 0, 0, 0];
        let w2 = wav(0x11, 1, 8000, 8, 4, &blk2);
        let p2 = Wav::parse(&w2).unwrap().decode().unwrap();
        assert_eq!(p2.samples[1], 11);
        blk2.clear();
    }

    #[test]
    fn ima_roundtrip_with_reference_encoder() {
        // Reference IMA encoder using the same tables: 1 channel, header sample + 64 coded samples.
        let src: Vec<i16> = (0..65)
            .map(|i| ((i as f32 * 0.3).sin() * 8000.0) as i16)
            .collect();
        let mut st = ImaState {
            pred: i32::from(src[0]),
            index: 0,
        };
        let mut blk = vec![src[0] as u8, (src[0] >> 8) as u8, 0, 0];
        let mut nibbles = Vec::new();
        let mut recon = vec![src[0]];
        for &s in &src[1..] {
            let step = IMA_STEPS[st.index as usize];
            let mut diff = i32::from(s) - st.pred;
            let mut code = 0u8;
            if diff < 0 {
                code = 8;
                diff = -diff;
            }
            if diff >= step {
                code |= 4;
                diff -= step;
            }
            if diff >= step >> 1 {
                code |= 2;
                diff -= step >> 1;
            }
            if diff >= step >> 2 {
                code |= 1;
            }
            recon.push(st.step(code));
            nibbles.push(code);
        }
        for pair in nibbles.chunks(2) {
            blk.push(pair[0] | (pair[1] << 4));
        }
        let w = wav(0x11, 1, 8000, blk.len() as u16, 4, &blk);
        let pcm = Wav::parse(&w).unwrap().decode().unwrap();
        assert_eq!(pcm.samples.len(), 65);
        // The decoder must reproduce exactly what the encoder's own reconstruction tracked.
        assert_eq!(pcm.samples, recon);
    }

    #[test]
    fn rejects_unknown_format() {
        let w = wav(0x55, 1, 8000, 1, 0, &[0; 4]);
        assert!(Wav::parse(&w).unwrap().decode().is_err());
    }
}
