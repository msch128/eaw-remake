//! Widgets: spawning the command bars and keeping them in sync with [`CommandBarData`].
//!
//! Layout is in *design pixels* of the original 1366x768 interface; [`crate::EawUiSettings`]
//! turns that into real pixels through Bevy's [`UiScale`], so the bars scale with the screen
//! height exactly like in the game and stay anchored to the bottom-left corner.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::ui::{ComputedNode, FocusPolicy, UiGlobalTransform};
use bevy::window::PrimaryWindow;

use crate::EawUiAssets;
use crate::EawUiSettings;
use crate::components::{CommandBarComponent, CommandBarDb, ComponentKind};
use crate::data::{CommandBarData, EawButtonClicked, EawButtonState, ForcedTooltip};
use crate::faction::UiFaction;
use crate::shell::{ShellLayout, rasterize};
use crate::textures::{UiTexture, UiTextures};

/// Which command bar to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CommandBarMode {
    /// Galactic conquest map (`i_galactic_controls_new.alo`).
    #[default]
    Galactic,
    /// Space battle (`i_tactical_controls_new.alo`).
    SpaceTactical,
}

impl CommandBarMode {
    /// Shell model of this bar.
    pub fn model(self) -> &'static str {
        match self {
            Self::Galactic => "I_Galactic_Controls_New.ALO",
            Self::SpaceTactical => "I_Tactical_Controls_New.ALO",
        }
    }
}

/// Root of a spawned command bar.
#[derive(Component, Debug, Clone, Copy)]
pub struct EawCommandBar {
    /// Which bar.
    pub mode: CommandBarMode,
    /// Faction skin.
    pub faction: UiFaction,
}

/// A clickable component. `name` is the lower-cased component name.
#[derive(Component, Debug, Clone)]
pub struct EawButton {
    /// Lower-cased component name.
    pub name: String,
    /// Tooltip text ids (`Tooltip_Text`).
    pub tooltips: Vec<String>,
    /// Keeps its pressed look after a click.
    pub toggle: bool,
    /// Whether the mouse was pressed on it (click fires on release).
    armed: bool,
}

#[derive(Component, Default)]
pub struct ButtonLayers {
    icon: Option<Entity>,
    over: Option<Entity>,
    selected: Option<Entity>,
    disabled: Option<Entity>,
}

/// What a slot kind is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Queue,
    Pad,
    Select,
    Ability,
}

/// A component whose content comes from a list in [`CommandBarData`].
#[derive(Component, Debug, Clone, Copy)]
pub struct Slot {
    kind: SlotKind,
    index: usize,
}

/// Which part of a slot a child entity renders.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotPart {
    Icon,
    Progress,
    Label,
}

/// A discrete bar of the selection (`s_health_NN`, `s_shield_NN`).
#[derive(Component, Debug, Clone, Copy)]
pub struct StatusBar {
    index: usize,
    shield: bool,
    max_level: u32,
}

#[derive(Component)]
pub struct BarLayer {
    fill: bool,
}

#[derive(Component, Debug, Clone)]
pub struct BarTextures {
    frame: Vec<Handle<Image>>,
    fill: Vec<Handle<Image>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Credits,
    Income,
    PlanetName,
    Day,
    TechLevel,
    Pop,
    Allegiance,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct BoundField(Field);

#[derive(Component)]
pub struct Tooltip;

fn rgba(c: [u8; 4]) -> Color {
    Color::srgba_u8(c[0], c[1], c[2], c[3])
}

fn field_for(name: &str) -> Option<Field> {
    Some(match name {
        "text_credits" | "text_credits_tactical" => Field::Credits,
        "text_planetary_income" => Field::Income,
        "text_planet" | "text_planet_tactical" => Field::PlanetName,
        "text_day_counter" => Field::Day,
        "text_tech_level" | "text_tactical_tech" => Field::TechLevel,
        "text_pop_cap" | "text_planetary_pop" => Field::Pop,
        "text_allegiance_a" | "text_allegiance_tactical" => Field::Allegiance,
        _ => return None,
    })
}

/// Builds `<prefix>NN` slot descriptors from component names.
fn slot_for(name: &str) -> Option<Slot> {
    let num = |p: &str| name.strip_prefix(p).and_then(|n| n.parse::<usize>().ok());
    if let Some(i) = num("tqueue").or_else(|| num("queue")) {
        return Some(Slot { kind: SlotKind::Queue, index: i });
    }
    if let Some(i) = num("b_create_") {
        return Some(Slot { kind: SlotKind::Pad, index: i });
    }
    if let Some(i) = num("s_select_") {
        return Some(Slot { kind: SlotKind::Select, index: i });
    }
    if let Some(i) = num("special_button_") {
        return Some(Slot { kind: SlotKind::Ability, index: i });
    }
    None
}

/// Parameters of [`spawn_command_bar`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandBarSpec {
    /// Galactic or tactical.
    pub mode: CommandBarMode,
    /// Faction skin.
    pub faction: UiFaction,
}

struct Builder<'a> {
    world: &'a mut World,
    textures: UiTextures,
    db: CommandBarDb,
    font: Option<String>,
    root: Entity,
    root_h: f32,
    faction: UiFaction,
}

/// Spawns a command bar and returns its root entity. The bar is a bottom-left anchored node;
/// despawn the entity to remove it. Loads textures synchronously (first call decodes the
/// 4096x4096 mega texture, which takes a moment).
pub fn spawn_command_bar(world: &mut World, spec: CommandBarSpec) -> Option<Entity> {
    let (fs, font, oversample) = {
        let a = world.get_resource::<EawUiAssets>()?;
        let s = world.get_resource::<EawUiSettings>()?;
        (a.fs.clone(), s.font_family.clone(), s.faceplate_oversample)
    };
    let layout = ShellLayout::load(&fs, spec.mode.model())?;
    let textures = world.remove_resource::<UiTextures>()?;
    let db = world.remove_resource::<CommandBarDb>()?;
    let (lo, hi) = layout.art_bounds(spec.faction);
    let (width, height) = (hi.x.max(1.0), hi.y.max(1.0));

    let camera = world.resource::<EawUiSettings>().camera;
    let root = world
        .spawn((
            Name::new(format!("EaW command bar ({:?})", spec.mode)),
            EawCommandBar {
                mode: spec.mode,
                faction: spec.faction,
            },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Px(width),
                height: Val::Px(height),
                ..default()
            },
            FocusPolicy::Pass,
        ))
        .id();
    if let Some(c) = camera {
        world.entity_mut(root).insert(UiTargetCamera(c));
    }
    let mut b = Builder {
        world,
        textures,
        db,
        font,
        root,
        root_h: height,
        faction: spec.faction,
    };

    // Frame art (rasterized meshes), back to front.
    let mut arts: Vec<_> = layout.art_for(spec.faction).collect();
    arts.sort_by(|a, c| a.z.total_cmp(&c.z));
    let mut decoded = HashMap::new();
    for art in arts {
        let tex = decoded
            .entry(art.texture.to_lowercase())
            .or_insert_with(|| b.textures.library().load(&art.texture));
        let Some(tex) = tex else { continue };
        let (alo, ahi) = art.bounds();
        let img = rasterize(&art.triangles, tex, alo, ahi, oversample);
        let handle = b.upload_raw(&art.name, img);
        b.image_at(root, &handle, (alo + ahi) * 0.5, ahi - alo, Color::WHITE, Some(&art.name));
        let _ = lo;
    }

    // Components in draw order (z, then base layer).
    let mut items: Vec<_> = layout.order.iter().filter_map(|n| layout.item(n)).collect();
    items.sort_by(|a, c| {
        let la = b.db.get(&a.name).map_or(0, |d| d.base_layer);
        let lc = b.db.get(&c.name).map_or(0, |d| d.base_layer);
        a.z.total_cmp(&c.z).then(la.cmp(&lc))
    });
    for item in items {
        // Components placed outside the bar (e.g. ground filters below it) belong to other panels.
        if item.center.y < 0.0 || item.center.y > height + 40.0 {
            continue;
        }
        let Some(def) = b.db.get(&item.name).cloned() else {
            continue;
        };
        b.spawn_component(&def, item.center, item.size);
    }

    let Builder {
        world, textures, db, ..
    } = b;
    world.insert_resource(textures);
    world.insert_resource(db);
    Some(root)
}

impl Builder<'_> {
    fn tex(&mut self, name: &str) -> Option<UiTexture> {
        let mut images = self.world.get_resource_mut::<Assets<Image>>()?;
        self.textures.get(&mut images, name)
    }

    fn upload_raw(&mut self, key: &str, img: alamo_model::image::Rgba8) -> Handle<Image> {
        let mut images = self.world.resource_mut::<Assets<Image>>();
        self.textures.insert(&mut images, key, img).handle
    }

    /// Spawns an image child of `parent`, centred at `center` (design px, y up, relative to the
    /// bar origin when `parent` is the root).
    fn image_at(
        &mut self,
        parent: Entity,
        image: &Handle<Image>,
        center: Vec2,
        size: Vec2,
        tint: Color,
        name: Option<&str>,
    ) -> Entity {
        let mut e = self.world.spawn((
            ImageNode::new(image.clone()).with_color(tint),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(center.x - size.x * 0.5),
                top: Val::Px(self.root_h - center.y - size.y * 0.5),
                width: Val::Px(size.x),
                height: Val::Px(size.y),
                ..default()
            },
            FocusPolicy::Pass,
            ChildOf(parent),
        ));
        if let Some(n) = name {
            e.insert(Name::new(n.to_string()));
        }
        e.id()
    }

    /// Image centred inside a component rectangle (`rect_size`), offset in design px (y up).
    fn layer(
        &mut self,
        parent: Entity,
        rect_size: Vec2,
        tex: &UiTexture,
        size: Vec2,
        offset: Vec2,
        tint: Color,
    ) -> Entity {
        self.world
            .spawn((
                ImageNode::new(tex.handle.clone()).with_color(tint),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px((rect_size.x - size.x) * 0.5 + offset.x),
                    top: Val::Px((rect_size.y - size.y) * 0.5 - offset.y),
                    width: Val::Px(size.x),
                    height: Val::Px(size.y),
                    ..default()
                },
                FocusPolicy::Pass,
                ChildOf(parent),
            ))
            .id()
    }

    fn text_layer(
        &mut self,
        parent: Entity,
        rect_size: Vec2,
        def: &CommandBarComponent,
        text: &str,
        offset: Vec2,
        color: Color,
    ) -> Entity {
        let justify = if def.right_justified {
            JustifyContent::FlexEnd
        } else if def.left_justified {
            JustifyContent::FlexStart
        } else {
            JustifyContent::Center
        };
        let font = match &self.font {
            Some(f) => TextFont {
                font: FontSource::Family(f.as_str().into()),
                font_size: FontSize::Px(def.font_size * 1.45),
                ..default()
            },
            None => TextFont::from_font_size(def.font_size * 1.45),
        };
        let holder = self
            .world
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(offset.x),
                    top: Val::Px(-offset.y),
                    width: Val::Px(rect_size.x),
                    height: Val::Px(rect_size.y),
                    justify_content: justify,
                    align_items: AlignItems::Center,
                    ..default()
                },
                FocusPolicy::Pass,
                ChildOf(parent),
            ))
            .id();
        self.world
            .spawn((
                Text::new(text),
                font,
                TextColor(color),
                TextLayout::no_wrap().with_justify(if def.right_justified {
                    Justify::Right
                } else if def.left_justified {
                    Justify::Left
                } else {
                    Justify::Center
                }),
                TextShadow {
                    offset: Vec2::splat(1.0),
                    color: Color::srgba(0.0, 0.0, 0.0, 0.85),
                },
                FocusPolicy::Pass,
                ChildOf(holder),
            ))
            .id()
    }

    fn spawn_component(&mut self, def: &CommandBarComponent, center: Vec2, size: Vec2) {
        let lname = def.name.to_lowercase();
        let fi = self.faction.list_index();

        // Discrete bars.
        if def.kind == Some(ComponentKind::Bar) {
            let Some(slot) = lname
                .strip_prefix("s_health_")
                .map(|n| (n, false))
                .or_else(|| lname.strip_prefix("s_shield_").map(|n| (n, true)))
            else {
                return;
            };
            let Ok(index) = slot.0.parse::<usize>() else {
                return;
            };
            let frame = def
                .bar_texture
                .iter()
                .filter_map(|n| self.tex(n).map(|t| t.handle))
                .collect();
            let fill = def
                .bar_overlay
                .iter()
                .filter_map(|n| self.tex(n).map(|t| t.handle))
                .collect();
            let bar = self
                .world
                .spawn((
                    Name::new(def.name.clone()),
                    StatusBar {
                        index,
                        shield: slot.1,
                        max_level: def.max_bar_level.max(1),
                    },
                    BarTextures { frame, fill },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(center.x - 17.0),
                        top: Val::Px(self.root_h - center.y - 1.0),
                        width: Val::Px(34.0),
                        height: Val::Px(2.0),
                        display: Display::None,
                        ..default()
                    },
                    ChildOf(self.root),
                ))
                .id();
            for fill in [false, true] {
                self.world.spawn((
                    BarLayer { fill },
                    ImageNode::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    ChildOf(bar),
                ));
            }
            return;
        }

        // Plain text fields.
        if let Some(field) = field_for(&lname) {
            let holder = self
                .world
                .spawn((
                    Name::new(def.name.clone()),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(center.x - size.x * 0.5),
                        top: Val::Px(self.root_h - center.y - size.y * 0.5),
                        width: Val::Px(size.x),
                        height: Val::Px(size.y),
                        ..default()
                    },
                    FocusPolicy::Pass,
                    ChildOf(self.root),
                ))
                .id();
            if field == Field::Allegiance {
                let list = def.icon_alt.clone();
                let tex = CommandBarComponent::pick(&list, fi).and_then(|n| self.tex(n));
                if let Some(t) = tex {
                    let sz = t.size.as_vec2() * def.scale;
                    let l = self.layer(holder, size, &t, sz, def.icon_offset, Color::WHITE);
                    self.world.entity_mut(l).insert(BoundField(field));
                }
                return;
            }
            let color = def.text_color.map_or(Color::WHITE, rgba);
            let t = self.text_layer(holder, size, def, "", def.text_offset, color);
            self.world.entity_mut(t).insert(BoundField(field));
            if let Some(icon) = def.icon.first().cloned()
                && let Some(tx) = self.tex(&icon)
            {
                let sz = tx.size.as_vec2() * def.scale;
                self.layer(holder, size, &tx, sz, def.icon_offset, Color::WHITE);
            }
            return;
        }

        if !matches!(
            def.kind,
            Some(ComponentKind::Button | ComponentKind::TextButton)
        ) {
            return;
        }
        // Components that are only hit areas (no texture at all) are not drawn.
        let has_art = [
            &def.blank,
            &def.icon,
            &def.icon_alt,
            &def.mouse_over,
            &def.selected,
            &def.disabled,
        ]
        .iter()
        .any(|l| !l.is_empty());
        if !has_art && slot_for(&lname).is_none() {
            return;
        }
        let slot = slot_for(&lname);
        let alt_icons = CommandBarComponent::pick(&def.icon_alt, 0).is_some();

        let button = self
            .world
            .spawn((
                Name::new(def.name.clone()),
                EawButton {
                    name: lname.clone(),
                    tooltips: def.tooltips.clone(),
                    toggle: def.toggle,
                    armed: false,
                },
                EawButtonState {
                    disabled: def.start_disabled,
                    ..default()
                },
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(center.x - size.x * 0.5),
                    top: Val::Px(self.root_h - center.y - size.y * 0.5),
                    width: Val::Px(size.x),
                    height: Val::Px(size.y),
                    ..default()
                },
                ChildOf(self.root),
            ))
            .id();
        if let Some(s) = slot {
            self.world.entity_mut(button).insert(s);
            // Slots stay invisible until data fills them.
            self.world.entity_mut(button).insert(Visibility::Hidden);
            self.world
                .entity_mut(button)
                .get_mut::<Node>()
                .map(|mut n| n.display = Display::None);
        }

        let mut layers = ButtonLayers::default();
        let tint = def.color.map_or(Color::WHITE, rgba);
        let scaled = |t: &UiTexture| t.size.as_vec2() * def.scale;
        if let Some(t) = CommandBarComponent::pick(&def.blank, fi).and_then(|n| self.tex(n)) {
            let sz = scaled(&t);
            self.layer(button, size, &t, sz, Vec2::ZERO, tint);
        }
        let icon_tex = CommandBarComponent::pick(&def.icon, fi)
            .or_else(|| {
                alt_icons
                    .then(|| CommandBarComponent::pick(&def.icon_alt, 0))
                    .flatten()
            })
            .and_then(|n| self.tex(n));
        if let Some(t) = &icon_tex {
            let sz = scaled(t);
            layers.icon = Some(self.layer(button, size, t, sz, def.icon_offset, tint));
        }
        if let Some(t) = CommandBarComponent::pick(&def.disabled, fi).and_then(|n| self.tex(n)) {
            let sz = scaled(&t);
            let e = self.layer(button, size, &t, sz, def.icon_offset, tint);
            self.world.entity_mut(e).get_mut::<Node>().map(|mut n| n.display = Display::None);
            layers.disabled = Some(e);
        }
        if let Some(t) = CommandBarComponent::pick(&def.selected, fi).and_then(|n| self.tex(n)) {
            let sz = scaled(&t);
            let e = self.layer(button, size, &t, sz, def.icon_offset, tint);
            self.world.entity_mut(e).get_mut::<Node>().map(|mut n| n.display = Display::None);
            layers.selected = Some(e);
        }
        if let Some(t) = CommandBarComponent::pick(&def.mouse_over, fi).and_then(|n| self.tex(n)) {
            let sz = scaled(&t);
            let e = self.layer(button, size, &t, sz, Vec2::ZERO, tint);
            self.world.entity_mut(e).get_mut::<Node>().map(|mut n| n.display = Display::None);
            layers.over = Some(e);
        }
        self.world.entity_mut(button).insert(layers);

        if let Some(s) = slot {
            // Slot content: unit icon, build progress, count/price label.
            let icon = self
                .world
                .spawn((
                    SlotPart::Icon,
                    ImageNode::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(size.x * 0.5 - 20.0),
                        top: Val::Px(size.y * 0.5 - 20.0),
                        width: Val::Px(40.0),
                        height: Val::Px(40.0),
                        display: Display::None,
                        ..default()
                    },
                    FocusPolicy::Pass,
                    ChildOf(button),
                ))
                .id();
            let _ = icon;
            self.world.spawn((
                SlotPart::Progress,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(size.x * 0.5 - 20.0),
                    bottom: Val::Px(size.y * 0.5 - 20.0),
                    width: Val::Px(40.0),
                    height: Val::Px(0.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.5, 0.9, 0.45)),
                FocusPolicy::Pass,
                ChildOf(button),
            ));
            let label_off = if def.text_offset == Vec2::ZERO {
                Vec2::new(0.0, -size.y * 0.5 - 6.0)
            } else {
                def.text_offset
            };
            let color = def.text_color.map_or(Color::srgb(0.85, 0.9, 1.0), rgba);
            let l = self.text_layer(button, size, def, "", label_off, color);
            self.world.entity_mut(l).insert(SlotPart::Label);
            let _ = s;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------------------------

/// Fires [`EawButtonClicked`] on release and handles toggle buttons.
pub(crate) fn button_clicks(
    keys: Res<ButtonInput<KeyCode>>,
    mut q: Query<(Entity, &mut EawButton, &Interaction, &mut EawButtonState)>,
    mut out: MessageWriter<EawButtonClicked>,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    for (entity, mut b, i, mut st) in &mut q {
        if st.disabled {
            b.armed = false;
            continue;
        }
        match *i {
            Interaction::Pressed => b.armed = true,
            Interaction::Hovered if b.armed => {
                b.armed = false;
                if b.toggle {
                    st.selected = !st.selected;
                }
                out.write(EawButtonClicked {
                    name: b.name.clone(),
                    entity,
                    shift,
                });
            }
            _ => b.armed = false,
        }
    }
}

/// Swaps the state layers of buttons.
pub(crate) fn button_visuals(
    buttons: Query<
        (&ButtonLayers, &Interaction, &EawButtonState),
        Or<(Changed<Interaction>, Changed<EawButtonState>)>,
    >,
    mut nodes: Query<&mut Node>,
) {
    for (l, i, st) in &buttons {
        let hover = matches!(i, Interaction::Hovered | Interaction::Pressed) || st.force_hover;
        let pressed = matches!(i, Interaction::Pressed) || st.selected;
        let set = |nodes: &mut Query<&mut Node>, e: Option<Entity>, on: bool| {
            if let Some(e) = e
                && let Ok(mut n) = nodes.get_mut(e)
            {
                n.display = if on { Display::Flex } else { Display::None };
            }
        };
        let show_disabled = st.disabled && l.disabled.is_some();
        let show_selected = !st.disabled && pressed && l.selected.is_some();
        set(&mut nodes, l.disabled, show_disabled);
        set(&mut nodes, l.selected, show_selected);
        set(&mut nodes, l.icon, !show_disabled && !show_selected);
        set(&mut nodes, l.over, hover && !st.disabled && !show_selected);
    }
}

/// Writes credits, planet name etc. into bound text fields and the allegiance symbol.
pub(crate) fn bound_fields(
    data: Res<CommandBarData>,
    assets: Res<EawUiAssets>,
    mut texts: Query<(&BoundField, &mut Text)>,
    mut icons: Query<(&BoundField, &mut ImageNode), Without<Text>>,
    mut tex: ResMut<UiTextures>,
    db: Res<CommandBarDb>,
    mut images: ResMut<Assets<Image>>,
) {
    if !data.is_changed() {
        return;
    }
    let label = |key: &str| assets.text.get(key).unwrap_or("").to_string();
    for (f, mut t) in &mut texts {
        t.0 = match f.0 {
            Field::Credits => group_digits(data.credits),
            Field::Income => group_digits(data.planet_income),
            Field::PlanetName => data.planet_name.clone(),
            Field::Day => format!("{} {}", label("TEXT_DAY"), data.day).trim().to_string(),
            Field::TechLevel => data.tech_level.to_string(),
            Field::Pop => format!("{}/{}", data.pop_current, data.pop_cap),
            Field::Allegiance => String::new(),
        };
    }
    if let Some(alt) = db.get("Text_Allegiance_A") {
        for (f, mut img) in &mut icons {
            if f.0 == Field::Allegiance
                && let Some(owner) = data.planet_owner
                && let Some(n) = CommandBarComponent::pick(&alt.icon_alt, owner.list_index())
                && let Some(t) = tex.get(&mut images, n)
            {
                img.image = t.handle;
            }
        }
    }
}

fn group_digits(v: i64) -> String {
    let s = v.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if v < 0 { format!("-{out}") } else { out }
}

/// Fills queue, pad, selection and ability slots from [`CommandBarData`].
#[allow(clippy::type_complexity)]
pub(crate) fn slots(
    data: Res<CommandBarData>,
    mut tex: ResMut<UiTextures>,
    mut images: ResMut<Assets<Image>>,
    mut slots: Query<(Entity, &Slot, &mut Node, &mut EawButtonState, &mut Visibility), Without<SlotPart>>,
    children: Query<&Children>,
    mut parts: Query<
        (&SlotPart, &mut Node, Option<&mut ImageNode>, Option<&mut Text>),
        Without<Slot>,
    >,
) {
    if !data.is_changed() {
        return;
    }
    for (e, slot, mut node, mut st, mut vis) in &mut slots {
        // (icon, progress, label, disabled, selected)
        let content: Option<(String, f32, String, bool, bool)> = match slot.kind {
            SlotKind::Queue => data.queue.get(slot.index).map(|q| {
                (
                    q.icon.clone(),
                    q.progress,
                    if q.count > 1 { q.count.to_string() } else { String::new() },
                    false,
                    false,
                )
            }),
            SlotKind::Pad => data.pads.get(slot.index).map(|p| {
                (
                    p.icon.clone(),
                    if p.building { 1.0 } else { 0.0 },
                    p.price.map(|v| v.to_string()).unwrap_or_default(),
                    p.disabled,
                    false,
                )
            }),
            SlotKind::Select => data.selection.get(slot.index).map(|s| {
                (
                    s.icon.clone(),
                    0.0,
                    if s.count > 1 { s.count.to_string() } else { String::new() },
                    false,
                    false,
                )
            }),
            SlotKind::Ability => data.abilities.get(slot.index).map(|a| {
                (a.icon.clone(), 1.0 - a.ready, String::new(), a.disabled, a.active)
            }),
        };
        let Some((icon, progress, label, disabled, selected)) = content else {
            node.display = Display::None;
            *vis = Visibility::Hidden;
            continue;
        };
        node.display = Display::Flex;
        *vis = Visibility::Inherited;
        st.disabled = disabled;
        st.selected = selected;
        for k in children.iter_descendants(e) {
            let Ok((part, mut n, img, text)) = parts.get_mut(k) else {
                continue;
            };
            match part {
                SlotPart::Icon => {
                    if let Some(mut img) = img {
                        match tex.get(&mut images, &icon) {
                            Some(t) => {
                                img.image = t.handle;
                                img.color = if disabled {
                                    Color::srgba(0.45, 0.45, 0.45, 1.0)
                                } else {
                                    Color::WHITE
                                };
                                n.display = Display::Flex;
                            }
                            None => n.display = Display::None,
                        }
                    }
                }
                SlotPart::Progress => n.height = Val::Px(40.0 * progress.clamp(0.0, 1.0)),
                SlotPart::Label => {
                    if let Some(mut t) = text {
                        t.0 = label.clone();
                    }
                }
            }
        }
    }
}

/// Picks the level texture of status bars from the selection.
pub(crate) fn status_bars(
    data: Res<CommandBarData>,
    mut bars: Query<(Entity, &StatusBar, &BarTextures, &mut Node), Without<BarLayer>>,
    children: Query<&Children>,
    mut layers: Query<(&BarLayer, &mut ImageNode)>,
) {
    if !data.is_changed() {
        return;
    }
    for (e, bar, tex, mut node) in &mut bars {
        let frac = data.selection.get(bar.index).and_then(|s| {
            if bar.shield { s.shield } else { Some(s.health) }
        });
        let Some(frac) = frac else {
            node.display = Display::None;
            continue;
        };
        node.display = Display::Flex;
        let level = (frac.clamp(0.0, 1.0) * bar.max_level as f32).round() as usize;
        let Ok(kids) = children.get(e) else { continue };
        for &k in kids {
            if let Ok((layer, mut img)) = layers.get_mut(k) {
                let list = if layer.fill { &tex.fill } else { &tex.frame };
                if let Some(h) = list.get(level).or_else(|| list.last()) {
                    img.image = h.clone();
                }
            }
        }
    }
}

/// Spawns the global tooltip panel.
pub(crate) fn spawn_tooltip(mut commands: Commands, settings: Res<EawUiSettings>) {
    let font = match &settings.font_family {
        Some(f) => TextFont {
            font: FontSource::Family(f.as_str().into()),
            font_size: FontSize::Px(10.5),
            ..default()
        },
        None => TextFont::from_font_size(10.5),
    };
    let mut tip = commands.spawn((
            Tooltip,
            Name::new("EaW tooltip"),
            Node {
                position_type: PositionType::Absolute,
                padding: UiRect::axes(Val::Px(7.0), Val::Px(4.0)),
                border: UiRect::all(Val::Px(1.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.05, 0.12, 0.92)),
            BorderColor::all(Color::srgba(0.35, 0.55, 0.85, 0.9)),
            GlobalZIndex(1000),
            FocusPolicy::Pass,
        ));
    if let Some(c) = settings.camera {
        tip.insert(UiTargetCamera(c));
    }
    tip.with_children(|p| {
        p.spawn((
            Text::new(""),
            font,
            TextColor(Color::srgb(0.9, 0.95, 1.0)),
            FocusPolicy::Pass,
        ));
    });
}

/// Shows the tooltip of the hovered (or forced) button next to it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tooltip(
    assets: Res<EawUiAssets>,
    forced: Res<ForcedTooltip>,
    buttons: Query<(&EawButton, &Interaction, &ComputedNode, &UiGlobalTransform)>,
    mut tip: Query<(&mut Node, &Children), With<Tooltip>>,
    mut texts: Query<&mut Text>,
    window: Query<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
) {
    let Ok((mut node, kids)) = tip.single_mut() else {
        return;
    };
    let mut hit = None;
    for (b, i, cn, gt) in &buttons {
        let forced_hit = forced.0.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&b.name));
        if (*i != Interaction::None || forced_hit) && !b.tooltips.is_empty() {
            hit = Some((b, cn, gt, forced_hit));
            if !forced_hit {
                break;
            }
        }
    }
    let Some((b, cn, gt, forced_hit)) = hit else {
        node.display = Display::None;
        return;
    };
    let text = b
        .tooltips
        .iter()
        .find_map(|k| assets.text.get(k).filter(|t| !t.is_empty()))
        .unwrap_or("");
    if text.is_empty() {
        node.display = Display::None;
        return;
    }
    if let Some(&k) = kids.first()
        && let Ok(mut t) = texts.get_mut(k)
    {
        t.0 = text.to_string();
    }
    node.display = Display::Flex;
    let inv = cn.inverse_scale_factor();
    let anchor = if forced_hit {
        // Above the button centre (physical px -> logical).
        let c = gt.translation * inv;
        Vec2::new(c.x - 10.0, c.y - cn.size().y * inv * 0.5 - 30.0 * scale.0)
    } else {
        window
            .single()
            .ok()
            .and_then(Window::cursor_position)
            .map_or(Vec2::ZERO, |p| p + Vec2::new(14.0, 18.0))
    };
    // Bevy positions logical px; UiScale is applied to Val::Px so undo it.
    node.left = Val::Px(anchor.x / scale.0);
    node.top = Val::Px(anchor.y / scale.0);
}

/// Keeps [`UiScale`] in sync with the target height (`Auto` mode).
pub(crate) fn ui_scale(
    settings: Res<EawUiSettings>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut scale: ResMut<UiScale>,
) {
    let v = match settings.scale {
        crate::UiScaleMode::Fixed(v) => v,
        crate::UiScaleMode::Auto => match window.single() {
            Ok(w) => w.height() / crate::shell::REFERENCE_HEIGHT,
            Err(_) => return,
        },
    };
    if (scale.0 - v).abs() > 1e-4 {
        scale.0 = v;
    }
}




