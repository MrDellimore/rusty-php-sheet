//! Fixture generator.
//!
//! ```text
//! cargo run -p gen_fixture -- edge  [--out-dir tests/fixtures/edge]
//! cargo run -p gen_fixture -- large [--rows 400000] [--cols 50] [--out tests/fixtures/generated/large.xlsx]
//! ```

mod edge;
mod large;
mod xlsx;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  gen_fixture edge  [--out-dir DIR]                         (default tests/fixtures/edge)
  gen_fixture large [--rows N] [--cols N] [--out FILE]      (default 400000 rows, 50 cols,
                                                             tests/fixtures/generated/large-<rows>.xlsx)";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let (cmd, rest) = args.split_first().ok_or("missing command")?;
    let flags = parse_flags(rest)?;
    let flag = |name: &str| {
        flags
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };

    match cmd.as_str() {
        "edge" => {
            let dir = PathBuf::from(flag("out-dir").unwrap_or("tests/fixtures/edge"));
            edge::generate(&dir).map_err(|e| e.to_string())
        }
        "large" => {
            let rows = parse_num(flag("rows").unwrap_or("400000"), "rows")?;
            let cols = parse_num(flag("cols").unwrap_or("50"), "cols")?;
            if rows == 0 || cols == 0 {
                return Err("--rows and --cols must be at least 1".into());
            }
            let out = flag("out").map(PathBuf::from).unwrap_or_else(|| {
                PathBuf::from(format!("tests/fixtures/generated/large-{rows}.xlsx"))
            });
            large::generate(&out, &large::Options { rows, cols }).map_err(|e| e.to_string())
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

fn parse_flags(args: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let name = arg
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument `{arg}`"))?;
        let value = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
        out.push((name.to_string(), value.clone()));
    }
    Ok(out)
}

fn parse_num(s: &str, name: &str) -> Result<usize, String> {
    s.replace('_', "")
        .parse()
        .map_err(|_| format!("--{name} must be a whole number, got `{s}`"))
}
