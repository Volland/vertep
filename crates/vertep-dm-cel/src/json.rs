//! A strict JSON reader for inline content (`<data>`, `<assign>`, `<content>` bodies).
//!
//! Integers become `int` (or `uint` above the int range), numbers with a fraction or
//! exponent `double`, objects maps with string keys. Nesting is bounded.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::value::{CelMap, CelValue, Key};

/// Parse `text` as one JSON value, or `None` when it is not JSON.
pub fn parse(text: &str, max_depth: u32) -> Option<CelValue> {
    let mut p = Json {
        t: text,
        s: text.as_bytes(),
        i: 0,
        max_depth,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}

struct Json<'a> {
    t: &'a str,
    s: &'a [u8],
    i: usize,
    max_depth: u32,
}

impl Json<'_> {
    fn ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn lit(&mut self, word: &[u8], v: CelValue) -> Option<CelValue> {
        if self.s[self.i..].starts_with(word) {
            self.i += word.len();
            Some(v)
        } else {
            None
        }
    }

    fn value(&mut self, depth: u32) -> Option<CelValue> {
        if depth > self.max_depth {
            return None;
        }
        match *self.s.get(self.i)? {
            b'n' => self.lit(b"null", CelValue::Null),
            b't' => self.lit(b"true", CelValue::Bool(true)),
            b'f' => self.lit(b"false", CelValue::Bool(false)),
            b'"' => self.string().map(|s| CelValue::String(Arc::from(s))),
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(CelValue::List(Arc::new(items)));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(CelValue::List(Arc::new(items)));
                        }
                        _ => return None,
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut m = CelMap::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Some(CelValue::Map(Arc::new(m)));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return None;
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return None;
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    m.insert(Key::String(Arc::from(k)), v);
                    self.ws();
                    match self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(CelValue::Map(Arc::new(m)));
                        }
                        _ => return None,
                    }
                }
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn number(&mut self) -> Option<CelValue> {
        let start = self.i;
        if self.s.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        let digits = |p: &mut Self| {
            let st = p.i;
            while p.s.get(p.i).is_some_and(u8::is_ascii_digit) {
                p.i += 1;
            }
            p.i > st
        };
        if self.s.get(self.i) == Some(&b'0') {
            self.i += 1;
        } else if !digits(self) {
            return None;
        }
        let mut float = false;
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            float = true;
            if !digits(self) {
                return None;
            }
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            float = true;
            if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !digits(self) {
                return None;
            }
        }
        let text = core::str::from_utf8(&self.s[start..self.i]).ok()?;
        if !float {
            if let Ok(i) = text.parse::<i64>() {
                return Some(CelValue::Int(i));
            }
            if let Ok(u) = text.parse::<u64>() {
                return Some(CelValue::Uint(u));
            }
        }
        text.parse::<f64>().ok().map(CelValue::Double)
    }

    fn string(&mut self) -> Option<String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self.t.get(self.i..)?.chars().next()?;
            self.i += c.len_utf8();
            match c {
                '"' => return Some(out),
                '\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if self.s.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return None;
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return None;
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code)?);
                        }
                        _ => return None,
                    }
                }
                c if (c as u32) < 0x20 => return None,
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let t = core::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(t, 16).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json() {
        let v = parse(r#" {"a": [1, 2.5, "xé", null, true], "b": {}} "#, 16).unwrap();
        let CelValue::Map(m) = v else { panic!() };
        let CelValue::List(a) = &m[&Key::str("a")] else {
            panic!()
        };
        assert!(matches!(a[0], CelValue::Int(1)));
        assert!(matches!(a[1], CelValue::Double(d) if d == 2.5));
        assert!(a[2].cel_eq(&CelValue::str("xé")));
        assert!(parse("{a: 1}", 16).is_none());
        assert!(parse("[1,]", 16).is_none());
        assert!(parse(&"[".repeat(100), 16).is_none());
    }
}
