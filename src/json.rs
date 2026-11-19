//! Minimal JSON reader: enough to pull named subtrees out of agent config files.
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Value>),
    Obj(BTreeMap<String, Value>),
}

pub fn parse(text: &str) -> Option<Value> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
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
            Value::Str(s) => quote(s),
            Value::Arr(a) => format!("[{}]", a.iter().map(Value::canonical).collect::<Vec<_>>().join(",")),
            Value::Obj(o) => format!(
                "{{{}}}",
                o.iter().map(|(k, v)| format!("{}:{}", quote(k), v.canonical())).collect::<Vec<_>>().join(",")
            ),
        }
    }

    /// Every (path, value) whose key is in `keys`, at any depth, in sorted order.
    pub fn find(&self, keys: &[&str]) -> Vec<(String, &Value)> {
        let mut out = Vec::new();
        self.find_into("$", keys, &mut out);
        out
    }

    fn find_into<'a>(&'a self, path: &str, keys: &[&str], out: &mut Vec<(String, &'a Value)>) {
        match self {
            Value::Obj(o) => {
                for (k, v) in o {
                    let p = format!("{path}.{}", quote(k));
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
            _ => {}
        }
    }
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
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
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Value::Str),
            b't' => self.eat("true").map(|_| Value::Bool(true)),
            b'f' => self.eat("false").map(|_| Value::Bool(false)),
            b'n' => self.eat("null").map(|_| Value::Null),
            _ => self.number(),
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

    fn string(&mut self) -> Option<String> {
        self.eat("\"")?;
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            self.i += 4;
                            char::from_u32(u32::from_str_radix(hex, 16).ok()?).unwrap_or('\u{fffd}')
                        }
                        other => other as char,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.i;
        while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        (self.i > start).then(|| Value::Num(String::from_utf8_lossy(&self.s[start..self.i]).into_owned()))
    }
}
