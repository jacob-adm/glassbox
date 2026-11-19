//! Strict JSON reader (RFC 8259): enough to pull named subtrees out of agent config files.
//!
//! The files it reads can be written by the agents glassbox audits, so it follows the
//! grammar exactly (what it accepts is what a real parser would load) and never recurses
//! without bound: a subtree nested deeper than `MAX_DEPTH` is kept as opaque raw text.
use std::collections::BTreeMap;

/// Deeper subtrees are stored as `Value::Opaque` instead of being parsed.
pub const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(String),
    /// The string in canonical escaped form (without the quotes): `"`, `\\` and control
    /// characters escaped, a lone surrogate as `\uXXXX`. Everything else is literal, so two
    /// strings are equal exactly when their decoded contents are.
    Str(String),
    Arr(Vec<Value>),
    Obj(BTreeMap<String, Value>),
    /// A subtree beyond `MAX_DEPTH`, as its raw text.
    Opaque(String),
}

pub fn parse(text: &str) -> Option<Value> {
    let mut p = Parser { s: text.as_bytes(), i: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}

impl Value {
    /// Serialization with object keys sorted, so key order never changes the result.
    pub fn canonical(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => n.clone(),
            Value::Str(s) => format!("\"{s}\""),
            Value::Opaque(raw) => raw.clone(),
            Value::Arr(a) => format!("[{}]", a.iter().map(Value::canonical).collect::<Vec<_>>().join(",")),
            Value::Obj(o) => format!(
                "{{{}}}",
                o.iter().map(|(k, v)| format!("\"{k}\":{}", v.canonical())).collect::<Vec<_>>().join(",")
            ),
        }
    }

    /// Every (path, value) whose key is in `keys`, in sorted order, plus every subtree past
    /// `MAX_DEPTH` (which may contain such a key).
    pub fn find(&self, keys: &[&str]) -> Vec<(String, &Value)> {
        let mut out = Vec::new();
        self.find_into("$", keys, &mut out);
        out
    }

    fn find_into<'a>(&'a self, path: &str, keys: &[&str], out: &mut Vec<(String, &'a Value)>) {
        match self {
            Value::Obj(o) => {
                for (k, v) in o {
                    let p = format!("{path}.\"{k}\"");
                    if keys.contains(&k.as_str()) {
                        out.push((p.clone(), v));
                    }
                    v.find_into(&p, keys, out);
                }
            }
            Value::Arr(a) => {
                for (i, v) in a.iter().enumerate() {
                    v.find_into(&format!("{path}[{i}]"), keys, out);
                }
            }
            // A subtree past MAX_DEPTH is not searched, so it could hide a selected key:
            // include it whole. Changes inside it are then always reported.
            Value::Opaque(_) => out.push((path.to_string(), self)),
            _ => {}
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> Option<()> {
        self.s[self.i..].starts_with(lit.as_bytes()).then(|| self.i += lit.len())
    }

    fn value(&mut self) -> Option<Value> {
        self.ws();
        match *self.s.get(self.i)? {
            b'{' | b'[' if self.depth >= MAX_DEPTH => self.opaque(),
            b'{' => self.nested(Self::object),
            b'[' => self.nested(Self::array),
            b'"' => self.string().map(Value::Str),
            b't' => self.eat("true").map(|_| Value::Bool(true)),
            b'f' => self.eat("false").map(|_| Value::Bool(false)),
            b'n' => self.eat("null").map(|_| Value::Null),
            _ => self.number(),
        }
    }

    fn nested(&mut self, f: fn(&mut Self) -> Option<Value>) -> Option<Value> {
        self.depth += 1;
        let v = f(self);
        self.depth -= 1;
        v
    }

    /// Check a too-deep object or array against the grammar with an explicit stack (no
    /// recursion) and keep its raw text. Key order and spacing inside it are not normalized.
    fn opaque(&mut self) -> Option<Value> {
        #[derive(PartialEq)]
        enum Next {
            Value,
            ValueOrClose,
            Key,
            KeyOrClose,
            Colon,
            CommaOrClose,
        }
        let start = self.i;
        let mut stack: Vec<u8> = Vec::new();
        let mut next = Next::Value;
        loop {
            self.ws();
            let c = *self.s.get(self.i)?;
            let mut closed = false;
            match next {
                Next::ValueOrClose if c == b']' => {
                    self.i += 1;
                    stack.pop();
                    closed = true;
                }
                Next::Value | Next::ValueOrClose => match c {
                    b'[' => {
                        self.i += 1;
                        stack.push(b'[');
                        next = Next::ValueOrClose;
                    }
                    b'{' => {
                        self.i += 1;
                        stack.push(b'{');
                        next = Next::KeyOrClose;
                    }
                    b'"' => {
                        self.string()?;
                        closed = true;
                    }
                    b't' | b'f' | b'n' => {
                        self.eat("true").or_else(|| self.eat("false")).or_else(|| self.eat("null"))?;
                        closed = true;
                    }
                    _ => {
                        self.number()?;
                        closed = true;
                    }
                },
                Next::KeyOrClose if c == b'}' => {
                    self.i += 1;
                    stack.pop();
                    closed = true;
                }
                Next::Key | Next::KeyOrClose => {
                    self.string()?;
                    next = Next::Colon;
                }
                Next::Colon => {
                    self.eat(":")?;
                    next = Next::Value;
                }
                Next::CommaOrClose => {
                    let top = *stack.last()?;
                    self.i += 1;
                    match (c, top) {
                        (b',', b'[') => next = Next::Value,
                        (b',', b'{') => next = Next::Key,
                        (b']', b'[') | (b'}', b'{') => {
                            stack.pop();
                            closed = true;
                        }
                        _ => return None,
                    }
                }
            }
            if closed {
                if stack.is_empty() {
                    let raw = std::str::from_utf8(&self.s[start..self.i]).ok()?;
                    return Some(Value::Opaque(raw.to_string()));
                }
                next = Next::CommaOrClose;
            }
        }
    }

    fn object(&mut self) -> Option<Value> {
        self.i += 1;
        let mut o = BTreeMap::new();
        self.ws();
        if self.eat("}").is_some() {
            return Some(Value::Obj(o));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            self.eat(":")?;
            o.insert(k, self.value()?);
            self.ws();
            if self.eat(",").is_none() {
                self.eat("}")?;
                return Some(Value::Obj(o));
            }
        }
    }

    fn array(&mut self) -> Option<Value> {
        self.i += 1;
        let mut a = Vec::new();
        self.ws();
        if self.eat("]").is_some() {
            return Some(Value::Arr(a));
        }
        loop {
            a.push(self.value()?);
            self.ws();
            if self.eat(",").is_none() {
                self.eat("]")?;
                return Some(Value::Arr(a));
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.s.get(self.i..self.i + 4)?;
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
    }

    /// A string, returned in canonical escaped form (see `Value::Str`).
    fn string(&mut self) -> Option<String> {
        self.eat("\"")?;
        let mut out = String::new();
        let mut plain = self.i;
        loop {
            match *self.s.get(self.i)? {
                b'"' => {
                    out.push_str(std::str::from_utf8(&self.s[plain..self.i]).ok()?);
                    self.i += 1;
                    return Some(out);
                }
                0x00..=0x1f => return None,
                b'\\' => {
                    out.push_str(std::str::from_utf8(&self.s[plain..self.i]).ok()?);
                    self.i += 1;
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' => out.push_str("\\\""),
                        b'\\' => out.push_str("\\\\"),
                        b'/' => out.push('/'),
                        b'b' => out.push_str("\\u0008"),
                        b'f' => out.push_str("\\u000c"),
                        b'n' => out.push_str("\\u000a"),
                        b'r' => out.push_str("\\u000d"),
                        b't' => out.push_str("\\u0009"),
                        b'u' => self.unicode_escape(&mut out)?,
                        _ => return None,
                    }
                    plain = self.i;
                }
                _ => self.i += 1,
            }
        }
    }

    /// After `\u`: one code point in canonical form. A surrogate pair is combined; a lone
    /// surrogate (no `char`) stays `\uXXXX`, which cannot collide with literal text because
    /// a literal backslash is always written `\\\\`.
    fn unicode_escape(&mut self, out: &mut String) -> Option<()> {
        let hi = self.hex4()?;
        if (0xD800..0xDC00).contains(&hi) && self.s[self.i..].starts_with(b"\\u") {
            let save = self.i;
            self.i += 2;
            let lo = self.hex4()?;
            if (0xDC00..0xE000).contains(&lo) {
                out.push(char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))?);
                return Some(());
            }
            self.i = save;
        }
        match char::from_u32(hi) {
            Some('"') => out.push_str("\\\""),
            Some('\\') => out.push_str("\\\\"),
            Some(ch) if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            Some(ch) => out.push(ch),
            None => out.push_str(&format!("\\u{hi:04x}")),
        }
        Some(())
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`
    fn number(&mut self) -> Option<Value> {
        let start = self.i;
        if self.s.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match self.s.get(self.i)? {
            b'0' => self.i += 1,
            b'1'..=b'9' => {
                self.digits();
            }
            _ => return None,
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !self.digits() {
                return None;
            }
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !self.digits() {
                return None;
            }
        }
        Some(Value::Num(String::from_utf8_lossy(&self.s[start..self.i]).into_owned()))
    }

    fn digits(&mut self) -> bool {
        let from = self.i;
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        self.i > from
    }
}
