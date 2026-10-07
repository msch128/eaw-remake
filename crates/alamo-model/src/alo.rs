//! Alamo model (`.alo`) parsing into engine-neutral data.
//!
//! # Coordinate conventions
//!
//! All data is returned exactly as stored in the file, in the engine's native space: the space of
//! 3ds Max, the DCC tool the assets are authored in. That is a **right-handed, Z-up** system where
//! one unit is roughly one meter-ish "world unit" of the game (ship scale is arbitrary per model).
//! Bone transforms are stored as a row-major 3x4 matrix `[R | t]` (column-vector convention,
//! `p' = R * p + t`) relative to the parent bone. Mesh vertices are in model space (the space of
//! the skeleton root); a rigid mesh is attached to a bone through a [`Connection`], a skinned
//! mesh uses per-vertex bone indices that go through the sub-mesh's [`SubMesh::bone_palette`].
//! UV origin is the top-left corner (Direct3D convention, `v` grows downwards).
//!
//! Converting to a Y-up right-handed system (glTF, Bevy) maps `(x, y, z) -> (x, z, -y)`.

use crate::chunk::{Chunk, Chunks, Reader, cstr};
use crate::error::{Error, Result};

/// Chunk ids used by `.alo` files.
pub mod ids {
    /// Skeleton container.
    pub const SKELETON: u32 = 0x200;
    /// Skeleton header (bone count).
    pub const SKELETON_INFO: u32 = 0x201;
    /// One bone (container).
    pub const BONE: u32 = 0x202;
    /// Bone name.
    pub const BONE_NAME: u32 = 0x203;
    /// Bone data, version 1 (no billboard mode).
    pub const BONE_DATA_V1: u32 = 0x205;
    /// Bone data, version 2.
    pub const BONE_DATA_V2: u32 = 0x206;
    /// Mesh container.
    pub const MESH: u32 = 0x400;
    /// Mesh name.
    pub const MESH_NAME: u32 = 0x401;
    /// Mesh header (sub-mesh count, bounding box, flags).
    pub const MESH_INFO: u32 = 0x402;
    /// Connections and proxies container.
    pub const CONNECTIONS: u32 = 0x600;
    /// Connection counts.
    pub const CONNECTION_INFO: u32 = 0x601;
    /// One object-to-bone connection.
    pub const CONNECTION: u32 = 0x602;
    /// One proxy.
    pub const PROXY: u32 = 0x603;
    /// Particle system root (not a model).
    pub const PARTICLE: u32 = 0x900;
    /// Sub-mesh geometry container.
    pub const SUBMESH_DATA: u32 = 0x10000;
    /// Sub-mesh counts.
    pub const SUBMESH_INFO: u32 = 0x10001;
    /// Vertex format name.
    pub const VERTEX_FORMAT: u32 = 0x10002;
    /// Index buffer (u16).
    pub const INDEX_BUFFER: u32 = 0x10004;
    /// Vertex buffer, 128 byte vertices (legacy).
    pub const VERTEX_BUFFER_128: u32 = 0x10005;
    /// Skin bone palette (u32 bone indices).
    pub const BONE_PALETTE: u32 = 0x10006;
    /// Vertex buffer, 144 byte vertices.
    pub const VERTEX_BUFFER_144: u32 = 0x10007;
    /// Material container.
    pub const MATERIAL: u32 = 0x10100;
    /// Shader file name.
    pub const SHADER: u32 = 0x10101;
    /// Integer shader parameter.
    pub const PARAM_INT: u32 = 0x10102;
    /// Float shader parameter.
    pub const PARAM_FLOAT: u32 = 0x10103;
    /// Float3 shader parameter.
    pub const PARAM_FLOAT3: u32 = 0x10104;
    /// Texture shader parameter.
    pub const PARAM_TEXTURE: u32 = 0x10105;
    /// Float4 shader parameter.
    pub const PARAM_FLOAT4: u32 = 0x10106;
    /// Light container.
    pub const LIGHT: u32 = 0x1300;
    /// Light name.
    pub const LIGHT_NAME: u32 = 0x1301;
    /// Light parameters.
    pub const LIGHT_DATA: u32 = 0x1302;
}

/// How a bone is oriented towards the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillboardMode {
    /// Normal bone.
    Disable,
    /// Faces the camera ("parallel").
    Parallel,
    /// Faces the camera ("face").
    Face,
    /// Rotates around the Z axis only.
    ZAxisView,
    /// Rotates around the Z axis only, locked in light direction.
    ZAxisLight,
    /// Rotates around the Z axis only, locked wind.
    ZAxisWind,
    /// Sun billboard.
    Sunlight,
    /// Unrecognized value.
    Other(u32),
}

impl BillboardMode {
    fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Disable,
            1 => Self::Parallel,
            2 => Self::Face,
            3 => Self::ZAxisView,
            4 => Self::ZAxisLight,
            5 => Self::ZAxisWind,
            6 => Self::Sunlight,
            o => Self::Other(o),
        }
    }
}

/// A skeleton bone.
#[derive(Debug, Clone)]
pub struct Bone {
    /// Bone name.
    pub name: String,
    /// Parent bone index, `None` for the root.
    pub parent: Option<usize>,
    /// Visibility flag of the bone (and the objects attached to it).
    pub visible: bool,
    /// Billboard behavior.
    pub billboard: BillboardMode,
    /// Local transform, row-major 3x4 `[R | t]`.
    pub transform: [[f32; 4]; 3],
}

/// A shader parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// Integer.
    Int(i32),
    /// Float.
    Float(f32),
    /// Three floats (colors, vectors).
    Float3([f32; 3]),
    /// Four floats (colors, vectors).
    Float4([f32; 4]),
    /// Texture file name (without directory), e.g. `W_Rock.tga`.
    Texture(String),
}

/// A sub-mesh material: shader plus named parameters in file order.
#[derive(Debug, Clone, Default)]
pub struct Material {
    /// Shader file name, e.g. `MeshBumpColorize.fx`.
    pub shader: String,
    /// Parameters.
    pub params: Vec<(String, ParamValue)>,
}

impl Material {
    /// Looks up a parameter by name (case-insensitive).
    pub fn param(&self, name: &str) -> Option<&ParamValue> {
        self.params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }

    /// Texture file name of parameter `name`.
    pub fn texture(&self, name: &str) -> Option<&str> {
        match self.param(name) {
            Some(ParamValue::Texture(t)) => Some(t),
            _ => None,
        }
    }

    /// Float4 parameter `name`; a Float3 is widened with `w = 1`.
    pub fn float4(&self, name: &str) -> Option<[f32; 4]> {
        match self.param(name)? {
            ParamValue::Float4(v) => Some(*v),
            ParamValue::Float3(v) => Some([v[0], v[1], v[2], 1.0]),
            _ => None,
        }
    }

    /// Float parameter `name`.
    pub fn float(&self, name: &str) -> Option<f32> {
        match self.param(name)? {
            ParamValue::Float(v) => Some(*v),
            _ => None,
        }
    }
}

/// One vertex with every attribute the file formats can carry; attributes absent from the vertex
/// format hold zero (identity weights `[1, 0, 0, 0]` for skinning).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vertex {
    /// Position.
    pub position: [f32; 3],
    /// Normal.
    pub normal: [f32; 3],
    /// Up to four UV sets.
    pub uv: [[f32; 2]; 4],
    /// Tangent.
    pub tangent: [f32; 3],
    /// Binormal.
    pub binormal: [f32; 3],
    /// Vertex color (RGBA, 0..1).
    pub color: [f32; 4],
    /// Bone palette indices.
    pub bone_indices: [u32; 4],
    /// Bone weights.
    pub bone_weights: [f32; 4],
}

/// A drawable chunk of a mesh: one material, one vertex and one index buffer (triangle list).
#[derive(Debug, Clone, Default)]
pub struct SubMesh {
    /// Material.
    pub material: Material,
    /// Vertex format name such as `alD3dVertRSkinNU2U3U3`.
    pub vertex_format: String,
    /// Vertices.
    pub vertices: Vec<Vertex>,
    /// Triangle list indices.
    pub indices: Vec<u16>,
    /// For skinned sub-meshes: maps vertex bone indices to skeleton bone indices.
    pub bone_palette: Vec<u32>,
}

impl SubMesh {
    /// True if the vertex format carries bone weights.
    pub fn is_skinned(&self) -> bool {
        self.vertex_format.contains("Skin") || self.vertex_format.contains("B4I4")
    }
}

/// A mesh: a named group of sub-meshes.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    /// Mesh name.
    pub name: String,
    /// Level of detail parsed from an `_ALTn` name suffix; 0 is the highest detail.
    pub lod: u32,
    /// Mesh is hidden by default (e.g. damage or alternate states).
    pub hidden: bool,
    /// Mesh participates in collision / picking.
    pub collision: bool,
    /// Bounding box minimum.
    pub bbox_min: [f32; 3],
    /// Bounding box maximum.
    pub bbox_max: [f32; 3],
    /// Sub-meshes.
    pub sub_meshes: Vec<SubMesh>,
}

impl Mesh {
    /// True for shadow volume meshes (they use the `MeshShadowVolume.fx` shader).
    pub fn is_shadow_volume(&self) -> bool {
        self.sub_meshes.iter().any(|s| {
            s.material
                .shader
                .eq_ignore_ascii_case("MeshShadowVolume.fx")
        })
    }
}

/// Parses the `_ALTn` suffix of a mesh name.
fn alt_level(name: &str) -> u32 {
    let upper = name.to_ascii_uppercase();
    upper
        .rfind("_ALT")
        .and_then(|i| upper[i + 4..].parse().ok())
        .unwrap_or(0)
}

/// Kind of light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightKind {
    /// Point light.
    Omni,
    /// Directional light.
    Directional,
    /// Spot light.
    Spot,
    /// Unrecognized value.
    Other(u32),
}

/// A light object.
#[derive(Debug, Clone)]
pub struct Light {
    /// Name.
    pub name: String,
    /// Kind.
    pub kind: LightKind,
    /// Color.
    pub color: [f32; 3],
    /// Intensity.
    pub intensity: f32,
    /// Far attenuation distance.
    pub far_attenuation: f32,
    /// Second distance parameter stored in the file (meaning not fully known).
    pub near_attenuation: f32,
    /// Spot inner cone angle in radians.
    pub inner_cone: f32,
    /// Spot outer cone angle in radians.
    pub outer_cone: f32,
}

/// Attaches object `object` (index into meshes followed by lights) to bone `bone`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connection {
    /// Object index: meshes first, then lights.
    pub object: usize,
    /// Bone index.
    pub bone: usize,
}

/// A named attachment point (particle emitter, hardpoint, light, ...) on a bone.
#[derive(Debug, Clone)]
pub struct Proxy {
    /// Name, e.g. `P_ATST_Die`.
    pub name: String,
    /// Bone index.
    pub bone: usize,
    /// Hidden flag.
    pub hidden: bool,
    /// Stays hidden when the alternate (lower) detail level is selected.
    pub alt_decrease_stay_hidden: bool,
}

/// A parsed `.alo` model.
#[derive(Debug, Clone, Default)]
pub struct Model {
    /// Skeleton bones (parents always precede children in practice, not guaranteed).
    pub bones: Vec<Bone>,
    /// Meshes.
    pub meshes: Vec<Mesh>,
    /// Lights.
    pub lights: Vec<Light>,
    /// Object-to-bone connections.
    pub connections: Vec<Connection>,
    /// Proxies.
    pub proxies: Vec<Proxy>,
}

impl Model {
    /// Parses a `.alo` file. Particle systems (`.alo` files that are not models) yield
    /// [`Error::NotAModel`].
    pub fn parse(data: &[u8]) -> Result<Model> {
        let mut model = Model::default();
        let mut connection_chunks = Vec::new();
        let mut seen_any = false;
        for chunk in Chunks::new(data) {
            let chunk = chunk?;
            seen_any = true;
            match chunk.id {
                ids::SKELETON => model.bones = parse_skeleton(&chunk)?,
                ids::MESH => model.meshes.push(parse_mesh(&chunk)?),
                ids::LIGHT => model.lights.push(parse_light(&chunk)?),
                ids::CONNECTIONS => connection_chunks.push(chunk),
                ids::PARTICLE => return Err(Error::NotAModel("particle system")),
                _ => {}
            }
        }
        if !seen_any {
            return Err(Error::Invalid("empty file".into()));
        }
        for c in connection_chunks {
            parse_connections(&c, &mut model)?;
        }
        let bones = model.bones.len();
        let objects = model.meshes.len() + model.lights.len();
        if let Some(c) = model
            .connections
            .iter()
            .find(|c| c.bone >= bones || c.object >= objects)
        {
            return Err(Error::Invalid(format!("connection out of range: {c:?}")));
        }
        if let Some(p) = model.proxies.iter().find(|p| p.bone >= bones) {
            return Err(Error::Invalid(format!(
                "proxy {} bone out of range",
                p.name
            )));
        }
        Ok(model)
    }

    /// Index of the bone called `name` (case-insensitive).
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones
            .iter()
            .position(|b| b.name.eq_ignore_ascii_case(name))
    }

    /// Bone an object (mesh or light index) is attached to; the root (0) when unconnected.
    pub fn bone_of_object(&self, object: usize) -> usize {
        self.connections
            .iter()
            .find(|c| c.object == object)
            .map_or(0, |c| c.bone)
    }

    /// Absolute (model space) transform of bone `i`, row-major 3x4.
    pub fn world_transform(&self, i: usize) -> [[f32; 4]; 3] {
        let mut m = IDENTITY;
        let mut cur = Some(i);
        let mut guard = 0;
        while let Some(b) = cur {
            let Some(bone) = self.bones.get(b) else { break };
            m = mul34(&bone.transform, &m);
            cur = bone.parent;
            guard += 1;
            if guard > self.bones.len() {
                break; // parent cycle in malformed data
            }
        }
        m
    }
}

/// Identity 3x4 transform.
pub const IDENTITY: [[f32; 4]; 3] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// Multiplies two row-major 3x4 affine transforms: `a * b`.
pub fn mul34(a: &[[f32; 4]; 3], b: &[[f32; 4]; 3]) -> [[f32; 4]; 3] {
    let mut out = [[0.0; 4]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
            if c == 3 {
                *v += a[r][3];
            }
        }
    }
    out
}

/// Iterates the mini-chunks (`u8 id`, `u8 size`, payload) of a payload.
pub fn minis(data: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut rest = data;
    std::iter::from_fn(move || {
        if rest.len() < 2 {
            return None;
        }
        let (id, size) = (rest[0], rest[1] as usize);
        if rest.len() < 2 + size {
            rest = &[];
            return None;
        }
        let payload = &rest[2..2 + size];
        rest = &rest[2 + size..];
        Some((id, payload))
    })
}

fn mini_u32(payload: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(payload.get(..4)?.try_into().ok()?))
}

fn parse_skeleton(sk: &Chunk) -> Result<Vec<Bone>> {
    let mut declared = None;
    let mut bones = Vec::new();
    for c in sk.children() {
        let c = c?;
        match c.id {
            ids::SKELETON_INFO => declared = Some(c.reader().u32()? as usize),
            ids::BONE => bones.push(parse_bone(&c)?),
            _ => {}
        }
    }
    if let Some(n) = declared
        && n != bones.len()
    {
        return Err(Error::Invalid(format!(
            "skeleton declares {n} bones but has {}",
            bones.len()
        )));
    }
    for (i, b) in bones.iter().enumerate() {
        if let Some(p) = b.parent
            && p >= bones.len()
        {
            return Err(Error::Invalid(format!("bone {i} parent {p} out of range")));
        }
    }
    Ok(bones)
}

fn parse_bone(c: &Chunk) -> Result<Bone> {
    let mut name = String::new();
    let mut bone = None;
    for ch in c.children() {
        let ch = ch?;
        match ch.id {
            ids::BONE_NAME => name = cstr(ch.data),
            ids::BONE_DATA_V1 | ids::BONE_DATA_V2 => {
                let mut r = ch.reader();
                let parent = r.i32()?;
                let visible = r.u32()? != 0;
                let billboard = if ch.id == ids::BONE_DATA_V2 {
                    BillboardMode::from_u32(r.u32()?)
                } else {
                    BillboardMode::Disable
                };
                let m = r.floats::<12>()?;
                bone = Some((parent, visible, billboard, m));
            }
            _ => {}
        }
    }
    let (parent, visible, billboard, m) =
        bone.ok_or_else(|| Error::Invalid(format!("bone {name:?} without data")))?;
    Ok(Bone {
        name,
        parent: usize::try_from(parent).ok(),
        visible,
        billboard,
        transform: [
            [m[0], m[1], m[2], m[3]],
            [m[4], m[5], m[6], m[7]],
            [m[8], m[9], m[10], m[11]],
        ],
    })
}

fn parse_mesh(c: &Chunk) -> Result<Mesh> {
    let mut mesh = Mesh::default();
    let mut pending: Option<Material> = None;
    for ch in c.children() {
        let ch = ch?;
        match ch.id {
            ids::MESH_NAME => {
                mesh.name = cstr(ch.data);
                mesh.lod = alt_level(&mesh.name);
            }
            ids::MESH_INFO => {
                let mut r = ch.reader();
                let _sub_count = r.u32()?;
                let bb = r.floats::<6>()?;
                mesh.bbox_min = [bb[0], bb[1], bb[2]];
                mesh.bbox_max = [bb[3], bb[4], bb[5]];
                let _unknown = r.u32()?;
                mesh.hidden = r.u32()? != 0;
                mesh.collision = r.u32()? != 0;
            }
            ids::MATERIAL => pending = Some(parse_material(&ch)?),
            ids::SUBMESH_DATA => {
                let mut sub = parse_submesh(&ch)?;
                sub.material = pending.take().unwrap_or_default();
                mesh.sub_meshes.push(sub);
            }
            _ => {}
        }
    }
    Ok(mesh)
}

fn parse_material(c: &Chunk) -> Result<Material> {
    let mut mat = Material::default();
    for ch in c.children() {
        let ch = ch?;
        if ch.id == ids::SHADER {
            mat.shader = cstr(ch.data);
            continue;
        }
        if !matches!(
            ch.id,
            ids::PARAM_INT
                | ids::PARAM_FLOAT
                | ids::PARAM_FLOAT3
                | ids::PARAM_TEXTURE
                | ids::PARAM_FLOAT4
        ) {
            continue;
        }
        let mut name = String::new();
        let mut value = None;
        for (id, payload) in minis(ch.data) {
            match id {
                1 => name = cstr(payload),
                2 => value = Some(payload),
                _ => {}
            }
        }
        let Some(v) = value else { continue };
        let mut r = Reader::new(v);
        let parsed = match ch.id {
            ids::PARAM_INT => ParamValue::Int(r.i32()?),
            ids::PARAM_FLOAT => ParamValue::Float(r.f32()?),
            ids::PARAM_FLOAT3 => ParamValue::Float3(r.floats()?),
            ids::PARAM_FLOAT4 => ParamValue::Float4(r.floats()?),
            _ => ParamValue::Texture(cstr(v)),
        };
        mat.params.push((name, parsed));
    }
    Ok(mat)
}

fn parse_submesh(c: &Chunk) -> Result<SubMesh> {
    let mut sub = SubMesh::default();
    let (mut vcount, mut tcount) = (0usize, 0usize);
    let mut vbuf: Option<(&[u8], usize)> = None;
    for ch in c.children() {
        let ch = ch?;
        match ch.id {
            ids::SUBMESH_INFO => {
                let mut r = ch.reader();
                vcount = r.u32()? as usize;
                tcount = r.u32()? as usize;
            }
            ids::VERTEX_FORMAT => sub.vertex_format = cstr(ch.data),
            ids::VERTEX_BUFFER_144 => vbuf = Some((ch.data, 144)),
            ids::VERTEX_BUFFER_128 => vbuf = Some((ch.data, 128)),
            ids::INDEX_BUFFER => {
                sub.indices = ch
                    .data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes(*b))
                    .collect();
            }
            ids::BONE_PALETTE => {
                sub.bone_palette = ch
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| u32::from_le_bytes(*b))
                    .collect();
            }
            _ => {}
        }
    }
    if let Some((data, stride)) = vbuf {
        if vcount.checked_mul(stride) != Some(data.len()) {
            return Err(Error::Invalid(format!(
                "vertex buffer is {} bytes, expected {vcount} x {stride}",
                data.len()
            )));
        }
        sub.vertices = data
            .chunks_exact(stride)
            .map(|b| decode_vertex(b, stride))
            .collect::<Result<_>>()?;
    }
    if sub.indices.len() != tcount * 3 {
        return Err(Error::Invalid(format!(
            "index count {} does not match {tcount} triangles",
            sub.indices.len()
        )));
    }
    if let Some(&bad) = sub
        .indices
        .iter()
        .find(|&&i| i as usize >= sub.vertices.len())
    {
        return Err(Error::Invalid(format!(
            "index {bad} out of range ({} vertices)",
            sub.vertices.len()
        )));
    }
    Ok(sub)
}

/// Decodes one vertex. Layout (all little endian): position, normal (3 f32 each), 4 UV sets (2 f32),
/// tangent, binormal, color (4 f32), then for the 144 byte layout 4 extra floats (unknown, usually 1),
/// 4 u32 bone indices, 4 f32 bone weights.
fn decode_vertex(b: &[u8], stride: usize) -> Result<Vertex> {
    let mut r = Reader::new(b);
    let mut v = Vertex {
        position: r.floats()?,
        normal: r.floats()?,
        ..Vertex::default()
    };
    for uv in &mut v.uv {
        *uv = r.floats()?;
    }
    v.tangent = r.floats()?;
    v.binormal = r.floats()?;
    v.color = r.floats()?;
    if stride == 144 {
        r.skip(16)?;
    }
    for i in &mut v.bone_indices {
        *i = r.u32()?;
    }
    v.bone_weights = r.floats()?;
    Ok(v)
}

fn parse_light(c: &Chunk) -> Result<Light> {
    let mut name = String::new();
    let mut data = None;
    for ch in c.children() {
        let ch = ch?;
        match ch.id {
            ids::LIGHT_NAME => name = cstr(ch.data),
            ids::LIGHT_DATA => data = Some(ch),
            _ => {}
        }
    }
    let d = data.ok_or_else(|| Error::Invalid(format!("light {name:?} without data")))?;
    let mut r = d.reader();
    let kind = match r.u32()? {
        0 => LightKind::Omni,
        1 => LightKind::Directional,
        2 => LightKind::Spot,
        o => LightKind::Other(o),
    };
    Ok(Light {
        name,
        kind,
        color: r.floats()?,
        intensity: r.f32()?,
        far_attenuation: r.f32()?,
        near_attenuation: r.f32()?,
        inner_cone: r.f32()?,
        outer_cone: r.f32()?,
    })
}

fn parse_connections(c: &Chunk, model: &mut Model) -> Result<()> {
    for ch in c.children() {
        let ch = ch?;
        match ch.id {
            ids::CONNECTION => {
                let (mut object, mut bone) = (None, None);
                for (id, p) in minis(ch.data) {
                    match id {
                        2 => object = mini_u32(p),
                        3 => bone = mini_u32(p),
                        _ => {}
                    }
                }
                if let (Some(o), Some(b)) = (object, bone) {
                    model.connections.push(Connection {
                        object: o as usize,
                        bone: b as usize,
                    });
                }
            }
            ids::PROXY => {
                let mut p = Proxy {
                    name: String::new(),
                    bone: 0,
                    hidden: false,
                    alt_decrease_stay_hidden: false,
                };
                for (id, v) in minis(ch.data) {
                    match id {
                        5 => p.name = cstr(v),
                        6 => p.bone = mini_u32(v).unwrap_or(0) as usize,
                        7 => p.hidden = mini_u32(v).unwrap_or(0) != 0,
                        8 => p.alt_decrease_stay_hidden = mini_u32(v).unwrap_or(0) != 0,
                        _ => {}
                    }
                }
                model.proxies.push(p);
            }
            _ => {}
        }
    }
    Ok(())
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

    fn bone(name: &str, parent: i32, tx: f32) -> Vec<u8> {
        let mut data = parent.to_le_bytes().to_vec();
        data.extend(1u32.to_le_bytes());
        data.extend(0u32.to_le_bytes());
        data.extend(f32s(&[1., 0., 0., tx, 0., 1., 0., 0., 0., 0., 1., 0.]));
        let mut inner = chunk(ids::BONE_NAME, false, &[name.as_bytes(), &[0]].concat());
        inner.extend(chunk(ids::BONE_DATA_V2, false, &data));
        chunk(ids::BONE, true, &inner)
    }

    fn triangle_mesh() -> Vec<u8> {
        let mut vert = Vec::new();
        for (i, p) in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
            .iter()
            .enumerate()
        {
            let mut v = f32s(p);
            v.extend(f32s(&[0., 0., 1.]));
            v.extend(f32s(&[i as f32, 0.5, 0., 0., 0., 0., 0., 0.]));
            v.extend(f32s(&[0.; 6]));
            v.extend(f32s(&[1.; 4]));
            v.extend(f32s(&[1.; 4]));
            v.extend([0u8; 16]);
            v.extend(f32s(&[1., 0., 0., 0.]));
            assert_eq!(v.len(), 144);
            vert.extend(v);
        }
        let mut info = 3u32.to_le_bytes().to_vec();
        info.extend(1u32.to_le_bytes());
        let mut sub = chunk(ids::SUBMESH_INFO, false, &info);
        sub.extend(chunk(ids::VERTEX_FORMAT, false, b"alD3dVertNU2\0"));
        sub.extend(chunk(ids::VERTEX_BUFFER_144, false, &vert));
        sub.extend(chunk(ids::INDEX_BUFFER, false, &[0, 0, 1, 0, 2, 0]));
        let mut mat = chunk(ids::SHADER, false, b"MeshGloss.fx\0");
        let mut tex = mini(1, b"BaseTexture\0");
        tex.extend(mini(2, b"Foo.tga\0"));
        mat.extend(chunk(ids::PARAM_TEXTURE, false, &tex));
        let mut col = mini(1, b"Color\0");
        col.extend(mini(2, &f32s(&[1., 0.5, 0.25, 1.])));
        mat.extend(chunk(ids::PARAM_FLOAT4, false, &col));
        let mut head = vec![1, 0, 0, 0];
        head.extend(f32s(&[0., 0., 0., 1., 1., 0.]));
        head.resize(128, 0);
        head[32] = 1; // hidden
        let mut mesh = chunk(ids::MESH_NAME, false, b"Hull_ALT2\0");
        mesh.extend(chunk(ids::MESH_INFO, false, &head));
        mesh.extend(chunk(ids::MATERIAL, true, &mat));
        mesh.extend(chunk(ids::SUBMESH_DATA, true, &sub));
        chunk(ids::MESH, true, &mesh)
    }

    fn sample() -> Vec<u8> {
        let mut sk = chunk(ids::SKELETON_INFO, false, &2u32.to_le_bytes());
        sk.extend(bone("Root", -1, 0.0));
        sk.extend(bone("Gun", 0, 5.0));
        let mut file = chunk(ids::SKELETON, true, &sk);
        file.extend(triangle_mesh());
        let mut cons = chunk(ids::CONNECTION_INFO, false, &mini(1, &1u32.to_le_bytes()));
        let mut c = mini(2, &0u32.to_le_bytes());
        c.extend(mini(3, &1u32.to_le_bytes()));
        cons.extend(chunk(ids::CONNECTION, false, &c));
        let mut p = mini(5, b"P_Fire\0");
        p.extend(mini(6, &1u32.to_le_bytes()));
        cons.extend(chunk(ids::PROXY, false, &p));
        file.extend(chunk(ids::CONNECTIONS, true, &cons));
        file
    }

    #[test]
    fn parses_synthetic_model() {
        let m = Model::parse(&sample()).unwrap();
        assert_eq!(m.bones.len(), 2);
        assert_eq!(m.bones[1].parent, Some(0));
        assert_eq!(m.world_transform(1)[0][3], 5.0);
        let mesh = &m.meshes[0];
        assert_eq!(
            (mesh.name.as_str(), mesh.lod, mesh.hidden),
            ("Hull_ALT2", 2, true)
        );
        assert!(!mesh.collision);
        let sub = &mesh.sub_meshes[0];
        assert_eq!(sub.material.shader, "MeshGloss.fx");
        assert_eq!(sub.material.texture("basetexture"), Some("Foo.tga"));
        assert_eq!(sub.material.float4("Color"), Some([1., 0.5, 0.25, 1.]));
        assert_eq!(sub.vertices[1].position, [1., 0., 0.]);
        assert_eq!(sub.vertices[2].uv[0], [2., 0.5]);
        assert_eq!(sub.vertices[0].bone_weights, [1., 0., 0., 0.]);
        assert_eq!(sub.indices, vec![0, 1, 2]);
        assert_eq!(m.bone_of_object(0), 1);
        assert_eq!(m.proxies[0].name, "P_Fire");
        assert_eq!(m.proxies[0].bone, 1);
    }

    #[test]
    fn truncation_is_an_error() {
        let data = sample();
        for cut in [3, 20, data.len() / 2, data.len() - 1] {
            assert!(Model::parse(&data[..cut]).is_err(), "cut {cut}");
        }
    }

    #[test]
    fn rejects_bad_index() {
        let mut data = sample();
        let pos = data
            .windows(6)
            .position(|w| w == [0, 0, 1, 0, 2, 0])
            .unwrap();
        data[pos + 4] = 9;
        assert!(Model::parse(&data).is_err());
    }

    #[test]
    fn particle_is_not_a_model() {
        let data = chunk(ids::PARTICLE, true, &[]);
        assert!(matches!(Model::parse(&data), Err(Error::NotAModel(_))));
    }
}
