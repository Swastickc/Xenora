// SPDX-License-Identifier: GPL-3.0-or-later

//! CSV parser for the `missing_imports.csv` produced by `xenora probe`.
//!
//! Format: `"module","symbol_or_ordinal","iat_rva"`
//! An optional `import_kind` column distinguishes numeric ordinals from names.

use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ImportRecord {
    /// DLL name in original casing, e.g. `"api-ms-win-crt-math-l1-1-0.dll"`
    pub module: String,
    /// Symbol name or `Ordinal#N`
    pub symbol: String,
    pub ordinal: Option<u16>,
    /// IAT RVA (informational only, not used for generation)
    pub iat_rva: u64,
}

/// Parse a CSV produced by `xenora probe`.
/// Returns records sorted by (module_lc, symbol).
pub fn parse_csv(path: &str) -> Result<Vec<ImportRecord>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let mut records = Vec::new();
    let mut kind_column = None;
    for (line_no, line) in content.lines().enumerate() {
        // Skip header (with or without quotes)
        let trimmed = line.trim();
        if line_no == 0 && (trimmed.starts_with("module") || trimmed.starts_with("\"module\"")) {
            kind_column = split_csv_line(trimmed)
                .iter()
                .position(|column| unquote(column) == "import_kind");
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = split_csv_line(line);
        if cols.len() < 2 {
            continue;
        }
        let module = unquote(cols[0]).into_owned();
        let symbol = unquote(cols[1]).into_owned();
        let ordinal = match kind_column {
            None => None,
            Some(index) => match cols.get(index).map(|field| unquote(field)).as_deref() {
                Some("name") => None,
                Some("ordinal") => Some(
                    symbol
                        .strip_prefix("Ordinal#")
                        .and_then(|value| value.parse().ok())
                        .ok_or_else(|| format!("{path}:{}: invalid ordinal", line_no + 1))?,
                ),
                _ => return Err(format!("{path}:{}: invalid import_kind", line_no + 1)),
            },
        };
        let iat_rva = if cols.len() >= 3 {
            u64::from_str_radix(unquote(cols[2]).trim_start_matches("0x"), 16).unwrap_or(0)
        } else {
            0
        };
        if !module.is_empty() && !symbol.is_empty() {
            records.push(ImportRecord {
                module,
                symbol,
                ordinal,
                iat_rva,
            });
        }
    }
    records.sort();
    records.dedup();
    Ok(records)
}

fn unquote(s: &str) -> Cow<'_, str> {
    let s = s.trim();
    if let Some(quoted) = s
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
    {
        if quoted.contains("\"\"") {
            return Cow::Owned(quoted.replace("\"\"", "\""));
        }
        Cow::Borrowed(quoted)
    } else {
        Cow::Borrowed(s)
    }
}

/// Split one probe record, honoring commas and doubled quotes inside quoted fields.
fn split_csv_line(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            // Find closing quote
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'"' {
                    i += 1;
                    if bytes.get(i) != Some(&b'"') {
                        break;
                    }
                }
                i += 1;
            }
            fields.push(&line[start..i]);
            // skip comma
            if i < bytes.len() && bytes[i] == b',' {
                i += 1;
            }
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            fields.push(&line[start..i]);
            if i < bytes.len() {
                i += 1;
            }
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_basic() {
        let cols = split_csv_line(r#""foo.dll","bar",0x1234"#);
        assert_eq!(cols, vec![r#""foo.dll""#, r#""bar""#, "0x1234"]);
    }

    #[test]
    fn probe_imports_round_trip_through_manifest_csv() {
        use crate::classify::SymbolDb;
        use crate::manifest::Manifest;

        let directory = std::env::temp_dir().join(format!(
            "xenora-import-csv-round-trip-{}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let probe = directory.join("missing_imports.csv");
        let generated = directory.join("generated_manifest.csv");
        std::fs::write(
            &probe,
            r#"module,symbol_or_ordinal,iat_rva,import_kind
"example,""module"".dll","Named""Export,with comma",0x1,name
"example.dll","Ordinal#7",0x2,name
"example.dll","Ordinal#7",0x3,ordinal
example.dll,literal""quotes,0x4,name
"#,
        )
        .unwrap();
        let records = parse_csv(probe.to_str().unwrap()).unwrap();
        Manifest::build(&records, &SymbolDb::builtin())
            .write_csv(generated.to_str().unwrap())
            .unwrap();
        let csv = std::fs::read_to_string(&generated).unwrap();
        let round_trip = parse_csv(generated.to_str().unwrap());
        std::fs::remove_dir_all(directory).unwrap();

        let record = |module: &str, symbol: &str, ordinal, iat_rva| ImportRecord {
            module: module.to_owned(),
            symbol: symbol.to_owned(),
            ordinal,
            iat_rva,
        };
        assert_eq!(
            records,
            [
                record(
                    r#"example,"module".dll"#,
                    r#"Named"Export,with comma"#,
                    None,
                    1
                ),
                record("example.dll", "Ordinal#7", None, 2),
                record("example.dll", "Ordinal#7", Some(7), 3),
                record("example.dll", r#"literal""quotes"#, None, 4),
            ]
        );
        assert!(csv.contains(r#""example,""module"".dll","Named""Export,with comma",0x00000001"#));
        assert_eq!(round_trip, Ok(records));
    }

    #[test]
    fn unquote_strips() {
        assert_eq!(unquote(r#""hello""#), "hello");
        assert_eq!(unquote("world"), "world");
    }
}
