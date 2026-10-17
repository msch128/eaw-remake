//! Conversion of parsed `.alo` models into Bevy meshes, materials and entity hierarchies.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use alamo_fs::{FileSource, GameFs};
use alamo_model::Model;
use alamo_model::alo::{BillboardMode, Material, ParamValue, SubMesh};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

use crate::animation::TurretAim;
use crate::material::{
    AlamoMaterial, AlamoParams, FLAG_ALPHA_FROM_BASE, FLAG_COLORIZE, FLAG_NORMAL_MAP,
    FLAG_SPEC_FROM_BASE_ALPHA, FLAG_VERTEX_COLOR, ShaderKind,
};
use crate::texture::TextureCache;
use crate::xmldata::XmlDefs;

/// Global rendering options, read when materials are created.
#[derive(Resource, Debug, Clone, Default)]
pub struct AlamoRenderSettings {
    /// Invert the green channel of normal maps (DirectX vs OpenGL convention).
    pub flip_normal_green: bool,
    /// Ships receive the sun's shadow maps (needs a shadow-casting sun, see
    /// [`spawn_sun_shadowed`](crate::spawn_sun_shadowed)).
    pub shadows: bool,
    /// Main-thread time per frame that `spawn_models` may spend; the rest of the new models is spawned
    /// on the following frames (at least one model per frame). `None` spawns everything at once.
    pub spawn_budget: Option<std::time::Duration>,
}

/// The game file system models and textures are read from.
#[derive(Resource, Clone)]
pub struct AlamoFs(pub Arc<GameFs>);

/// Level of detail selection for a model.
///
/// Meshes of a model are grouped by name without their `_ALTn` suffix; each group renders as one
/// of its variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LodSelect {
    /// Per group the variant with the most triangles; groups that only exist at `_ALT2` or above
    /// (debug overlays such as the planet grid) are left out.
    #[default]
    MostDetailed,
    /// Per group the variant with the highest `_ALTn` not above `n`; groups without such a
    /// variant are left out. `Level(0)` is the lowest numbered ALT.
    Level(u32),
}

/// Spawns an original Empire at War model below this entity. Insert it together with a
/// [`Transform`]; changing `team_color` afterwards recolors the instance.
#[derive(Component, Debug, Clone)]
#[require(Transform, Visibility)]
pub struct AlamoModel {
    /// Path of the `.alo`, either full (`Art/Models/X.ALO`) or just the file name.
    pub path: String,
    /// Faction color applied where `BaseTexture` alpha masks (`*Colorize` shaders).
    pub team_color: Color,
    /// Level of detail.
    pub lod: LodSelect,
    /// Rotate the model by 180 degrees around the up axis so that a ship's bow (Alamo -Y) points
    /// along Bevy's forward direction (-Z). Without it the plain Z-up to Y-up conversion is
    /// used and the bow points along +Z.
    pub bow_forward: bool,
    /// XML name of the space unit (`Allegiance_Star_Destroyer`); when set, the unit's hardpoints
    /// (turrets and art pieces named by `Model_To_Attach`) are attached to the ship's bones.
    pub unit: Option<String>,
    /// The model hangs below a bone of another model (a turret on a ship), so the Z-up to Y-up
    /// conversion of the parent already applies and none is added.
    pub attached: bool,
    /// A sky or nebula model: drawn at the far plane depth, behind everything (see `FLAG_BACKGROUND`).
    pub background: bool,
}

impl AlamoModel {
    /// Attaches the hardpoints of the XML unit `name` (see [`AlamoModel::unit`]).
    pub fn with_unit(mut self, name: impl Into<String>) -> Self {
        self.unit = Some(name.into());
        self
    }

    /// Marks the model as a sky backdrop that must never occlude scene geometry.
    pub fn as_background(mut self) -> Self {
        self.background = true;
        self
    }

    /// Marks the model as attached below a bone of another model.
    pub fn attached_to_bone(mut self) -> Self {
        self.attached = true;
        self
    }

    /// A model with white team color and default LOD.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            team_color: Color::WHITE,
            lod: LodSelect::default(),
            bow_forward: false,
            unit: None,
            attached: false,
            background: false,
        }
    }

    /// Sets the team color.
    pub fn with_team_color(mut self, color: Color) -> Self {
        self.team_color = color;
        self
    }

    /// Sets the level of detail.
    pub fn with_lod(mut self, lod: LodSelect) -> Self {
        self.lod = lod;
        self
    }

    /// Makes the ship's bow point along -Z.
    pub fn with_bow_forward(mut self) -> Self {
        self.bow_forward = true;
        self
    }
}

/// Marker added to a model root once it has been spawned.
#[derive(Component, Debug)]
pub struct AlamoModelLoaded {
    /// Bone entities in skeleton order.
    pub bones: Vec<Entity>,
    /// Entities of the spawned mesh parts.
    pub parts: Vec<Entity>,
    /// Proxy entities ([`AlamoProxy`]) in file order.
    pub proxies: Vec<Entity>,
    /// The `alamo_axis` node every bone and mesh hangs below; hidden by distance culling.
    pub axis: Entity,
    /// Hardpoint entities ([`AlamoHardpoint`]) of the unit, empty without [`AlamoModel::unit`].
    pub hardpoints: Vec<Entity>,
    /// Bone entities by lowercase bone name.
    pub bone_by_name: std::collections::HashMap<String, Entity>,
    /// Center of the visible geometry in the model root's space.
    pub center: Vec3,
    /// Radius of a sphere around `center` that contains the visible geometry.
    pub radius: f32,
}

/// A hardpoint of a ship: a named attachment (turret, shield generator, art piece) on a bone.
#[derive(Component, Debug, Clone)]
pub struct AlamoHardpoint {
    /// Hardpoint name from the XML.
    pub name: String,
    /// Destroyed (hidden, with its damage effects on).
    pub destroyed: bool,
    /// The attached turret model root, if the hardpoint has a `Model_To_Attach`; it carries a
    /// [`TurretAim`] that game code can enable and aim.
    pub turret: Option<Entity>,
}

/// Skeleton bone of a spawned model.
#[derive(Component, Debug, Clone)]
pub struct AlamoBone {
    /// Bone index in the skeleton.
    pub index: usize,
    /// Rest-pose local transform from the model file.
    pub rest: Transform,
}

/// Proxy (particle / hardpoint / light attachment point) of a spawned model; a child of its bone.
#[derive(Component, Debug, Clone)]
pub struct AlamoProxy {
    /// Proxy name, e.g. `P_ATST_Die`.
    pub name: String,
}

/// A spawned sub-mesh; remembers where its material comes from.
#[derive(Component, Debug, Clone, Copy)]
pub struct AlamoPart {
    mesh: usize,
    sub: usize,
    /// The part uses the shield shader (`MeshShield`); see [`crate::ShieldHit`].
    pub shield: bool,
}

/// Bone that faces the camera.
#[derive(Component, Debug, Clone, Copy)]
pub struct AlamoBillboard {
    /// Billboard mode of the bone.
    pub mode: BillboardMode,
}

/// Triggered on the model root entity when [`AlamoModel`] has been spawned.
#[derive(EntityEvent, Debug)]
pub struct ModelLoaded {
    /// Model root.
    pub entity: Entity,
}

/// Triggered on the model root entity when a model could not be loaded.
#[derive(EntityEvent, Debug)]
pub struct ModelLoadFailed {
    /// Model root.
    pub entity: Entity,
    /// What went wrong.
    pub error: String,
}

struct CachedPart {
    mesh: Handle<Mesh>,
    /// Material without team color.
    material: AlamoMaterial,
    colorize: bool,
}

struct CachedMesh {
    /// Index into `Model::meshes`.
    orig: usize,
    name: String,
    bone: usize,
    hidden: bool,
    skinned: bool,
    parts: Vec<CachedPart>,
    /// Selected under `MostDetailed`.
    most_detailed: bool,
}

/// A parsed and GPU-uploaded model shared by all its instances.
pub struct CachedModel {
    model: Model,
    meshes: Vec<CachedMesh>,
    inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
}

/// Cache of loaded models, materials and diagnostics, shared by all instances.
#[derive(Resource, Default)]
pub struct ModelCache {
    models: HashMap<String, Arc<CachedModel>>,
    /// Models parsed by the preload workers, waiting for `load_model` to build their meshes.
    preparsed: HashMap<String, Model>,
    materials: HashMap<(String, usize, usize, [u32; 3]), Handle<AlamoMaterial>>,
    logged_shaders: HashSet<String>,
}

impl ModelCache {
    /// True if `path` (a resolved model path) is loaded or waiting as a preparsed model.
    pub fn contains(&self, path: &str) -> bool {
        let k = path.to_ascii_lowercase();
        self.models.contains_key(&k) || self.preparsed.contains_key(&k)
    }

    pub(crate) fn insert_preparsed(&mut self, path: &str, model: Model) {
        self.preparsed.insert(path.to_ascii_lowercase(), model);
    }

    /// Number of distinct models loaded.
    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// Number of distinct materials created.
    pub fn material_count(&self) -> usize {
        self.materials.len()
    }
}

/// Resolves a model name to a game path.
const ATMOSPHERE_RING_GAIN: f32 = 2.2;

pub fn model_path(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    // Some XML references omit the extension.
    let path = if file.contains('.') {
        path.to_string()
    } else {
        format!("{path}.ALO")
    };
    if path.contains('/') || path.contains('\\') {
        path
    } else {
        format!("Art/Models/{path}")
    }
}
/// Name of a mesh group: the mesh name without a trailing `_ALTn`.
pub fn group_name(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    match upper.rfind("_ALT") {
        Some(i) if upper[i + 4..].parse::<u32>().is_ok() => name[..i].to_ascii_lowercase(),
        _ => name.to_ascii_lowercase(),
    }
}

/// Selects the visible mesh variants; returns one flag per mesh of `model`.
///
/// Per group one `_ALTn` value is chosen (see [`LodSelect`]); every mesh of the group with that
/// value is selected.
pub fn select_lod(model: &Model, lod: LodSelect) -> Vec<bool> {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in model.meshes.iter().enumerate() {
        groups.entry(group_name(&m.name)).or_default().push(i);
    }
    let tris = |i: usize| -> usize {
        model.meshes[i]
            .sub_meshes
            .iter()
            .map(|s| s.indices.len() / 3)
            .sum()
    };
    let mut chosen = vec![false; model.meshes.len()];
    for members in groups.values() {
        let mut levels: Vec<u32> = members.iter().map(|&i| model.meshes[i].lod).collect();
        levels.sort_unstable();
        levels.dedup();
        let level = match lod {
            LodSelect::MostDetailed => {
                if levels[0] >= 2 {
                    None
                } else {
                    levels.iter().copied().max_by_key(|&l| {
                        let total: usize = members
                            .iter()
                            .filter(|&&i| model.meshes[i].lod == l)
                            .map(|&i| tris(i))
                            .sum();
                        (total, std::cmp::Reverse(l))
                    })
                }
            }
            LodSelect::Level(n) => levels.iter().copied().filter(|&l| l <= n).max(),
        };
        if let Some(l) = level {
            for &i in members {
                chosen[i] = model.meshes[i].lod == l;
            }
        }
    }
    chosen
}
fn is_collision_only(mesh: &alamo_model::alo::Mesh) -> bool {
    !mesh.sub_meshes.is_empty()
        && mesh.sub_meshes.iter().all(|s| {
            s.material
                .shader
                .to_ascii_lowercase()
                .starts_with("meshcollision")
        })
}

fn normalize_shader(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_suffix(".fxo")
        .or_else(|| lower.strip_suffix(".fx"))
        .unwrap_or(&lower)
        .to_string()
}

fn f4(m: &Material, name: &str) -> Option<Vec4> {
    m.float4(name).map(Vec4::from)
}

/// Local radius of a flat disc mesh (all vertices in one plane, one common normal), 0 for anything else. The backdrop planets of the space maps
/// are such discs (`w_planet_*_low_light.alo`: 156 vertices, normal -Z, z extent 0.003 against a radius of 1600).
fn flat_disc_radius(sub: &SubMesh) -> f32 {
    let Some(first) = sub.vertices.first() else { return 0.0 };
    let n0 = Vec3::from(first.normal);
    let (mut zmin, mut zmax, mut r) = (f32::MAX, f32::MIN, 0.0f32);
    for v in &sub.vertices {
        if Vec3::from(v.normal).dot(n0) < 0.999 {
            return 0.0;
        }
        let p = Vec3::from(v.position);
        let along = p.dot(n0);
        zmin = zmin.min(along);
        zmax = zmax.max(along);
        r = r.max((p - n0 * along).length());
    }
    if r > 0.0 && zmax - zmin < r * 0.01 { r } else { 0.0 }
}

/// Maps an EaW material (shader name + params) to the closest [`AlamoMaterial`] flavour.
/// Returns the material and whether it uses team colorization.
pub fn describe_material(
    m: &Material,
    has_tangents: bool,
    has_vertex_color: bool,
    mut texture: impl FnMut(&str) -> Option<Handle<Image>>,
    mut note_unknown: impl FnMut(&str),
) -> (AlamoMaterial, bool) {
    let shader = normalize_shader(&m.shader);
    let mut p = AlamoParams::default();
    let mut flags = 0;
    let mut colorize = false;
    let bump = shader.contains("bump");
    let known = |k: ShaderKind, p: &mut AlamoParams| p.kind = k as u32;

    match shader.as_str() {
        "planet" | "planet_deathstar" => known(ShaderKind::Planet, &mut p),
        // Scrolling nebula cloud: additive, texture times vertex color (the per-vertex wave
        // distortion of the original is not reproduced).
        "nebula" => {
            known(ShaderKind::Additive, &mut p);
            flags |= FLAG_VERTEX_COLOR;
        }
        s if s.contains("additive") => known(ShaderKind::Additive, &mut p),
        s if s.contains("shield") => known(ShaderKind::Shield, &mut p),
        "meshsolidcolor" => known(ShaderKind::Solid, &mut p),
        s if s.contains("alpha") => {
            known(ShaderKind::Alpha, &mut p);
            flags |= FLAG_ALPHA_FROM_BASE;
        }
        s if s.contains("gloss")
            || bump
            || s == "aldefault"
            || s == "default"
            || s.starts_with("meshgloss")
            || s.starts_with("rskin")
            || s.starts_with("mesh") => {}
        other => note_unknown(other),
    }
    if p.kind == ShaderKind::Lit as u32 || p.kind == ShaderKind::Alpha as u32 {
        if shader.contains("colorize") {
            flags |= FLAG_COLORIZE;
            colorize = true;
        }
        if !bump {
            flags |= FLAG_SPEC_FROM_BASE_ALPHA;
        }
    }
    if has_vertex_color && (shader.contains("vcolor") || p.kind == ShaderKind::Lit as u32) {
        flags |= FLAG_VERTEX_COLOR;
    }

    // Parameters.
    if p.kind == ShaderKind::Additive as u32 {
        p.diffuse = f4(m, "Color").unwrap_or(Vec4::ONE);
    } else {
        p.diffuse = f4(m, "Diffuse").unwrap_or(Vec4::ONE);
    }
    p.emissive = f4(m, "Emissive").unwrap_or(Vec4::ZERO);
    p.specular = f4(m, "Specular").unwrap_or(Vec4::ZERO);
    if p.kind == ShaderKind::Solid as u32 {
        p.diffuse = f4(m, "Color").unwrap_or(p.diffuse);
    }
    if p.kind == ShaderKind::Shield as u32 {
        // Shield parameters share the planet slots: x EdgeBrightness, y BaseUVScale, z WaveUVScale.
        p.atmosphere = Vec4::new(
            m.float("EdgeBrightness").unwrap_or(0.5),
            m.float("BaseUVScale").unwrap_or(1.0),
            m.float("WaveUVScale").unwrap_or(1.0),
            0.0,
        );
    }
    p.shininess = m.float("Shininess").unwrap_or(32.0);
    if let Some(c) = f4(m, "Atmosphere") {
        p.atmosphere = c;
    }
    if let Some(c) = f4(m, "CityColor") {
        p.city_color = c;
    }
    p.atmosphere_power = m.float("AtmospherePower").unwrap_or(4.0);
    p.cloud_scroll_rate = m.float("CloudScrollRate").unwrap_or(0.0);
    if let Some(o) = f4(m, "UVOffset") {
        p.uv_offset.x = o.x;
        p.uv_offset.y = o.y;
    }
    if let Some(s) = f4(m, "UVScrollRate") {
        p.uv_offset.z = s.x;
        p.uv_offset.w = s.y;
    }
    if let Some(ParamValue::Float(_)) = m.param("UVScrollRate") {
        // scalar scroll rates are not used by the known shaders
    }

    let base = m.texture("BaseTexture").and_then(&mut texture);
    let normal = if has_tangents {
        m.texture("NormalTexture").and_then(&mut texture)
    } else {
        None
    };
    if normal.is_some() {
        flags |= FLAG_NORMAL_MAP;
    }
    let cloud = m.texture("CloudTexture").and_then(&mut texture);
    let cloud_normal = m.texture("CloudNormalTexture").and_then(&mut texture);
    p.flags = flags;
    (
        AlamoMaterial {
            params: p,
            base,
            normal,
            cloud,
            cloud_normal,
        },
        colorize,
    )
}

/// Builds a Bevy mesh from a sub-mesh. Vertex bone indices are remapped through the palette so
/// they index the skeleton directly.
pub fn build_mesh(sub: &SubMesh) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    let v = &sub.vertices;
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        v.iter().map(|x| x.position).collect::<Vec<[f32; 3]>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        v.iter().map(|x| x.normal).collect::<Vec<[f32; 3]>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        v.iter().map(|x| x.uv[0]).collect::<Vec<[f32; 2]>>(),
    );
    if has_tangents(sub) {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_TANGENT,
            v.iter()
                .map(|x| {
                    let n = Vec3::from(x.normal);
                    let t = Vec3::from(x.tangent);
                    let b = Vec3::from(x.binormal);
                    let w = if n.cross(t).dot(b) < 0.0 { -1.0 } else { 1.0 };
                    [t.x, t.y, t.z, w]
                })
                .collect::<Vec<[f32; 4]>>(),
        );
    }
    if has_vertex_color(sub) {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            v.iter().map(|x| x.color).collect::<Vec<[f32; 4]>>(),
        );
    }
    if sub.is_skinned() {
        let palette = |i: u32| -> u16 {
            sub.bone_palette
                .get(i as usize)
                .copied()
                .unwrap_or(0)
                .min(u16::MAX as u32) as u16
        };
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(
                v.iter()
                    .map(|x| x.bone_indices.map(palette))
                    .collect::<Vec<[u16; 4]>>(),
            ),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_WEIGHT,
            v.iter().map(|x| x.bone_weights).collect::<Vec<[f32; 4]>>(),
        );
    }
    mesh.insert_indices(Indices::U16(sub.indices.clone()));
    mesh
}

fn has_tangents(sub: &SubMesh) -> bool {
    sub.vertex_format.contains("U3U3")
}

fn has_vertex_color(sub: &SubMesh) -> bool {
    sub.vertex_format.ends_with('C')
}

pub(crate) fn mat4_from_34(t: &[[f32; 4]; 3]) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(t[0][0], t[1][0], t[2][0], 0.0),
        Vec4::new(t[0][1], t[1][1], t[2][1], 0.0),
        Vec4::new(t[0][2], t[1][2], t[2][2], 0.0),
        Vec4::new(t[0][3], t[1][3], t[2][3], 1.0),
    )
}

/// Everything needed to load models, bundled as one system parameter.
#[derive(bevy::ecs::system::SystemParam)]
pub struct LoaderParams<'w> {
    fs: Res<'w, AlamoFs>,
    cache: ResMut<'w, ModelCache>,
    textures: ResMut<'w, TextureCache>,
    images: ResMut<'w, Assets<Image>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<AlamoMaterial>>,
    inverse_bindposes: ResMut<'w, Assets<SkinnedMeshInverseBindposes>>,
    settings: Res<'w, AlamoRenderSettings>,
    defs: Res<'w, XmlDefs>,
}

fn load_model(p: &mut LoaderParams, key: &str) -> Result<Arc<CachedModel>, String> {
    if let Some(m) = p.cache.models.get(&key.to_ascii_lowercase()) {
        return Ok(m.clone());
    }
    let t_start = std::time::Instant::now();
    let preloaded = p.cache.preparsed.contains_key(&key.to_ascii_lowercase());
    // A model parsed by the preload workers skips the file read and the parse.
    let (model, t_read) = if let Some(m) = p.cache.preparsed.remove(&key.to_ascii_lowercase()) {
        (m, t_start.elapsed())
    } else {
        let data =
            p.fs.0
                .read(key)
                .map_err(|e| format!("cannot read {key}: {e}"))?;
        let t_read = t_start.elapsed();
        (
            Model::parse(&data).map_err(|e| format!("cannot parse {key}: {e}"))?,
            t_read,
        )
    };
    let t_parse = t_start.elapsed();
    let mut t_tex = std::time::Duration::ZERO;
    let most = select_lod(&model, LodSelect::MostDetailed);

    let mut meshes = Vec::new();
    for (mi, mesh) in model.meshes.iter().enumerate() {
        if mesh.is_shadow_volume() || is_collision_only(mesh) || mesh.sub_meshes.is_empty() {
            continue;
        }
        let bone = model.bone_of_object(mi);
        let mut parts = Vec::new();
        let mut skinned = false;
        for sub in &mesh.sub_meshes {
            if sub.vertices.is_empty() || sub.indices.is_empty() {
                continue;
            }
            skinned |= sub.is_skinned();
            let LoaderParams {
                fs,
                cache,
                textures,
                images,
                ..
            } = p;
            let (material, colorize) = describe_material(
                &sub.material,
                has_tangents(sub),
                has_vertex_color(sub),
                |name| {
                    let t0 = std::time::Instant::now();
                    let r = textures.get(&fs.0, images, name);
                    t_tex += t0.elapsed();
                    r.or_else(|| {
                        // Missing base textures get the loud placeholder, others are ignored.
                        name.to_ascii_lowercase()
                            .contains("texture")
                            .then(|| None)
                            .flatten()
                    })
                },
                |shader| {
                    if cache.logged_shaders.insert(shader.to_string()) {
                        info!("unknown EaW shader `{shader}`, rendering it as a plain lit mesh");
                    }
                },
            );
            let mut material = material;
            if p.settings.flip_normal_green {
                material.params.flags |= crate::material::FLAG_FLIP_GREEN;
            }
            if p.settings.shadows {
                material.params.flags |= crate::material::FLAG_SHADOWS;
            }
            if material.params.kind == crate::material::ShaderKind::Planet as u32 {
                material.params.disc_radius = flat_disc_radius(sub);
            }
            if mesh.name.to_ascii_lowercase().starts_with("atmosphere") {
                // The additive atmosphere ring is brighter in the original than the plain texture product (Remake video: wide pale-blue haze at the limb).
                material.params.diffuse *= Vec4::new(ATMOSPHERE_RING_GAIN, ATMOSPHERE_RING_GAIN, ATMOSPHERE_RING_GAIN, 1.0);
            }
            if material.base.is_none() && sub.material.texture("BaseTexture").is_some() {
                // Additive shaders (sky, glows) get black, lit surfaces a neutral grey so a hull never turns black.
                let additive = sub.material.shader.to_ascii_lowercase().contains("additive");
                material.base = Some(p.textures.fallback(&mut p.images, additive));
            }
            let handle = p.meshes.add(build_mesh(sub));
            parts.push(CachedPart {
                mesh: handle,
                material,
                colorize,
            });
        }
        if parts.is_empty() {
            continue;
        }
        meshes.push(CachedMesh {
            orig: mi,
            name: mesh.name.clone(),
            bone,
            hidden: mesh.hidden,
            skinned,
            parts,
            most_detailed: most[mi],
        });
    }

    let inverse: Vec<Mat4> = (0..model.bones.len())
        .map(|i| mat4_from_34(&model.world_transform(i)).inverse())
        .collect();
    let inverse_bindposes = p
        .inverse_bindposes
        .add(SkinnedMeshInverseBindposes::from(inverse));
    let cached = Arc::new(CachedModel {
        model,
        meshes,
        inverse_bindposes,
    });
    p.cache
        .models
        .insert(key.to_ascii_lowercase(), cached.clone());
    let total = t_start.elapsed();
    crate::frame_trace::note(|| format!("model {key}{} {total:.1?} (textures {t_tex:.1?})", if preloaded { " (preloaded)" } else { "" }));
    if total.as_millis() >= 20 {
        info!(
            "load_model {key}: {total:.1?} (read {t_read:.1?}, parse {:.1?}, textures {t_tex:.1?}, rest {:.1?})",
            t_parse - t_read,
            total - t_parse - t_tex
        );
    }
    Ok(cached)
}

fn team_key(c: Color) -> [u32; 3] {
    let l = c.to_linear();
    [l.red.to_bits(), l.green.to_bits(), l.blue.to_bits()]
}

/// Gamma-space team color, as the shaders expect it (they work on stored, non-linear values).
fn team_vec(c: Color) -> Vec4 {
    let s = c.to_srgba();
    Vec4::new(s.red, s.green, s.blue, 1.0)
}

fn part_material(
    cache: &mut ModelCache,
    materials: &mut Assets<AlamoMaterial>,
    key: &str,
    mesh_idx: usize,
    sub_idx: usize,
    part: &CachedPart,
    team: Color,
    background: bool,
) -> Handle<AlamoMaterial> {
    let tk = if part.colorize {
        team_key(team)
    } else {
        [0; 3]
    };
    cache
        .materials
        .entry((if background { format!("{}#bg", key.to_ascii_lowercase()) } else { key.to_ascii_lowercase() }, mesh_idx, sub_idx, tk))
        .or_insert_with(|| {
            let mut m = part.material.clone();
            if background {
                m.params.flags |= crate::material::FLAG_BACKGROUND;
            }
            if part.colorize {
                m.params.colorization = team_vec(team);
            }
            materials.add(m)
        })
        .clone()
}

/// Spawns models for newly added [`AlamoModel`] components.
pub fn spawn_models(
    mut commands: Commands,
    mut loader: LoaderParams,
    new: Query<(Entity, &AlamoModel), Without<AlamoModelLoaded>>,
    failed: Query<(), With<AlamoLoadFailed>>,
) {
    let started = std::time::Instant::now();
    let budget = loader.settings.spawn_budget;
    let mut spawned = 0u32;
    let materials_before = loader.cache.materials.len();
    for (root, spec) in &new {
        if failed.contains(root) {
            continue;
        }
        if let Some(b) = budget
            && spawned > 0
            && started.elapsed() >= b
        {
            break;
        }
        spawned += 1;
        let key = model_path(&spec.path);
        let cached = match load_model(&mut loader, &key) {
            Ok(c) => c,
            Err(error) => {
                error!("{error}");
                commands.entity(root).insert(AlamoLoadFailed);
                commands.trigger(ModelLoadFailed {
                    entity: root,
                    error,
                });
                continue;
            }
        };
        let axis_rot = if spec.attached {
            Quat::IDENTITY
        } else if spec.bow_forward {
            Quat::from_rotation_y(std::f32::consts::PI)
                * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        } else {
            Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        };
        let axis = commands
            .spawn((
                Name::new("alamo_axis"),
                Visibility::Inherited,
                Transform::from_rotation(axis_rot),
                ChildOf(root),
            ))
            .id();

        let model = &cached.model;
        let bones: Vec<Entity> = (0..model.bones.len())
            .map(|_| commands.spawn_empty().id())
            .collect();
        for (i, b) in model.bones.iter().enumerate() {
            let parent = b.parent.map_or(axis, |p| bones[p]);
            let mut e = commands.entity(bones[i]);
            e.insert((
                Name::new(b.name.clone()),
                Visibility::Inherited,
                Transform::from_matrix(mat4_from_34(&b.transform)),
                AlamoBone {
                    index: i,
                    rest: Transform::from_matrix(mat4_from_34(&b.transform)),
                },
                ChildOf(parent),
            ));
            if !b.visible {
                e.insert(Visibility::Hidden);
            }
            if !matches!(b.billboard, BillboardMode::Disable) {
                e.insert(AlamoBillboard { mode: b.billboard });
            }
        }
        let mut proxy_entities = Vec::new();
        for p in &model.proxies {
            let id = commands
                .spawn((
                    Name::new(p.name.clone()),
                    AlamoProxy {
                        name: p.name.clone(),
                    },
                    Transform::IDENTITY,
                    ChildOf(bones[p.bone]),
                    if p.hidden {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    },
                ))
                .id();
            proxy_entities.push(id);
        }

        let selection = if spec.lod == LodSelect::MostDetailed {
            None
        } else {
            Some(select_lod(model, spec.lod))
        };
        let mut parts = Vec::new();
        let mut selected_meshes = Vec::new();
        for (ci, mesh) in cached.meshes.iter().enumerate() {
            let selected = match &selection {
                None => mesh.most_detailed,
                Some(s) => s[mesh.orig],
            };
            if !selected {
                continue;
            }
            selected_meshes.push(mesh.orig);
            for (si, part) in mesh.parts.iter().enumerate() {
                let material = part_material(
                    &mut loader.cache,
                    &mut loader.materials,
                    &key,
                    ci,
                    si,
                    part,
                    spec.team_color,
                    spec.background,
                );
                let parent = if mesh.skinned {
                    axis
                } else {
                    bones.get(mesh.bone).copied().unwrap_or(axis)
                };
                let mut e = commands.spawn((
                    Name::new(mesh.name.clone()),
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    AlamoPart {
                        mesh: ci,
                        sub: si,
                        shield: part.material.shader_kind() == ShaderKind::Shield,
                    },
                    ChildOf(parent),
                ));
                if mesh.hidden {
                    e.insert(Visibility::Hidden);
                }
                if mesh.skinned {
                    e.insert(SkinnedMesh {
                        inverse_bindposes: cached.inverse_bindposes.clone(),
                        joints: bones.clone(),
                    });
                }
                parts.push(e.id());
            }
        }
        let (center, radius) = visible_bounds(&cached, &selected_meshes, axis_rot);
        let mut hardpoint_entities = Vec::new();
        if let Some(unit) = spec.unit.as_deref().and_then(|u| loader.defs.unit(u)) {
            let bone_ids: HashMap<String, Entity> = model
                .bones
                .iter()
                .zip(&bones)
                .map(|(b, &e)| (b.name.to_ascii_lowercase(), e))
                .collect();
            for name in &unit.hardpoints {
                let Some(def) = loader.defs.hardpoint(name) else {
                    continue;
                };
                let Some(&bone) = bone_ids.get(&def.bone.to_ascii_lowercase()) else {
                    continue;
                };
                let hp = commands
                    .spawn((
                        Name::new(def.name.clone()),
                        AlamoHardpoint {
                            name: def.name.clone(),
                            destroyed: false,
                            turret: None,
                        },
                        Transform::IDENTITY,
                        Visibility::Inherited,
                        ChildOf(bone),
                    ))
                    .id();
                if let Some(path) = def.model.as_deref() {
                    let turret = commands
                        .spawn((
                            AlamoModel::new(path)
                                .with_team_color(spec.team_color)
                                .with_lod(spec.lod)
                                .attached_to_bone(),
                            Transform::IDENTITY,
                            TurretAim::from_hardpoint(def),
                            ChildOf(hp),
                        ))
                        .id();
                    commands.entity(hp).insert(AlamoHardpoint {
                        name: def.name.clone(),
                        destroyed: false,
                        turret: Some(turret),
                    });
                }
                hardpoint_entities.push(hp);
            }
        }
        commands.entity(root).insert(AlamoModelLoaded {
            axis,
            hardpoints: hardpoint_entities,
            bones: bones.clone(),
            parts,
            proxies: proxy_entities,
            bone_by_name: model
                .bones
                .iter()
                .zip(&bones)
                .map(|(b, &e)| (b.name.to_ascii_lowercase(), e))
                .collect(),
            center,
            radius,
        });
        commands.trigger(ModelLoaded { entity: root });
    }
    if spawned > 0 && started.elapsed().as_millis() >= 15 {
        info!("spawn_models: {spawned} models in {:.1?}", started.elapsed());
    }
    if spawned > 0 {
        let new_materials = loader.cache.materials.len() - materials_before;
        crate::frame_trace::note(|| format!("spawn_models {spawned} models, {new_materials} new materials, {:.1?}", started.elapsed()));
    }
}

/// Bounding sphere of the selected, visible meshes in root space.
fn visible_bounds(cached: &CachedModel, selected: &[usize], axis_rot: Quat) -> (Vec3, f32) {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &mi in selected {
        let mesh = &cached.model.meshes[mi];
        if mesh.hidden {
            continue;
        }
        let skinned = mesh.sub_meshes.iter().any(SubMesh::is_skinned);
        let m = if skinned {
            Mat4::IDENTITY
        } else {
            mat4_from_34(
                &cached
                    .model
                    .world_transform(cached.model.bone_of_object(mi)),
            )
        };
        for c in 0..8 {
            let pick = |bit: usize, a: [f32; 3], b: [f32; 3], k: usize| {
                if c >> bit & 1 == 0 { a[k] } else { b[k] }
            };
            let p = Vec3::new(
                pick(0, mesh.bbox_min, mesh.bbox_max, 0),
                pick(1, mesh.bbox_min, mesh.bbox_max, 1),
                pick(2, mesh.bbox_min, mesh.bbox_max, 2),
            );
            let w = axis_rot * m.transform_point3(p);
            lo = lo.min(w);
            hi = hi.max(w);
        }
    }
    if lo.x > hi.x {
        return (Vec3::ZERO, 1.0);
    }
    ((lo + hi) * 0.5, ((hi - lo) * 0.5).length())
}

/// Marker for models that failed to load, so they are not retried every frame.
#[derive(Component, Debug)]
pub struct AlamoLoadFailed;

/// Applies a changed team color to already spawned models.
pub fn update_team_colors(
    mut cache: ResMut<ModelCache>,
    mut materials: ResMut<Assets<AlamoMaterial>>,
    models: Query<(&AlamoModel, &AlamoModelLoaded), Changed<AlamoModel>>,
    mut parts: Query<(&AlamoPart, &mut MeshMaterial3d<AlamoMaterial>)>,
) {
    for (spec, loaded) in &models {
        let key = model_path(&spec.path);
        let Some(cached) = cache.models.get(&key.to_ascii_lowercase()).cloned() else {
            continue;
        };
        for &e in &loaded.parts {
            let Ok((part, mut handle)) = parts.get_mut(e) else {
                continue;
            };
            let cp = &cached.meshes[part.mesh].parts[part.sub];
            if cp.colorize {
                handle.0 = part_material(
                    &mut cache,
                    &mut materials,
                    &key,
                    part.mesh,
                    part.sub,
                    cp,
                    spec.team_color,
                    spec.background,
                );
            }
        }
    }
}

/// Turns billboard bones towards the camera.
pub fn update_billboards(
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut bones: Query<(&AlamoBillboard, &mut Transform, &ChildOf)>,
    globals: Query<&GlobalTransform>,
) {
    let Some((_, cam)) = cameras.iter().find(|(c, _)| c.is_active) else {
        return;
    };
    let cam_rot = cam.rotation();
    let cam_pos = cam.translation();
    for (bb, mut tf, parent) in &mut bones {
        let Ok(parent_gt) = globals.get(parent.parent()) else {
            continue;
        };
        let (_, parent_rot, parent_pos) = parent_gt.to_scale_rotation_translation();
        let world_rot = match bb.mode {
            BillboardMode::ZAxisView | BillboardMode::ZAxisLight | BillboardMode::ZAxisWind => {
                // Rotate around the bone's local Z axis only.
                let z = (parent_rot * Vec3::Z).normalize_or_zero();
                let bone_pos = parent_pos + parent_rot * tf.translation;
                let to_cam = cam_pos - bone_pos;
                let y = (to_cam - z * to_cam.dot(z)).normalize_or_zero();
                if y == Vec3::ZERO || z == Vec3::ZERO {
                    continue;
                }
                let x = y.cross(z);
                Quat::from_mat3(&Mat3::from_cols(x, y, z))
            }
            // Quad plane parallel to the screen: bone Y axis towards the viewer, Z up.
            _ => cam_rot * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        };
        tf.rotation = parent_rot.inverse() * world_rot;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alamo_model::alo::{Mesh as AloMesh, Vertex};

    fn mesh(name: &str, lod: u32, tris: usize) -> AloMesh {
        AloMesh {
            name: name.into(),
            lod,
            sub_meshes: vec![SubMesh {
                vertices: vec![Vertex::default(); 3],
                indices: [0u16, 1, 2].repeat(tris),
                ..SubMesh::default()
            }],
            ..AloMesh::default()
        }
    }

    fn planet_like() -> Model {
        Model {
            meshes: vec![
                mesh("planet_ALT0", 0, 10),
                mesh("planet_ALT1", 1, 100),
                mesh("grid_ALT2", 2, 5),
                mesh("hull", 0, 7),
            ],
            ..Model::default()
        }
    }

    #[test]
    fn paths_and_groups() {
        assert_eq!(model_path("X.alo"), "Art/Models/X.alo");
        assert_eq!(model_path("Art/Models/X.alo"), "Art/Models/X.alo");
        assert_eq!(group_name("Planet_ALT12"), "planet");
        assert_eq!(group_name("Hull"), "hull");
        assert_eq!(group_name("Foo_ALTx"), "foo_altx");
    }

    #[test]
    fn lod_most_detailed_picks_biggest_and_drops_overlays() {
        let sel = select_lod(&planet_like(), LodSelect::MostDetailed);
        assert_eq!(sel, vec![false, true, false, true]);
    }

    #[test]
    fn lod_levels() {
        let m = planet_like();
        assert_eq!(
            select_lod(&m, LodSelect::Level(0)),
            vec![true, false, false, true]
        );
        assert_eq!(
            select_lod(&m, LodSelect::Level(1)),
            vec![false, true, false, true]
        );
        assert_eq!(
            select_lod(&m, LodSelect::Level(2)),
            vec![false, true, true, true]
        );
    }

    fn material(shader: &str, params: &[(&str, ParamValue)]) -> Material {
        Material {
            shader: shader.into(),
            params: params
                .iter()
                .map(|(n, v)| (n.to_string(), v.clone()))
                .collect(),
        }
    }

    #[test]
    fn shader_mapping() {
        let kind = |s: &str| {
            let mut unknown = Vec::new();
            let (m, colorize) = describe_material(
                &material(s, &[]),
                true,
                false,
                |_| None,
                |u| unknown.push(u.to_string()),
            );
            (m.params.kind, m.params.flags, colorize, unknown.len())
        };
        assert_eq!(kind("MeshBumpColorize.fx").0, ShaderKind::Lit as u32);
        assert!(kind("MeshBumpColorize.fx").2);
        assert!(kind("RSkinBumpColorize.fx").2);
        assert_eq!(kind("MeshAdditive.fxo").0, ShaderKind::Additive as u32);
        assert_eq!(kind("MeshAlpha.fx").0, ShaderKind::Alpha as u32);
        assert_eq!(kind("Planet.fx").0, ShaderKind::Planet as u32);
        assert_eq!(kind("MeshShield.fx").0, ShaderKind::Shield as u32);
        assert_ne!(kind("MeshGloss.fx").1 & FLAG_SPEC_FROM_BASE_ALPHA, 0);
        assert_eq!(kind("Nebula.fx").0, ShaderKind::Additive as u32);
        assert_eq!(kind("Tree.fx").3, 1);
    }

    #[test]
    fn planet_params() {
        let m = material(
            "Planet.fx",
            &[
                ("Atmosphere", ParamValue::Float4([0.1, 0.2, 0.3, 1.0])),
                ("AtmospherePower", ParamValue::Float(4.5)),
                ("CloudScrollRate", ParamValue::Float(-0.005)),
            ],
        );
        let (a, _) = describe_material(&m, true, false, |_| None, |_| {});
        assert_eq!(a.params.atmosphere, Vec4::new(0.1, 0.2, 0.3, 1.0));
        assert_eq!(a.params.atmosphere_power, 4.5);
        assert_eq!(a.params.cloud_scroll_rate, -0.005);
    }

    #[test]
    fn mesh_has_expected_attributes() {
        let sub = SubMesh {
            vertex_format: "alD3dVertRSkinNU2U3U3".into(),
            vertices: vec![Vertex::default(); 3],
            indices: vec![0, 1, 2],
            bone_palette: vec![5, 9],
            ..SubMesh::default()
        };
        let mesh = build_mesh(&sub);
        assert!(mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
        assert_eq!(mesh.count_vertices(), 3);
    }
}
