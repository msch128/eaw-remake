//! A tiny Lua lexer used to inventory the functions scripts define and call, without running them.

use std::collections::{BTreeMap, BTreeSet};

/// Result of scanning one script.
#[derive(Debug, Default)]
pub struct ScriptScan {
    /// Names defined with `function name`, `function a.b`, `function a:b` or `name = function`.
    pub defined: BTreeSet<String>,
    /// Call sites: `name(...)`, `a.b(...)` or `a:b(...)` (method calls are keyed as `:b`), with counts.
    pub calls: BTreeMap<String, usize>,
    /// Plain names that are assigned, declared `local`, used as `for` variable or function parameter.
    pub assigned: BTreeSet<String>,
    /// First segment of every name chain that is read or called (`a` in `a.b(c)`), with counts.
    pub reads: BTreeMap<String, usize>,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Scans Lua source. Comments and string contents are skipped.
pub fn scan(src: &str) -> ScriptScan {
    let b = src.as_bytes();
    let mut out = ScriptScan::default();
    let mut i = 0;
    let mut prev_word = String::new();
    let mut collecting = false;
    let mut pending_params = false;
    while i < b.len() {
        let c = b[i];
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            i += 2;
            if let Some(level) = long_bracket(b, i) {
                i = skip_long(b, i, level);
            } else {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            continue;
        }
        if c == b'"' || c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != c && b[i] != b'\n' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == b'['
            && let Some(level) = long_bracket(b, i)
        {
            i = skip_long(b, i, level);
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            let mut sep_last = false;
            let mut method = start > 0 && b[start - 1] == b':';
            loop {
                while i < b.len() && is_ident(b[i]) {
                    i += 1;
                }
                if i + 1 < b.len() && (b[i] == b'.' || b[i] == b':') && is_ident_start(b[i + 1]) {
                    method = b[i] == b':';
                    i += 1;
                    sep_last = true;
                    continue;
                }
                break;
            }
            let _ = sep_last;
            let chain = &src[start..i];
            let mut j = i;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            let next = b.get(j).copied();
            let after_function = prev_word == "function";
            if collecting && !chain.contains(['.', ':']) && !is_keyword(chain) {
                out.assigned.insert(chain.to_string());
            }
            if !is_keyword(chain) && !(start > 0 && (b[start - 1] == b'.' || b[start - 1] == b':'))
            {
                let first = chain.split(['.', ':']).next().unwrap_or(chain);
                *out.reads.entry(first.to_string()).or_default() += 1;
            }
            if chain == "local" || chain == "for" {
                collecting = true;
            } else if chain == "in" {
                collecting = false;
            }
            if chain == "function" && next == Some(b'(') {
                pending_params = true;
            }
            if after_function {
                pending_params = true;
            }
            if !chain.contains(['.', ':'])
                && !is_keyword(chain)
                && next == Some(b'=')
                && b.get(j + 1) != Some(&b'=')
            {
                out.assigned.insert(chain.to_string());
            }
            if after_function {
                out.defined.insert(chain.to_string());
                if let Some(pos) = chain.rfind([':', '.']) {
                    out.defined.insert(chain[pos + 1..].to_string());
                }
            } else if !is_keyword(chain) {
                let is_call = matches!(next, Some(b'(' | b'"' | b'\'' | b'{'));
                if is_call {
                    let key = if method {
                        format!(":{}", &chain[chain.rfind(':').map_or(0, |p| p + 1)..])
                    } else {
                        chain.to_string()
                    };
                    *out.calls.entry(key).or_default() += 1;
                } else if next == Some(b'=') && b.get(j + 1) != Some(&b'=') {
                    // `name = function`
                    let mut k = j + 1;
                    while k < b.len() && b[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if src[k..].starts_with("function") {
                        out.defined.insert(chain.to_string());
                    }
                }
            }
            prev_word = chain.to_string();
            continue;
        }
        match c {
            b'(' if pending_params => {
                collecting = true;
                pending_params = false;
            }
            b',' => {}
            _ if c.is_ascii_whitespace() => {}
            _ => collecting = false,
        }
        if !c.is_ascii_whitespace() {
            prev_word.clear();
        }
        i += 1;
    }
    out
}

fn is_keyword(w: &str) -> bool {
    matches!(
        w,
        "and"
            | "break"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "until"
            | "while"
    )
}

/// If a long bracket (`[[`, `[=[`...) opens at `i`, returns its level.
fn long_bracket(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while b.get(j) == Some(&b'=') {
        j += 1;
    }
    (b.get(j) == Some(&b'[')).then_some(j - i - 1)
}

fn skip_long(b: &[u8], start: usize, level: usize) -> usize {
    let mut i = start + level + 2;
    while i < b.len() {
        if b[i] == b']' {
            let mut j = i + 1;
            while b.get(j) == Some(&b'=') {
                j += 1;
            }
            if j - i - 1 == level && b.get(j) == Some(&b']') {
                return j + 1;
            }
        }
        i += 1;
    }
    b.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_calls_and_definitions() {
        let s = scan(
            r#"
            -- Foo(commented)
            function Define(a) return Find_Player("Rebel"):Get_Credits() end
            local x = "Skip(me)"
            Obj.Method = function() GlobalValue.Set("A", 1) end
            --[[ Hidden(1) ]]
            function Cls:Meth() table.getn(t) end
        "#,
        );
        assert_eq!(s.calls.get("Find_Player"), Some(&1));
        assert_eq!(s.calls.get(":Get_Credits"), Some(&1));
        assert_eq!(s.calls.get("GlobalValue.Set"), Some(&1));
        assert_eq!(s.calls.get("table.getn"), Some(&1));
        assert!(
            !s.calls.contains_key("Skip")
                && !s.calls.contains_key("Foo")
                && !s.calls.contains_key("Hidden")
        );
        assert!(
            s.defined.contains("Define")
                && s.defined.contains("Obj.Method")
                && s.defined.contains("Meth")
        );
    }
}
