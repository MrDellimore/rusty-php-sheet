//! Small helpers over quick-xml shared by the XLSX part parsers.

use std::borrow::Cow;
use std::io::BufRead;

use quick_xml::Reader;
use quick_xml::events::{BytesRef, BytesStart, BytesText};

use crate::Error;

/// Buffer size for the decompressed XML stream.
pub const XML_BUF: usize = 128 * 1024;

pub fn reader<R: BufRead>(inner: R) -> Reader<R> {
    let mut reader = Reader::from_reader(inner);
    let config = reader.config_mut();
    config.trim_text(false);
    // We track structure ourselves; skipping end-name checks is measurably faster.
    config.check_end_names = false;
    reader
}

pub fn xml_error(part: &str) -> impl Fn(quick_xml::Error) -> Error + '_ {
    move |source| Error::Xml {
        part: part.to_string(),
        source,
    }
}

/// Raw (still escaped) attribute value by local name, ignoring any namespace prefix.
/// Fine for attributes that never contain escapes (`r`, `t`, `s`, ids).
pub fn attr<'a>(e: &'a BytesStart<'_>, local: &str) -> Option<Cow<'a, str>> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == local)
        .map(|a| a.value)
}

/// Attribute value with XML escapes decoded (`&amp;` → `&`). Use for names and targets.
pub fn attr_unescaped(e: &BytesStart<'_>, local: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == local)
        .map(
            |a| match a.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
                Ok(v) => v.into_owned(),
                Err(_) => a.value.into_owned(),
            },
        )
}

/// Attribute by local name that also carries a namespace prefix, e.g. `r:id`.
pub fn prefixed_attr(e: &BytesStart<'_>, local: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.prefix().is_some() && a.key.local_name().as_ref() == local)
        .map(|a| a.value.into_owned())
}

pub fn local_name<'a>(e: &'a BytesStart<'_>) -> &'a str {
    let name = e.name().0;
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

pub fn end_local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

/// Append character data, with XML 1.0 end-of-line normalisation (as libxml does).
pub fn push_text(out: &mut String, text: &BytesText<'_>) {
    out.push_str(&text.xml10_content());
}

/// Append a general reference (`&amp;`, `&#169;`). Unknown named entities are kept verbatim.
pub fn push_ref(out: &mut String, r: &BytesRef<'_>) {
    match r.resolve_char_ref() {
        Ok(Some(ch)) => out.push(ch),
        _ => match &**r {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            other => {
                out.push('&');
                out.push_str(other);
                out.push(';');
            }
        },
    }
}

/// PHP's `trim()` character set: space, \t, \n, \r, \0, \x0B.
pub fn php_trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r', '\0', '\x0B'])
}
