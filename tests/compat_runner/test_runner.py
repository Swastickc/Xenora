# SPDX-License-Identifier: GPL-3.0-or-later
import argparse
import contextlib
import fcntl
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "compat"))
import runner


class RunnerTests(unittest.TestCase):
    def test_capture_preserves_exit_and_limits_both_streams(self):
        result = runner.capture(
            [sys.executable, "-c", "import sys; print('a'*40000); print('b'*40000,file=sys.stderr); sys.exit(7)"],
            10, os.environ.copy(),
        )
        self.assertEqual((result["status"], result["returncode"]), ("failed", 7))
        self.assertTrue(result["stdout_truncated"])
        self.assertTrue(result["stderr_truncated"])
        self.assertLessEqual(len(result["stdout"]), runner.OUTPUT_LIMIT)
        self.assertLessEqual(len(result["stderr"]), runner.OUTPUT_LIMIT)

    def test_deadline_terminates_child(self):
        result = runner.capture([sys.executable, "-c", "import time; time.sleep(30)"], 0.1, os.environ.copy())
        self.assertEqual(result["status"], "timeout")
        self.assertLess(result["elapsed_seconds"], 5)

    def test_deadline_terminates_descendant_retaining_pipes(self):
        descendant = """
import fcntl
import sys
import time
with open(sys.argv[1], "w") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    print("descendant ready", flush=True)
    time.sleep(10)
"""
        parent = """
import subprocess
import sys
subprocess.Popen([sys.executable, "-c", sys.argv[1], sys.argv[2]])
"""
        with tempfile.TemporaryDirectory() as directory:
            lock_path = Path(directory) / "descendant.lock"
            result = runner.capture(
                [sys.executable, "-c", parent, descendant, str(lock_path)],
                2, os.environ.copy(),
            )
            self.assertEqual(result["status"], "timeout")
            self.assertLess(result["elapsed_seconds"], 5)
            self.assertIn("descendant ready", result["stdout"])
            # SIGKILL can be delivered before the descendant releases its file lock.
            deadline = time.monotonic() + 2
            with lock_path.open() as lock:
                while True:
                    try:
                        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        break
                    except BlockingIOError:
                        if time.monotonic() >= deadline:
                            self.fail("descendant still holds its lock after the deadline")
                        time.sleep(0.01)

    def test_missing_program_is_not_a_pass(self):
        result = runner.capture(["xenora-nonexistent-compat-executable"], 1, os.environ.copy())
        self.assertEqual(result["status"], "missing-tool")

    def test_invalid_timeouts_are_rejected(self):
        for value in ("0", "-1", "nan", "inf", "3601"):
            with self.subTest(value=value), self.assertRaises(argparse.ArgumentTypeError):
                runner.timeout_seconds(value)

    def test_compare_identifies_a_regression(self):
        def result(status):
            return {"workloads": [{"id": "a", "stages": [{"name": "fixture-command", "status": status}]}]}
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertNotEqual(runner.compare(result("passed"), result("failed")), 0)
            self.assertEqual(runner.compare(result("failed"), result("passed")), 0)
            with self.assertRaises(ValueError):
                runner.compare(result("passed"), {"workloads": []})

    def test_output_cannot_escape_artifacts(self):
        with self.assertRaises(ValueError):
            runner.output_path(ROOT / "CHANGELOG.md")

    def test_report_reader_rejects_duplicate_workloads(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            path.write_text('{"schema_version":1,"workloads":[{"id":"x","status":"passed","stages":[{"name":"one","status":"passed"}]},{"id":"x","status":"passed","stages":[{"name":"one","status":"passed"}]}]}')
            with self.assertRaises(ValueError):
                runner.read_result(path)

    def test_compare_rejects_different_stage_scope_for_same_workload(self):
        def result(stage):
            return {"workloads": [{"id": "a", "stages": [{"name": stage, "status": "passed"}]}]}

        with self.assertRaisesRegex(
            ValueError, r"missing: a/host-build; extra: a/fixture-command"
        ):
            runner.compare(result("host-build"), result("fixture-command"))

    def test_output_rejects_existing_and_symlink_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            results_dir = Path(directory).resolve() / "results"
            results_dir.mkdir()
            existing = results_dir / "existing.json"
            existing.write_text("keep")
            dangling = results_dir / "dangling.json"
            dangling.symlink_to(results_dir / "missing.json")
            escape = results_dir / "escape"
            escape.symlink_to(results_dir.parent, target_is_directory=True)
            with patch.object(runner, "RESULTS_DIR", results_dir):
                for path in (existing, dangling):
                    with self.subTest(path=path), self.assertRaises(FileExistsError):
                        runner.output_path(path)
                with self.assertRaises(ValueError):
                    runner.output_path(escape / "result.json")
            self.assertEqual(existing.read_text(), "keep")

    def test_write_result_preserves_file_created_after_output_check(self):
        with tempfile.TemporaryDirectory() as directory:
            results_dir = Path(directory).resolve()
            with patch.object(runner, "RESULTS_DIR", results_dir):
                path = runner.output_path(results_dir / "result.json")
                path.write_text("concurrent writer")
                with self.assertRaises(FileExistsError):
                    runner.write_result(path, {"workloads": []})
            self.assertEqual(path.read_text(), "concurrent writer")
            self.assertEqual(list(results_dir.iterdir()), [path])

    def test_report_reader_rejects_malformed_reports(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            for expected, changes, stage_changes in (
                ("Unsupported result schema", {"schema_version": True}, {}),
                ("Invalid context", {"context": []}, {}),
                ("Invalid commit metadata", {"context": {"commit": {"status": "passed"}}}, {}),
                ("Invalid status", {}, {"status": "made-up"}),
                ("Invalid detail", {}, {"detail": {"message": "bad"}}),
                ("Invalid return code", {}, {"returncode": True}),
                ("Inconsistent stage outcome", {}, {"status": "failed"}),
            ):
                with self.subTest(expected=expected), self.assertRaisesRegex(ValueError, expected):
                    stage = {"name": "fixture-command", "status": "passed", **stage_changes}
                    result = {
                        "schema_version": runner.SCHEMA_VERSION,
                        "workloads": [{"id": "a", "status": "passed", "stages": [stage]}],
                        **changes,
                    }
                    path.write_text(json.dumps(result))
                    runner.read_result(path)

    def test_build_failure_skips_fixture_and_survives_report_round_trip(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture_path = Path(directory) / "fixture-ran"
            workload = {
                "suite": "test",
                "build_argv": [sys.executable, "-c", "raise SystemExit(7)"],
                "argv": [
                    sys.executable, "-c",
                    "from pathlib import Path; import sys; Path(sys.argv[1]).touch()",
                    str(fixture_path),
                ],
            }
            output = runner.run_workload("example", workload, 5, os.environ.copy())
            self.assertFalse(fixture_path.exists())
            self.assertEqual(output["status"], "failed")
            build, fixture = output["stages"]
            self.assertEqual(
                (build["name"], build["status"], build["returncode"]),
                ("host-build", "failed", 7),
            )
            self.assertEqual((fixture["name"], fixture["status"]), ("fixture-command", "skipped"))
            self.assertIsNone(fixture.get("returncode"))
            path = Path(directory) / "result.json"
            result = {"schema_version": runner.SCHEMA_VERSION, "workloads": [output]}
            runner.write_result(path, result)
            self.assertEqual(runner.read_result(path), result)

    def test_successful_build_preserves_fixture_failure(self):
        workload = {
            "suite": "test",
            "build_argv": [sys.executable, "-c", "print('compiled')"],
            "argv": [sys.executable, "-c", "raise SystemExit(9)"],
        }
        output = runner.run_workload("example", workload, 5, os.environ.copy())
        self.assertEqual(output["status"], "failed")
        self.assertEqual(
            [(stage["name"], stage["status"], stage["returncode"]) for stage in output["stages"]],
            [("host-build", "passed", 0), ("fixture-command", "failed", 9)],
        )


if __name__ == "__main__":
    unittest.main()
