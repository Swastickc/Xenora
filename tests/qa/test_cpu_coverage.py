# SPDX-License-Identifier: GPL-3.0-or-later
import json
import tempfile
import unittest
from pathlib import Path

from qa.cpu_coverage import (
    blocker_manifest,
    build_report,
    compare_reports,
    dumps_report,
    scan_imports,
    scan_opcodes,
)

ROOT = Path(__file__).resolve().parents[2]


class CpuCoverage(unittest.TestCase):
    def test_scan_opcodes_positive_and_negative(self):
        text = """
        match opcode {
            0x89 => Op::Mov,
            0x90 => unimplemented!("nop"),
            other => unsupported(0xFF),
        }
        let litter = 0xAB;
        """
        entries = {e.opcode: e.status for e in scan_opcodes(text, source="t.rs")}
        self.assertEqual(entries.get("89"), "supported")
        self.assertEqual(entries.get("90"), "unsupported")

    def test_scan_imports_registration_kinds(self):
        text = '''
        const KERNEL32: &str = "kernel32.dll";
        registry.register(KERNEL32, "CreateFileW", |_, _, _| Ok(0));
        registry.register_with_status(
            "kernel32.dll",
            "Sleep",
            ServiceStatus::Stub,
            |_, _, _| Ok(0),
        );
        registry.register_with_status(
            "kernel32.dll", "WaitForIt", ServiceStatus::Partial, |_, _, _| Ok(0),
        );
        registry.alias("api-ms-win-core-file-l1-1-0.dll", KERNEL32, "CreateFileW");
        '''
        entries = {(e.dll, e.name): e.kind for e in scan_imports(text, source="imports.rs")}
        self.assertEqual(entries.get(("kernel32.dll", "CreateFileW")), "functional")
        self.assertEqual(entries.get(("kernel32.dll", "Sleep")), "stub")
        self.assertEqual(entries.get(("kernel32.dll", "WaitForIt")), "partial")
        self.assertEqual(
            entries.get(("api-ms-win-core-file-l1-1-0.dll", "CreateFileW")),
            "alias",
        )

    def test_scan_imports_ignores_arbitrary_literals_and_test_modules(self):
        text = '''
        enum ImportName { Name(String) }
        let name = "ImportName";
        let helper = "not_a_guest_import";
        #[cfg(test)]
        mod tests {
            fn register_tests() {
                registry.register("kernel32.dll", "TestOnly", |_, _, _| Ok(0));
            }
        }
        '''
        entries = scan_imports(text, source="imports.rs")
        self.assertEqual(entries, [])

    def test_scan_imports_reads_generated_service_metadata(self):
        text = '''
        ServiceMetadata {
            library: "kernel32.dll",
            symbol: "GetCurrentProcess",
            parameters: &[],
            result: "handle",
            status: ServiceStatus::Implemented,
        }
        ServiceMetadata {
            library: "ucrtbase.dll",
            symbol: "fopen",
            parameters: &[],
            result: "u64",
            status: ServiceStatus::Partial,
        }
        '''
        entries = {(e.dll, e.name): e.kind for e in scan_imports(text, source="runtime_table.rs")}
        self.assertEqual(entries[("kernel32.dll", "GetCurrentProcess")], "functional")
        self.assertEqual(entries[("ucrtbase.dll", "fopen")], "partial")

    def test_scan_imports_keeps_same_symbol_in_distinct_modules(self):
        text = '''
        registry.register("kernel32.dll", "SharedName", |_, _, _| Ok(0));
        registry.register("ucrtbase.dll", "SharedName", |_, _, _| Ok(0));
        '''
        entries = scan_imports(text, source="services.rs")
        self.assertEqual(
            sorted((e.dll, e.name) for e in entries),
            [("kernel32.dll", "SharedName"), ("ucrtbase.dll", "SharedName")],
        )

    def test_repo_report_and_blockers(self):
        report = build_report(ROOT)
        self.assertTrue(report.opcodes or report.imports or report.notes)
        imports = {(entry.dll, entry.name) for entry in report.imports}
        self.assertIn(("kernel32.dll", "GetCurrentProcess"), imports)
        self.assertNotIn(("imports", "ImportName"), imports)
        self.assertNotIn(("imports", "Defaulted"), imports)
        self.assertNotIn(("imports_sync", "CARGO_BIN_EXE_xenora"), imports)
        payload = json.loads(dumps_report(report))
        blockers = blocker_manifest(report, limit=10)
        self.assertIsInstance(blockers, list)
        delta = compare_reports(payload, payload)
        self.assertEqual(delta["newly_supported"], [])


if __name__ == "__main__":
    unittest.main()
