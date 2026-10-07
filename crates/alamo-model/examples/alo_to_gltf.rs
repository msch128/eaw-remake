//! Exports a model to a binary glTF for visual inspection (static, no skinning).
//!
//! `alo_to_gltf <path-in-gamefs> <out.glb> [--workshop <id>|-] [--all]`
//!
//! Bones become nodes (the Alamo Z-up to glTF Y-up conversion is a single rotation on a root
//! node, everything below stays in Alamo space), rigid meshes hang under their bone, proxies and
//! lights become empty nodes. Textures are decoded from DDS/TGA and embedded as PNG. Without
//! `--all`, shadow volumes, collision meshes, hidden meshes and lower detail levels are skipped.

use std::collections::HashMap;
use std::fmt::Write as _;

use alamo_fs::{FileSource, GameFs, GameInstall};
use alamo_model::Model;
use alamo_model::alo::{ParamValue, SubMesh};
use alamo_model::image::{decode_texture, encode_png};

fn esc(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 32 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn num(v: f32) -> String {
    if v.is_finite() {
        format!("{v}")
    } else {
        "0".into()
    }
}

fn nums(v: &[f32]) -> String {
    v.iter().map(|&x| num(x)).collect::<Vec<_>>().join(",")
}

#[derive(Default)]
struct Bin {
    data: Vec<u8>,
    views: Vec<String>,
}

impl Bin {
    /// Appends a buffer view, returns its index.
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let off = self.data.len();
        self.data.extend_from_slice(bytes);
        let mut s = format!(
            "{{\"buffer\":0,\"byteOffset\":{off},\"byteLength\":{}",
            bytes.len()
        );
        if let Some(t) = target {
            let _ = write!(s, ",\"target\":{t}");
        }
        s.push('}');
        self.views.push(s);
        self.views.len() - 1
    }
}

fn f32_bytes(v: impl Iterator<Item = f32>) -> Vec<u8> {
    v.flat_map(|f| f.to_le_bytes()).collect()
}

/// Y-up conversion of a direction/position is done by the root node, so data is written as is.
fn primitive(bin: &mut Bin, accessors: &mut Vec<String>, sub: &SubMesh, material: usize) -> String {
    let n = sub.vertices.len();
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &sub.vertices {
        for k in 0..3 {
            mn[k] = mn[k].min(v.position[k]);
            mx[k] = mx[k].max(v.position[k]);
        }
    }
    let mut add = |bin: &mut Bin,
                   bytes: Vec<u8>,
                   ty: &str,
                   comp: u32,
                   count: usize,
                   target: u32,
                   extra: &str| {
        let view = bin.view(&bytes, Some(target));
        accessors.push(format!(
            "{{\"bufferView\":{view},\"componentType\":{comp},\"count\":{count},\"type\":\"{ty}\"{extra}}}"
        ));
        accessors.len() - 1
    };
    let pos = add(
        bin,
        f32_bytes(sub.vertices.iter().flat_map(|v| v.position)),
        "VEC3",
        5126,
        n,
        34962,
        &format!(",\"min\":[{}],\"max\":[{}]", nums(&mn), nums(&mx)),
    );
    let nor = add(
        bin,
        f32_bytes(sub.vertices.iter().flat_map(|v| v.normal)),
        "VEC3",
        5126,
        n,
        34962,
        "",
    );
    let uv = add(
        bin,
        f32_bytes(sub.vertices.iter().flat_map(|v| v.uv[0])),
        "VEC2",
        5126,
        n,
        34962,
        "",
    );
    let idx = add(
        bin,
        sub.indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
        "SCALAR",
        5123,
        sub.indices.len(),
        34963,
        "",
    );
    format!(
        "{{\"attributes\":{{\"POSITION\":{pos},\"NORMAL\":{nor},\"TEXCOORD_0\":{uv}}},\"indices\":{idx},\"material\":{material}}}"
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut pos_args = Vec::new();
    let (mut workshop, mut all) = ("1770851727".to_string(), false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--all" => all = true,
            "--workshop" => workshop = it.next().cloned().unwrap_or_default(),
            _ => pos_args.push(a.clone()),
        }
    }
    let [path, out] = pos_args.as_slice() else {
        eprintln!("usage: alo_to_gltf <path-in-gamefs> <out.glb> [--workshop <id>|-] [--all]");
        std::process::exit(2);
    };
    let install = GameInstall::detect().expect("Empire at War install not found");
    let fs = GameFs::for_mod(&install, (workshop != "-").then_some(workshop.as_str())).expect("fs");
    let model = Model::parse(&fs.read(path).expect("read")).expect("parse");

    let mut bin = Bin::default();
    let mut accessors = Vec::new();
    let mut materials: Vec<String> = Vec::new();
    let mut textures: Vec<String> = Vec::new();
    let mut images: Vec<String> = Vec::new();
    let mut tex_cache: HashMap<String, Option<usize>> = HashMap::new();
    let mut meshes: Vec<String> = Vec::new();
    // nodes: 0 = axis conversion root, 1.. = bones, then objects
    let mut nodes: Vec<(String, Vec<usize>, String)> =
        vec![("AlamoToGltfYUp".into(), vec![], String::new())];
    nodes[0].2 = "\"rotation\":[-0.70710678,0,0,0.70710678]".into();
    let bone_node = |i: usize| i + 1;
    for (i, b) in model.bones.iter().enumerate() {
        let t = &b.transform;
        let m: Vec<f32> = (0..4)
            .flat_map(|c| (0..4).map(move |r| (r, c)))
            .map(|(r, c)| if r == 3 { f32::from(c == 3) } else { t[r][c] })
            .collect();
        nodes.push((
            b.name.clone(),
            vec![],
            format!(
                "\"matrix\":[{}],\"extras\":{{\"bone\":{i},\"visible\":{}}}",
                nums(&m),
                b.visible
            ),
        ));
    }
    for (i, b) in model.bones.iter().enumerate() {
        let parent = b.parent.map_or(0, bone_node);
        nodes[parent].1.push(bone_node(i));
    }

    let mut load_texture = |name: &str, bin: &mut Bin| -> Option<usize> {
        if let Some(c) = tex_cache.get(&name.to_ascii_lowercase()) {
            return *c;
        }
        let stem = name.rsplit_once('.').map_or(name, |s| s.0);
        let found = [
            name.to_string(),
            format!("{stem}.dds"),
            format!("{stem}.tga"),
        ]
        .iter()
        .find_map(|n| fs.read(&format!("Art/Textures/{n}")).ok());
        let res = found.and_then(|d| match decode_texture(&d) {
            Ok(img) => {
                let view = bin.view(&encode_png(&img), None);
                images.push(format!(
                    "{{\"bufferView\":{view},\"mimeType\":\"image/png\",\"name\":{}}}",
                    esc(name)
                ));
                textures.push(format!("{{\"source\":{}}}", images.len() - 1));
                Some(textures.len() - 1)
            }
            Err(e) => {
                eprintln!("texture {name}: {e}");
                None
            }
        });
        tex_cache.insert(name.to_ascii_lowercase(), res);
        res
    };

    for (oi, mesh) in model.meshes.iter().enumerate() {
        let skip = mesh.is_shadow_volume()
            || mesh.collision
                && mesh
                    .sub_meshes
                    .iter()
                    .all(|s| s.material.shader.to_ascii_lowercase().contains("collision"))
            || mesh.hidden
            || mesh.lod > 0;
        if skip && !all {
            continue;
        }
        let mut prims = Vec::new();
        for sub in &mesh.sub_meshes {
            let m = &sub.material;
            let mut mat = format!("{{\"name\":{}", esc(&m.shader));
            let mut pbr = String::from("\"metallicFactor\":0,\"roughnessFactor\":0.6");
            if let Some(t) = m
                .texture("BaseTexture")
                .and_then(|t| load_texture(t, &mut bin))
            {
                let _ = write!(pbr, ",\"baseColorTexture\":{{\"index\":{t}}}");
            }
            let _ = write!(mat, ",\"pbrMetallicRoughness\":{{{pbr}}}");
            if let Some(t) = m
                .texture("NormalTexture")
                .and_then(|t| load_texture(t, &mut bin))
            {
                let _ = write!(mat, ",\"normalTexture\":{{\"index\":{t}}}");
            }
            let lower = m.shader.to_ascii_lowercase();
            if lower.contains("additive") || lower.contains("alpha") {
                mat.push_str(",\"alphaMode\":\"BLEND\"");
            }
            mat.push_str(",\"doubleSided\":true,\"extras\":{\"shader\":");
            mat.push_str(&esc(&m.shader));
            mat.push_str(",\"params\":{");
            let params: Vec<String> = m
                .params
                .iter()
                .map(|(n, v)| {
                    let val = match v {
                        ParamValue::Int(i) => i.to_string(),
                        ParamValue::Float(f) => num(*f),
                        ParamValue::Float3(a) => format!("[{}]", nums(a)),
                        ParamValue::Float4(a) => format!("[{}]", nums(a)),
                        ParamValue::Texture(t) => esc(t),
                    };
                    format!("{}:{val}", esc(n))
                })
                .collect();
            mat.push_str(&params.join(","));
            mat.push_str("}}}");
            materials.push(mat);
            prims.push(primitive(
                &mut bin,
                &mut accessors,
                sub,
                materials.len() - 1,
            ));
        }
        meshes.push(format!(
            "{{\"name\":{},\"primitives\":[{}]}}",
            esc(&mesh.name),
            prims.join(",")
        ));
        let node = (
            mesh.name.clone(),
            vec![],
            format!("\"mesh\":{}", meshes.len() - 1),
        );
        nodes.push(node);
        let skinned = mesh.sub_meshes.iter().any(SubMesh::is_skinned);
        let parent = if skinned || model.bones.is_empty() {
            0
        } else {
            bone_node(model.bone_of_object(oi))
        };
        let ni = nodes.len() - 1;
        nodes[parent].1.push(ni);
    }
    for p in &model.proxies {
        nodes.push((
            p.name.clone(),
            vec![],
            format!("\"extras\":{{\"proxy\":true,\"hidden\":{}}}", p.hidden),
        ));
        let ni = nodes.len() - 1;
        nodes[bone_node(p.bone)].1.push(ni);
    }
    for (li, l) in model.lights.iter().enumerate() {
        nodes.push((
            l.name.clone(),
            vec![],
            format!(
                "\"extras\":{{\"light\":\"{:?}\",\"color\":[{}],\"intensity\":{}}}",
                l.kind,
                nums(&l.color),
                num(l.intensity)
            ),
        ));
        let ni = nodes.len() - 1;
        nodes[bone_node(model.bone_of_object(model.meshes.len() + li))]
            .1
            .push(ni);
    }

    let node_json: Vec<String> = nodes
        .iter()
        .map(|(name, children, body)| {
            let mut s = format!("{{\"name\":{}", esc(name));
            if !children.is_empty() {
                let _ = write!(
                    s,
                    ",\"children\":[{}]",
                    children
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            if !body.is_empty() {
                s.push(',');
                s.push_str(body);
            }
            s.push('}');
            s
        })
        .collect();
    let list = |key: &str, v: &[String]| {
        if v.is_empty() {
            String::new()
        } else {
            format!(",\"{key}\":[{}]", v.join(","))
        }
    };
    let json = format!(
        "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"alamo-model alo_to_gltf\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{}],\"buffers\":[{{\"byteLength\":{}}}],\"bufferViews\":[{}]{}{}{}{}{}}}",
        node_json.join(","),
        bin.data.len(),
        bin.views.join(","),
        list("accessors", &accessors),
        list("meshes", &meshes),
        list("materials", &materials),
        list("textures", &textures),
        list("images", &images),
    );
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.data.len().is_multiple_of(4) {
        bin.data.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.data.len();
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend((total as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(&json);
    glb.extend((bin.data.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(&bin.data);
    std::fs::write(out, &glb).expect("write");
    println!(
        "wrote {out}: {} nodes, {} meshes, {} materials, {} textures, {} bytes",
        nodes.len(),
        meshes.len(),
        materials.len(),
        textures.len(),
        glb.len()
    );
}
