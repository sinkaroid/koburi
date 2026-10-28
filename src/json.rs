// Minimal hand-rolled JSON helpers.
#![allow(dead_code)]
//
// We do NOT use serde or any crate. We only need to:
//   * parse a small set of well-known shapes from docker/podman responses
//   * emit a fixed-shape stats object
//
// For parsing, we use a tiny streaming scanner that finds the next scalar
// (string / number / bool / null) or object/array start, and exposes
// helpers to find a value by key inside an object, and to walk arrays.

use std::io;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

pub fn classify(src: &[u8], start: usize) -> Option<Span> {
    let i = skip_ws(src, start)?;
    let b = *src.get(i)?;
    let s = match b {
        b'{' => scan_object(src, i)?,
        b'[' => scan_array(src, i)?,
        b'"' => scan_string(src, i)?,
        b't' | b'f' => scan_bool(src, i)?,
        b'n' => scan_null(src, i)?,
        b'-' | b'0'..=b'9' => scan_number(src, i)?,
        _ => return None,
    };
    Some(s)
}

fn scan_object(src: &[u8], start: usize) -> Option<Span> {
    // start points at '{'. We return a span whose end is just past the
    // matching '}'.
    let mut depth = 0i32;
    let mut i = start;
    let mut in_str = false;
    let mut esc = false;
    while i < src.len() {
        let c = src[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Span {
                            start,
                            end: i + 1,
                            kind: Kind::Object,
                        });
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn scan_array(src: &[u8], start: usize) -> Option<Span> {
    let mut depth = 0i32;
    let mut i = start;
    let mut in_str = false;
    let mut esc = false;
    while i < src.len() {
        let c = src[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Span {
                            start,
                            end: i + 1,
                            kind: Kind::Array,
                        });
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn scan_string(src: &[u8], start: usize) -> Option<Span> {
    debug_assert_eq!(src[start], b'"');
    let mut i = start + 1;
    let mut esc = false;
    while i < src.len() {
        let c = src[i];
        if esc {
            esc = false;
        } else if c == b'\\' {
            esc = true;
        } else if c == b'"' {
            return Some(Span {
                start,
                end: i + 1,
                kind: Kind::String,
            });
        }
        i += 1;
    }
    None
}

fn scan_bool(src: &[u8], start: usize) -> Option<Span> {
    if src.get(start..start + 4)? == b"true" {
        Some(Span {
            start,
            end: start + 4,
            kind: Kind::Bool,
        })
    } else if src.get(start..start + 5)? == b"false" {
        Some(Span {
            start,
            end: start + 5,
            kind: Kind::Bool,
        })
    } else {
        None
    }
}

fn scan_null(src: &[u8], start: usize) -> Option<Span> {
    if src.get(start..start + 4)? == b"null" {
        Some(Span {
            start,
            end: start + 4,
            kind: Kind::Null,
        })
    } else {
        None
    }
}

fn scan_number(src: &[u8], start: usize) -> Option<Span> {
    let mut i = start;
    if src.get(i) == Some(&b'-') {
        i += 1;
    }
    while let Some(c) = src.get(i) {
        match c {
            b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => i += 1,
            _ => break,
        }
    }
    if i == start || i == start + 1 && src[start] == b'-' {
        return None;
    }
    Some(Span {
        start,
        end: i,
        kind: Kind::Number,
    })
}

fn skip_ws(src: &[u8], mut i: usize) -> Option<usize> {
    while i < src.len() {
        match src[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            _ => return Some(i),
        }
    }
    None
}

// Find the value span for `key` inside the object whose span is `obj`.
// Returns None if the key is absent or the JSON is malformed.
pub fn object_get<'a>(src: &'a [u8], obj: Span, key: &str) -> Option<Span> {
    debug_assert_eq!(obj.kind, Kind::Object);
    let body = &src[obj.start + 1..obj.end - 1];
    let mut i = 0;
    while i < body.len() {
        let k = classify(body, i)?;
        if k.kind != Kind::String {
            return None;
        }
        let name = unquote(&body[k.start..k.end]).ok()?;
        i = k.end;
        // skip ws + colon
        i = skip_ws(body, i)?;
        if body.get(i)? != &b':' {
            return None;
        }
        i = skip_ws(body, i + 1)?;
        let v = classify(body, i)?;
        if name == key {
            return Some(Span {
                start: obj.start + 1 + v.start,
                end: obj.start + 1 + v.end,
                kind: v.kind,
            });
        }
        i = v.end;
        // skip optional comma + ws
        i = skip_ws(body, i)?;
        if body.get(i) == Some(&b',') {
            i += 1;
            i = skip_ws(body, i)?;
        }
    }
    None
}

pub fn array_iter<'a>(src: &'a [u8], arr: Span, mut f: impl FnMut(Span) -> bool) {
    debug_assert_eq!(arr.kind, Kind::Array);
    let body = &src[arr.start + 1..arr.end - 1];
    let mut i = 0;
    while let Some(start) = skip_ws(body, i) {
        let Some(s) = classify(body, start) else { break };
        if !f(Span {
            start: arr.start + 1 + s.start,
            end: arr.start + 1 + s.end,
            kind: s.kind,
        }) {
            return;
        }
        i = s.end;
        i = skip_ws(body, i).unwrap_or(i);
        if body.get(i) == Some(&b',') {
            i += 1;
        }
    }
}

pub fn as_str<'a>(src: &'a [u8], s: Span) -> Option<&'a str> {
    if s.kind != Kind::String {
        return None;
    }
    std::str::from_utf8(&src[s.start + 1..s.end - 1]).ok()
}

pub fn unquote(raw: &[u8]) -> io::Result<String> {
    debug_assert_eq!(raw.first(), Some(&b'"'));
    debug_assert_eq!(raw.last(), Some(&b'"'));
    let mut out = String::with_capacity(raw.len() - 2);
    let mut i = 0;
    let end = raw.len() - 1;
    let bytes = &raw[1..end];
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' {
            let esc = *bytes.get(i + 1).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "bad escape")
            })?;
            let ch = match esc {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'n' => '\n',
                b't' => '\t',
                b'r' => '\r',
                b'b' => '\u{08}',
                b'f' => '\u{0C}',
                b'u' => {
                    let hex = std::str::from_utf8(bytes.get(i + 2..i + 6).ok_or_else(
                        || io::Error::new(io::ErrorKind::InvalidData, "bad unicode"),
                    )?)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                    let cp = u32::from_str_radix(hex, 16).map_err(|e| {
                        io::Error::new(io::ErrorKind::InvalidData, e)
                    })?;
                    i += 4;
                    match char::from_u32(cp) {
                        Some(c) => c,
                        None => '\u{FFFD}',
                    }
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unknown escape",
                    ))
                }
            };
            out.push(ch);
            i += 2;
        } else {
            out.push(c as char);
            i += 1;
        }
    }
    Ok(out)
}

pub fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn parse_f64(s: &[u8]) -> Option<f64> {
    std::str::from_utf8(s).ok()?.parse().ok()
}
