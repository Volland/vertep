//! Escaping, character checks and source positions shared by the reader and the writer.

use quick_xml::Reader;
use quick_xml::events::Event;
use vertep_core::ir::Span;

/// Whether `c` matches the XML 1.0 `Char` production.
pub(crate) fn is_xml_char(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

/// Append `s` escaped as character data. `\r` is written as a character reference so that
/// line-ending normalization does not change it.
pub(crate) fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

/// Append `s` escaped for a double-quoted attribute value. Tab, newline and carriage return
/// are written as character references so that attribute-value normalization keeps them.
pub(crate) fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

/// Resolve a predefined entity name.
pub(crate) fn predefined_entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ => None,
    }
}

/// Whether `s` is a well-formed XML fragment (elements and text, balanced) containing at least
/// one element. The writer emits such strings as markup and everything else as escaped text.
pub(crate) fn looks_like_xml(s: &str) -> bool {
    if !s.contains('<') {
        return false;
    }
    let wrapped = format!("<w>{s}</w>");
    let mut reader = Reader::from_str(&wrapped);
    let mut depth = 0usize;
    let mut has_element = false;
    let mut closed = false;
    loop {
        let ev = match reader.read_event() {
            Ok(ev) => ev,
            Err(_) => return false,
        };
        if closed && !matches!(ev, Event::Eof) {
            return false;
        }
        match ev {
            Event::Start(_) => {
                depth += 1;
                if depth > 1 {
                    has_element = true;
                }
            }
            Event::Empty(_) => {
                if depth == 0 {
                    return false;
                }
                has_element = true;
            }
            Event::End(_) => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
                if depth == 0 {
                    closed = true;
                }
            }
            Event::GeneralRef(r) => {
                let ok = if r.is_char_ref() {
                    matches!(r.resolve_char_ref(), Ok(Some(_)))
                } else {
                    predefined_entity(&r).is_some()
                };
                if !ok {
                    return false;
                }
            }
            Event::Decl(_) | Event::DocType(_) => return false,
            Event::Eof => return closed && has_element,
            _ => {}
        }
    }
}

/// Maps byte offsets in a document to 1-based line and column spans
/// ([[openspec:scxml-format#Source Locations]]).
///
/// Columns count Unicode scalar values, not bytes. The reader attaches the span of every
/// element's `<` to the IR node it produces.
// @tg: implements:: [[openspec:scxml-format#Source Locations]]
#[derive(Clone, Debug)]
pub struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    /// Index the line starts of `text`.
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        LineIndex { text, starts }
    }

    /// The span of the byte at `offset` (clamped to the text).
    pub fn span(&self, offset: usize) -> Span {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.starts[line];
        let column = self.text[start..offset].chars().count() + 1;
        Span::new(
            u32::try_from(line + 1).unwrap_or(u32::MAX),
            u32::try_from(column).unwrap_or(u32::MAX),
        )
    }
}

/// Split an attribute value on XML whitespace.
pub(crate) fn split_ws(s: &str) -> Vec<String> {
    s.split([' ', '\t', '\n', '\r'])
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether `s` is empty or XML whitespace only.
pub(crate) fn is_ws(s: &str) -> bool {
    s.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

/// Whether `s` is an XML `NCName` (approximated with Unicode alphanumerics).
pub(crate) fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c == '-' || c == '.' || c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_counts_chars() {
        let idx = LineIndex::new("ab\nсд<x/>\n");
        let s = idx.span(7);
        assert_eq!((s.line, s.column), (2, 3));
        let s = idx.span(0);
        assert_eq!((s.line, s.column), (1, 1));
    }

    #[test]
    fn xml_detection() {
        assert!(looks_like_xml("<a/>"));
        assert!(looks_like_xml(" x <a b=\"1\">t</a> y"));
        assert!(!looks_like_xml("a < b"));
        assert!(!looks_like_xml("</w><x/><w>"));
        assert!(!looks_like_xml("plain"));
        assert!(!looks_like_xml("&unknown; <a/>"));
    }
}
