//! Large synthetic workbook shaped like the real-world file that motivated this
//! project (~400k rows × 50 columns, two sheets, shared strings, date-formatted
//! columns). Entirely synthetic: no client data.
//!
//! Output is deterministic for a given (rows, cols), so benchmark numbers compare
//! like for like across machines and commits.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

use crate::xlsx::{self, BookSpec, Package, SheetEntry};

pub struct Options {
    pub rows: usize,
    pub cols: usize,
}

/// Style indexes into `cell_xfs` below.
const S_DATE: u8 = 1; // numFmtId 14 (built-in m/d/yyyy)
const S_DATETIME: u8 = 2; // numFmtId 164 (custom yyyy-mm-dd hh:mm:ss)
const S_MONEY: u8 = 3; // numFmtId 4 (#,##0.00), not a date

/// What each column holds, cycling every 10 columns.
#[derive(Clone, Copy)]
enum Kind {
    Id,         // row-based integer
    Status,     // ~20 repeated strings (shared)
    Name,       // ~5,000 repeated strings (shared)
    Money,      // float with 2 decimals, number format #,##0.00
    Date,       // integer serial, date format
    Reference,  // unique string per cell (grows the shared-string table)
    DateTime,   // fractional serial, datetime format
    Flag,       // boolean
    SparseInt,  // integer, 30% missing (gaps in the row)
    SparseText, // repeated text, 40% missing
}

const KINDS: [Kind; 10] = [
    Kind::Id,
    Kind::Status,
    Kind::Name,
    Kind::Money,
    Kind::Date,
    Kind::Reference,
    Kind::DateTime,
    Kind::Flag,
    Kind::SparseInt,
    Kind::SparseText,
];

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Id => "ID",
            Kind::Status => "Status",
            Kind::Name => "Name",
            Kind::Money => "Amount",
            Kind::Date => "Date",
            Kind::Reference => "Reference",
            Kind::DateTime => "Updated At",
            Kind::Flag => "Flag",
            Kind::SparseInt => "Count",
            Kind::SparseText => "Notes",
        }
    }
}

/// xorshift64*: tiny, fast, deterministic. Not for anything but test data.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Shared-string table built while streaming the sheets.
#[derive(Default)]
struct Sst {
    index: HashMap<String, u32>,
    items: Vec<String>,
}

impl Sst {
    fn id(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        let i = self.items.len() as u32;
        self.items.push(s.to_string());
        self.index.insert(s.to_string(), i);
        i
    }
}

struct Pools {
    statuses: Vec<String>,
    names: Vec<String>,
    notes: Vec<String>,
}

impl Pools {
    fn new() -> Self {
        let statuses = [
            "Open",
            "Closed",
            "Pending",
            "Approved",
            "Denied",
            "In Review",
            "Escalated",
            "On Hold",
            "Reopened",
            "Paid",
            "Partially Paid",
            "Void",
            "Submitted",
            "Draft",
            "Cancelled",
            "Matched",
            "Unmatched",
            "Duplicate",
            "Error",
            "Archived",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let first = [
            "Alex", "Blair", "Casey", "Drew", "Emery", "Finley", "Gray", "Harper", "Indy",
            "Jordan", "Kai", "Logan", "Morgan", "Noel", "Oakley", "Parker", "Quinn", "Reese",
            "Sage", "Taylor", "Uma", "Val", "Wren", "Xen", "Yael", "Zion", "Avery", "Brook",
            "Cameron", "Dakota", "Ellis", "Frankie", "Gale", "Hayden", "Islay", "Jesse", "Kendall",
            "Lane", "Marlow", "Nico", "Ocean", "Peyton", "Riley", "Shay", "Tatum", "Umber",
            "Vesper", "Winter", "Xan", "Yuri",
        ];
        let last = [
            "Abbott",
            "Barker",
            "Carver",
            "Dalton",
            "Easton",
            "Fisher",
            "Garner",
            "Hollis",
            "Ingram",
            "Jensen",
            "Keller",
            "Larkin",
            "Mercer",
            "Nolan",
            "Osborne",
            "Prescott",
            "Quill",
            "Ramsey",
            "Sutton",
            "Thorne",
            "Underwood",
            "Vance",
            "Whitaker",
            "Xiong",
            "Young",
            "Zeller",
            "Ashby",
            "Bishop",
            "Crane",
            "Dorsey",
            "Ellery",
            "Foley",
            "Garrity",
            "Hale",
            "Irwin",
            "Jarvis",
            "Kearney",
            "Lowell",
            "Maddox",
            "Nash",
            "Oakes",
            "Pruitt",
            "Quade",
            "Rowan",
            "Sloane",
            "Tierney",
            "Upton",
            "Voss",
            "Wilder",
            "Yates",
            "Zane",
            "Ainsley",
            "Bramble",
            "Colby",
            "Dunmore",
            "Everly",
            "Fenwick",
            "Gilliam",
            "Hargrove",
            "Ivers",
            "Jolley",
            "Kingsley",
            "Lindell",
            "Merritt",
            "Norwood",
            "Ogden",
            "Pembrook",
            "Quarles",
            "Radley",
            "Stanton",
            "Tolliver",
            "Ulrich",
            "Vickers",
            "Wolcott",
            "Yardley",
            "Zimmer",
            "Atwood",
            "Bellamy",
            "Calloway",
            "Delaney",
            "Eastwood",
            "Fairchild",
            "Goodwin",
            "Hensley",
            "Iverson",
            "Jameson",
            "Kirkland",
            "Lockhart",
            "Montague",
            "Northrop",
            "Ormsby",
            "Pendleton",
            "Quimby",
            "Rutherford",
            "Sinclair",
            "Thatcher",
            "Umstead",
            "Vanderbilt",
            "Westbrook",
            "Yarborough",
        ];
        let names = first
            .iter()
            .flat_map(|f| last.iter().map(move |l| format!("{f} {l}")))
            .collect();

        let words = [
            "follow up",
            "awaiting docs",
            "verified",
            "needs review",
            "called member",
            "left voicemail",
            "resubmitted",
            "per policy",
            "see attachment",
            "adjusted",
        ];
        let notes = (0..200)
            .map(|i| format!("{} #{}", words[i % words.len()], i))
            .collect();

        Pools {
            statuses,
            names,
            notes,
        }
    }
}

pub fn generate(out: &Path, opts: &Options) -> io::Result<()> {
    let started = Instant::now();
    let sheets = vec![
        SheetEntry::standard(1, "Detail"),
        SheetEntry::standard(2, "Summary"),
    ];
    let row_counts = [opts.rows, (opts.rows / 10).max(1)];

    let spec = BookSpec {
        sheets,
        has_shared_strings: true,
        num_fmts: vec![(164, "yyyy-mm-dd hh:mm:ss".into())],
        cell_xfs: vec![0, 14, 164, 4],
        date1904: false,
    };

    let mut pkg = Package::create(out)?;
    pkg.write_book_parts(&spec)?;

    let pools = Pools::new();
    let mut sst = Sst::default();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let last_col = xlsx::col_letters(opts.cols - 1);
    let mut total_cells = 0usize;

    for (sheet, &rows) in spec.sheets.iter().zip(&row_counts) {
        let dimension = format!("A1:{last_col}{}", rows + 1);
        let w = pkg.start_part(&sheet.part)?;
        w.write_all(xlsx::worksheet_open(Some(&dimension)).as_bytes())?;

        let mut buf = String::with_capacity(8 * 1024);
        let cols: Vec<(String, Kind)> = (0..opts.cols)
            .map(|c| (xlsx::col_letters(c), KINDS[c % KINDS.len()]))
            .collect();

        // Header row (shared strings, like Excel).
        buf.push_str("<row r=\"1\">");
        for (c, (letters, kind)) in cols.iter().enumerate() {
            let id = sst.id(&format!("{} {}", kind.label(), c + 1));
            let _ = write!(buf, "<c r=\"{letters}1\" t=\"s\"><v>{id}</v></c>");
        }
        buf.push_str("</row>");
        w.write_all(buf.as_bytes())?;

        for r in 2..=rows + 1 {
            buf.clear();
            let _ = write!(buf, "<row r=\"{r}\">");
            for (c, (letters, kind)) in cols.iter().enumerate() {
                let cell = format!("{letters}{r}");
                match kind {
                    Kind::Id => {
                        let _ = write!(buf, "<c r=\"{cell}\"><v>{}</v></c>", 100_000 + r);
                    }
                    Kind::Status => {
                        let s = &pools.statuses[rng.below(pools.statuses.len() as u64) as usize];
                        let _ = write!(buf, "<c r=\"{cell}\" t=\"s\"><v>{}</v></c>", sst.id(s));
                    }
                    Kind::Name => {
                        let s = &pools.names[rng.below(pools.names.len() as u64) as usize];
                        let _ = write!(buf, "<c r=\"{cell}\" t=\"s\"><v>{}</v></c>", sst.id(s));
                    }
                    Kind::Money => {
                        let cents = rng.below(10_000_000) as f64;
                        let _ = write!(
                            buf,
                            "<c r=\"{cell}\" s=\"{S_MONEY}\"><v>{}</v></c>",
                            cents / 100.0
                        );
                    }
                    Kind::Date => {
                        // 2015-01-01 (42005) .. ~2025-12-31
                        let serial = 42_005 + rng.below(4_018);
                        let _ = write!(buf, "<c r=\"{cell}\" s=\"{S_DATE}\"><v>{serial}</v></c>");
                    }
                    Kind::Reference => {
                        let s = format!("REF-{r:07}-{c:02}");
                        let _ = write!(buf, "<c r=\"{cell}\" t=\"s\"><v>{}</v></c>", sst.id(&s));
                    }
                    Kind::DateTime => {
                        let day = 42_005 + rng.below(4_018);
                        let secs = rng.below(86_400);
                        let serial = day as f64 + secs as f64 / 86_400.0;
                        let _ = write!(
                            buf,
                            "<c r=\"{cell}\" s=\"{S_DATETIME}\"><v>{serial}</v></c>"
                        );
                    }
                    Kind::Flag => {
                        let _ = write!(buf, "<c r=\"{cell}\" t=\"b\"><v>{}</v></c>", rng.below(2));
                    }
                    Kind::SparseInt => {
                        if !rng.chance(30) {
                            let _ = write!(buf, "<c r=\"{cell}\"><v>{}</v></c>", rng.below(500));
                        }
                    }
                    Kind::SparseText => {
                        if !rng.chance(40) {
                            let s = &pools.notes[rng.below(pools.notes.len() as u64) as usize];
                            let _ = write!(buf, "<c r=\"{cell}\" t=\"s\"><v>{}</v></c>", sst.id(s));
                        }
                    }
                }
                total_cells += 1;
            }
            buf.push_str("</row>");
            w.write_all(buf.as_bytes())?;
        }
        w.write_all(xlsx::WORKSHEET_CLOSE.as_bytes())?;
        eprintln!("  sheet {:?}: {} data rows", sheet.name, rows);
    }

    let unique = sst.items.len();
    let items: Vec<String> = sst
        .items
        .into_iter()
        .map(|s| format!("<t>{}</t>", xlsx::xml_escape(&s)))
        .collect();
    pkg.write_part("xl/sharedStrings.xml", &xlsx::shared_strings_xml(&items))?;
    pkg.finish()?;

    let size = std::fs::metadata(out)?.len();
    eprintln!(
        "wrote {} ({:.1} MB): {} cells, {} unique shared strings, in {:.1}s",
        out.display(),
        size as f64 / 1_048_576.0,
        total_cells,
        unique,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
