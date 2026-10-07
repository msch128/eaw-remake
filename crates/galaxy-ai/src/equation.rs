//! Perceptual equations: the expression language of `Data/Xml/AI/PerceptualEquations/*.xml`.
//!
//! ```text
//! Function_HighImportance.Evaluate * (Variable_Target.StarbaseLevel == 0.0)
//! clamp((500.0 - Variable_Self.NetIncomeUnnormalized / Variable_Self.PlanetsControlledUnnormalized), 1, 750)
//! Variable_Target.StructureCount{Parameter_Type = "E_Shipyard_1", Parameter_Type = "R_Shipyard_1"} > 0
//! ```
//!
//! Grammar (precedence from low to high): comparisons `== != < > <= >=` (non-associative, result 1.0 / 0.0),
//! `+ -`, `* /`, unary `-`, primaries. A primary is a number, a parenthesised expression,
//! `clamp(a, b, c)` / `min` / `max` / `abs`, a call of another equation `Function_Name.Evaluate`,
//! or a variable `Variable_Scope.Path.To.Value {Parameter_X = value, ...}`. `Game.X` is a global variable.
//!
//! The precedence of comparisons relative to arithmetic is inferred (C-like); the engine's own parser is
//! not available. Division by zero yields 0.

use std::collections::HashMap;
use std::fmt;

/// Error while parsing an equation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// A parameter value inside `{ ... }`.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// Number.
    Num(f64),
    /// Quoted string or bare word.
    Str(String),
    /// `EnumType[Value]`.
    Enum(String, String),
}

/// The parameters of a variable reference. Keys may repeat (`Parameter_Type` lists).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params(pub Vec<(String, ParamValue)>);

impl Params {
    /// All values of a key (without the `Parameter_` prefix, lower case).
    pub fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a ParamValue> + 'a {
        self.0.iter().filter(move |(k, _)| k == key).map(|(_, v)| v)
    }

    /// First numeric value of a key.
    pub fn num(&self, key: &str) -> Option<f64> {
        self.all(key).find_map(|v| match v {
            ParamValue::Num(n) => Some(*n),
            _ => None,
        })
    }

    /// First string (or enum member) value of a key.
    pub fn str<'a>(&'a self, key: &'a str) -> Option<&'a str> {
        self.all(key).find_map(|v| match v {
            ParamValue::Str(s) | ParamValue::Enum(_, s) => Some(s.as_str()),
            ParamValue::Num(_) => None,
        })
    }

    /// All string values of a key.
    pub fn strs<'a>(&'a self, key: &'a str) -> Vec<&'a str> {
        self.all(key)
            .filter_map(|v| match v {
                ParamValue::Str(s) | ParamValue::Enum(_, s) => Some(s.as_str()),
                ParamValue::Num(_) => None,
            })
            .collect()
    }
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `a#b`: uniform random value between a and b.
    Rand,
}

/// Expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Literal.
    Num(f64),
    /// Unary minus.
    Neg(Box<Expr>),
    /// Binary operation.
    Bin(Op, Box<Expr>, Box<Expr>),
    /// `clamp(value, min, max)`.
    Clamp(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `min(a, b)`.
    Min(Box<Expr>, Box<Expr>),
    /// `max(a, b)`.
    Max(Box<Expr>, Box<Expr>),
    /// `abs(a)`.
    Abs(Box<Expr>),
    /// Another equation (lower-cased name).
    Call(String),
    /// `Variable_<Scope>.<Path>{params}`: scope (`self`, `target`, ...), dotted path, parameters.
    Var {
        /// Lower-cased scope.
        scope: String,
        /// Dotted path as written (case preserved).
        path: String,
        /// Parameters.
        params: Params,
    },
    /// `Script_<Name>.Evaluate{params}`: a Lua evaluator from `Scripts/Evaluators`.
    Script {
        /// Evaluator name.
        name: String,
        /// Parameters.
        params: Params,
    },
    /// `Game.<Path>`.
    Game {
        /// Dotted path.
        path: String,
        /// Parameters.
        params: Params,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Str(String),
    Sym(&'static str),
}

fn lex(src: &str) -> Result<Vec<Tok>, ParseError> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let n = src[start..i].parse().map_err(|_| ParseError(format!("bad number '{}'", &src[start..i])))?;
            out.push(Tok::Num(n));
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            // A trailing '.' cannot belong to the identifier.
            let mut end = i;
            while src[start..end].ends_with('.') {
                end -= 1;
            }
            i = end;
            out.push(Tok::Ident(src[start..end].to_string()));
        } else if c == b'"' {
            let start = i + 1;
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += 1;
            }
            if i >= b.len() {
                return Err(ParseError("unterminated string".into()));
            }
            out.push(Tok::Str(src[start..i].to_string()));
            i += 1;
        } else {
            let two = src.get(i..i + 2);
            let sym = match two {
                Some("==") => Some("=="),
                Some("!=") => Some("!="),
                Some("<=") => Some("<="),
                Some(">=") => Some(">="),
                _ => None,
            };
            if let Some(s) = sym {
                out.push(Tok::Sym(s));
                i += 2;
                continue;
            }
            let s = match c {
                b'+' => "+",
                b'-' => "-",
                b'*' => "*",
                b'/' => "/",
                b'<' => "<",
                b'>' => ">",
                b'(' => "(",
                b')' => ")",
                b'{' => "{",
                b'}' => "}",
                b'[' => "[",
                b']' => "]",
                b',' => ",",
                b'=' => "=",
                b'#' => "#",
                b'|' => "|",
                _ => return Err(ParseError(format!("unexpected character '{}'", c as char))),
            };
            out.push(Tok::Sym(s));
            i += 1;
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn sym(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(x)) if *x == s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> Result<(), ParseError> {
        if self.sym(s) { Ok(()) } else { Err(ParseError(format!("expected '{s}' at token {}", self.pos))) }
    }

    fn comparison(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.additive()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym("==")) => Op::Eq,
                Some(Tok::Sym("!=")) => Op::Ne,
                Some(Tok::Sym("<=")) => Op::Le,
                Some(Tok::Sym(">=")) => Op::Ge,
                Some(Tok::Sym("<")) => Op::Lt,
                Some(Tok::Sym(">")) => Op::Gt,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            let rhs = self.additive()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym("+")) => Op::Add,
                Some(Tok::Sym("-")) => Op::Sub,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            let rhs = self.term()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym("*")) => Op::Mul,
                Some(Tok::Sym("/")) => Op::Div,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            let rhs = self.unary()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.sym("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        let mut lhs = self.primary()?;
        while self.sym("#") {
            let rhs = self.primary()?;
            lhs = Expr::Bin(Op::Rand, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Sym("(")) => {
                self.pos += 1;
                let e = self.comparison()?;
                self.expect(")")?;
                Ok(e)
            }
            Some(Tok::Ident(id)) => {
                self.pos += 1;
                self.ident(&id)
            }
            other => Err(ParseError(format!("unexpected token {other:?}"))),
        }
    }

    fn args(&mut self, n: usize) -> Result<Vec<Expr>, ParseError> {
        self.expect("(")?;
        let mut v = vec![self.comparison()?];
        while self.sym(",") {
            v.push(self.comparison()?);
        }
        self.expect(")")?;
        if v.len() != n {
            return Err(ParseError(format!("expected {n} arguments, got {}", v.len())));
        }
        Ok(v)
    }

    fn ident(&mut self, id: &str) -> Result<Expr, ParseError> {
        let lower = id.to_ascii_lowercase();
        match lower.as_str() {
            "clamp" => {
                let mut a = self.args(3)?;
                let (c, b, x) = (a.pop().unwrap(), a.pop().unwrap(), a.pop().unwrap());
                return Ok(Expr::Clamp(Box::new(x), Box::new(b), Box::new(c)));
            }
            "min" | "max" => {
                let mut a = self.args(2)?;
                let (b, x) = (a.pop().unwrap(), a.pop().unwrap());
                return Ok(if lower == "min" { Expr::Min(Box::new(x), Box::new(b)) } else { Expr::Max(Box::new(x), Box::new(b)) });
            }
            "abs" => return Ok(Expr::Abs(Box::new(self.args(1)?.pop().unwrap()))),
            _ => {}
        }
        if let Some(rest) = lower.strip_prefix("function_") {
            let name = rest.strip_suffix(".evaluate").unwrap_or(rest);
            return Ok(Expr::Call(name.to_string()));
        }
        if let Some(rest) = lower.strip_prefix("script_") {
            let name = rest.strip_suffix(".evaluate").unwrap_or(rest);
            let params = self.params()?;
            return Ok(Expr::Script { name: name.to_string(), params });
        }
        if lower.starts_with("variable_") {
            let rest = &id["variable_".len()..];
            let (scope, path) = rest.split_once('.').ok_or_else(|| ParseError(format!("variable without path: {id}")))?;
            let params = self.params()?;
            return Ok(Expr::Var { scope: scope.to_ascii_lowercase(), path: path.to_string(), params });
        }
        if lower.starts_with("game.") {
            let params = self.params()?;
            return Ok(Expr::Game { path: id["game.".len()..].to_string(), params });
        }
        Err(ParseError(format!("unknown identifier '{id}'")))
    }

    fn params(&mut self) -> Result<Params, ParseError> {
        let mut out = Params::default();
        if !self.sym("{") {
            return Ok(out);
        }
        while !self.sym("}") {
            let key = match self.toks.get(self.pos).cloned() {
                Some(Tok::Ident(k)) => k,
                other => return Err(ParseError(format!("bad parameter name {other:?}"))),
            };
            self.pos += 1;
            self.expect("=")?;
            let value = match self.toks.get(self.pos).cloned() {
                Some(Tok::Num(n)) => {
                    self.pos += 1;
                    ParamValue::Num(n)
                }
                Some(Tok::Sym("-")) => {
                    self.pos += 1;
                    match self.toks.get(self.pos) {
                        Some(Tok::Num(n)) => {
                            let n = *n;
                            self.pos += 1;
                            ParamValue::Num(-n)
                        }
                        other => return Err(ParseError(format!("bad negative parameter {other:?}"))),
                    }
                }
                Some(Tok::Str(s)) => {
                    self.pos += 1;
                    ParamValue::Str(s)
                }
                Some(Tok::Ident(w)) => {
                    self.pos += 1;
                    if self.sym("[") {
                        // `Type[A | B]`: several members, kept joined with '|'.
                        let mut member = String::new();
                        loop {
                            match self.toks.get(self.pos).cloned() {
                                Some(Tok::Ident(m)) => member.push_str(&m),
                                other => return Err(ParseError(format!("bad enum member {other:?}"))),
                            }
                            self.pos += 1;
                            if self.sym("|") {
                                member.push('|');
                            } else {
                                break;
                            }
                        }
                        self.expect("]")?;
                        ParamValue::Enum(w, member)
                    } else {
                        ParamValue::Str(w)
                    }
                }
                other => return Err(ParseError(format!("bad parameter value {other:?}"))),
            };
            let key = key.strip_prefix("Parameter_").or_else(|| key.strip_prefix("parameter_")).unwrap_or(&key).to_ascii_lowercase();
            out.0.push((key, value));
            self.sym(",");
        }
        Ok(out)
    }
}

/// Parses one equation body.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(ParseError("empty equation".into()));
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.comparison()?;
    if p.pos != p.toks.len() {
        return Err(ParseError(format!("trailing tokens from {}", p.pos)));
    }
    Ok(e)
}

/// Supplies the values of variables during evaluation.
pub trait EquationContext {
    /// `Variable_<scope>.<path>{params}`; `None` if the variable is not known (counted and read as 0).
    fn variable(&mut self, scope: &str, path: &str, params: &Params) -> Option<f64>;
    /// `Game.<path>`.
    fn game(&mut self, path: &str, params: &Params) -> Option<f64>;
    /// `Script_<name>.Evaluate{params}`: runs a Lua evaluator.
    fn script(&mut self, _name: &str, _params: &Params) -> Option<f64> {
        None
    }
    /// A uniform random number in `[0, 1)` for the `a#b` operator.
    fn random(&mut self) -> f64 {
        0.5
    }
}

/// Transitive dependencies of an equation.
#[derive(Debug, Default, Clone)]
pub struct Dependencies {
    /// Equations (lower case), including the root.
    pub equations: std::collections::BTreeSet<String>,
    /// Variables as scope.Path.
    pub variables: std::collections::BTreeSet<String>,
    /// Lua evaluator scripts.
    pub scripts: std::collections::BTreeSet<String>,
}

/// A named set of equations.
#[derive(Debug, Default, Clone)]
pub struct EquationSet {
    equations: HashMap<String, Expr>,
    /// Equations that failed to parse: name, error.
    pub parse_errors: Vec<(String, String)>,
}

/// Evaluation statistics.
#[derive(Debug, Default, Clone)]
pub struct EvalStats {
    /// Variables the context did not know, with counts.
    pub unknown_variables: HashMap<String, usize>,
    /// Calls of equations that do not exist.
    pub missing_equations: HashMap<String, usize>,
}

impl EquationSet {
    /// Adds an equation (replacing an earlier one with the same name).
    pub fn insert(&mut self, name: &str, body: &str) {
        match parse(body) {
            Ok(e) => {
                self.equations.insert(name.to_ascii_lowercase(), e);
            }
            Err(e) => self.parse_errors.push((name.to_string(), e.to_string())),
        }
    }

    /// Looks an equation up.
    pub fn get(&self, name: &str) -> Option<&Expr> {
        self.equations.get(&name.to_ascii_lowercase())
    }

    /// Names of the equations, variables and scripts an equation depends on, transitively.
    pub fn dependencies(&self, name: &str, out: &mut Dependencies) {
        let name = name.to_ascii_lowercase();
        if !out.equations.insert(name.clone()) {
            return;
        }
        if let Some(e) = self.equations.get(&name) {
            self.collect(e, out);
        }
    }

    fn collect(&self, e: &Expr, out: &mut Dependencies) {
        match e {
            Expr::Num(_) => {}
            Expr::Neg(a) | Expr::Abs(a) => self.collect(a, out),
            Expr::Bin(_, a, b) | Expr::Min(a, b) | Expr::Max(a, b) => {
                self.collect(a, out);
                self.collect(b, out);
            }
            Expr::Clamp(a, b, c) => {
                self.collect(a, out);
                self.collect(b, out);
                self.collect(c, out);
            }
            Expr::Call(n) => self.dependencies(n, out),
            Expr::Var { scope, path, .. } => {
                out.variables.insert(format!("{scope}.{path}"));
            }
            Expr::Script { name, .. } => {
                out.scripts.insert(name.clone());
            }
            Expr::Game { path, .. } => {
                out.variables.insert(format!("game.{path}"));
            }
        }
    }

    /// Number of parsed equations.
    pub fn len(&self) -> usize {
        self.equations.len()
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.equations.is_empty()
    }

    /// Names of all equations.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.equations.keys().map(String::as_str)
    }

    /// Evaluates the named equation.
    pub fn evaluate(&self, name: &str, ctx: &mut dyn EquationContext, stats: &mut EvalStats) -> f64 {
        self.call(&name.to_ascii_lowercase(), ctx, stats, 0, &mut EvalCache::disabled())
    }

    /// Like [`evaluate`](Self::evaluate) but remembers the result of every equation called on the way in
    /// `cache`; valid as long as the context (player, target) stays the same.
    pub fn evaluate_cached(&self, name: &str, ctx: &mut dyn EquationContext, stats: &mut EvalStats, cache: &mut EvalCache) -> f64 {
        self.call(&name.to_ascii_lowercase(), ctx, stats, 0, cache)
    }

    fn call(&self, name: &str, ctx: &mut dyn EquationContext, stats: &mut EvalStats, depth: u32, cache: &mut EvalCache) -> f64 {
        if depth > 40 {
            return 0.0;
        }
        if cache.enabled
            && let Some(v) = cache.map.get(name)
        {
            return *v;
        }
        let v = match self.equations.get(name) {
            Some(e) => self.eval(e, ctx, stats, depth, cache),
            None => {
                *stats.missing_equations.entry(name.to_string()).or_default() += 1;
                0.0
            }
        };
        if cache.enabled {
            cache.map.insert(name.to_string(), v);
        }
        v
    }

    fn eval(&self, e: &Expr, ctx: &mut dyn EquationContext, stats: &mut EvalStats, depth: u32, cache: &mut EvalCache) -> f64 {
        let b = |x: bool| if x { 1.0 } else { 0.0 };
        match e {
            Expr::Num(n) => *n,
            Expr::Neg(a) => -self.eval(a, ctx, stats, depth, cache),
            Expr::Bin(op, l, r) => {
                let (l, r) = (self.eval(l, ctx, stats, depth, cache), self.eval(r, ctx, stats, depth, cache));
                match op {
                    Op::Add => l + r,
                    Op::Sub => l - r,
                    Op::Mul => l * r,
                    Op::Div => {
                        if r == 0.0 {
                            0.0
                        } else {
                            l / r
                        }
                    }
                    Op::Eq => b((l - r).abs() < 1e-9),
                    Op::Ne => b((l - r).abs() >= 1e-9),
                    Op::Lt => b(l < r),
                    Op::Gt => b(l > r),
                    Op::Le => b(l <= r),
                    Op::Ge => b(l >= r),
                    Op::Rand => l + (r - l) * ctx.random(),
                }
            }
            Expr::Clamp(v, lo, hi) => {
                let (v, lo, hi) = (self.eval(v, ctx, stats, depth, cache), self.eval(lo, ctx, stats, depth, cache), self.eval(hi, ctx, stats, depth, cache));
                v.max(lo).min(hi.max(lo))
            }
            Expr::Min(a, c) => self.eval(a, ctx, stats, depth, cache).min(self.eval(c, ctx, stats, depth, cache)),
            Expr::Max(a, c) => self.eval(a, ctx, stats, depth, cache).max(self.eval(c, ctx, stats, depth, cache)),
            Expr::Abs(a) => self.eval(a, ctx, stats, depth, cache).abs(),
            Expr::Call(name) => self.call(name, ctx, stats, depth + 1, cache),
            Expr::Var { scope, path, params } => match ctx.variable(scope, path, params) {
                Some(v) => v,
                None => {
                    *stats.unknown_variables.entry(format!("{scope}.{path}")).or_default() += 1;
                    0.0
                }
            },
            Expr::Script { name, params } => match ctx.script(name, params) {
                Some(v) => v,
                None => {
                    *stats.unknown_variables.entry(format!("script.{name}")).or_default() += 1;
                    0.0
                }
            },
            Expr::Game { path, params } => match ctx.game(path, params) {
                Some(v) => v,
                None => {
                    *stats.unknown_variables.entry(format!("game.{path}")).or_default() += 1;
                    0.0
                }
            },
        }
    }
}

/// Memoized equation results for one `(player, target)`.
#[derive(Debug, Default)]
pub struct EvalCache {
    map: HashMap<String, f64>,
    enabled: bool,
}

impl EvalCache {
    /// A cache that remembers results.
    pub fn new() -> Self {
        Self { map: HashMap::new(), enabled: true }
    }

    /// A cache that remembers nothing.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Forgets everything (the context changed).
    pub fn clear(&mut self) {
        self.map.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    struct Ctx;
    impl EquationContext for Ctx {
        fn variable(&mut self, scope: &str, path: &str, params: &Params) -> Option<f64> {
            match (scope, path) {
                ("self", "Credits") => Some(1000.0),
                ("target", "Count") => Some(params.strs("type").len() as f64),
                _ => None,
            }
        }
        fn game(&mut self, path: &str, _: &Params) -> Option<f64> {
            (path == "Age").then_some(42.0)
        }
    }

    fn eval(set: &EquationSet, name: &str) -> (f64, EvalStats) {
        let mut stats = EvalStats::default();
        (set.evaluate(name, &mut Ctx, &mut stats), stats)
    }

    #[test]
    fn arithmetic_and_comparisons() {
        let mut s = EquationSet::default();
        s.insert("a", "1 + 2 * 3 - 4 / 2");
        s.insert("b", "(Variable_Self.Credits > 500) * 10 + (Variable_Self.Credits == 1000.0)");
        s.insert("c", "clamp(Variable_Self.Credits / 10, 0, 50) - -1");
        s.insert("d", "Game.Age >= 42");
        s.insert("e", "Variable_Target.Count{Parameter_Type = \"A\", Parameter_Type = \"B\", Parameter_Faction = Enum[X]}");
        s.insert("f", "Function_A.Evaluate * 2 + Function_Missing.Evaluate");
        s.insert("g", "Variable_Self.Nope + 1");
        s.insert("h", "1 / (Variable_Target.Count - 0)");
        assert!(s.parse_errors.is_empty(), "{:?}", s.parse_errors);
        assert_eq!(eval(&s, "a").0, 5.0);
        assert_eq!(eval(&s, "b").0, 11.0);
        assert_eq!(eval(&s, "c").0, 51.0);
        assert_eq!(eval(&s, "d").0, 1.0);
        assert_eq!(eval(&s, "e").0, 2.0);
        let (f, stats) = eval(&s, "f");
        assert_eq!(f, 10.0);
        assert_eq!(stats.missing_equations.get("missing"), Some(&1));
        let (g, stats) = eval(&s, "g");
        assert_eq!(g, 1.0);
        assert_eq!(stats.unknown_variables.get("self.Nope"), Some(&1));
        assert_eq!(eval(&s, "h").0, 0.0, "division by zero reads as 0");
    }

    #[test]
    fn parse_errors_are_collected() {
        let mut s = EquationSet::default();
        s.insert("bad", "1 + * 2");
        s.insert("bad2", "Foo.Bar");
        assert_eq!(s.parse_errors.len(), 2);
    }
}
