# SPDX-License-Identifier: GPL-3.0-or-later
"""Build or verify a Xenora contributor preview on native Apple Silicon."""

import argparse
import json
import os
import platform
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

from .evidence import APP_ARCHIVE, copy_notices, read_verified_manifest, write_manifest
from .package import APP_NAME, archive_app, extract_app, inspect_app
from .signing import notarize, sign_bundle, verify_distribution, verify_local
from .source import Source, snapshot, stamp_bundle, validate_build_number

FIXTURE = "fixtures/pe/trusted-runner.exe"
METAL_VALIDATION = {
    "MTL_DEBUG_LAYER": "1",
    "MTL_SHADER_VALIDATION": "1",
    "MTL_SHADER_VALIDATION_REPORT_TO_STDERR": "1",
    "MTL_SHADER_VALIDATION_ABORT_ON_FAULT": "1",
}


def run(args, *, capture=False, sensitive=False, stderr=None, **kwargs) -> str:
    if capture or sensitive:
        kwargs["stdout"] = subprocess.PIPE
        if stderr is None:
            stderr = subprocess.PIPE
    try:
        completed = subprocess.run(args, check=True, text=True, stderr=stderr, **kwargs)
    except (OSError, subprocess.SubprocessError) as error:
        if sensitive:
            raise RuntimeError(f"{args[0]} failed. Check its configured credentials.") from None
        if isinstance(error, subprocess.CalledProcessError):
            detail = "\n".join(
                output.strip() for output in (error.stdout, error.stderr) if output and output.strip()
            )
            raise RuntimeError(
                f"{args[0]} exited with status {error.returncode}" + (f":\n{detail}" if detail else "")
            ) from None
        raise
    return completed.stdout or ""


def environment(*, run) -> dict:
    return {
        "architecture": platform.machine(),
        "hardware": run(["sysctl", "-n", "machdep.cpu.brand_string"], capture=True).strip(),
        "macos": platform.mac_ver()[0],
        "rust": run(["rustc", "--version"], capture=True).strip(),
        "cargo": run(["cargo", "--version"], capture=True).strip(),
        "swift": run(["swift", "--version"], capture=True).splitlines()[0],
        "sdk": run(["xcrun", "--sdk", "macosx", "--show-sdk-version"], capture=True).strip(),
    }


def check_fixture(app: Path, source_root: Path, *, run) -> dict:
    executables = app / "Contents" / "MacOS"
    options = {
        "capture": True,
        "stderr": subprocess.STDOUT,
        "timeout": 60,
        "env": {**os.environ, **METAL_VALIDATION},
    }
    frame = run([executables / "Xenora", "--smoke-frame"], **options)
    if "status=0 frame=4x3 first_pixel=255,0,0,255" not in frame.splitlines():
        raise RuntimeError("packaged app did not reproduce the original fixture frame")

    worker = run(
        [executables / "xenora-cli", "run", "--experimental", "--startup-report",
         "--pe", source_root / FIXTURE],
        **options,
    )
    prefix = "xenora-startup "
    reports = [line[len(prefix):] for line in worker.splitlines() if line.startswith(prefix)]
    if len(reports) != 1:
        raise RuntimeError("packaged worker did not produce one startup report")
    report = json.loads(reports[0])
    if (
        report.get("last_completed_stage") != "framebuffer_verified"
        or report.get("protocol") != "complete"
        or report.get("outcome") != {"kind": "verified", "worker_code": 0}
        or "trace stage=framebuffer extent=4x3 verified_pixels=12" not in worker.splitlines()
    ):
        raise RuntimeError("packaged worker did not verify all 12 original fixture pixels")
    return {
        "fixture": FIXTURE,
        "metal_validation_environment": METAL_VALIDATION,
        "app": {"width": 4, "height": 3, "first_pixel_rgba": [255, 0, 0, 255]},
        "worker": {"startup_stage": "framebuffer_verified", "verified_pixels": 12},
    }


def check_archive(directory: Path, extraction: Path, source: Source, mode: str, *, run):
    extraction.mkdir()
    app = extract_app(directory / APP_ARCHIVE, extraction, run=run)
    evidence = inspect_app(app, run=run)
    if any(evidence["info_plist"].get(key) != value for key, value in source.bundle_fields().items()):
        raise ValueError("signed app version or source does not match the preview manifest")
    if mode == "developer-id":
        verify_distribution(app, run=run)
    else:
        verify_local(app, run=run)
    return app, evidence


def build(
    checkout: Path,
    destination: Path,
    build_number: str,
    *,
    identity: str | None = None,
    profile: str | None = None,
    run=run,
) -> None:
    validate_build_number(build_number)
    if (identity is None) != (profile is None):
        raise ValueError("provide both --developer-id and --notary-profile for Developer ID mode")
    for value in (identity, profile):
        if value is not None and (not value.strip() or value.startswith("-")):
            raise ValueError("signing identity and notary profile must be nonempty names")

    destination = destination.parent.resolve(strict=True) / destination.name
    destination.mkdir()  # Reserve a new, empty output path. An existing preview is never reused.
    published = False
    try:
        with TemporaryDirectory(prefix=".xenora-preview-", dir=destination.parent) as temp:
            temporary = Path(temp)
            source_root = temporary / "source"
            source = snapshot(checkout, source_root, build_number, run=run)
            toolchain = environment(run=run)
            # Use the native build's own expected target layout in the isolated snapshot.
            build_env = {**os.environ, "CARGO_TARGET_DIR": str(source_root / "target")}
            run(["sh", "scripts/build-app.sh"], cwd=source_root, env=build_env)
            app = source_root / "build" / APP_NAME
            stamp_bundle(app, source)
            sign_bundle(app, identity=identity, run=run)

            mode = "ad-hoc"
            submission = None
            if profile is not None:
                # Exercise trusted code under Hardened Runtime before sending the archive to Apple.
                check_fixture(app, source_root, run=run)
                submission_zip = temporary / "for-notarization.zip"
                archive_app(app, submission_zip, run=run)
                submission = notarize(app, submission_zip, profile=profile, run=run)
                mode = "developer-id"

            staged = temporary / "delivery"
            staged.mkdir()
            archive_app(app, staged / APP_ARCHIVE, run=run)
            extracted, app_evidence = check_archive(staged, temporary / "extracted", source, mode, run=run)
            fixtures = check_fixture(extracted, source_root, run=run)
            copy_notices(source_root, staged, source, mode)
            write_manifest(
                staged, source, mode=mode, submission_id=submission, app=app_evidence,
                build={"environment": toolchain, "locked_app_build_and_smoke": True, "fixtures": fixtures},
            )
            staged.rename(destination)
            published = True
    finally:
        if not published:
            destination.rmdir()


def verify(directory: Path, *, require_developer_id=False, run=run):
    source, mode = read_verified_manifest(directory)
    if require_developer_id and mode != "developer-id":
        raise ValueError("preview manifest does not identify a Developer ID build")
    with TemporaryDirectory(prefix="xenora-preview-verify-") as temp:
        check_archive(directory, Path(temp) / "extracted", source, mode, run=run)
    return source, mode


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    package = commands.add_parser("build", help="build a clean Git revision into a new preview directory")
    package.add_argument("--output", type=Path, required=True)
    package.add_argument("--build-number", required=True, help="incrementing build number, e.g. 42 or 1.2.3")
    package.add_argument("--developer-id", help="Developer ID Application signing identity")
    package.add_argument("--notary-profile", help="explicitly submit to Apple using this notarytool profile")
    check = commands.add_parser("verify", help="verify preview checksums and macOS bundle without launching it")
    check.add_argument("directory", type=Path)
    check.add_argument("--require-developer-id", action="store_true")
    args = parser.parse_args(argv)

    if platform.system() != "Darwin" or platform.machine() != "arm64":
        parser.error("run on native Apple Silicon macOS")
    try:
        if args.command == "build":
            build(Path.cwd(), args.output, args.build_number, identity=args.developer_id, profile=args.notary_profile)
            print(f"Preview written to {args.output}")
        else:
            source, mode = verify(args.directory, require_developer_id=args.require_developer_id)
            print(f"Verified {mode} preview {source.version} build {source.build_number} at {source.revision}")
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(f"preview: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
