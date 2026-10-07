//! Lenient XML reader for Alamo engine data files.
//!
//! The original engine's XML parser is very forgiving and mod authors rely on that: comments
//! containing `--`, stray `&`, duplicate attributes, mismatched tag case, unclosed or stray end
//! tags, BOMs in various encodings and trailing garbage are all common. This module implements a
//! small tolerant parser that never fails; problems are reported as [`Warning`]s instead.
//!
//! Tag and attribute names are lower-cased while parsing, so all lookups are case-insensitive
//! as long as callers pass lower-case names. Attribute *values* and text keep their case.

use std::fmt;

/// A non-fatal problem found while loading data (malformed XML, missing file, bad value ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// File the problem was found in (path as given to the file source), may be empty.
    pub file: String,
    /// 1-based line number, 0 if unknown.
    pub line: u32,
    /// Human readable description.
    pub message: String,
}

impl Warning {
    pub(crate) fn new(file: &str, line: u32, message: impl Into<String>) -> Self {
        Self { file: file.to_string(), line, message: message.into() }
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.file.is_empty(), self.line) {
            (true, _) => write!(f, "{}", self.message),
            (false, 0) => write!(f, "{}: {}", self.file, self.message),
            (false, l) => write!(f, "{}:{}: {}", self.file, l, self.message),
        }
    }
}

/// An XML element with lower-cased name, its attributes, child elements and direct text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Element {
    /// Lower-cased tag name.
    pub name: String,
    /// Attributes as `(lower-cased name, value)`. Duplicates are dropped (first wins).
    pub attrs: Vec<(String, String)>,
    /// Child elements in document order.
    pub children: Vec<Element>,
    /// All direct text of this element, concatenated (not trimmed).
    pub text: String,
    /// 1-based line of the start tag.
    pub line: u32,
}

impl Element {
    /// Returns the attribute value for the (lower-case) attribute `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// First child element named `name` (lower-case).
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    /// All child elements named `name` (lower-case).
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Trimmed text of this element.
    pub fn text_trimmed(&self) -> &str {
        self.text.trim()
    }

    /// Trimmed text of the first child named `name`.
    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(Element::text_trimmed)
    }
}

/// Decodes raw file bytes to text: handles UTF-8/UTF-16 BOMs, falls back to Latin-1 for
/// bytes that are not valid UTF-8.
pub fn decode_bytes(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn decode_utf16(bytes: &[u8], conv: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&c| conv(c)).collect();
    String::from_utf16_lossy(&units)
}

/// Removes characters that no XML consumer accepts: NULs, control characters other than
/// tab/newline/carriage return, and (embedded) byte order marks.
pub fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|&c| c == '\t' || c == '\n' || c == '\r' || (c >= ' ' && c != '\u{FEFF}' && c != '\u{FFFE}' && c != '\u{FFFF}'))
        .collect()
}

/// Decodes the five predefined entities and numeric character references. Anything that is
/// not a valid entity (e.g. a stray `&`) is kept literally.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let decoded = rest[1..].find(';').filter(|&e| e <= 10).and_then(|e| {
            let ent = &rest[1..1 + e];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => ent
                    .strip_prefix("#x")
                    .or_else(|| ent.strip_prefix("#X"))
                    .map(|h| u32::from_str_radix(h, 16))
                    .or_else(|| ent.strip_prefix('#').map(|d| d.parse::<u32>()))
                    .and_then(Result::ok)
                    .and_then(char::from_u32),
            };
            ch.map(|c| (c, e + 2))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Maps byte offsets to 1-based line numbers.
struct Lines(Vec<usize>);

impl Lines {
    fn new(s: &str) -> Self {
        Self(s.bytes().enumerate().filter(|&(_, b)| b == b'\n').map(|(i, _)| i).collect())
    }

    fn line(&self, offset: usize) -> u32 {
        (self.0.partition_point(|&nl| nl < offset) + 1) as u32
    }
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':'
}

/// Parses `bytes` leniently and returns the root element, or `None` if the document contains
/// no element at all. Problems are appended to `warnings` (tagged with `file`).
pub fn parse(bytes: &[u8], file: &str, warnings: &mut Vec<Warning>) -> Option<Element> {
    let text = sanitize(&decode_bytes(bytes));
    parse_str(&text, file, warnings)
}

/// Like [`parse`] but for already decoded text.
pub fn parse_str(s: &str, file: &str, warnings: &mut Vec<Warning>) -> Option<Element> {
    let lines = Lines::new(s);
    let n = s.len();
    let mut i = 0;
    // stack[0] is a virtual document element holding the top-level elements.
    let mut stack: Vec<Element> = vec![Element { name: "#document".into(), ..Element::default() }];

    while i < n {
        let rest = &s[i..];
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            let top = stack.last_mut().expect("document element");
            top.text.push_str(&decode_entities(&rest[..end]));
            i += end;
        } else if let Some(body) = rest.strip_prefix("<!--") {
            match body.find("-->") {
                Some(e) => i += 4 + e + 3,
                None => {
                    warnings.push(Warning::new(file, lines.line(i), "unterminated comment, ignoring rest of file"));
                    i = n;
                }
            }
        } else if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            let (body, adv) = match cdata.find("]]>") {
                Some(e) => (&cdata[..e], 9 + e + 3),
                None => (cdata, rest.len()),
            };
            stack.last_mut().expect("document element").text.push_str(body);
            i += adv;
        } else if rest.starts_with("<?") {
            i += rest.find("?>").map_or(rest.len(), |e| e + 2);
        } else if rest.starts_with("<!") {
            i += rest.find('>').map_or(rest.len(), |e| e + 1);
        } else if let Some(close) = rest.strip_prefix("</") {
            let end = close.find('>');
            let name = close[..end.unwrap_or(close.len())].trim().to_ascii_lowercase();
            i += 2 + end.map_or(close.len(), |e| e + 1);
            close_element(&mut stack, &name, file, lines.line(i), warnings);
        } else if rest[1..].chars().next().is_some_and(is_name_start) {
            i += parse_open_tag(rest, i, &lines, file, warnings, &mut stack);
        } else {
            // A lone '<' that does not start a tag: treat as text.
            stack.last_mut().expect("document element").text.push('<');
            i += 1;
        }
    }

    if stack.len() > 1 {
        warnings.push(Warning::new(
            file,
            stack.last().map_or(0, |e| e.line),
            format!("unclosed <{}> at end of file", stack.last().map_or("", |e| e.name.as_str())),
        ));
    }
    while stack.len() > 1 {
        let done = stack.pop().expect("non-empty");
        stack.last_mut().expect("document element").children.push(done);
    }
    let mut doc = stack.pop().expect("document element");
    if doc.children.len() > 1 {
        warnings.push(Warning::new(file, doc.children[1].line, "multiple top-level elements, using the first"));
    }
    if doc.children.is_empty() { None } else { Some(doc.children.swap_remove(0)) }
}

/// Handles `</name>`: closes the nearest matching open element, implicitly closing any
/// elements opened inside it; ignores end tags that match nothing.
fn close_element(stack: &mut Vec<Element>, name: &str, file: &str, line: u32, warnings: &mut Vec<Warning>) {
    let Some(pos) = stack.iter().rposition(|e| e.name == name).filter(|&p| p > 0) else {
        warnings.push(Warning::new(file, line, format!("stray end tag </{name}>")));
        return;
    };
    while stack.len() > pos + 1 {
        let done = stack.pop().expect("non-empty");
        warnings.push(Warning::new(file, done.line, format!("<{}> not closed before </{name}>", done.name)));
        stack.last_mut().expect("document element").children.push(done);
    }
    let done = stack.pop().expect("non-empty");
    stack.last_mut().expect("document element").children.push(done);
}

/// Parses a start tag at the beginning of `rest` (which starts with `<`), pushes the element
/// and returns the number of bytes consumed.
fn parse_open_tag(rest: &str, offset: usize, lines: &Lines, file: &str, warnings: &mut Vec<Warning>, stack: &mut Vec<Element>) -> usize {
    let line = lines.line(offset);
    let bytes = rest.as_bytes();
    let n = rest.len();
    let mut p = 1;
    let name_end = rest[1..]
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>' || c == '<')
        .map_or(n, |e| e + 1);
    let mut el = Element { name: rest[1..name_end].to_ascii_lowercase(), line, ..Element::default() };
    p = p.max(name_end);
    let mut self_closing = false;

    loop {
        while p < n && (bytes[p] as char).is_whitespace() {
            p += 1;
        }
        if p >= n {
            warnings.push(Warning::new(file, line, format!("unterminated start tag <{}>", el.name)));
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
            b'<' => {
                warnings.push(Warning::new(file, line, format!("start tag <{}> not terminated before next tag", el.name)));
                break;
            }
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
                        value = decode_entities(&rest[start..end]);
                        p = (end + 1).min(n);
                    } else {
                        let end = rest[p..].find(|c: char| c.is_whitespace() || c == '>').map_or(n, |e| p + e);
                        value = decode_entities(&rest[p..end]);
                        p = end;
                    }
                }
                if key.is_empty() {
                    p += 1; // Avoid looping forever on junk like `="x"`.
                } else if el.attrs.iter().any(|(k, _)| *k == key) {
                    warnings.push(Warning::new(file, line, format!("duplicate attribute '{key}' on <{}>", el.name)));
                } else {
                    el.attrs.push((key, value));
                }
            }
        }
    }

    if self_closing {
        stack.last_mut().expect("document element").children.push(el);
    } else {
        stack.push(el);
    }
    p
}

/// Splits a value like `"A, B C;D"` into tokens on commas, semicolons and whitespace.
pub fn split_list(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parses an Alamo boolean (`yes`/`true`/`1` are true, anything else false).
pub fn parse_bool(s: &str) -> bool {
    matches!(s.trim().to_ascii_lowercase().as_str(), "yes" | "true" | "1" | "on")
}

/// Parses a comma/space separated list of floats; unparsable items become 0.
pub fn parse_floats(s: &str) -> Vec<f32> {
    split_list(s).iter().map(|t| t.parse().unwrap_or(0.0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> (Option<Element>, Vec<Warning>) {
        let mut w = Vec::new();
        let e = parse(s.as_bytes(), "t.xml", &mut w);
        (e, w)
    }

    #[test]
    fn basic_and_case_insensitive() {
        let (e, w) = p("<?xml version=\"1.0\"?>\n<Root><Planet Name=\"A\"><X_Y> 1, 2 </x_y></Planet></ROOT>");
        let e = e.unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(e.name, "root");
        let pl = e.child("planet").unwrap();
        assert_eq!(pl.attr("name"), Some("A"));
        assert_eq!(pl.child_text("x_y"), Some("1, 2"));
    }

    #[test]
    fn bad_comments_and_ampersand() {
        let (e, _) = p("<a><!-- bad -- comment --><b>Tom & Jerry &amp; &#65;&#x42;</b></a>");
        assert_eq!(e.unwrap().child_text("b"), Some("Tom & Jerry & AB"));
    }

    #[test]
    fn duplicate_attributes_and_unquoted() {
        let (e, w) = p("<a><b n=\"1\" n=\"2\" k=v flag/></a>");
        let e = e.unwrap();
        let b = e.child("b").unwrap();
        assert_eq!(b.attr("n"), Some("1"));
        assert_eq!(b.attr("k"), Some("v"));
        assert_eq!(b.attr("flag"), Some(""));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn mismatched_and_trailing_garbage() {
        let (e, w) = p("\u{FEFF}<a><b>x<c>y</b>z</a> trailing </junk> <");
        let e = e.unwrap();
        assert_eq!(e.child("b").unwrap().child("c").unwrap().text_trimmed(), "y");
        assert!(!w.is_empty());
    }

    #[test]
    fn unclosed_root_and_empty() {
        let (e, w) = p("<a><b>1</b>");
        assert_eq!(e.unwrap().child_text("b"), Some("1"));
        assert_eq!(w.len(), 1);
        assert!(p("just text").0.is_none());
    }

    #[test]
    fn bom_and_utf16() {
        assert_eq!(decode_bytes(&[0xEF, 0xBB, 0xBF, b'a']), "a");
        assert_eq!(decode_bytes(&[0xFF, 0xFE, b'a', 0, b'b', 0]), "ab");
        assert_eq!(decode_bytes(&[b'a', 0xE9]), "a\u{e9}");
    }

    #[test]
    fn sanitize_strips_control_chars() {
        assert_eq!(sanitize("a\0b\u{1}c\u{FEFF}d\te\n"), "abcd\te\n");
    }

    #[test]
    fn lists_and_values() {
        assert_eq!(split_list(" A, B  C;D ,"), ["A", "B", "C", "D"]);
        assert!(parse_bool(" Yes "));
        assert!(!parse_bool("No"));
        assert_eq!(parse_floats("1.5, 2 ,x"), [1.5, 2.0, 0.0]);
    }

    #[test]
    fn line_numbers() {
        let (e, _) = p("<a>\n<b/>\n\n<c/></a>");
        let e = e.unwrap();
        assert_eq!(e.child("b").unwrap().line, 2);
        assert_eq!(e.child("c").unwrap().line, 4);
    }
}
