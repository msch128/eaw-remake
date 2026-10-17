//! Render states of the shipped Direct3D 9 effect files (`Art/Shaders/**/*.FXO`, effect binary `0xFEFF0901`, compiled, no HLSL source):
//! `fx_states <path in the game files>...`, for example `Art/Shaders/Engine/PrimAdditive.fxo Art/Shaders/MeshShield.fxo`.
//!
//! One line per technique pass with its blend, depth, cull and alpha-test states. Values are the D3D enums: blend factors
//! 1 ZERO, 2 ONE, 3 SRCCOLOR, 4 INVSRCCOLOR, 5 SRCALPHA, 6 INVSRCALPHA; `CULLMODE` 1 none; `ZFUNC` 4 less-equal.
//!
//! Layout (read from the files, cross-checked with Wine's d3dx9 effect parser): after the 8-byte header `magic, offset` the effect body
//! starts at byte `8 + offset` with `parameters, techniques, unused, objects` counts; every offset inside the file is relative to
//! byte 8. A parameter is `typedef, value, flags, annotations` followed by 8 bytes per annotation; a technique is `name,
//! annotations, passes`, a pass `name, annotations, states`, a state `operation, index, typedef, value` (the value is a `u32` at
//! `8 + value`). The operation numbers are the compiler's state table: 0 ZENABLE ... 13 ALPHABLENDENABLE (the render states
//! follow the D3DRS order), 146/147 vertex/pixel shader. Only the states below are named; others print as `op<N>`.
use alamo_fs::{FileSource, GameFs, GameInstall};

fn word(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn c_str(d: &[u8], o: usize) -> String {
    let end = d[o..].iter().position(|&b| b == 0).unwrap_or(0);
    String::from_utf8_lossy(&d[o..o + end]).into_owned()
}

fn state_name(op: u32) -> Option<&'static str> {
    Some(match op {
        0 => "ZENABLE",
        3 => "ZWRITEENABLE",
        4 => "ALPHATESTENABLE",
        6 => "SRCBLEND",
        7 => "DESTBLEND",
        8 => "CULLMODE",
        9 => "ZFUNC",
        10 => "ALPHAREF",
        11 => "ALPHAFUNC",
        13 => "ALPHABLENDENABLE",
        14 => "FOGENABLE",
        _ => return None,
    })
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: fx_states <path in the game files>...");
        std::process::exit(2);
    }
    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("file system");
    for path in &paths {
        let d = fs.read(path).expect("read");
        let base = 8;
        let mut p = base + word(&d, 4) as usize;
        let (params, techniques) = (word(&d, p) as usize, word(&d, p + 4) as usize);
        p += 16;
        for _ in 0..params {
            p += 16 + word(&d, p + 12) as usize * 8;
        }
        println!("== {path}: {params} parameters, {techniques} techniques");
        for t in 0..techniques {
            let name = c_str(&d, base + word(&d, p) as usize + 4);
            let (annotations, passes) = (word(&d, p + 4) as usize, word(&d, p + 8) as usize);
            p += 12 + annotations * 8;
            for pass in 0..passes {
                let (annotations, states) = (word(&d, p + 4) as usize, word(&d, p + 8) as usize);
                p += 12 + annotations * 8;
                let mut line = format!("  technique {t} '{name}' pass {pass}:");
                for _ in 0..states {
                    let (op, index, value) = (word(&d, p), word(&d, p + 4), base + word(&d, p + 12) as usize);
                    p += 16;
                    match state_name(op) {
                        Some(n) => line.push_str(&format!(" {n}={}", word(&d, value))),
                        // Shader objects and sampler states: not decoded here.
                        None if op == 146 || op == 147 => {}
                        None => line.push_str(&format!(" op{op}[{index}]={}", word(&d, value))),
                    }
                }
                println!("{line}");
            }
        }
    }
}
