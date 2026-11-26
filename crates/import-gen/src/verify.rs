// SPDX-License-Identifier: GPL-3.0-or-later

//! Verification and conformance test runner for import bindings.
//!
//! Runs the conformance and runtime integration suites for generated bindings.

use std::process::Command;

const TARGETS: [&str; 2] = ["generated_conformance", "generated_runtime_integration"];

/// Result of running the verification suite.
#[derive(Debug)]
pub struct VerificationResult {
    pub passed: usize,
    pub failed: usize,
    pub total: usize,
    pub details: Vec<String>,
}

/// Run automated behavioral and conformance tests for generated bindings.
pub fn run_verification() -> Result<VerificationResult, String> {
    let mut combined = VerificationResult {
        passed: 0,
        failed: 0,
        total: 0,
        details: Vec::new(),
    };
    // Isolate each target so a missing suite cannot be hidden by another
    // target's output, and still run the second suite after a test failure.
    for target in TARGETS {
        let output = Command::new("cargo")
            .args([
                "test",
                "--locked",
                "--color",
                "never",
                "-p",
                "xenora-loader",
                "--test",
                target,
                "--",
                "--format",
                "pretty",
            ])
            .output()
            .map_err(|error| format!("{target}: failed to execute cargo test: {error}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let suite = parse_suite(&stdout, output.status.success()).map_err(|error| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!("{target}: {error} ({})\n{stderr}\n{stdout}", output.status)
        })?;
        combined.passed += suite.passed;
        combined.failed += suite.failed;
        combined.total += suite.total;
        combined.details.extend(suite.details);
    }
    Ok(combined)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    passed: usize,
    failed: usize,
    ignored: usize,
}

fn parse_suite(stdout: &str, success: bool) -> Result<VerificationResult, &'static str> {
    let stdout = stdout.trim_end();
    let (report, summary) = stdout.rsplit_once('\n').unwrap_or(("", stdout));
    let summary = summary
        .strip_prefix("test result: ")
        .ok_or("cargo produced no final test summary")?;
    let (summary_success, expected) = parse_summary(summary).ok_or("malformed libtest summary")?;
    if success != summary_success || summary_success != (expected.failed == 0) {
        return Err("cargo exit status and test result disagree");
    }

    let mut observed = Counts::default();
    let mut details = Vec::new();
    // Captured output from failing tests can contain arbitrary text.
    for line in report.lines().take_while(|line| *line != "failures:") {
        if line.starts_with("test result: ") {
            return Err("cargo produced more than one test summary");
        }
        if let Some((_, outcome)) = line
            .strip_prefix("test ")
            .and_then(|test| test.rsplit_once(" ... "))
        {
            match outcome {
                "ok" => observed.passed += 1,
                "FAILED" => observed.failed += 1,
                "ignored" => observed.ignored += 1,
                reason if reason.starts_with("ignored, ") => observed.ignored += 1,
                _ => continue,
            };
            details.push(line.to_string());
        }
    }

    if observed != expected {
        return Err("individual test outcomes and summary counts disagree");
    }
    let Counts { passed, failed, .. } = observed;
    if passed + failed == 0 {
        return Err("no tests ran");
    }

    // Report ignored tests in the details without counting them as executed.
    Ok(VerificationResult {
        passed,
        failed,
        total: passed + failed,
        details,
    })
}

fn parse_summary(text: &str) -> Option<(bool, Counts)> {
    let (result, counts) = text.split_once(". ")?;
    let success = match result {
        "ok" => true,
        "FAILED" => false,
        _ => return None,
    };
    let mut fields = counts.split("; ");
    let count = |field: Option<&str>, label: &str| field?.strip_suffix(label)?.parse().ok();
    let counts = Counts {
        passed: count(fields.next(), " passed")?,
        failed: count(fields.next(), " failed")?,
        ignored: count(fields.next(), " ignored")?,
    };
    if count(fields.next(), " measured")? != 0 || count(fields.next(), " filtered out")? != 0 {
        return None;
    }
    Some((success, counts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_runtime_tests_and_keeps_skips_out_of_the_total() {
        let suite = parse_suite(
            "test crt_handlers_validate_memory_boundaries ... ok\n\
             test a_separate_case ... ignored, needs another environment\n\
             test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s",
            true,
        )
        .unwrap();
        assert_eq!((suite.passed, suite.failed, suite.total), (1, 0, 1));
        assert_eq!(suite.details.len(), 2);
    }

    #[test]
    fn reports_normal_test_failures_and_rejects_inconsistent_exit_status() {
        let inline = "test one ... FAILED\n\
                      failures:\n\
                      test captured_output ... ok\n\
                      test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s";
        assert!(parse_suite(inline, true).is_err());
        let suite = parse_suite(inline, false).unwrap();
        assert_eq!((suite.passed, suite.failed, suite.total), (0, 1, 1));
    }

    #[test]
    fn refuses_missing_ambiguous_unexecuted_and_malformed_results() {
        let valid = "test one ... ok\n\
                     test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s";
        for stdout in [
            String::new(),
            format!("{valid}\n{valid}"),
            valid.replace("1 passed", "2 passed"),
            valid.replace("1 passed", "zero passed"),
            valid.replace("0 filtered out", "1 filtered out"),
            valid
                .replace(
                    "1 passed; 0 failed; 0 ignored",
                    "0 passed; 0 failed; 1 ignored",
                )
                .replace("... ok", "... ignored"),
        ] {
            assert!(parse_suite(&stdout, true).is_err(), "{stdout}");
        }
    }
}
