//! Disassembles the Direct3D 9 shader bytecode (vs_1_1 .. ps_3_0) embedded in the shipped effect files (`Art/Shaders/**/*.FXO`):
//! `fx_disasm <path in the game files>...`, for example `Art/Shaders/Engine/PrimAdditive.fxo`.
//!
//! The blobs are found by scanning for shader version tokens (`0xFFFF0200` ps_2_0, `0xFFFE0101` vs_1_1, ...) and decoded until their end
//! token; `fx_states` lists the render states around them. What this showed: `PrimAdditive` is `tex t0; mul r0, t0, v0` blended
//! ONE/ONE (alpha unused), `MeshShield` technique `t1` a vs_1_1 that scales the object-space normal's z by `EdgeBrightness` and
//! scrolls three UV sets, and a ps_2_0 that multiplies base (offset by 0.25 times the distortion texture), wave and the vertex colour.
//! Register type 4 prints as `oPos` whatever its index (index 1 is `oFog`, 2 `oPts`).
use alamo_fs::{FileSource, GameFs, GameInstall};

fn word(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn opcode_name(op: u32) -> &'static str {
    match op {
        0 => "nop", 1 => "mov", 2 => "add", 3 => "sub", 4 => "mad", 5 => "mul", 6 => "rcp", 7 => "rsq", 8 => "dp3", 9 => "dp4",
        10 => "min", 11 => "max", 12 => "slt", 13 => "sge", 14 => "exp", 15 => "log", 16 => "lit", 17 => "dst", 18 => "lrp",
        19 => "frc", 20 => "m4x4", 21 => "m4x3", 22 => "m3x4", 23 => "m3x3", 24 => "m3x2", 25 => "call", 26 => "callnz",
        27 => "loop", 28 => "ret", 29 => "endloop", 30 => "label", 31 => "dcl", 32 => "pow", 33 => "crs", 34 => "sgn",
        35 => "abs", 36 => "nrm", 37 => "sincos", 38 => "rep", 39 => "endrep", 40 => "if", 41 => "ifc", 42 => "else",
        43 => "endif", 44 => "break", 45 => "breakc", 46 => "mova", 47 => "defb", 48 => "defi",
        64 => "texcoord", 65 => "texkill", 66 => "tex", 67 => "texbem", 68 => "texbeml", 69 => "texreg2ar", 70 => "texreg2gb",
        71 => "texm3x2pad", 72 => "texm3x2tex", 73 => "texm3x3pad", 74 => "texm3x3tex", 76 => "texm3x3spec", 77 => "texm3x3vspec",
        78 => "expp", 79 => "logp", 80 => "cnd", 81 => "def", 82 => "texreg2rgb", 83 => "texdp3tex", 84 => "texm3x2depth",
        85 => "texdp3", 86 => "texm3x3", 87 => "texdepth", 88 => "cmp", 89 => "bem", 90 => "dp2add", 91 => "dsx", 92 => "dsy",
        93 => "texldd", 94 => "setp", 95 => "texldl", 96 => "breakp",
        _ => "?",
    }
}

fn register(pixel: bool, token: u32) -> String {
    let kind = ((token >> 28) & 7) | (((token >> 11) & 3) << 3);
    let n = token & 0x7ff;
    match kind {
        0 => format!("r{n}"),
        1 => format!("v{n}"),
        2 => format!("c{n}"),
        3 if pixel => format!("t{n}"),
        3 => format!("a{n}"),
        4 => "oPos".into(),
        5 => format!("oD{n}"),
        6 => format!("oT{n}"),
        7 => format!("i{n}"),
        8 => format!("oC{n}"),
        9 => "oDepth".into(),
        10 => format!("s{n}"),
        _ => format!("reg{kind}_{n}"),
    }
}

fn swizzle(token: u32) -> String {
    let s = (token >> 16) & 0xff;
    if s == 0xe4 {
        return String::new();
    }
    let lane = |i: u32| ['x', 'y', 'z', 'w'][((s >> (2 * i)) & 3) as usize];
    if (1..4).all(|i| lane(i) == lane(0)) {
        format!(".{}", lane(0))
    } else {
        format!(".{}{}{}{}", lane(0), lane(1), lane(2), lane(3))
    }
}

fn source(pixel: bool, token: u32) -> String {
    let reg = format!("{}{}", register(pixel, token), swizzle(token));
    match (token >> 24) & 0xf {
        0 => reg,
        1 => format!("-{reg}"),
        2 => format!("{reg}_bias"),
        3 => format!("-{reg}_bias"),
        4 => format!("{reg}_bx2"),
        5 => format!("-{reg}_bx2"),
        6 => format!("(1-{reg})"),
        7 => format!("{reg}_x2"),
        8 => format!("-{reg}_x2"),
        9 => format!("{reg}_dz"),
        10 => format!("{reg}_dw"),
        11 => format!("abs({reg})"),
        12 => format!("-abs({reg})"),
        m => format!("{reg}_m{m}"),
    }
}

fn destination(pixel: bool, token: u32) -> String {
    let mask = (token >> 16) & 0xf;
    let mut text = register(pixel, token);
    if mask != 0xf {
        text.push('.');
        text.extend(['x', 'y', 'z', 'w'].iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, c)| *c));
    }
    if (token >> 20) & 1 != 0 {
        text.push_str("_sat");
    }
    let shift = (token >> 24) & 0xf;
    if shift != 0 {
        let exp = if shift >= 8 { shift as i32 - 16 } else { shift as i32 };
        text.push_str(&format!("_x{}", 2f32.powi(exp)));
    }
    text
}

/// Operand count of a 1.x instruction, which has no length field in its token.
fn operands_1x(op: u32) -> usize {
    match op {
        0 | 28 => 0,
        64..=66 | 87 => 1,
        1 | 6 | 7 | 14 | 15 | 16 | 19 | 31 | 35 | 36 | 46 | 69..=74 | 77..=79 | 82..=86 => 2,
        4 | 18 | 80 | 88 | 90 => 4,
        81 => 5,
        _ => 3,
    }
}

fn usage_name(usage: u32) -> &'static str {
    ["position", "blendweight", "blendindices", "normal", "psize", "texcoord", "tangent", "binormal", "tessfactor", "positiont", "color", "fog", "depth", "sample"]
        .get(usage as usize)
        .copied()
        .unwrap_or("usage")
}

/// Disassembles the blob at `start`; returns the text and the offset after its end token.
fn disassemble(d: &[u8], start: usize) -> Option<(String, usize)> {
    let version = word(d, start);
    let pixel = version >> 16 == 0xffff;
    let (major, minor) = ((version >> 8) & 0xff, version & 0xff);
    let mut out = format!("{}_{major}_{minor}\n", if pixel { "ps" } else { "vs" });
    let mut p = start + 4;
    for _ in 0..800 {
        if p + 4 > d.len() {
            return None;
        }
        let token = word(d, p);
        let op = token & 0xffff;
        match op {
            0xffff => return Some((out, p + 4)),
            0xfffe => {
                // Comment block (the constant table): skip.
                p += 4 + ((token >> 16) & 0x7fff) as usize * 4;
                continue;
            }
            97.. => return None,
            _ => {}
        }
        let len = if major >= 2 { ((token >> 24) & 0xf) as usize } else { operands_1x(op) };
        if p + 4 + len * 4 > d.len() {
            return None;
        }
        let ops: Vec<u32> = (0..len).map(|i| word(d, p + 4 + i * 4)).collect();
        let text = match (op, len) {
            (81, 5) => {
                let f = |i: usize| f32::from_bits(ops[i + 1]);
                format!("def {}, {:.4}, {:.4}, {:.4}, {:.4}", register(pixel, ops[0]), f(0), f(1), f(2), f(3))
            }
            (31, 2) => {
                let dst = ops[1];
                let kind = ((dst >> 28) & 7) | (((dst >> 11) & 3) << 3);
                if kind == 10 {
                    let texture = match (ops[0] >> 27) & 0xf {
                        2 => "2d",
                        3 => "cube",
                        4 => "volume",
                        _ => "?",
                    };
                    format!("dcl_{texture} {}", register(pixel, dst))
                } else if pixel {
                    format!("dcl {}", destination(pixel, dst))
                } else {
                    format!("dcl_{}{} {}", usage_name(ops[0] & 0x1f), (ops[0] >> 16) & 0xf, destination(pixel, dst))
                }
            }
            (66, 3) if major >= 2 => format!("texld {}, {}, {}", destination(pixel, ops[0]), source(pixel, ops[1]), source(pixel, ops[2])),
            (_, 0) => opcode_name(op).to_string(),
            _ => {
                let mut parts = vec![destination(pixel, ops[0])];
                parts.extend(ops[1..].iter().map(|&o| source(pixel, o)));
                format!("{} {}", opcode_name(op), parts.join(", "))
            }
        };
        let co_issue = if token & 0x4000_0000 != 0 { "+" } else { " " };
        out.push_str(&format!(" {co_issue} {text}\n"));
        p += 4 + len * 4;
    }
    None
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: fx_disasm <path in the game files>...");
        std::process::exit(2);
    }
    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("file system");
    for path in &paths {
        let d = fs.read(path).expect("read");
        println!("===== {path}");
        let mut p = 8;
        while p + 8 < d.len() {
            let version = word(&d, p);
            let (major, minor) = ((version >> 8) & 0xff, version & 0xff);
            let known = matches!(version >> 16, 0xfffe | 0xffff) && ((major == 1 && minor <= 4) || ((2..=3).contains(&major) && minor == 0));
            if known && let Some((text, end)) = disassemble(&d, p) {
                println!("--- shader at {p:#x}, {} bytes\n{text}", end - p);
                p = end;
            } else {
                p += 4;
            }
        }
    }
}
