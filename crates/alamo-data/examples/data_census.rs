//! Census of every file listed by the index files the loader reads.
//!
//! Usage: `cargo run -p alamo-data --example data_census`
//!
//! For each listed file: found or missing, whether `xml::parse` returned a root,
//! that parser's warnings, and how many named objects the loader kept versus how
//! many top-level object elements the file contains. The expected count is a
//! separate walk, so a parser that drops a root still shows the loss.
//!
//! Duplicate names follow the loader: the first definition in index order wins.
//! That rule is the loader's, not a measured Alamo engine rule.

use std::collections::HashMap;

use alamo_data::xml;
use alamo_data::load_galaxy;
use alamo_fs::{FileSource, GameFs, GameInstall};

const INDEXES: &[(&str, Kind)] = &[
    ("Xml/GameObjectFiles.XML", Kind::Object),
    ("Xml/HardpointDataFiles.XML", Kind::Object),
    ("Xml/CampaignFiles.xml", Kind::Campaign),
    ("Xml/TradeRouteFiles.xml", Kind::TradeRoute),
    ("Xml/FACTIONFILES.XML", Kind::Faction),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Object,
    Campaign,
    TradeRoute,
    Faction,
}

impl Kind {
    fn accepts(self, tag: &str) -> bool {
        match self {
            Kind::Object => true,
            Kind::Campaign => tag.eq_ignore_ascii_case("campaign"),
            Kind::TradeRoute => tag.eq_ignore_ascii_case("traderoute"),
            Kind::Faction => tag.eq_ignore_ascii_case("faction"),
        }
    }
}

struct Obj {
    tag: String,
    name: String,
}

struct Shape {
    /// Root tag of the first element, if the file has one.
    root: Option<String>,
    /// `Name` on that root, when the root itself is the only object.
    root_name: Option<String>,
    /// Extra roots after the first (`xml::parse` keeps only the first).
    extra_roots: usize,
    /// Top-level object elements: named children of the first root, or the root when it is the object.
    objects: Vec<Obj>,
    /// Named elements of an object kind nested past the first level (a swallowed sibling, or a real nested object).
    deep_objects: Vec<Obj>,
    /// `<` that starts neither a tag, a comment, nor a declaration. Snippet plus 1-based line.
    bad_starts: Vec<(u32, String)>,
    unclosed: usize,
}

fn main() {
    let install = GameInstall::detect().expect("game install not found");
    let fs = GameFs::for_mod(&install, Some("1770851727")).expect("cannot open game files");
    let (data, load_warnings) = load_galaxy(&fs);
    eprintln!(
        "load_galaxy: {} objects, {} campaigns, {} trade routes, {} factions, {} warnings",
        data.objects.len(),
        data.campaigns.len(),
        data.trade_routes.len(),
        data.factions.len(),
        load_warnings.len()
    );
    for w in &load_warnings {
        eprintln!("WARN\t{}", w);
    }

    // Name key -> file the loader kept (first definition that actually parsed).
    let mut winner: HashMap<String, String> = HashMap::new();
    for o in data.objects.iter() {
        winner.entry(o.name.to_ascii_lowercase()).or_insert_with(|| norm(&o.source_file));
    }
    for c in &data.campaigns {
        winner.entry(format!("campaign:{}", c.name.to_ascii_lowercase())).or_insert_with(|| norm(&c.source_file));
    }
    for r in &data.trade_routes {
        winner.entry(format!("route:{}", r.name.to_ascii_lowercase())).or_insert_with(|| norm(&r.source_file));
    }
    for f in &data.factions {
        winner.entry(format!("faction:{}", f.name.to_ascii_lowercase())).or_insert_with(|| norm(&f.source_file));
    }

    println!("index\tfile\tfound\tparsed\treason\texpected\tloaded\tlost\tdup\tencoding\troot\twalk_root");
    let mut seen_files = std::collections::HashSet::new();
    let mut defined: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut credited: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut files = 0usize;
    let mut missing = 0usize;
    let mut parse_fail = 0usize;
    let mut loss_files = 0usize;
    let mut lost_objects = 0usize;
    let mut dup_names = 0usize;
    let mut bad_tag_files = 0usize;

    for &(index, kind) in INDEXES {
        for file in index_files(&fs, index) {
            files += 1;
            let key = norm(&file);
            let path = xml_path(&file);
            if !seen_files.insert(key.clone()) {
                println!(
                    "{}\t{}\tduplicate-listing\t\talready listed earlier\t\t\t\t\t\t",
                    index, file
                );
                continue;
            }
            if !fs.exists(&path) {
                missing += 1;
                println!("{index}\t{file}\tmissing\t\tnot found\t\t\t\t\t\t");
                continue;
            }
            let bytes = fs.read(&path).expect("exists but read failed");
            let encoding = encoding_of(&bytes);
            let mut warnings = Vec::new();
            let parsed = xml::parse(&bytes, &path, &mut warnings);
            let text = xml::sanitize(&xml::decode_bytes(&bytes));
            let shape = walk(&text, kind);
            let parsed_ok = parsed.is_some();
            if !parsed_ok {
                parse_fail += 1;
            }
            let parsed_root = parsed.as_ref().map(|e| e.raw_name.clone());
            let mut reason = String::new();
            if !parsed_ok {
                reason = warnings
                    .iter()
                    .map(|w| w.message.clone())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("; ");
                if reason.is_empty() {
                    reason = "no XML element found".into();
                }
            } else if parsed_root.as_deref() != shape.root.as_deref() {
                reason = format!(
                    "parser root <{}> is not the file root <{}>",
                    parsed_root.as_deref().unwrap_or(""),
                    shape.root.as_deref().unwrap_or("")
                );
                if let Some(w) = warnings.iter().find(|w| w.message.contains("multiple top-level")) {
                    reason.push_str("; ");
                    reason.push_str(&w.message);
                }
            } else if shape.extra_roots > 0 {
                reason = format!("{} extra top-level elements, parser keeps the first", shape.extra_roots);
            }
            if !shape.bad_starts.is_empty() {
                bad_tag_files += 1;
                let (line, snip) = &shape.bad_starts[0];
                let extra = format!("bad tag start line {line}: {snip}");
                if reason.is_empty() {
                    reason = extra;
                } else {
                    reason = format!("{reason}; {extra}");
                }
            }

            let mut expected = 0usize;
            let mut loaded = 0usize;
            let mut lost = 0usize;
            let mut dups = 0usize;
            let mut lost_names: Vec<String> = Vec::new();
            for obj in &shape.objects {
                expected += 1;
                let nk = name_key(kind, &obj.name);
                let store_file = winner.get(&nk);
                let here = store_file == Some(&key);
                if here && credited.insert((key.clone(), nk.clone())) {
                    loaded += 1;
                } else if here || (defined.contains(&nk) && store_file.is_some()) {
                    dups += 1;
                    dup_names += 1;
                    eprintln!(
                        "DUP\t{nk}\twinner={}\tlater={file}\ttag={}",
                        store_file.map(String::as_str).unwrap_or(""),
                        obj.tag
                    );
                } else {
                    lost += 1;
                    lost_objects += 1;
                    lost_names.push(format!("{}:{}", obj.tag, obj.name));
                }
                defined.insert(nk);
            }
            if lost > 0 {
                loss_files += 1;
                eprintln!(
                    "LOSS\t{file}\texpected={expected}\tloaded={loaded}\tlost={lost}\troot={:?}\tparser_root={:?}\tnames={}",
                    shape.root,
                    parsed_root,
                    lost_names.join(",")
                );
            }
            for d in &shape.deep_objects {
                eprintln!("DEEP\t{file}\t{}:{}", d.tag, d.name);
            }
            for (line, snip) in &shape.bad_starts {
                eprintln!("BADTAG\t{file}:{line}\t{snip}");
            }
            if shape.unclosed > 0 {
                eprintln!("UNCLOSED\t{file}\t{}", shape.unclosed);
            }
            println!(
                "{index}\t{file}\tfound\t{}\t{reason}\t{expected}\t{loaded}\t{lost}\t{dups}\t{encoding}\t{}\t{}",
                if parsed_ok { "parsed" } else { "failed" },
                parsed_root.as_deref().unwrap_or(""),
                shape.root.as_deref().unwrap_or("")
            );
        }
    }

    eprintln!(
        "SUMMARY files={files} missing={missing} parse_fail={parse_fail} loss_files={loss_files} lost_objects={lost_objects} duplicate_names={dup_names} bad_tag_files={bad_tag_files} load_warnings={}",
        load_warnings.len()
    );
}

fn name_key(kind: Kind, name: &str) -> String {
    match kind {
        Kind::Object => name.to_ascii_lowercase(),
        Kind::Campaign => format!("campaign:{}", name.to_ascii_lowercase()),
        Kind::TradeRoute => format!("route:{}", name.to_ascii_lowercase()),
        Kind::Faction => format!("faction:{}", name.to_ascii_lowercase()),
    }
}

fn norm(s: &str) -> String {
    s.trim().replace('\\', "/").to_ascii_lowercase()
}

fn xml_path(file: &str) -> String {
    format!("Xml/{}", file.trim().replace('\\', "/"))
}

fn index_files(src: &dyn FileSource, path: &str) -> Vec<String> {
    let Ok(bytes) = src.read(path) else {
        eprintln!("INDEX MISSING {path}");
        return Vec::new();
    };
    let mut w = Vec::new();
    xml::parse(&bytes, path, &mut w)
        .map(|root| {
            root.children_named("file")
                .map(|f| f.text_trimmed().to_string())
                .filter(|f| !f.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn encoding_of(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        "utf-16le"
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        "utf-16be"
    } else if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        if std::str::from_utf8(&bytes[3..]).is_ok() {
            "utf-8-bom"
        } else {
            "utf-8-bom-invalid"
        }
    } else if std::str::from_utf8(bytes).is_ok() {
        "utf-8"
    } else {
        "latin-1"
    }
}

fn is_tag_start(c: char) -> bool {
    c.is_alphabetic() || c.is_ascii_digit() || c == '_' || c == ':'
}

fn line_at(s: &str, offset: usize) -> u32 {
    s.as_bytes()[..offset.min(s.len())].iter().filter(|&&b| b == b'\n').count() as u32 + 1
}

/// Top-level object elements, accepting tag names that start with a digit.
fn walk(s: &str, kind: Kind) -> Shape {
    let n = s.len();
    let mut i = 0;
    let mut stack: Vec<String> = Vec::new();
    let mut shape = Shape {
        root: None,
        root_name: None,
        extra_roots: 0,
        objects: Vec::new(),
        deep_objects: Vec::new(),
        bad_starts: Vec::new(),
        unclosed: 0,
    };
    // Per root: (tag, own Name, named children).
    let mut roots: Vec<(String, Option<String>, Vec<Obj>)> = Vec::new();

    while i < n {
        let rest = &s[i..];
        if !rest.starts_with('<') {
            i += rest.find('<').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("<!--") {
            i += rest.find("-->").map_or(rest.len(), |e| e + 3);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i += rest.find("]]>").map_or(rest.len(), |e| e + 3);
            continue;
        }
        if rest.starts_with("<?") {
            i += rest.find("?>").map_or(rest.len(), |e| e + 2);
            continue;
        }
        if rest.starts_with("<!") {
            i += rest.find('>').map_or(rest.len(), |e| e + 1);
            continue;
        }
        if rest.starts_with("</") {
            let close = &rest[2..];
            let end = close.find('>');
            let name = close[..end.unwrap_or(close.len())].trim().to_ascii_lowercase();
            i += 2 + end.map_or(close.len(), |e| e + 1);
            if let Some(pos) = stack.iter().rposition(|e| e == &name) {
                stack.truncate(pos);
            }
            continue;
        }
        let Some(next) = rest[1..].chars().next() else {
            break;
        };
        if !is_tag_start(next) {
            let snip: String = rest.chars().take(24).collect();
            shape.bad_starts.push((line_at(s, i), snip));
            i += 1;
            continue;
        }
        let (adv, tag, attrs, self_closing) = scan_tag(rest);
        i += adv;
        let name_attr = attrs
            .iter()
            .find(|(k, _)| k == "name")
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let depth = stack.len();
        if depth == 0 {
            roots.push((tag.clone(), name_attr.clone(), Vec::new()));
        } else if depth == 1 {
            if kind.accepts(&tag)
                && let Some(name) = name_attr.clone()
            {
                if let Some(root) = roots.last_mut() {
                    root.2.push(Obj { tag: tag.clone(), name });
                }
            }
        } else if depth > 1
            && is_object_kind(&tag)
            && let Some(name) = name_attr.clone()
        {
            shape.deep_objects.push(Obj { tag: tag.clone(), name });
        }
        if !self_closing {
            stack.push(tag.to_ascii_lowercase());
        }
    }
    // Named children are the objects. A root with a Name and no named children is itself the object.
    for (i, (tag, name, children)) in roots.iter().enumerate() {
        if i == 0 {
            shape.root = Some(tag.clone());
            shape.root_name = name.clone();
        }
        if !children.is_empty() {
            shape.objects.extend(children.iter().map(|o| Obj { tag: o.tag.clone(), name: o.name.clone() }));
        } else if let Some(name) = name.clone()
            && kind.accepts(tag)
        {
            shape.objects.push(Obj { tag: tag.clone(), name });
        }
    }
    shape.extra_roots = roots.len().saturating_sub(1);
    shape.unclosed = stack.len();
    shape
}

fn is_object_kind(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "spaceunit"
            | "uniqueunit"
            | "squadron"
            | "transportunit"
            | "hardpoint"
            | "projectile"
            | "planet"
            | "campaign"
            | "traderoute"
            | "faction"
            | "container"
            | "specialstructure"
            | "spacestructure"
            | "techbuilding"
            | "secondarystructure"
            | "groundbase"
            | "groundcompany"
            | "herocompany"
            | "starbase"
            | "starship"
            | "fighter"
            | "bomber"
    )
}

/// Start tag at `rest` (begins with `<`). Returns bytes consumed, raw tag, lower-case attrs, self-closing.
fn scan_tag(rest: &str) -> (usize, String, Vec<(String, String)>, bool) {
    let n = rest.len();
    let bytes = rest.as_bytes();
    let name_end = rest[1..]
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>' || c == '<')
        .map_or(n, |e| e + 1);
    let tag = rest[1..name_end].to_string();
    let mut p = name_end;
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while p < n && (bytes[p] as char).is_whitespace() {
            p += 1;
        }
        if p >= n {
            break;
        }
        match bytes[p] {
            b'>' => {
                p += 1;
                break;
            }
            b'/' => {
                self_closing = true;
                p += 1;
                if p < n && bytes[p] == b'>' {
                    p += 1;
                    break;
                }
            }
            b'<' => break,
            _ => {
                let key_end = rest[p..]
                    .find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/' | '<'))
                    .map_or(n, |e| p + e);
                let key = rest[p..key_end].to_ascii_lowercase();
                p = key_end;
                while p < n && (bytes[p] as char).is_whitespace() {
                    p += 1;
                }
                let mut value = String::new();
                if p < n && bytes[p] == b'=' {
                    p += 1;
                    while p < n && (bytes[p] as char).is_whitespace() {
                        p += 1;
                    }
                    if p < n && (bytes[p] == b'"' || bytes[p] == b'\'') {
                        let q = bytes[p] as char;
                        let start = p + 1;
                        let end = rest[start..].find(q).map_or(n, |e| start + e);
                        value = xml::decode_entities(&rest[start..end]);
                        p = (end + 1).min(n);
                    } else {
                        let end = rest[p..]
                            .find(|c: char| c.is_whitespace() || c == '>')
                            .map_or(n, |e| p + e);
                        value = xml::decode_entities(&rest[p..end]);
                        p = end;
                    }
                }
                if key.is_empty() {
                    p += 1;
                } else if !attrs.iter().any(|(k, _)| k == &key) {
                    attrs.push((key, value));
                }
            }
        }
    }
    (p, tag, attrs, self_closing)
}

