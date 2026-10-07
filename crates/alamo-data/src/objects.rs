//! Generic game object store: every named object found in the object index files
//! (`GameObjectFiles.XML`, `HardpointDataFiles.XML`) with `Variant_Of_Existing_Type`
//! inheritance resolution.
//!
//! # Duplicates
//! Object names are global and case-insensitive. When several files define the same name the
//! **first definition in index order wins** and a warning is recorded. The engine's exact rule is
//! not documented; the mod's index lists are ordered so that base definitions come first.
//!
//! # Inheritance
//! `<Variant_Of_Existing_Type>Parent</Variant_Of_Existing_Type>` copies the parent's tags; every
//! tag name the child defines *replaces all* occurrences of that tag in the parent (lists such as
//! `<HardPoints>` are replaced, not merged). This is resolved lazily by [`GameObjects::resolve`].

use std::collections::{BTreeMap, HashMap};

use alamo_fs::FileSource;

use crate::galaxy::{index_files, read_xml, xml_path};
use crate::xml::{Element, Warning, parse_bool, split_list};

/// Index files that list object definition files, in load order.
const OBJECT_INDEXES: [&str; 2] = ["Xml/GameObjectFiles.XML", "Xml/HardpointDataFiles.XML"];

/// One named object definition exactly as written in its file (not inheritance-resolved).
#[derive(Debug, Clone)]
pub struct GameObject {
    /// Object name as written.
    pub name: String,
    /// Root tag with original capitalization, e.g. `SpaceUnit`, `UniqueUnit`, `HardPoint`.
    pub kind: String,
    /// `Variant_Of_Existing_Type`, if any.
    pub variant_of: Option<String>,
    /// The element tree with the object's own tags.
    pub element: Element,
    /// File (relative to `Data/Xml`) the object was defined in.
    pub source_file: String,
}

/// The store of all objects, with case-insensitive lookup.
#[derive(Debug, Default)]
pub struct GameObjects {
    objects: Vec<GameObject>,
    index: HashMap<String, usize>,
}

impl GameObjects {
    /// Loads every object file listed by the index files. Missing files and duplicate names are
    /// reported as warnings.
    pub fn load(src: &dyn FileSource, warnings: &mut Vec<Warning>) -> Self {
        let mut store = GameObjects::default();
        let mut seen_files = std::collections::HashSet::new();
        for index in OBJECT_INDEXES {
            for file in index_files(src, index, warnings) {
                if !seen_files.insert(file.to_ascii_lowercase()) {
                    continue;
                }
                let path = xml_path(&file);
                if !src.exists(&path) {
                    warnings.push(Warning::new(
                        &path,
                        0,
                        format!("object file listed in {index} not found"),
                    ));
                    continue;
                }
                let Some(root) = read_xml(src, &path, warnings) else {
                    continue;
                };
                store.add_file(root, &file, &path, warnings);
            }
        }
        store.check_parents(warnings);
        store
    }

    fn add_file(&mut self, root: Element, file: &str, path: &str, warnings: &mut Vec<Warning>) {
        for el in root.children {
            let Some(name) = el
                .attr("name")
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
            else {
                continue;
            };
            let key = name.to_ascii_lowercase();
            if let Some(&prev) = self.index.get(&key) {
                warnings.push(Warning::new(
                    path,
                    el.line,
                    format!(
                        "duplicate object '{name}' ignored (first defined in {})",
                        self.objects[prev].source_file
                    ),
                ));
                continue;
            }
            let variant_of = el
                .child_text("variant_of_existing_type")
                .filter(|v| !v.is_empty())
                .map(str::to_string);
            self.index.insert(key, self.objects.len());
            self.objects.push(GameObject {
                kind: el.raw_name.clone(),
                name,
                variant_of,
                element: el,
                source_file: file.to_string(),
            });
        }
    }

    /// Adds the children of `root` as objects (test helper).
    #[cfg(test)]
    pub(crate) fn add_file_for_tests(&mut self, root: Element) {
        let mut w = Vec::new();
        self.add_file(root, "t.xml", "Xml/t.xml", &mut w);
    }

    fn check_parents(&self, warnings: &mut Vec<Warning>) {
        for o in &self.objects {
            if let Some(parent) = &o.variant_of
                && self.get(parent).is_none()
            {
                warnings.push(Warning::new(
                    &o.source_file,
                    o.element.line,
                    format!("'{}' is a variant of unknown object '{parent}'", o.name),
                ));
            }
        }
    }

    /// Number of objects.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// True if no objects were loaded.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// All objects in load order.
    pub fn iter(&self) -> impl Iterator<Item = &GameObject> {
        self.objects.iter()
    }

    /// Object by case-insensitive name.
    pub fn get(&self, name: &str) -> Option<&GameObject> {
        self.index
            .get(&name.trim().to_ascii_lowercase())
            .map(|&i| &self.objects[i])
    }

    /// All objects whose root tag equals `kind` (case-insensitive).
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a GameObject> {
        self.objects
            .iter()
            .filter(move |o| o.kind.eq_ignore_ascii_case(kind))
    }

    /// Number of objects per kind (original capitalization of the first occurrence).
    pub fn count_by_kind(&self) -> BTreeMap<String, usize> {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for o in &self.objects {
            *m.entry(o.kind.clone()).or_default() += 1;
        }
        m
    }

    /// Resolves `Variant_Of_Existing_Type` inheritance for `name`.
    pub fn resolve(&self, name: &str) -> Option<Resolved<'_>> {
        let leaf = self.get(name)?;
        // Collect the chain leaf -> root, guarding against cycles.
        let mut chain = vec![leaf];
        while let Some(parent) = chain
            .last()
            .and_then(|o| o.variant_of.as_deref())
            .and_then(|p| self.get(p))
        {
            if chain.iter().any(|o| std::ptr::eq(*o, parent)) {
                break;
            }
            chain.push(parent);
        }
        let mut children: Vec<&Element> = Vec::new();
        for obj in chain.iter().rev() {
            let own = &obj.element.children;
            children.retain(|c| !own.iter().any(|o| o.name == c.name));
            children.extend(own.iter().filter(|c| c.name != "variant_of_existing_type"));
        }
        Some(Resolved {
            object: leaf,
            chain,
            children,
        })
    }
}

/// An object with inherited tags merged in. Accessors return the first occurrence of a tag.
#[derive(Debug, Clone)]
pub struct Resolved<'a> {
    /// The object itself (the leaf of the inheritance chain).
    pub object: &'a GameObject,
    /// Inheritance chain, leaf first, root ancestor last.
    pub chain: Vec<&'a GameObject>,
    children: Vec<&'a Element>,
}

impl<'a> Resolved<'a> {
    /// Object name.
    pub fn name(&self) -> &'a str {
        &self.object.name
    }

    /// Root tag of the leaf object, e.g. `SpaceUnit`.
    pub fn kind(&self) -> &'a str {
        &self.object.kind
    }

    /// All effective child elements.
    pub fn elements(&self) -> &[&'a Element] {
        &self.children
    }

    /// First effective element with tag `tag` (lower-case).
    pub fn element(&self, tag: &str) -> Option<&'a Element> {
        self.children.iter().copied().find(|c| c.name == tag)
    }

    /// All effective elements with tag `tag` (lower-case).
    pub fn elements_named<'s>(&'s self, tag: &'s str) -> impl Iterator<Item = &'a Element> + 's {
        self.children.iter().copied().filter(move |c| c.name == tag)
    }

    /// Trimmed text of the first `tag`; `None` if the tag is absent.
    pub fn get(&self, tag: &str) -> Option<&'a str> {
        self.element(tag).map(Element::text_trimmed)
    }

    /// Like [`Resolved::get`] but `None` for empty values too.
    pub fn string(&self, tag: &str) -> Option<String> {
        self.get(tag).filter(|s| !s.is_empty()).map(str::to_string)
    }

    /// Trimmed texts of every `tag` occurrence.
    pub fn get_all(&self, tag: &str) -> Vec<&'a str> {
        self.elements_named(tag)
            .map(Element::text_trimmed)
            .collect()
    }

    /// First `tag` split on commas/whitespace/`|`.
    pub fn list(&self, tag: &str) -> Vec<String> {
        self.get(tag).map(split_list).unwrap_or_default()
    }

    /// First `tag` parsed as a number, `None` when absent or unparsable.
    pub fn num<T: std::str::FromStr>(&self, tag: &str) -> Option<T> {
        self.get(tag).and_then(|s| {
            s.split([',', ' '])
                .find(|t| !t.is_empty())
                .unwrap_or("")
                .parse()
                .ok()
        })
    }

    /// First `tag` parsed as a boolean, `None` when absent.
    pub fn flag(&self, tag: &str) -> Option<bool> {
        self.get(tag).map(parse_bool)
    }

    /// First `tag` as a list of floats (e.g. positions, colors).
    pub fn floats(&self, tag: &str) -> Vec<f32> {
        self.get(tag)
            .map(crate::xml::parse_floats)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::parse_str;

    fn store(xml: &str) -> GameObjects {
        let mut w = Vec::new();
        let root = parse_str(xml, "t.xml", &mut w).unwrap();
        let mut s = GameObjects::default();
        s.add_file(root, "t.xml", "Xml/t.xml", &mut w);
        s
    }

    #[test]
    fn inheritance_overrides_by_tag() {
        let s = store(
            "<R><SpaceUnit Name=\"Base\"><Max_Speed>1</Max_Speed><HardPoints>A, B</HardPoints><Tech_Level>2</Tech_Level></SpaceUnit>\
             <SpaceUnit Name=\"Mid\"><Variant_Of_Existing_Type>base</Variant_Of_Existing_Type><Max_Speed>5</Max_Speed></SpaceUnit>\
             <UniqueUnit Name=\"Leaf\"><Variant_Of_Existing_Type>Mid</Variant_Of_Existing_Type><HardPoints>C</HardPoints></UniqueUnit></R>",
        );
        let r = s.resolve("LEAF").unwrap();
        assert_eq!(r.kind(), "UniqueUnit");
        assert_eq!(r.num::<f32>("max_speed"), Some(5.0));
        assert_eq!(r.list("hardpoints"), ["C"]);
        assert_eq!(r.num::<i32>("tech_level"), Some(2));
        assert_eq!(r.chain.len(), 3);
        assert_eq!(s.count_by_kind().len(), 2);
    }

    #[test]
    fn cycles_do_not_hang() {
        let s = store(
            "<R><A Name=\"a\"><Variant_Of_Existing_Type>b</Variant_Of_Existing_Type></A><A Name=\"b\"><Variant_Of_Existing_Type>a</Variant_Of_Existing_Type></A></R>",
        );
        assert_eq!(s.resolve("a").unwrap().chain.len(), 2);
    }

    #[test]
    fn duplicates_first_wins() {
        let mut w = Vec::new();
        let root = parse_str(
            "<R><A Name=\"x\"><V>1</V></A><A Name=\"X\"><V>2</V></A></R>",
            "t",
            &mut w,
        )
        .unwrap();
        let mut s = GameObjects::default();
        s.add_file(root, "t.xml", "Xml/t.xml", &mut w);
        assert_eq!(s.len(), 1);
        assert_eq!(s.resolve("x").unwrap().get("v"), Some("1"));
        assert_eq!(w.len(), 1);
    }
}
