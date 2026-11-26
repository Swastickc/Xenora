# SPDX-License-Identifier: GPL-3.0-or-later
"""Unit tests for scripts/audit/runtime_inventory.py."""

import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/audit/runtime_inventory.py"
SPEC = importlib.util.spec_from_file_location("runtime_inventory_audit", SCRIPT)
audit = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = audit
SPEC.loader.exec_module(audit)

InventoryError = audit.InventoryError
JSON_NAME = audit.JSON_NAME
build_report = audit.build_report
parse_snapshot = audit.parse_snapshot
read_wanted = audit.read_wanted
render_json = audit.render_json
render_markdown = audit.render_markdown
render_outputs = audit.render_outputs
write_or_check = audit.write_or_check


def service(library, symbol, status, *, kind="name", explicit=True, source="manual", data=False):
    return {
        "library": library,
        "symbol": symbol,
        "kind": kind,
        "status": status,
        "status_explicit": explicit,
        "source": source,
        "data_import": data,
    }


def key(
    library,
    symbol,
    target_library,
    target_symbol,
    *,
    kind="name",
    target_kind="name",
    canonical=False,
    api=False,
):
    return {
        "library": library,
        "symbol": symbol,
        "kind": kind,
        "canonical_library": target_library,
        "canonical_symbol": target_symbol,
        "canonical_kind": target_kind,
        "canonical": canonical,
        "api_set": api,
    }


def tiny_snapshot():
    return {
        "schema": 1,
        "services": [
            service("xmem.dll", "XMemFree", "stub"),
            service("ucrtbase.dll", "printf", "partial", explicit=False, source="typed"),
            service("kernel32.dll", "CreateFileW", "implemented"),
        ],
        "keys": [
            key("ucrtbase.dll", "printf", "ucrtbase.dll", "printf", canonical=True),
            key("xmem.dll", "XMemFree", "xmem.dll", "XMemFree", canonical=True),
            key("kernel32.dll", "CreateFileW", "kernel32.dll", "CreateFileW", canonical=True),
            key(
                "api-ms-win-core-file-l1-1-0.dll",
                "CreateFileW",
                "kernel32.dll",
                "CreateFileW",
                api=True,
            ),
            key("msvcrt.dll", "printf", "ucrtbase.dll", "printf"),
            key(
                "ucrtbase.dll",
                "Ordinal#7",
                "ucrtbase.dll",
                "printf",
                kind="ordinal",
                target_kind="name",
            ),
        ],
        "typed": [
            {
                "library": "ucrtbase.dll",
                "symbol": "printf",
                "kind": "name",
                "status": "partial",
                "aliases": ["api-ms-win-crt-stdio-l1-1-0.dll"],
            }
        ],
    }


def snapshot_bytes(snapshot):
    return json.dumps(snapshot).encode()


def parsed_tiny():
    return parse_snapshot(snapshot_bytes(tiny_snapshot()))


def collision_snapshot():
    return {
        "schema": 1,
        "services": [
            service("collision.dll", "Ordinal#7", "implemented", kind="name", source="typed"),
            service("collision.dll", "Ordinal#7", "stub", kind="ordinal"),
        ],
        "keys": [
            key(
                "collision.dll",
                "Ordinal#7",
                "collision.dll",
                "Ordinal#7",
                kind="name",
                target_kind="name",
                canonical=True,
            ),
            key(
                "collision.dll",
                "Ordinal#7",
                "collision.dll",
                "Ordinal#7",
                kind="ordinal",
                target_kind="ordinal",
                canonical=True,
            ),
        ],
        "typed": [
            {
                "library": "collision.dll",
                "symbol": "Ordinal#7",
                "kind": "name",
                "status": "implemented",
                "aliases": [],
            }
        ],
    }


class SnapshotTests(unittest.TestCase):
    def test_report_is_deterministic_for_shuffled_input_rows(self):
        snapshot = tiny_snapshot()
        shuffled = copy.deepcopy(snapshot)
        shuffled["services"].reverse()
        shuffled["keys"].reverse()
        shuffled["typed"].reverse()
        self.assertEqual(
            render_json(build_report(parse_snapshot(snapshot_bytes(snapshot)))),
            render_json(build_report(parse_snapshot(snapshot_bytes(shuffled)))),
        )

    def test_aliases_statuses_and_implicit_status_are_reported(self):
        report = build_report(parsed_tiny())
        self.assertEqual(report["summary"]["api_set_aliases"], 1)
        self.assertEqual(report["summary"]["ordinal_aliases"], 1)
        self.assertEqual(report["summary"]["implemented"], 1)
        self.assertEqual(report["summary"]["partial"], 1)
        self.assertEqual(report["summary"]["stub"], 1)
        self.assertEqual(
            report["findings"]["implicit_status_services"],
            [
                {
                    "library": "ucrtbase.dll",
                    "symbol": "printf",
                    "kind": "name",
                    "status": "partial",
                    "source": "typed",
                }
            ],
        )
        alias = next(row for row in report["aliases"] if row["library"] == "api-ms-win-core-file-l1-1-0.dll")
        self.assertEqual(alias["status"], "implemented")
        self.assertEqual(alias["canonical_kind"], "name")

    def test_duplicate_and_conflict_detection(self):
        snapshot = tiny_snapshot()
        snapshot["services"].append(dict(snapshot["services"][2]))
        snapshot["keys"].append(
            key(
                "api-ms-win-core-file-l1-1-0.dll",
                "CreateFileW",
                "ucrtbase.dll",
                "printf",
                api=True,
            )
        )
        report = build_report(parse_snapshot(snapshot_bytes(snapshot)))
        self.assertEqual(len(report["findings"]["duplicate_registrations"]), 1)
        self.assertEqual(len(report["findings"]["duplicate_resolution_keys"]), 1)
        self.assertEqual(len(report["findings"]["conflicting_resolution_keys"]), 1)
        self.assertEqual(report["summary"]["duplicate_registrations"], 1)

    def test_missing_target_detection_remains_a_reporting_check(self):
        snapshot = parsed_tiny()
        snapshot["keys"].append(key("missing.dll", "Gone", "absent.dll", "Gone"))
        report = build_report(snapshot)
        self.assertEqual(len(report["findings"]["aliases_missing_target"]), 1)

    def test_typed_discrepancies_are_findings(self):
        report = build_report(parsed_tiny())
        self.assertEqual(len(report["findings"]["typed_aliases_missing"]), 1)
        snapshot = tiny_snapshot()
        snapshot["typed"][0]["status"] = "implemented"
        report = build_report(parse_snapshot(snapshot_bytes(snapshot)))
        self.assertEqual(len(report["findings"]["typed_status_mismatches"]), 1)

    def test_name_spelled_like_an_ordinal_does_not_conflate_services(self):
        snapshot = parse_snapshot(snapshot_bytes(collision_snapshot()))
        report = build_report(snapshot)
        self.assertEqual(report["summary"]["canonical_services"], 2)
        self.assertEqual(report["findings"]["source_mismatches"], [])
        sources = {(row["kind"], row["symbol"]): row["source"] for row in report["services"]}
        self.assertEqual(sources[("name", "Ordinal#7")], "typed")
        self.assertEqual(sources[("ordinal", "Ordinal#7")], "manual")

        wanted = {
            "format": "simple-csv",
            "rows": 2,
            "entries": [
                {
                    "library": "collision.dll",
                    "symbol": "Ordinal#7",
                    "kind": "name",
                    "family": "collision",
                    "occurrences": 1,
                    "import_tables": set(),
                },
                {
                    "library": "collision.dll",
                    "symbol": "Ordinal#7",
                    "kind": "ordinal",
                    "family": "collision",
                    "occurrences": 1,
                    "import_tables": set(),
                },
            ],
        }
        report = build_report(snapshot, wanted)
        matches = {(row["kind"], row["symbol"]): row for row in report["wanted"]["matches"]}
        self.assertEqual(matches[("name", "Ordinal#7")]["status"], "implemented")
        self.assertEqual(matches[("name", "Ordinal#7")]["canonical_kind"], "name")
        self.assertEqual(matches[("ordinal", "Ordinal#7")]["status"], "stub")
        self.assertEqual(matches[("ordinal", "Ordinal#7")]["canonical_kind"], "ordinal")

    def test_data_import_rows_are_preserved(self):
        snapshot = tiny_snapshot()
        snapshot["services"][0]["data_import"] = True
        report = build_report(parse_snapshot(snapshot_bytes(snapshot)))
        row = next(row for row in report["services"] if row["library"] == "xmem.dll")
        self.assertTrue(row["data_import"])
        self.assertEqual(report["summary"]["data_import_services"], 1)

    def test_wanted_summary_counts_are_null_without_wanted_input(self):
        report = build_report(parsed_tiny())
        self.assertIsNone(report["summary"]["wanted_imports"])
        self.assertIsNone(report["summary"]["wanted_missing"])


class StructuralValidationTests(unittest.TestCase):
    def assert_invalid(self, snapshot, message):
        with self.assertRaisesRegex(InventoryError, message):
            parse_snapshot(snapshot_bytes(snapshot))

    def test_canonical_row_must_resolve_to_its_own_kind_aware_key(self):
        snapshot = tiny_snapshot()
        canonical = next(row for row in snapshot["keys"] if row["canonical"] and row["library"] == "kernel32.dll")
        canonical["canonical_library"] = "xmem.dll"
        canonical["canonical_symbol"] = "XMemFree"
        self.assert_invalid(snapshot, "canonical row must resolve to its own")

    def test_canonical_target_must_exist_with_the_same_kind(self):
        snapshot = tiny_snapshot()
        snapshot["services"].append(service("ucrtbase.dll", "Ordinal#8", "partial"))
        snapshot["keys"].append(
            key("ucrtbase.dll", "Ordinal#8", "ucrtbase.dll", "Ordinal#8", canonical=True)
        )
        ordinal = next(row for row in snapshot["keys"] if row["kind"] == "ordinal")
        ordinal["canonical_kind"] = "ordinal"
        ordinal["canonical_symbol"] = "Ordinal#8"
        self.assert_invalid(snapshot, "has no service row")

    def test_missing_canonical_target_is_rejected(self):
        snapshot = tiny_snapshot()
        alias = next(row for row in snapshot["keys"] if row["library"] == "msvcrt.dll")
        alias["canonical_library"] = "absent.dll"
        alias["canonical_symbol"] = "Gone"
        self.assert_invalid(snapshot, "has no service row")

    def test_noncanonical_row_must_not_resolve_to_itself(self):
        snapshot = tiny_snapshot()
        snapshot["services"].append(service("msvcrt.dll", "printf", "partial"))
        snapshot["keys"].append(key("msvcrt.dll", "printf", "msvcrt.dll", "printf", canonical=True))
        alias = next(row for row in snapshot["keys"] if row["library"] == "msvcrt.dll" and not row["canonical"])
        alias["canonical_library"] = "msvcrt.dll"
        self.assert_invalid(snapshot, "noncanonical row resolves to itself")

    def test_each_service_needs_exactly_one_canonical_resolution_key(self):
        missing = tiny_snapshot()
        missing["keys"] = [
            row for row in missing["keys"] if not (row["canonical"] and row["library"] == "ucrtbase.dll")
        ]
        self.assert_invalid(missing, "expected exactly one canonical resolution key, found 0")

        duplicated = tiny_snapshot()
        duplicated["keys"].append(
            key("ucrtbase.dll", "printf", "ucrtbase.dll", "printf", canonical=True)
        )
        self.assert_invalid(duplicated, "expected exactly one canonical resolution key, found 2")

    def test_typed_registry_rows_are_always_named(self):
        snapshot = tiny_snapshot()
        snapshot["typed"][0]["kind"] = "ordinal"
        self.assert_invalid(snapshot, "typed registry entries are always named")


class WantedInputTests(unittest.TestCase):
    def write_input(self, text):
        temporary = tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False, newline="")
        self.addCleanup(lambda: Path(temporary.name).unlink(missing_ok=True))
        with temporary:
            temporary.write(text)
        return Path(temporary.name)

    def test_wanted_matching_groups_statuses_by_family_and_resolves_aliases(self):
        path = self.write_input(
            "library,symbol,kind,family\n"
            "kernel32.dll,CreateFileW,name,windows-files\n"
            "api-ms-win-core-file-l1-1-0.dll,CreateFileW,name,windows-files\n"
            "ucrtbase.dll,printf,name,c-output\n"
            "xmem.dll,XMemFree,name,xbox-memory\n"
            "ucrtbase.dll,absent,name,c-output\n"
            "ucrtbase.dll,7,ordinal,c-output\n"
        )
        report = build_report(parsed_tiny(), read_wanted(path))
        wanted = report["wanted"]
        self.assertEqual(wanted["status_counts"], {"implemented": 2, "partial": 2, "stub": 1, "missing": 1})
        self.assertEqual(report["summary"]["wanted_imports"], 6)
        self.assertEqual(report["summary"]["wanted_missing"], 1)
        families = {row["family"]: row for row in wanted["families"]}
        self.assertEqual(families["windows-files"]["implemented"], 2)
        self.assertEqual(families["c-output"]["partial"], 2)
        self.assertEqual(families["c-output"]["missing"], 1)
        alias = next(row for row in wanted["matches"] if row["library"].startswith("api-ms-"))
        self.assertEqual(alias["resolution"], "api-set-alias")
        ordinal = next(row for row in wanted["matches"] if row["kind"] == "ordinal")
        self.assertEqual(ordinal["resolution"], "ordinal-alias")
        self.assertEqual(ordinal["canonical_symbol"], "printf")
        self.assertEqual(ordinal["canonical_kind"], "name")

    def test_probe_input_deduplicates_slots_categories_and_distinguishes_ordinals(self):
        path = self.write_input(
            "module,symbol_or_ordinal,iat_rva,import_table,category,registered_binding,implementation_status,is_alias,canonical_module,startup_priority,startup_cluster,trusted_runner_binding,import_kind\n"
            "\"kernel32.dll\",\"CreateFileW\",0x00001000,ordinary,synchronous file,true,implemented,false,\"\",1,core,false,name\n"
            "\"kernel32.dll\",\"CreateFileW\",0x00001008,ordinary,synchronous file,true,implemented,false,\"\",1,core,false,name\n"
            "\"ucrtbase.dll\",\"Ordinal#7\",0x00002000,ordinary,C/C++ CRT Runtime,true,partial,false,\"\",1,core,false,ordinal\n"
            "\"space dll.dll\",\"name with spaces\",0x00003000,delayed,probe other,false,missing,false,\"\",1,other,false,name\n"
        )
        wanted = read_wanted(path)
        self.assertEqual(wanted["format"], "probe-csv")
        self.assertEqual(wanted["rows"], 4)
        self.assertEqual(len(wanted["entries"]), 3)
        families = {row["symbol"]: row["family"] for row in wanted["entries"]}
        self.assertEqual(families["CreateFileW"], "synchronous file")
        self.assertEqual(families["Ordinal#7"], "C/C++ CRT Runtime")
        self.assertEqual(families["name with spaces"], "probe other")
        ordinal = next(row for row in wanted["entries"] if row["kind"] == "ordinal")
        self.assertEqual(ordinal["symbol"], "Ordinal#7")

    def test_probe_input_without_a_category_uses_the_module_fallback(self):
        path = self.write_input(
            "module,symbol_or_ordinal,iat_rva,import_kind\n"
            "kernel32.dll,CreateFileW,0x00001000,name\n"
        )
        wanted = read_wanted(path)
        self.assertEqual(wanted["entries"][0]["family"], "windows-system")

    def test_rejects_malformed_header_bad_ordinal_and_conflicting_duplicate(self):
        with self.assertRaises(InventoryError):
            read_wanted(self.write_input("module,symbol\nkernel32.dll,CreateFileW\n"))
        with self.assertRaises(InventoryError):
            read_wanted(self.write_input("library,symbol,kind\nkernel32.dll,not-a-number,ordinal\n"))
        with self.assertRaises(InventoryError):
            read_wanted(
                self.write_input(
                    "library,symbol,family\n"
                    "kernel32.dll,CreateFileW,one\n"
                    "KERNEL32.DLL,CreateFileW,two\n"
                )
            )

    def test_overlong_ordinal_is_an_inventory_error_not_a_conversion_traceback(self):
        path = self.write_input(f"library,symbol,kind\nkernel32.dll,{'9' * 5000},ordinal\n")
        with self.assertRaisesRegex(InventoryError, "outside the 16-bit PE ordinal range"):
            read_wanted(path)


class OutputTests(unittest.TestCase):
    def test_write_and_check_accepts_fresh_output_and_rejects_stale_output(self):
        report = build_report(parsed_tiny())
        outputs = render_outputs(report)
        with tempfile.TemporaryDirectory() as directory:
            output_dir = Path(directory)
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertTrue(write_or_check(outputs, output_dir, check=False))
                self.assertTrue(write_or_check(outputs, output_dir, check=True))
            json_path = output_dir / JSON_NAME
            self.assertEqual(json_path.stat().st_mode & 0o777, 0o644)
            json_path.write_text(json_path.read_text(encoding="utf-8") + " ", encoding="utf-8")
            error_output = io.StringIO()
            with contextlib.redirect_stderr(error_output):
                self.assertFalse(write_or_check(outputs, output_dir, check=True))
            self.assertIn("runtime inventory report is stale", error_output.getvalue())

    def test_markdown_contains_summary_findings_and_input_note(self):
        report = build_report(parsed_tiny())
        markdown = render_markdown(report).decode()
        self.assertIn("Statuses never explicitly set", markdown)
        self.assertIn("api-ms-win-crt", markdown)
        self.assertIn("No wanted-imports file", markdown)
        self.assertIn("Data-import services", markdown)

    def test_markdown_escapes_user_controlled_family_labels(self):
        temporary = tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False, newline="")
        self.addCleanup(lambda: Path(temporary.name).unlink(missing_ok=True))
        with temporary:
            temporary.write(
                "library,symbol,kind,family\n"
                "kernel32.dll,CreateFileW,name,bad|<script>`label`\n"
            )
        path = Path(temporary.name)
        report = build_report(parsed_tiny(), read_wanted(path))
        markdown = render_markdown(report).decode()
        self.assertNotIn("<script>", markdown)
        self.assertIn("&lt;script&gt;", markdown)
        self.assertIn("bad&#124;", markdown)


if __name__ == "__main__":
    unittest.main()
