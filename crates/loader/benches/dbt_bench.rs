// SPDX-License-Identifier: GPL-3.0-or-later

//! Synthetic interpreter and DBT timings.
//!
//! Set `XENORA_DBT_BENCH_JSON=<path>` to also write the results as one JSON
//! document (format version 1). The text output does not change. Values the
//! benchmark cannot determine, such as the git revision outside a checkout,
//! are written as `null`.
//!
//! The benchmark target disables Cargo's test harness. To run the percentile
//! unit tests, build with
//! `cargo rustc --locked -p xenora-loader --bench dbt_bench --message-format=json -- --test`
//! and run the executable named by the benchmark's compiler-artifact message.

use std::hint::black_box;
use std::time::{Duration, Instant};
use xenora_loader::cpu::{CpuState, ExecutionResult, ExitReason, Interpreter, Register};
use xenora_loader::dbt::{DbtEngine, DbtMetrics};
use xenora_loader::memory::MemoryPermissions;
use xenora_loader::pe::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_SCN_MEM_EXECUTE, IMAGE_SCN_MEM_READ, PE32_PLUS_MAGIC, ParsedPe,
};

fn build_pe(code: &[u8]) -> Vec<u8> {
    let mut pe = Vec::new();
    pe.extend_from_slice(b"MZ");
    pe.resize(0x3C, 0);
    let e_lfanew: u32 = 0x80;
    pe.extend_from_slice(&e_lfanew.to_le_bytes());
    pe.resize(0x80, 0);
    pe.extend_from_slice(b"PE\0\0");
    pe.extend_from_slice(&IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
    let num_sections: u16 = 1;
    pe.extend_from_slice(&num_sections.to_le_bytes());
    pe.extend_from_slice(&0x67300000u32.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    let opt_hdr_size: u16 = 112;
    pe.extend_from_slice(&opt_hdr_size.to_le_bytes());
    pe.extend_from_slice(&0x0022u16.to_le_bytes());
    pe.extend_from_slice(&PE32_PLUS_MAGIC.to_le_bytes());
    pe.push(14);
    pe.push(0);
    pe.extend_from_slice(&(code.len() as u32).to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    let entry_point_rva: u32 = 0x1000;
    pe.extend_from_slice(&entry_point_rva.to_le_bytes());
    pe.extend_from_slice(&0x1000u32.to_le_bytes());
    let image_base: u64 = 0x0000000140000000;
    pe.extend_from_slice(&image_base.to_le_bytes());
    pe.extend_from_slice(&0x1000u32.to_le_bytes());
    pe.extend_from_slice(&0x200u32.to_le_bytes());
    pe.extend_from_slice(&6u16.to_le_bytes());
    pe.extend_from_slice(&0u16.to_le_bytes());
    pe.extend_from_slice(&0u16.to_le_bytes());
    pe.extend_from_slice(&0u16.to_le_bytes());
    pe.extend_from_slice(&6u16.to_le_bytes());
    pe.extend_from_slice(&0u16.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    let size_of_image = 0x1000 + ((code.len() as u32 + 0xfff) & !0xfff);
    pe.extend_from_slice(&size_of_image.to_le_bytes());
    pe.extend_from_slice(&0x200u32.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    pe.extend_from_slice(&3u16.to_le_bytes());
    pe.extend_from_slice(&0x8160u16.to_le_bytes());
    pe.extend_from_slice(&0x100000u64.to_le_bytes());
    pe.extend_from_slice(&0x1000u64.to_le_bytes());
    pe.extend_from_slice(&0x100000u64.to_le_bytes());
    pe.extend_from_slice(&0x1000u64.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    let mut sec_name = [0u8; 8];
    sec_name[..5].copy_from_slice(b".text");
    pe.extend_from_slice(&sec_name);
    pe.extend_from_slice(&(code.len() as u32).to_le_bytes());
    pe.extend_from_slice(&0x1000u32.to_le_bytes());
    let raw_size = (code.len() as u32 + 0x1ff) & !0x1ff;
    pe.extend_from_slice(&raw_size.to_le_bytes());
    pe.extend_from_slice(&0x200u32.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes());
    pe.extend_from_slice(&0u16.to_le_bytes());
    pe.extend_from_slice(&0u16.to_le_bytes());
    pe.extend_from_slice(&(IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_EXECUTE).to_le_bytes());
    pe.resize(0x200, 0);
    pe.extend_from_slice(code);
    pe.resize(0x200 + raw_size as usize, 0);
    pe
}

const ADDS: usize = 10_000;
const INSTRUCTIONS: usize = ADDS + 2;
const BUDGET: usize = 20_000;
const STACK_TOP: u64 = 0x2000_8000;

fn check(result: ExecutionResult) {
    assert_eq!(result.exit_reason, ExitReason::Halt);
    assert_eq!(result.instructions_executed, INSTRUCTIONS);
    assert_eq!(result.final_state.get(Register::Rax), ADDS as u64);
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    // Interpolate between adjacent values at rank (n - 1) * fraction.
    let rank = (sorted.len() - 1) as f64 * fraction;
    let lower = sorted[rank.floor() as usize];
    let upper = sorted[rank.ceil() as usize];
    lower + (upper - lower).mul_f64(rank.fract())
}

/// Order statistics of one timing series, shared by the text and JSON reports.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Summary {
    samples: usize,
    median: Duration,
    min: Duration,
    max: Duration,
    p95: Duration,
    p99: Duration,
    iqr: Duration,
    mips: f64,
}

fn summarize(samples: &mut [Duration]) -> Summary {
    samples.sort_unstable();
    let median = percentile(samples, 0.50);
    Summary {
        samples: samples.len(),
        median,
        min: samples[0],
        max: samples[samples.len() - 1],
        p95: percentile(samples, 0.95),
        p99: percentile(samples, 0.99),
        iqr: percentile(samples, 0.75) - percentile(samples, 0.25),
        mips: INSTRUCTIONS as f64 / median.as_secs_f64() / 1_000_000.0,
    }
}

fn report_line(label: &str, s: &Summary) -> String {
    format!(
        "{label}: {} samples, median {:?}, min {:?}, max {:?}, p95 {:?}, p99 {:?}, IQR {:?}, median {:.2} MIPS",
        s.samples, s.median, s.min, s.max, s.p95, s.p99, s.iqr, s.mips
    )
}

/// Facts about the run that are not timings. `None` means the benchmark could not
/// determine the value.
struct RunInfo {
    unix_time: Option<u64>,
    cpu: Option<String>,
    os: &'static str,
    os_version: Option<String>,
    arch: &'static str,
    revision: Option<String>,
    dirty: Option<bool>,
    debug_assertions: bool,
    guest_code_bytes: usize,
    warmup: usize,
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn run_info(guest_code_bytes: usize, warmup: usize) -> RunInfo {
    let non_empty = |value: Option<String>| value.filter(|v| !v.is_empty());
    RunInfo {
        unix_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs()),
        cpu: non_empty(command_output(
            "sysctl",
            &["-n", "machdep.cpu.brand_string"],
        )),
        os: std::env::consts::OS,
        os_version: non_empty(command_output("sw_vers", &["-productVersion"])),
        arch: std::env::consts::ARCH,
        revision: non_empty(command_output("git", &["rev-parse", "HEAD"])),
        dirty: command_output("git", &["status", "--porcelain", "--untracked-files=no"])
            .map(|status| !status.is_empty()),
        debug_assertions: cfg!(debug_assertions),
        guest_code_bytes,
        warmup,
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_option<T>(value: Option<T>, render: impl FnOnce(T) -> String) -> String {
    value.map_or_else(|| "null".to_owned(), render)
}

/// JSON has no infinity or NaN, so a non-finite rate (a zero median) is `null`.
fn json_f64(value: f64) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "null".to_owned()
    }
}

fn json_object(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(key, value)| format!("{}:{value}", json_string(key)))
        .collect();
    format!("{{{}}}", body.join(","))
}

fn json_summary(s: &Summary) -> String {
    json_object(&[
        ("samples", s.samples.to_string()),
        ("median_ns", s.median.as_nanos().to_string()),
        ("min_ns", s.min.as_nanos().to_string()),
        ("max_ns", s.max.as_nanos().to_string()),
        ("p95_ns", s.p95.as_nanos().to_string()),
        ("p99_ns", s.p99.as_nanos().to_string()),
        ("iqr_ns", s.iqr.as_nanos().to_string()),
        ("median_mips", json_f64(s.mips)),
    ])
}

fn json_document(
    info: &RunInfo,
    interpreter: &Summary,
    cold: &Summary,
    warm: &Summary,
    metrics: &DbtMetrics,
) -> String {
    let string = |value: &String| json_string(value);
    json_object(&[
        ("format_version", "1".to_owned()),
        ("benchmark", json_string("dbt_bench")),
        ("unix_time", json_option(info.unix_time, |t| t.to_string())),
        (
            "host",
            json_object(&[
                ("cpu", json_option(info.cpu.as_ref(), string)),
                ("os", json_string(info.os)),
                ("os_version", json_option(info.os_version.as_ref(), string)),
                ("arch", json_string(info.arch)),
            ]),
        ),
        (
            "source",
            json_object(&[
                ("revision", json_option(info.revision.as_ref(), string)),
                ("dirty", json_option(info.dirty, |d| d.to_string())),
            ]),
        ),
        (
            "build",
            json_object(&[("debug_assertions", info.debug_assertions.to_string())]),
        ),
        (
            "workload",
            json_object(&[
                ("name", json_string("add_rax_chain")),
                ("guest_instructions", INSTRUCTIONS.to_string()),
                ("guest_code_bytes", info.guest_code_bytes.to_string()),
                ("instruction_budget", BUDGET.to_string()),
                ("warmup_rounds", info.warmup.to_string()),
            ]),
        ),
        (
            "timings",
            json_object(&[
                ("interpreter", json_summary(interpreter)),
                ("cold_dbt", json_summary(cold)),
                ("warm_dbt", json_summary(warm)),
            ]),
        ),
        (
            "cold_dbt_metrics",
            json_object(&[
                ("blocks_compiled", metrics.blocks_compiled.to_string()),
                ("cache_hits", metrics.cache_hits.to_string()),
                ("cache_misses", metrics.cache_misses.to_string()),
                (
                    "native_bytes_emitted",
                    metrics.native_bytes_emitted.to_string(),
                ),
                ("dbt_dispatches", metrics.dbt_dispatches.to_string()),
                (
                    "instructions_executed",
                    metrics.instructions_executed.to_string(),
                ),
                (
                    "interpreter_fallbacks",
                    metrics.interpreter_fallbacks.to_string(),
                ),
                ("service_calls", metrics.service_calls.to_string()),
            ]),
        ),
    ])
}

fn main() {
    let sample_count = std::env::var("XENORA_DBT_BENCH_SAMPLES")
        .map_or(101, |value| value.parse::<usize>().expect("sample count"));
    assert!(sample_count > 0);
    let warmup_count = std::env::var("XENORA_DBT_BENCH_WARMUP")
        .map_or(10, |value| value.parse::<usize>().expect("warmup count"));

    let mut code = vec![0x48, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0]; // mov rax, 0
    for _ in 0..ADDS {
        code.extend_from_slice(&[0x48, 0x83, 0xC0, 0x01]); // add rax, 1
    }
    code.push(0xF4); // hlt

    let pe_bytes = build_pe(&code);
    let parsed = ParsedPe::parse(&pe_bytes).unwrap();
    let mut loaded = parsed.load_into_memory(&pe_bytes).unwrap();
    loaded
        .memory
        .map_region(0x2000_0000, 0x10000, MemoryPermissions::READ_WRITE)
        .unwrap();
    let initial_cpu = CpuState::new(loaded.entry_point, STACK_TOP);
    let mut interpreter = Vec::with_capacity(sample_count);
    let mut cold = Vec::with_capacity(sample_count);
    let mut warm = Vec::with_capacity(sample_count);
    let mut cold_metrics = None;

    for record in
        std::iter::repeat_n(false, warmup_count).chain(std::iter::repeat_n(true, sample_count))
    {
        let mut interp = Interpreter::new(initial_cpu.clone(), &mut loaded.memory, BUDGET);
        let start = Instant::now();
        let result = black_box(interp.run());
        let interpreter_time = start.elapsed();
        check(result);

        let mut dbt = DbtEngine::new(4 * 1024 * 1024).unwrap();
        let mut cpu = initial_cpu.clone();
        let start = Instant::now();
        let result = black_box(dbt.run(&mut cpu, &mut loaded.memory, BUDGET));
        let cold_time = start.elapsed();
        check(result);
        assert!(dbt.metrics.blocks_compiled > 0);
        assert_eq!(dbt.metrics.cache_hits, 0);
        assert_eq!(dbt.metrics.cache_misses, dbt.metrics.blocks_compiled);
        let metrics = dbt.metrics;

        let mut cpu = initial_cpu.clone();
        let start = Instant::now();
        let result = black_box(dbt.run(&mut cpu, &mut loaded.memory, BUDGET));
        let warm_time = start.elapsed();
        check(result);
        assert_eq!(dbt.metrics.blocks_compiled, metrics.blocks_compiled);
        assert_eq!(dbt.metrics.cache_hits, dbt.metrics.blocks_compiled);
        assert_eq!(dbt.metrics.cache_misses, dbt.metrics.blocks_compiled);

        if record {
            interpreter.push(interpreter_time);
            cold.push(cold_time);
            warm.push(warm_time);
            cold_metrics = Some(metrics);
        }
    }

    println!("{INSTRUCTIONS} guest instructions per sample, final RAX={ADDS}");
    println!("{warmup_count} warmup rounds excluded (XENORA_DBT_BENCH_WARMUP, default 10)");
    println!(
        "Cold DBT includes translation with a fresh engine; warm DBT reuses that engine's cache."
    );
    println!(
        "Percentiles describe this synthetic workload on this host; tail estimates vary with sample count."
    );
    println!("Even sample counts use the midpoint of the two middle values for the median.");
    let interpreter = summarize(&mut interpreter);
    let cold = summarize(&mut cold);
    let warm = summarize(&mut warm);
    println!("{}", report_line("Interpreter", &interpreter));
    println!("{}", report_line("Cold DBT", &cold));
    println!("{}", report_line("Warm DBT", &warm));
    let metrics = cold_metrics.unwrap();
    println!(
        "Cold DBT compiled {} blocks and emitted {} native bytes in {} dispatches",
        metrics.blocks_compiled, metrics.native_bytes_emitted, metrics.dbt_dispatches
    );

    if let Some(path) = std::env::var_os("XENORA_DBT_BENCH_JSON") {
        let info = run_info(code.len(), warmup_count);
        let document = json_document(&info, &interpreter, &cold, &warm, &metrics);
        std::fs::write(&path, document + "\n").expect("write XENORA_DBT_BENCH_JSON");
        println!("JSON results written to {}", path.to_string_lossy());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn single_sample_has_no_spread() {
        use super::{Duration, percentile};

        let samples = [Duration::from_micros(50)];
        for fraction in [0.25, 0.50, 0.75, 0.95, 0.99] {
            assert_eq!(percentile(&samples, fraction), samples[0]);
        }
    }

    #[test]
    fn percentiles_interpolate_across_even_samples() {
        use super::{Duration, percentile};

        let samples = [10, 20, 30, 100].map(Duration::from_micros);
        assert_eq!(percentile(&samples, 0.50), Duration::from_micros(25));
        assert_eq!(percentile(&samples, 0.95), Duration::from_nanos(89_500));
        assert_eq!(percentile(&samples, 0.99), Duration::from_nanos(97_900));
        assert_eq!(
            percentile(&samples, 0.75) - percentile(&samples, 0.25),
            Duration::from_nanos(30_000)
        );
    }

    #[test]
    fn median_uses_the_middle_of_odd_samples() {
        use super::{Duration, percentile};

        let samples = [10, 20, 100].map(Duration::from_micros);
        assert_eq!(percentile(&samples, 0.50), Duration::from_micros(20));
    }

    #[test]
    fn json_strings_escape_quotes_backslashes_and_controls() {
        use super::json_string;

        assert_eq!(json_string("plain"), r#""plain""#);
        assert_eq!(json_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(json_string("l1\nl2\r\t"), r#""l1\nl2\r\t""#);
        assert_eq!(json_string("\u{1}\u{1f}"), r#""\u0001\u001f""#);
        // Non-ASCII text needs no escape in UTF-8 JSON.
        assert_eq!(json_string("Zażółć"), "\"Zażółć\"");
    }

    #[test]
    fn unknown_values_and_non_finite_rates_are_null() {
        use super::{json_f64, json_option};

        assert_eq!(json_option(None::<u64>, |v| v.to_string()), "null");
        assert_eq!(json_option(Some(7_u64), |v| v.to_string()), "7");
        assert_eq!(json_f64(f64::INFINITY), "null");
        assert_eq!(json_f64(f64::NAN), "null");
        assert_eq!(json_f64(12.5), "12.5");
    }

    #[test]
    fn document_has_every_field_with_the_summary_values() {
        fn fixed_summary(median_us: u64) -> super::Summary {
            use super::Duration;
            super::Summary {
                samples: 3,
                median: Duration::from_micros(median_us),
                min: Duration::from_micros(10),
                max: Duration::from_micros(100),
                p95: Duration::from_nanos(92_000),
                p99: Duration::from_nanos(98_400),
                iqr: Duration::from_micros(45),
                mips: 500.25,
            }
        }

        fn fixed_info() -> super::RunInfo {
            super::RunInfo {
                unix_time: Some(1_700_000_000),
                cpu: Some("Test \"CPU\"\\1".to_owned()),
                os: "macos",
                os_version: None,
                arch: "aarch64",
                revision: Some("0123abcd".to_owned()),
                dirty: Some(false),
                debug_assertions: false,
                guest_code_bytes: 40_011,
                warmup: 10,
            }
        }

        use super::{DbtMetrics, INSTRUCTIONS, json_document};

        let metrics = DbtMetrics {
            blocks_compiled: 626,
            cache_hits: 0,
            cache_misses: 626,
            native_bytes_emitted: 245_168,
            dbt_dispatches: 626,
            instructions_executed: 10_002,
            interpreter_fallbacks: 1,
            service_calls: 0,
        };
        let document = json_document(
            &fixed_info(),
            &fixed_summary(20),
            &fixed_summary(30),
            &fixed_summary(40),
            &metrics,
        );
        let summary = |median: &str| {
            format!(
                r#"{{"samples":3,"median_ns":{median},"min_ns":10000,"max_ns":100000,"p95_ns":92000,"p99_ns":98400,"iqr_ns":45000,"median_mips":500.25}}"#
            )
        };
        let expected = format!(
            concat!(
                r#"{{"format_version":1,"benchmark":"dbt_bench","unix_time":1700000000,"#,
                r#""host":{{"cpu":"Test \"CPU\"\\1","os":"macos","os_version":null,"arch":"aarch64"}},"#,
                r#""source":{{"revision":"0123abcd","dirty":false}},"#,
                r#""build":{{"debug_assertions":false}},"#,
                r#""workload":{{"name":"add_rax_chain","guest_instructions":{},"guest_code_bytes":40011,"instruction_budget":20000,"warmup_rounds":10}},"#,
                r#""timings":{{"interpreter":{},"cold_dbt":{},"warm_dbt":{}}},"#,
                r#""cold_dbt_metrics":{{"blocks_compiled":626,"cache_hits":0,"cache_misses":626,"native_bytes_emitted":245168,"dbt_dispatches":626,"instructions_executed":10002,"interpreter_fallbacks":1,"service_calls":0}}}}"#,
            ),
            INSTRUCTIONS,
            summary("20000"),
            summary("30000"),
            summary("40000"),
        );
        assert_eq!(document, expected);
    }

    #[test]
    fn text_report_keeps_its_format() {
        fn fixed_summary(median_us: u64) -> super::Summary {
            use super::Duration;
            super::Summary {
                samples: 3,
                median: Duration::from_micros(median_us),
                min: Duration::from_micros(10),
                max: Duration::from_micros(100),
                p95: Duration::from_nanos(92_000),
                p99: Duration::from_nanos(98_400),
                iqr: Duration::from_micros(45),
                mips: 500.25,
            }
        }

        use super::report_line;

        assert_eq!(
            report_line("Cold DBT", &fixed_summary(20)),
            "Cold DBT: 3 samples, median 20µs, min 10µs, max 100µs, p95 92µs, p99 98.4µs, IQR 45µs, median 500.25 MIPS"
        );
    }

    #[test]
    fn summary_matches_the_percentile_helper() {
        use super::{Duration, INSTRUCTIONS, summarize};

        let mut samples = [100, 10, 30, 20].map(Duration::from_micros);
        let s = summarize(&mut samples);
        assert_eq!(s.samples, 4);
        assert_eq!(s.median, Duration::from_micros(25));
        assert_eq!(s.min, Duration::from_micros(10));
        assert_eq!(s.max, Duration::from_micros(100));
        assert_eq!(s.p95, Duration::from_nanos(89_500));
        assert_eq!(s.p99, Duration::from_nanos(97_900));
        assert_eq!(s.iqr, Duration::from_micros(30));
        assert_eq!(s.mips, INSTRUCTIONS as f64 / 25e-6 / 1e6);
    }
}
