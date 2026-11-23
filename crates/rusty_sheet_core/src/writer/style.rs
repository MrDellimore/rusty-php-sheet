//! Cell styles and `styles.xml`, mirroring OpenSpout's XLSX `StyleRegistry` and
//! `StyleManager` so the output is byte-identical:
//! - styles are identified by handle (OpenSpout: object identity), and each gets
//!   its own font and `<xf>`, numbered in order of **first use**;
//! - fills are shared per background colour, number formats per format code
//!   (built-in codes keep their built-in id, others count up from 164);
//! - style 0 is the fallback style: Calibri 12 (`Writer\XLSX\Options::FALLBACK_STYLE`);
//! - a string containing "\n" gets its style's wrap-text variant, registered once
//!   per style (OpenSpout 5.15 `getWrapTextStyleId`).
//!
//! Borders are not supported (callers fall back to OpenSpout for them).

use std::collections::HashMap;

use super::escape::escape;

/// A cell style. Defaults are OpenSpout's `Style` defaults (Arial 11, black).
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub font_size: i64,
    /// `RRGGBB`.
    pub font_color: String,
    pub font_name: String,
    /// OpenSpout `CellAlignment` value: left, right, center or justify.
    pub horizontal: Option<String>,
    /// OpenSpout `CellVerticalAlignment` value: auto, baseline, bottom, center,
    /// distributed, justify or top.
    pub vertical: Option<String>,
    pub wrap_text: Option<bool>,
    pub text_rotation: i64,
    pub shrink_to_fit: Option<bool>,
    /// `RRGGBB`, written as given.
    pub background: Option<String>,
    pub format: Option<String>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            font_size: 11,
            font_color: "000000".into(),
            font_name: "Arial".into(),
            horizontal: None,
            vertical: None,
            wrap_text: None,
            text_rotation: 0,
            shrink_to_fit: None,
            background: None,
            format: None,
        }
    }
}

impl Style {
    /// OpenSpout's XLSX `FALLBACK_STYLE` (style id 0).
    pub fn fallback() -> Self {
        Style {
            font_size: 12,
            font_name: "Calibri".into(),
            ..Style::default()
        }
    }

    /// `Style::$shouldApplyFont`: anything differs from the default font.
    fn should_apply_font(&self) -> bool {
        let d = Style::default();
        self.bold
            || self.italic
            || self.underline
            || self.strikethrough
            || self.font_size != d.font_size
            || self.font_color != d.font_color
            || self.font_name != d.font_name
    }

    fn has_alignment(&self) -> bool {
        self.horizontal.is_some()
            || self.vertical.is_some()
            || self.wrap_text.is_some()
            || self.shrink_to_fit.is_some()
            || self.text_rotation != 0
    }
}

/// Built-in number formats (OpenSpout's `builtinNumFormatToIdMapping`).
const BUILTIN_FORMATS: &[(&str, u32)] = &[
    ("General", 0),
    ("0", 1),
    ("0.00", 2),
    ("#,##0", 3),
    ("#,##0.00", 4),
    ("$#,##0,\\-$#,##0", 5),
    ("$#,##0,[Red]\\-$#,##0", 6),
    ("$#,##0.00,\\-$#,##0.00", 7),
    ("$#,##0.00,[Red]\\-$#,##0.00", 8),
    ("0%", 9),
    ("0.00%", 10),
    ("0.00E+00", 11),
    ("# ?/?", 12),
    ("# ??/??", 13),
    ("mm-dd-yy", 14),
    ("d-mmm-yy", 15),
    ("d-mmm", 16),
    ("mmm-yy", 17),
    ("h:mm AM/PM", 18),
    ("h:mm:ss AM/PM", 19),
    ("h:mm", 20),
    ("h:mm:ss", 21),
    ("m/d/yy h:mm", 22),
    ("#,##0 ,(#,##0)", 37),
    ("#,##0 ,[Red](#,##0)", 38),
    ("#,##0.00,(#,##0.00)", 39),
    ("#,##0.00,[Red](#,##0.00)", 40),
    (
        "_(\"$\"* #,##0.00_),_(\"$\"* \\(#,##0.00\\),_(\"$\"* \"-\"??_),_(@_)",
        44,
    ),
    ("mm:ss", 45),
    ("[h]:mm:ss", 46),
    ("mm:ss.0", 47),
    ("##0.0E+0", 48),
    ("@", 49),
    ("[$-404]e/m/d", 27),
    ("m/d/yy", 30),
    ("t0", 59),
    ("t0.00", 60),
    ("t#,##0", 61),
    ("t#,##0.00", 62),
    ("t0%", 67),
    ("t0.00%", 68),
    ("t# ?/?", 69),
    ("t# ??/??", 70),
];

/// Registered styles, in xf order.
#[derive(Debug)]
pub struct StyleRegistry {
    styles: Vec<Style>,
    /// Caller's style handle → xf id.
    handle_to_id: HashMap<usize, u32>,
    format_of: Vec<u32>,
    fill_of: Vec<u32>,
    /// Format code → numFmtId, in registration order.
    formats: Vec<(String, u32)>,
    next_format_id: u32,
    /// Background colour → fill id, in registration order.
    fills: Vec<(String, u32)>,
    next_fill_id: u32,
    /// Style id → its wrap-text variant.
    wrap_of: HashMap<u32, u32>,
}

impl Default for StyleRegistry {
    fn default() -> Self {
        let mut r = StyleRegistry {
            styles: Vec::new(),
            handle_to_id: HashMap::new(),
            format_of: Vec::new(),
            fill_of: Vec::new(),
            formats: Vec::new(),
            next_format_id: 164,
            fills: Vec::new(),
            next_fill_id: 2,
            wrap_of: HashMap::new(),
        };
        r.push(Style::fallback());
        r
    }
}

impl StyleRegistry {
    /// The xf id for `handle`, registering `style` on first use.
    pub fn id_for(&mut self, handle: usize, style: &Style) -> u32 {
        if let Some(&id) = self.handle_to_id.get(&handle) {
            return id;
        }
        let id = self.push(style.clone());
        self.handle_to_id.insert(handle, id);
        id
    }

    /// The style id for a multiline string in style `id`: the same style with wrap
    /// text on, unless the style sets wrap text itself (either way).
    pub fn wrap_text_id(&mut self, id: u32) -> u32 {
        if let Some(&w) = self.wrap_of.get(&id) {
            return w;
        }
        let style = &self.styles[id as usize];
        let w = if style.wrap_text.is_none() {
            let wrapped = Style {
                wrap_text: Some(true),
                ..style.clone()
            };
            self.push(wrapped)
        } else {
            id
        };
        self.wrap_of.insert(id, w);
        w
    }

    fn push(&mut self, style: Style) -> u32 {
        let id = self.styles.len() as u32;
        let fill = match &style.background {
            None => 0,
            Some(color) => match self.fills.iter().find(|(c, _)| c == color) {
                Some(&(_, f)) => f,
                None => {
                    let f = self.next_fill_id;
                    self.next_fill_id += 1;
                    self.fills.push((color.clone(), f));
                    f
                }
            },
        };
        let format = match &style.format {
            None => 0,
            Some(code) => match self.formats.iter().find(|(c, _)| c == code) {
                Some(&(_, f)) => f,
                None => {
                    let f = BUILTIN_FORMATS
                        .iter()
                        .find(|(c, _)| c == code)
                        .map(|&(_, f)| f)
                        .unwrap_or_else(|| {
                            let f = self.next_format_id;
                            self.next_format_id += 1;
                            f
                        });
                    self.formats.push((code.clone(), f));
                    f
                }
            },
        };
        self.styles.push(style);
        self.fill_of.push(fill);
        self.format_of.push(format);
        id
    }

    /// `shouldApplyStyleOnEmptyCell`: empty cells are only written when their style
    /// has a fill or a number format.
    pub fn applies_to_empty_cell(&self, id: u32) -> bool {
        let i = id as usize;
        self.fill_of[i] != 0 || self.format_of[i] != 0
    }

    /// The style registered as `id`.
    pub fn style(&self, id: u32) -> &Style {
        &self.styles[id as usize]
    }

    /// `StyleManager::getStylesXMLFileContent`.
    pub fn styles_xml(&self) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
        );

        let custom: Vec<&(String, u32)> =
            self.formats.iter().filter(|(_, id)| *id >= 164).collect();
        s.push_str(&format!("<numFmts count=\"{}\">", custom.len()));
        for (code, id) in custom {
            s.push_str(&format!(
                "<numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>",
                escape(code)
            ));
        }
        s.push_str("</numFmts>");

        s.push_str(&format!("<fonts count=\"{}\">", self.styles.len()));
        for st in &self.styles {
            // OpenSpout writes these unescaped; escaping keeps the XML valid for odd
            // values and is identical for real font names and RRGGBB colours.
            s.push_str(&format!(
                "<font><sz val=\"{}\"/><color rgb=\"FF{}\"/><name val=\"{}\"/>",
                st.font_size,
                escape(&st.font_color),
                escape(&st.font_name)
            ));
            if st.bold {
                s.push_str("<b/>");
            }
            if st.italic {
                s.push_str("<i/>");
            }
            if st.underline {
                s.push_str("<u/>");
            }
            if st.strikethrough {
                s.push_str("<strike/>");
            }
            s.push_str("</font>");
        }
        s.push_str("</fonts>");

        s.push_str(&format!("<fills count=\"{}\">", self.fills.len() + 2));
        s.push_str("<fill><patternFill patternType=\"none\"/></fill>");
        s.push_str("<fill><patternFill patternType=\"gray125\"/></fill>");
        for (color, _) in &self.fills {
            s.push_str(&format!(
                "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"{}\"/></patternFill></fill>",
                escape(color)
            ));
        }
        s.push_str("</fills>");

        s.push_str(
            "<borders count=\"1\"><border><left/><right/><top/><bottom/></border></borders>\
             <cellStyleXfs count=\"1\">\n    <xf borderId=\"0\" fillId=\"0\" fontId=\"0\" numFmtId=\"0\"/>\n</cellStyleXfs>",
        );

        s.push_str(&format!("<cellXfs count=\"{}\">", self.styles.len()));
        for (id, st) in self.styles.iter().enumerate() {
            let (fill, fmt) = if id == 0 {
                (0, 0)
            } else {
                (self.fill_of[id], self.format_of[id])
            };
            s.push_str(&format!(
                "<xf numFmtId=\"{fmt}\" fontId=\"{id}\" fillId=\"{fill}\" borderId=\"0\" xfId=\"0\""
            ));
            if st.should_apply_font() {
                s.push_str(" applyFont=\"1\"");
            }
            s.push_str(" applyBorder=\"0\"");
            if st.has_alignment() {
                s.push_str(" applyAlignment=\"1\"><alignment");
                if let Some(h) = &st.horizontal {
                    s.push_str(&format!(" horizontal=\"{h}\""));
                }
                if let Some(v) = &st.vertical {
                    s.push_str(&format!(" vertical=\"{v}\""));
                }
                if let Some(w) = st.wrap_text {
                    s.push_str(&format!(" wrapText=\"{}\"", u8::from(w)));
                }
                if st.shrink_to_fit == Some(true) {
                    s.push_str(" shrinkToFit=\"true\"");
                }
                if st.text_rotation != 0 {
                    s.push_str(&format!(" textRotation=\"{}\"", st.text_rotation));
                }
                s.push_str("/></xf>");
            } else {
                s.push_str("/>");
            }
        }
        s.push_str("</cellXfs>");
        s.push_str(
            "<cellStyles count=\"1\">\n    <cellStyle builtinId=\"0\" name=\"Normal\" xfId=\"0\"/>\n</cellStyles></styleSheet>",
        );
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_first_use_and_dedupe_fills_and_formats() {
        let mut r = StyleRegistry::default();
        let money = Style {
            format: Some("#,##0.00".into()),
            ..Style::default()
        };
        let date = Style {
            format: Some("yyyy-mm-dd".into()),
            background: Some("FFFF00".into()),
            ..Style::default()
        };
        assert_eq!(r.id_for(10, &date), 1);
        assert_eq!(r.id_for(20, &money), 2);
        assert_eq!(r.id_for(10, &date), 1);
        // A different handle with identical content is a new style (object identity).
        assert_eq!(r.id_for(30, &date), 3);
        assert_eq!(r.format_of, [0, 164, 4, 164]);
        assert_eq!(r.fill_of, [0, 2, 0, 2]);
        assert!(r.applies_to_empty_cell(1));
        assert!(!r.applies_to_empty_cell(0));
    }
}
