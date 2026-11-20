//! Minimal JSON for the control socket (one object per line). Enough for the fixture protocol
//! of testbench/docs/testlist-format.md section 4; not a general JSON library.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(BTreeMap<String, J>),
}

impl J {
    pub fn get(&self, k: &str) -> Option<&J> {
        match self {
            J::Obj(m) => m.get(k),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            J::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn obj(pairs: Vec<(&str, J)>) -> J {
        J::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn dump(&self) -> String {
        match self {
            J::Null => "null".into(),
            J::Bool(b) => b.to_string(),
            J::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 9.0e15 { format!("{}", *n as i64) } else { n.to_string() }
            }
            J::Str(s) => {
                let mut o = String::from("\"");
                for c in s.chars() {
                    match c {
                        '"' => o.push_str("\\\""),
                        '\\' => o.push_str("\\\\"),
                        '\n' => o.push_str("\\n"),
                        c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                        c => o.push(c),
                    }
                }
                o.push('"');
                o
            }
            J::Arr(a) => format!("[{}]", a.iter().map(|x| x.dump()).collect::<Vec<_>>().join(",")),
            J::Obj(m) => format!(
                "{{{}}}",
                m.iter().map(|(k, v)| format!("{}:{}", J::Str(k.clone()).dump(), v.dump())).collect::<Vec<_>>().join(",")
            ),
        }
    }
}

pub fn parse(s: &str) -> Result<J, String> {
    let b = s.as_bytes();
    let mut i = 0;
    let v = value(b, &mut i)?;
    ws(b, &mut i);
    if i != b.len() {
        return Err(format!("trailing data at {i}"));
    }
    Ok(v)
}

fn ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && (b[*i] as char).is_ascii_whitespace() {
        *i += 1;
    }
}

fn value(b: &[u8], i: &mut usize) -> Result<J, String> {
    ws(b, i);
    match b.get(*i) {
        None => Err("unexpected end".into()),
        Some(b'{') => {
            *i += 1;
            let mut m = BTreeMap::new();
            ws(b, i);
            if b.get(*i) == Some(&b'}') {
                *i += 1;
                return Ok(J::Obj(m));
            }
            loop {
                ws(b, i);
                let J::Str(k) = value(b, i)? else { return Err("key".into()) };
                ws(b, i);
                if b.get(*i) != Some(&b':') {
                    return Err("expected :".into());
                }
                *i += 1;
                m.insert(k, value(b, i)?);
                ws(b, i);
                match b.get(*i) {
                    Some(b',') => *i += 1,
                    Some(b'}') => {
                        *i += 1;
                        return Ok(J::Obj(m));
                    }
                    _ => return Err("expected , or }".into()),
                }
            }
        }
        Some(b'[') => {
            *i += 1;
            let mut a = Vec::new();
            ws(b, i);
            if b.get(*i) == Some(&b']') {
                *i += 1;
                return Ok(J::Arr(a));
            }
            loop {
                a.push(value(b, i)?);
                ws(b, i);
                match b.get(*i) {
                    Some(b',') => *i += 1,
                    Some(b']') => {
                        *i += 1;
                        return Ok(J::Arr(a));
                    }
                    _ => return Err("expected , or ]".into()),
                }
            }
        }
        Some(b'"') => {
            *i += 1;
            let mut s = String::new();
            while let Some(&c) = b.get(*i) {
                *i += 1;
                match c {
                    b'"' => return Ok(J::Str(s)),
                    b'\\' => {
                        let e = *b.get(*i).ok_or("escape")?;
                        *i += 1;
                        match e {
                            b'n' => s.push('\n'),
                            b't' => s.push('\t'),
                            b'u' => {
                                let h = std::str::from_utf8(b.get(*i..*i + 4).ok_or("u")?).map_err(|e| e.to_string())?;
                                s.push(char::from_u32(u32::from_str_radix(h, 16).map_err(|e| e.to_string())?).unwrap_or('?'));
                                *i += 4;
                            }
                            c => s.push(c as char),
                        }
                    }
                    _ => {
                        let start = *i - 1;
                        let mut end = *i;
                        while end < b.len() && b[end] != b'"' && b[end] != b'\\' {
                            end += 1;
                        }
                        s.push_str(std::str::from_utf8(&b[start..end]).map_err(|e| e.to_string())?);
                        *i = end;
                    }
                }
            }
            Err("unterminated string".into())
        }
        Some(b't') if b[*i..].starts_with(b"true") => {
            *i += 4;
            Ok(J::Bool(true))
        }
        Some(b'f') if b[*i..].starts_with(b"false") => {
            *i += 5;
            Ok(J::Bool(false))
        }
        Some(b'n') if b[*i..].starts_with(b"null") => {
            *i += 4;
            Ok(J::Null)
        }
        Some(_) => {
            let start = *i;
            while *i < b.len() && matches!(b[*i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                *i += 1;
            }
            std::str::from_utf8(&b[start..*i]).unwrap().parse::<f64>().map(J::Num).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let v = parse(r#"{"op":"sensor.set","args":{"name":"fwd_power_mw","value":-1.5e3},"x":[true,null,"a\"b"]}"#).unwrap();
        assert_eq!(v.get("op").unwrap().as_str(), Some("sensor.set"));
        assert_eq!(v.get("args").unwrap().get("value").unwrap().as_f64(), Some(-1500.0));
        assert_eq!(parse(&v.dump()).unwrap(), v);
    }
}
