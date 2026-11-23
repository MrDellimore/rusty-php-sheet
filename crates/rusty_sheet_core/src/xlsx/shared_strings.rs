//! `xl/sharedStrings.xml` → a compact in-memory string table.
//!
//! This is the component that makes or breaks large-file performance. OpenSpout
//! budgets 12 KB per string and falls back to temp files, which collapses
//! throughput (see `docs/parity-notes.md`). We store every string in **one**
//! `String` buffer plus a `u32` end offset per string: about `len + 4` bytes each,
//! and two allocations in total regardless of string count.
//!
//! Decoding follows OpenSpout (parity):
//! - only `<t>` elements whose parent is `<si>` or `<r>` count, so phonetic
//!   (`<rPh>`) runs are excluded;
//! - each `<t>` is PHP-`trim()`med unless it has `xml:space="preserve"`;
//! - `_xHHHH_` escapes are decoded as OpenSpout's `Escaper\XLSX::unescape` does
//!   (control characters only, so e.g. `_x000D_` stays literal).

use std::io::BufReader;

use quick_xml::events::Event;

use super::package::Package;
use super::unescape::unescape;
use super::xml;
use crate::{Error, Result};

#[derive(Debug, Default)]
pub struct SharedStrings {
    buf: String,
    /// `ends[i]` is the end offset of string `i` in `buf`; it starts where `i - 1` ends.
    ends: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tag {
    Si,
    R,
    T,
    Other,
}

impl SharedStrings {
    pub fn read(pkg: &Package, part: &str) -> Result<Self> {
        let stream = BufReader::with_capacity(xml::XML_BUF, pkg.open_part(part)?);
        let mut reader = xml::reader(stream);
        let mut ev = Vec::with_capacity(4096);
        let mut table = SharedStrings::default();

        let mut stack: Vec<Tag> = Vec::with_capacity(8);
        let mut item = String::new(); // the <si> being built
        let mut text = String::new(); // the <t> being read
        let mut collecting = false; // inside a <t> that counts
        let mut preserve = false;

        loop {
            match reader
                .read_event_into(&mut ev)
                .map_err(xml::xml_error(part))?
            {
                Event::Start(e) => {
                    let tag = match xml::local_name(&e) {
                        "sst" => {
                            if let Some(n) =
                                xml::attr(&e, "uniqueCount").and_then(|v| v.parse::<usize>().ok())
                            {
                                // Trust the hint only up to a sane bound.
                                table.ends.reserve(n.min(10_000_000));
                            }
                            Tag::Other
                        }
                        "si" => {
                            item.clear();
                            Tag::Si
                        }
                        "r" => Tag::R,
                        "t" => {
                            collecting = matches!(stack.last(), Some(Tag::Si | Tag::R));
                            preserve = xml::attr(&e, "space").as_deref() == Some("preserve");
                            text.clear();
                            Tag::T
                        }
                        _ => Tag::Other,
                    };
                    stack.push(tag);
                }
                Event::Empty(e) => {
                    // <si/> is an empty string; an empty <t/> adds nothing.
                    if xml::local_name(&e) == "si" {
                        table.push("")?;
                    }
                }
                Event::Text(t) if collecting => xml::push_text(&mut text, &t),
                Event::GeneralRef(r) if collecting => xml::push_ref(&mut text, &r),
                Event::CData(c) if collecting => text.push_str(&c),
                Event::End(_) => match stack.pop() {
                    Some(Tag::T) => {
                        if collecting {
                            item.push_str(if preserve {
                                &text
                            } else {
                                xml::php_trim(&text)
                            });
                        }
                        collecting = false;
                    }
                    Some(Tag::Si) => table.push(&unescape(&item))?,
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
            ev.clear();
        }
        table.buf.shrink_to_fit();
        table.ends.shrink_to_fit();
        Ok(table)
    }

    fn push(&mut self, s: &str) -> Result<()> {
        self.buf.push_str(s);
        let end = u32::try_from(self.buf.len()).map_err(|_| Error::InvalidData {
            part: "sharedStrings.xml".into(),
            message: "shared strings exceed 4 GiB".into(),
        })?;
        self.ends.push(end);
        Ok(())
    }

    #[inline]
    pub fn get(&self, index: u32) -> Option<&str> {
        let i = index as usize;
        let end = *self.ends.get(i)? as usize;
        let start = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        // Offsets are always pushed at char boundaries, so slicing can't panic.
        Some(&self.buf[start..end])
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Approximate heap usage, for diagnostics and benchmarks.
    pub fn heap_bytes(&self) -> usize {
        self.buf.capacity() + self.ends.capacity() * std::mem::size_of::<u32>()
    }

    #[cfg(test)]
    pub(crate) fn from_strs(items: &[&str]) -> Self {
        let mut t = SharedStrings::default();
        for s in items {
            t.push(s).unwrap();
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_lookup() {
        let t = SharedStrings::from_strs(&["a", "", "café", "zz"]);
        assert_eq!(t.get(0), Some("a"));
        assert_eq!(t.get(1), Some(""));
        assert_eq!(t.get(2), Some("café"));
        assert_eq!(t.get(3), Some("zz"));
        assert_eq!(t.get(4), None);
        assert_eq!(t.len(), 4);
    }
}
