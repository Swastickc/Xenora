//! `xenora shaders`: instruction inventory and stream inspection.

use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use xenora_decoder::{Architecture, SEMANTIC_FORMS, decode_at};
use xenora_isa::meta::{self, EncodingFamily, FORMS, identify};

const USAGE: &str = "usage: xenora shaders <coverage [--json] | inspect <gfx1030.bin>>";
const MAX_SHADER_BYTES: usize = 1024 * 1024;
const TOO_LARGE: &str = "shader file exceeds the 1 MiB inspection limit";

pub fn command(args: &[String]) -> ExitCode {
    match args {
        [subcommand] if subcommand == "coverage" => coverage(false),
        [subcommand, option] if subcommand == "coverage" && option == "--json" => coverage(true),
        [subcommand, path] if subcommand == "inspect" => inspect(path),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn semantic(family: EncodingFamily) -> usize {
    SEMANTIC_FORMS
        .iter()
        .filter(|(_, form_family)| *form_family == family)
        .count()
}

/// Per-family counts. "Discovered" forms come from AMD's XML and are identified
/// with their length; "semantic" forms decode into executable instructions.
/// Reference, Metal and native GPU evidence per form stays in docs/status.toml.
fn coverage(json: bool) -> ExitCode {
    print!("{}", coverage_report(json));
    ExitCode::SUCCESS
}

fn coverage_report(json: bool) -> String {
    use std::fmt::Write as _;
    let rows: Vec<_> = meta::inventory()
        .into_iter()
        .map(|(family, discovered)| (family, discovered, semantic(family)))
        .collect();
    let total: usize = rows.iter().map(|row| row.1).sum();
    let supported: usize = rows.iter().map(|row| row.2).sum();
    let mut out = String::new();
    if json {
        let families: Vec<String> = rows
            .iter()
            .map(|(family, discovered, semantic)| {
                format!(
                    "{{\"family\":\"{}\",\"discovered\":{discovered},\"semantic\":{semantic}}}",
                    family.name()
                )
            })
            .collect();
        writeln!(
            out,
            "{{\"architecture\":\"gfx1030\",\"source_sha256\":\"{}\",\"discovered\":{total},\"semantic\":{supported},\"families\":[{}]}}",
            meta::SOURCE_SHA256,
            families.join(",")
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "{:<10} {:>10} {:>9}",
            "family", "discovered", "semantic"
        )
        .unwrap();
        for (family, discovered, semantic) in &rows {
            writeln!(out, "{:<10} {discovered:>10} {semantic:>9}", family.name()).unwrap();
        }
        writeln!(out, "{:<10} {total:>10} {supported:>9}", "total").unwrap();
        writeln!(
            out,
            "Semantic forms decode for the operand subsets in docs/status.toml only."
        )
        .unwrap();
    }
    out
}

fn require_regular(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular shader file",
        ))
    }
}

fn read_shader(path: &Path) -> io::Result<Vec<u8>> {
    require_regular(&fs::metadata(path)?)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;

        // Darwin sys/fcntl.h. The path could have become a FIFO since stat.
        const O_NONBLOCK: i32 = 0x00000004;
        options.custom_flags(O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    require_regular(&metadata)?;
    if metadata.len() > MAX_SHADER_BYTES as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, TOO_LARGE));
    }
    read_shader_bytes(file)
}

fn read_shader_bytes(input: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input
        .take((MAX_SHADER_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SHADER_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, TOO_LARGE));
    }
    Ok(bytes)
}

fn inspect(path: &str) -> ExitCode {
    let bytes = match read_shader(Path::new(path)) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let listing = listing(&bytes);
    for row in &listing.rows {
        println!("{row}");
    }
    if let Some(failure) = listing.failure {
        println!("{failure}");
        return ExitCode::FAILURE;
    }
    println!(
        "{} instructions inspected against {} gfx1030 forms; {} without semantic decoding",
        listing.rows.len(),
        FORMS.len(),
        listing.unsupported
    );
    ExitCode::SUCCESS
}

/// The printed rows of `xenora shaders inspect`, the number of rows that do
/// not decode into executable instructions and the identification failure
/// that ended the stream early, if any.
struct Listing {
    rows: Vec<String>,
    unsupported: usize,
    failure: Option<String>,
}

fn listing(bytes: &[u8]) -> Listing {
    let mut listing = Listing {
        rows: Vec::new(),
        unsupported: 0,
        failure: None,
    };
    // Identify every instruction first: a branch target is valid only if it
    // lands on one of these boundaries, which no single instruction can show.
    let mut forms = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        match identify(&bytes[offset..]) {
            Ok(found) => {
                forms.push((offset, found));
                offset += found.len;
            }
            Err(error) => {
                listing.failure = Some(format!("{offset:#06x}  decode failure: {error}"));
                break;
            }
        }
    }
    let boundary = |target: usize| forms.binary_search_by_key(&target, |form| form.0).is_ok();
    // Past the last identified instruction the boundaries are unknown.
    let identified = forms
        .last()
        .map_or(0, |&(offset, found)| offset + found.len);
    for &(offset, found) in &forms {
        let instruction = &bytes[offset..offset + found.len];
        let words: Vec<String> = instruction
            .chunks(4)
            .map(|chunk| format!("{:08x}", u32::from_le_bytes(chunk.try_into().unwrap())))
            .collect();
        let status = match decode_at(Architecture::Gfx1030, &bytes[..offset + found.len], offset) {
            Ok(decoded) => match decoded.branch_target() {
                Some(target) if listing.failure.is_some() && target >= identified => {
                    format!("branch target {target:#04x} unverified: stream not identified there")
                }
                Some(target) if !boundary(target) => {
                    format!("invalid branch target {target:#04x}")
                }
                _ => "semantic".to_owned(),
            },
            Err(error) if SEMANTIC_FORMS.contains(&(found.form.name, found.form.family)) => {
                format!("unsupported operands: {error}")
            }
            Err(_) => "unsupported instruction semantics".to_owned(),
        };
        if status != "semantic" {
            listing.unsupported += 1;
        }
        listing.rows.push(format!(
            "{offset:#06x}  {:<8} {:<28} {:<26} {status}",
            found.form.family.name(),
            found.form.name.to_ascii_lowercase(),
            words.join(" ")
        ));
    }
    listing
}

#[cfg(test)]
mod tests {
    use super::{MAX_SHADER_BYTES, TOO_LARGE, coverage_report, listing, read_shader_bytes};
    use std::io::{self, Read};
    use xenora_decoder::SEMANTIC_FORMS;
    use xenora_isa::meta::{EncodingFamily, FORMS};

    #[test]
    fn bounded_read_accepts_the_limit_and_rejects_more_bytes() {
        let exact = io::repeat(0x55).take(MAX_SHADER_BYTES as u64);
        let bytes = read_shader_bytes(exact).unwrap();
        assert_eq!(bytes.len(), MAX_SHADER_BYTES);
        assert!(bytes.iter().all(|&byte| byte == 0x55));

        let mut growing = io::repeat(0x55).take(MAX_SHADER_BYTES as u64 + 2);
        let error = read_shader_bytes(&mut growing).unwrap_err();
        assert_eq!(error.to_string(), TOO_LARGE);
        assert_eq!(growing.limit(), 1, "the read passed the one-byte sentinel");
    }

    #[test]
    fn coverage_totals_match_the_generated_table() {
        let json = coverage_report(true);
        assert!(json.starts_with("{\"architecture\":\"gfx1030\""), "{json}");
        assert!(json.ends_with("}]}\n"), "{json}");
        assert_eq!(json.matches('{').count(), json.matches('}').count());
        let totals = format!(
            "\"discovered\":{},\"semantic\":{},\"families\":[",
            FORMS.len(),
            SEMANTIC_FORMS.len()
        );
        assert!(json.contains(&totals), "{json}");
        let families: Vec<usize> = json
            .split("{\"family\":")
            .skip(1)
            .map(|row| {
                let count = row.split("\"discovered\":").nth(1).unwrap();
                count[..count.find(',').unwrap()].parse().unwrap()
            })
            .collect();
        assert_eq!(families.len(), EncodingFamily::ALL.len());
        assert_eq!(families.iter().sum::<usize>(), FORMS.len());
        let text = coverage_report(false);
        let total = format!(
            "{:<10} {:>10} {:>9}\n",
            "total",
            FORMS.len(),
            SEMANTIC_FORMS.len()
        );
        assert!(text.contains(&total), "{text}");
    }

    fn program(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn branches_in_a_valid_program_are_semantic() {
        let fixture = include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin");
        let listing = listing(fixture);
        assert_eq!(listing.failure, None);
        assert_eq!(listing.rows.len(), 15);
        let branches: Vec<_> = listing
            .rows
            .iter()
            .filter(|row| row.contains("branch"))
            .collect();
        assert_eq!(branches.len(), 3);
        for row in &branches {
            assert!(row.ends_with(" semantic"), "{row}");
        }
        assert_eq!(listing.unsupported, 0);
    }

    #[test]
    fn targets_past_an_identification_failure_are_unverified() {
        // s_branch 1 targets 0x08; 0xbfff0000 is not a gfx1030 form, so
        // nothing past 0x04 is identified; s_endpgm.
        let listing = listing(&program(&[0xbf82_0001, 0xbfff_0000, 0xbf81_0000]));
        assert!(listing.failure.as_deref().unwrap().starts_with("0x0004"));
        assert_eq!(listing.rows.len(), 1);
        assert!(
            listing.rows[0].ends_with("branch target 0x08 unverified: stream not identified there"),
            "{}",
            listing.rows[0]
        );
    }

    #[test]
    fn add_fixture_is_fully_semantic() {
        let listing = listing(include_bytes!("../../../fixtures/rdna2/add.bin"));
        assert_eq!(listing.failure, None);
        assert_eq!(listing.rows.len(), 7);
        assert_eq!(listing.unsupported, 0);
    }

    #[test]
    fn branch_targets_are_checked_against_the_whole_stream() {
        // s_branch 1 skips one word; s_cbranch_scc0 1 lands inside the
        // two-word v_mov_b32 literal form; s_endpgm.
        let listing = listing(&program(&[
            0xbf82_0001,
            0xbf84_0001,
            0x7e04_02ff,
            0x00ff_00ff,
            0xbf81_0000,
        ]));
        assert_eq!(listing.failure, None);
        assert!(
            listing.rows[0].ends_with(" semantic"),
            "{}",
            listing.rows[0]
        );
        assert!(
            listing.rows[1].ends_with("invalid branch target 0x0c"),
            "{}",
            listing.rows[1]
        );
        assert_eq!(listing.unsupported, 1);
    }
}
