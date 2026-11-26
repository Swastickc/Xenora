#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

"""Generate the deterministic live runtime-service inventory.

The live data source is the small `runtime_inventory` Cargo example in
`crates/loader`. It constructs the standard `ImportRegistry` and prints every
canonical registration and resolution key as JSON. No PE image is loaded, no
guest instruction runs and no service handler is called.
"""

import argparse
import csv
import difflib
import html
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT_DIR = Path("docs/runtime")
JSON_NAME = "inventory.json"
MARKDOWN_NAME = "inventory.md"
SNAPSHOT_SCHEMA = 1
REPORT_SCHEMA = 1
MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024
MAX_WANTED_BYTES = 1024 * 1024
MAX_SERVICES = 10_000
MAX_KEYS = 50_000
MAX_TYPED = 10_000
MAX_WANTED_ROWS = 16_384
MAX_NAME_LEN = 4096
MAX_FAMILY_LEN = 80
STATUSES = ("implemented", "partial", "stub")
WANTED_STATUSES = (*STATUSES, "missing")
KINDS = ("name", "ordinal")


class InventoryError(ValueError):
    """Input or live snapshot is outside the documented inventory format."""


def read_bounded(path, limit, description):
    try:
        with path.open("rb") as source:
            data = source.read(limit + 1)
    except OSError as error:
        raise InventoryError(f"{path}: {error}") from error
    if len(data) > limit:
        raise InventoryError(f"{path}: {description} exceeds {limit} bytes")
    return data


def printable(value, where, maximum=MAX_NAME_LEN, allow_space=False):
    lowest = 0x20 if allow_space else 0x21
    if (
        not isinstance(value, str)
        or not value
        or len(value) > maximum
        or any(not (lowest <= ord(character) <= 0x7E) for character in value)
    ):
        raise InventoryError(f"{where}: expected nonempty printable ASCII of at most {maximum} bytes")
    return value


def boolean(value, where):
    if type(value) is not bool:
        raise InventoryError(f"{where}: expected a boolean")
    return value


def exact_fields(row, fields, where):
    if not isinstance(row, dict) or set(row) != set(fields):
        raise InventoryError(f"{where}: expected exactly the fields {', '.join(fields)}")


def normalized_ordinal(value, where):
    if not isinstance(value, str):
        raise InventoryError(f"{where}: expected an ordinal string")
    digits = value.removeprefix("Ordinal#")
    if not digits or not digits.isascii() or not digits.isdigit():
        raise InventoryError(f"{where}: expected a decimal ordinal or Ordinal#<number>")
    # Strip leading zeros before conversion so the decimal conversion itself is
    # bounded to the five significant digits of a 16-bit ordinal. Python's
    # bigint conversion limit otherwise turns a very long row into a traceback.
    significant = digits.lstrip("0") or "0"
    if len(significant) > len(str(0xFFFF)):
        raise InventoryError(f"{where}: ordinal is outside the 16-bit PE ordinal range")
    try:
        number = int(significant, 10)
    except ValueError as error:
        raise InventoryError(f"{where}: expected a decimal ordinal") from error
    if number > 0xFFFF:
        raise InventoryError(f"{where}: ordinal is outside the 16-bit PE ordinal range")
    return f"Ordinal#{number}"


def normalized_key(library, kind, symbol):
    return (library.lower(), kind, symbol)


def service_key(row):
    return normalized_key(row["library"], row["kind"], row["symbol"])


def target_key(row):
    return normalized_key(row["canonical_library"], row["canonical_kind"], row["canonical_symbol"])


def identity_text(identity):
    library, kind, symbol = identity
    return f"{library}!{symbol} ({kind})"


def service_sort_key(row):
    return service_key(row)


def key_sort_key(row):
    return (*service_key(row), *target_key(row), row["canonical"], row["api_set"])


def module_family(library):
    """Assign the documented fallback family for a wanted import or service."""
    module = library.lower()
    if module in {"ucrtbase.dll", "msvcrt.dll"} or module.startswith("api-ms-win-crt-"):
        return "c-runtime"
    if module in {"libgcc_s_seh-1.dll", "libstdc++-6.dll", "libwinpthread-1.dll"}:
        return "c++-support"
    if module == "ws2_32.dll":
        return "windows-networking"
    if module in {"xmem.dll", "xgameruntime.dll"}:
        return "xbox-gaming"
    if module.startswith("xenora"):
        return "xenora-fixture"
    if (
        module in {"kernel32.dll", "kernelbase.dll", "advapi32.dll", "psapi.dll"}
        or module.startswith("api-ms-win-core-")
        or module.startswith("api-ms-win-downlevel-")
        or module.startswith("ext-ms-")
    ):
        return "windows-system"
    return "other"


def kind(value, where):
    if value not in KINDS:
        raise InventoryError(f"{where}: expected name or ordinal")
    return value


def symbol_for_kind(value, symbol_kind, where):
    if symbol_kind == "ordinal":
        return normalized_ordinal(value, where)
    return printable(value, where)


def validate_snapshot_structure(services, keys):
    """Validate the referential structure every snapshot consumer relies on."""
    service_ids = {service_key(row) for row in services}
    canonical_counts = {}
    for index, row in enumerate(keys):
        where = f"keys[{index}]"
        source = service_key(row)
        target = target_key(row)
        if target not in service_ids:
            raise InventoryError(
                f"{where}: canonical target {identity_text(target)} has no service row; "
                "check the target library, symbol and kind"
            )
        if row["canonical"]:
            if source != target:
                raise InventoryError(
                    f"{where}: a canonical row must resolve to its own library, symbol and kind, "
                    f"not {identity_text(target)}"
                )
            canonical_counts[target] = canonical_counts.get(target, 0) + 1
        elif source == target:
            raise InventoryError(f"{where}: a noncanonical row resolves to itself")

    for service_id in sorted(service_ids):
        count = canonical_counts.get(service_id, 0)
        if count != 1:
            raise InventoryError(
                f"service {identity_text(service_id)}: expected exactly one canonical "
                f"resolution key, found {count}"
            )


def parse_snapshot(data):
    if len(data) > MAX_SNAPSHOT_BYTES:
        raise InventoryError(f"live registry snapshot exceeds {MAX_SNAPSHOT_BYTES} bytes")
    try:
        document = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise InventoryError(f"live registry snapshot is not valid JSON: {error}") from error
    if (
        not isinstance(document, dict)
        or type(document.get("schema")) is not int
        or document["schema"] != SNAPSHOT_SCHEMA
    ):
        raise InventoryError(f"live registry snapshot: expected schema {SNAPSHOT_SCHEMA}")
    if set(document) != {"schema", "services", "keys", "typed"}:
        raise InventoryError("live registry snapshot: expected schema, services, keys and typed")

    services = document["services"]
    keys = document["keys"]
    typed = document["typed"]
    if not isinstance(services, list) or not services or len(services) > MAX_SERVICES:
        raise InventoryError(f"services: expected 1 to {MAX_SERVICES} rows")
    if not isinstance(keys, list) or not keys or len(keys) > MAX_KEYS:
        raise InventoryError(f"keys: expected 1 to {MAX_KEYS} rows")
    if not isinstance(typed, list) or len(typed) > MAX_TYPED:
        raise InventoryError(f"typed: expected at most {MAX_TYPED} rows")

    for index, row in enumerate(services):
        where = f"services[{index}]"
        exact_fields(
            row,
            ("library", "symbol", "kind", "status", "status_explicit", "source", "data_import"),
            where,
        )
        printable(row["library"], f"{where}.library")
        row["kind"] = kind(row["kind"], f"{where}.kind")
        row["symbol"] = symbol_for_kind(row["symbol"], row["kind"], f"{where}.symbol")
        if row["status"] not in STATUSES:
            raise InventoryError(f"{where}.status: unknown status {row['status']!r}")
        boolean(row["status_explicit"], f"{where}.status_explicit")
        if row["source"] not in {"typed", "manual"}:
            raise InventoryError(f"{where}.source: expected typed or manual")
        boolean(row["data_import"], f"{where}.data_import")

    for index, row in enumerate(keys):
        where = f"keys[{index}]"
        exact_fields(
            row,
            (
                "library",
                "symbol",
                "kind",
                "canonical_library",
                "canonical_symbol",
                "canonical_kind",
                "canonical",
                "api_set",
            ),
            where,
        )
        printable(row["library"], f"{where}.library")
        printable(row["canonical_library"], f"{where}.canonical_library")
        row["kind"] = kind(row["kind"], f"{where}.kind")
        row["canonical_kind"] = kind(row["canonical_kind"], f"{where}.canonical_kind")
        row["symbol"] = symbol_for_kind(row["symbol"], row["kind"], f"{where}.symbol")
        row["canonical_symbol"] = symbol_for_kind(
            row["canonical_symbol"], row["canonical_kind"], f"{where}.canonical_symbol"
        )
        boolean(row["canonical"], f"{where}.canonical")
        boolean(row["api_set"], f"{where}.api_set")

    for index, row in enumerate(typed):
        where = f"typed[{index}]"
        exact_fields(row, ("library", "symbol", "kind", "status", "aliases"), where)
        printable(row["library"], f"{where}.library")
        printable(row["symbol"], f"{where}.symbol")
        if row["kind"] != "name":
            raise InventoryError(f"{where}.kind: typed registry entries are always named")
        if row["status"] not in STATUSES:
            raise InventoryError(f"{where}.status: unknown status {row['status']!r}")
        if not isinstance(row["aliases"], list) or any(
            not isinstance(alias, str) for alias in row["aliases"]
        ):
            raise InventoryError(f"{where}.aliases: expected a list of API-set names")
        row["aliases"] = [printable(alias, f"{where}.aliases[]") for alias in row["aliases"]]

    validate_snapshot_structure(services, keys)
    return {
        "services": sorted(services, key=service_sort_key),
        "keys": sorted(keys, key=key_sort_key),
        "typed": sorted(typed, key=service_sort_key),
    }


def first_unique(rows, key):
    result = {}
    for row in rows:
        result.setdefault(key(row), row)
    return result


def duplicate_rows(rows, key):
    seen = {}
    duplicates = []
    for row in rows:
        identifier = key(row)
        if identifier in seen:
            duplicates.append({name: row[name] for name in ("library", "symbol", "kind") if name in row})
        else:
            seen[identifier] = row
    return duplicates


def status_counts(rows):
    return {status: sum(row["status"] == status for row in rows) for status in STATUSES}


def analyze_snapshot(snapshot):
    services = snapshot["services"]
    keys = snapshot["keys"]
    typed = snapshot["typed"]

    service_by_key = first_unique(services, service_key)
    key_by_key = first_unique(keys, service_key)
    typed_by_key = first_unique(typed, service_key)

    duplicate_services = duplicate_rows(services, service_key)
    duplicate_keys = duplicate_rows(keys, service_key)
    duplicate_typed = duplicate_rows(typed, service_key)

    aliases = []
    missing_targets = []
    conflicting_keys = []
    for row in keys:
        identifier = service_key(row)
        first = key_by_key.get(identifier)
        if first is not None and first is not row and target_key(first) != target_key(row):
            conflicting_keys.append(
                {
                    "library": row["library"],
                    "symbol": row["symbol"],
                    "kind": row["kind"],
                }
            )
        if row["canonical"]:
            continue
        target = service_by_key.get(target_key(row))
        if target is None:
            missing_targets.append(
                {
                    "library": row["library"],
                    "symbol": row["symbol"],
                    "kind": row["kind"],
                    "canonical_library": row["canonical_library"],
                    "canonical_symbol": row["canonical_symbol"],
                    "canonical_kind": row["canonical_kind"],
                }
            )
        aliases.append(
            {
                "library": row["library"],
                "symbol": row["symbol"],
                "kind": row["kind"],
                "canonical_library": row["canonical_library"],
                "canonical_symbol": row["canonical_symbol"],
                "canonical_kind": row["canonical_kind"],
                "api_set": row["api_set"],
                "status": target["status"] if target else "missing",
            }
        )

    typed_missing = []
    typed_status_mismatches = []
    typed_alias_missing = []
    source_mismatches = []
    for row in typed:
        identifier = normalized_key(row["library"], "name", row["symbol"])
        service = service_by_key.get(identifier)
        if service is None:
            typed_missing.append({"library": row["library"], "symbol": row["symbol"], "kind": "name"})
            continue
        if service["status"] != row["status"]:
            typed_status_mismatches.append(
                {
                    "library": row["library"],
                    "symbol": row["symbol"],
                    "kind": "name",
                    "typed_status": row["status"],
                    "live_status": service["status"],
                }
            )
        for alias in row["aliases"]:
            alias_key = normalized_key(alias, "name", row["symbol"])
            resolution = key_by_key.get(alias_key)
            if resolution is None or resolution["canonical"] or target_key(resolution) != identifier:
                typed_alias_missing.append(
                    {
                        "library": alias,
                        "symbol": row["symbol"],
                        "kind": "name",
                        "canonical_library": row["library"],
                    }
                )
    for row in services:
        identifier = service_key(row)
        expected = "typed" if identifier in typed_by_key else "manual"
        if row["source"] != expected:
            source_mismatches.append(
                {
                    "library": row["library"],
                    "symbol": row["symbol"],
                    "kind": row["kind"],
                    "expected_source": expected,
                    "actual_source": row["source"],
                }
            )

    implicit = [
        {
            "library": row["library"],
            "symbol": row["symbol"],
            "kind": row["kind"],
            "status": row["status"],
            "source": row["source"],
        }
        for row in services
        if not row["status_explicit"]
    ]
    services_out = [
        {
            "library": row["library"],
            "symbol": row["symbol"],
            "kind": row["kind"],
            "status": row["status"],
            "status_explicit": row["status_explicit"],
            "source": row["source"],
            "data_import": row["data_import"],
            "family": module_family(row["library"]),
        }
        for row in services
    ]

    counts = status_counts(services)
    findings = {
        "duplicate_registrations": duplicate_services,
        "duplicate_resolution_keys": duplicate_keys,
        "conflicting_resolution_keys": conflicting_keys,
        "duplicate_typed_entries": duplicate_typed,
        "aliases_missing_target": missing_targets,
        "implicit_status_services": implicit,
        "typed_entries_missing_live": typed_missing,
        "typed_status_mismatches": typed_status_mismatches,
        "typed_aliases_missing": typed_alias_missing,
        "source_mismatches": source_mismatches,
    }
    summary = {
        "canonical_services": len(services),
        "typed_services": sum(row["source"] == "typed" for row in services),
        "manual_services": sum(row["source"] == "manual" for row in services),
        "typed_registry_entries": len(typed),
        "libraries": len({row["library"].lower() for row in services}),
        "aliases": len(aliases),
        "api_set_aliases": sum(row["api_set"] for row in aliases),
        "ordinal_aliases": sum(row["kind"] == "ordinal" for row in aliases),
        "implemented": counts["implemented"],
        "partial": counts["partial"],
        "stub": counts["stub"],
        "implicit_status_services": len(implicit),
        "data_import_services": sum(row["data_import"] for row in services),
        "duplicate_registrations": len(duplicate_services),
        "structural_consistency_findings": sum(
            len(findings[name])
            for name in (
                "duplicate_registrations",
                "duplicate_resolution_keys",
                "conflicting_resolution_keys",
                "duplicate_typed_entries",
                "aliases_missing_target",
                "typed_entries_missing_live",
                "typed_status_mismatches",
                "typed_aliases_missing",
                "source_mismatches",
            )
        ),
        "wanted_imports": None,
        "wanted_missing": None,
    }
    report = {
        "schema": REPORT_SCHEMA,
        "summary": summary,
        "services": services_out,
        "aliases": aliases,
        "findings": findings,
        "wanted": None,
    }
    return report, service_by_key, key_by_key


def validate_family(value, where):
    if (
        not isinstance(value, str)
        or not value.strip()
        or len(value) > MAX_FAMILY_LEN
        or any(ord(character) < 0x20 or ord(character) == 0x7F for character in value)
    ):
        raise InventoryError(f"{where}: expected a nonempty family label of at most {MAX_FAMILY_LEN} characters")
    return value.strip()


def read_wanted(path):
    """Read probe imports.csv/missing_imports.csv or the documented simple CSV."""
    data = read_bounded(path, MAX_WANTED_BYTES, "wanted-imports input")
    try:
        text = data.decode("utf-8-sig")
    except UnicodeDecodeError as error:
        raise InventoryError(f"{path}: wanted-imports input is not UTF-8: {error}") from error

    try:
        reader = csv.DictReader(io.StringIO(text, newline=""))
        fields = reader.fieldnames
        if not fields or len(fields) != len(set(fields)):
            raise InventoryError(f"{path}: expected a unique CSV header row")
        if {"module", "symbol_or_ordinal"} <= set(fields):
            library_field = "module"
            symbol_field = "symbol_or_ordinal"
            input_format = "probe-csv"
        elif {"library", "symbol"} <= set(fields):
            library_field = "library"
            symbol_field = "symbol"
            input_format = "simple-csv"
        else:
            raise InventoryError(
                f"{path}: expected probe columns module,symbol_or_ordinal or simple columns library,symbol"
            )

        entries = {}
        row_count = 0
        for row_number, row in enumerate(reader, 2):
            row_count += 1
            if row_count > MAX_WANTED_ROWS:
                raise InventoryError(f"{path}: more than {MAX_WANTED_ROWS} wanted rows")
            where = f"{path}:{row_number}"
            if None in row or row.get(library_field) is None or row.get(symbol_field) is None:
                raise InventoryError(f"{where}: row does not match the CSV header")
            library = printable(row[library_field], f"{where}.library", allow_space=True)
            raw_symbol = row[symbol_field]
            kinds = [value for value in (row.get("kind"), row.get("import_kind")) if value]
            if len(set(kinds)) > 1:
                raise InventoryError(f"{where}: kind and import_kind disagree")
            symbol_kind = kinds[0] if kinds else "name"
            if symbol_kind not in KINDS:
                raise InventoryError(f"{where}: kind must be name or ordinal")
            if symbol_kind == "ordinal":
                symbol = normalized_ordinal(raw_symbol, f"{where}.symbol")
            else:
                symbol = printable(raw_symbol, f"{where}.symbol", allow_space=True)

            family = row.get("family")
            if not family and input_format == "probe-csv":
                family = row.get("category")
            if not family:
                family = module_family(library)
            family = validate_family(family, f"{where}.family")
            import_table = row.get("import_table") or ""
            if import_table and import_table not in {"ordinary", "delayed"}:
                raise InventoryError(f"{where}: import_table must be ordinary or delayed")

            identifier = normalized_key(library, symbol_kind, symbol)
            tables = {import_table} if import_table else set()
            if identifier in entries:
                entry = entries[identifier]
                if entry["family"] != family:
                    raise InventoryError(f"{where}: duplicate import has conflicting families")
                entry["occurrences"] += 1
                entry["import_tables"] |= tables
            else:
                entries[identifier] = {
                    "library": library,
                    "symbol": symbol,
                    "kind": symbol_kind,
                    "family": family,
                    "occurrences": 1,
                    "import_tables": tables,
                }
    except csv.Error as error:
        raise InventoryError(f"{path}: malformed CSV: {error}") from error

    ordered = sorted(
        entries.values(),
        key=lambda row: (row["family"], row["library"].lower(), row["kind"], row["symbol"]),
    )
    return {"format": input_format, "rows": row_count, "entries": ordered}


def match_wanted(wanted, service_by_key, key_by_key):
    matches = []
    for entry in wanted["entries"]:
        identifier = normalized_key(entry["library"], entry["kind"], entry["symbol"])
        resolution = key_by_key.get(identifier)
        canonical_library = None
        canonical_symbol = None
        canonical_kind = None
        status = "missing"
        resolution_kind = "missing"
        if resolution is not None:
            canonical_library = resolution["canonical_library"]
            canonical_symbol = resolution["canonical_symbol"]
            canonical_kind = resolution["canonical_kind"]
            target = service_by_key.get(target_key(resolution))
            if target is not None:
                status = target["status"]
                if resolution["canonical"]:
                    resolution_kind = "canonical"
                elif resolution["kind"] == "ordinal":
                    resolution_kind = "ordinal-alias"
                elif resolution["api_set"]:
                    resolution_kind = "api-set-alias"
                else:
                    resolution_kind = "module-alias"
        matches.append(
            {
                "family": entry["family"],
                "library": entry["library"],
                "symbol": entry["symbol"],
                "kind": entry["kind"],
                "status": status,
                "resolution": resolution_kind,
                "canonical_library": canonical_library,
                "canonical_symbol": canonical_symbol,
                "canonical_kind": canonical_kind,
                "occurrences": entry["occurrences"],
                "import_tables": sorted(entry["import_tables"]),
            }
        )

    overall = {status: 0 for status in WANTED_STATUSES}
    families = {}
    for row in matches:
        overall[row["status"]] += 1
        family = families.setdefault(
            row["family"],
            {"family": row["family"], "total": 0, **{status: 0 for status in WANTED_STATUSES}},
        )
        family["total"] += 1
        family[row["status"]] += 1
    return {
        "format": wanted["format"],
        "input_rows": wanted["rows"],
        "distinct_imports": len(matches),
        "status_counts": overall,
        "families": sorted(families.values(), key=lambda row: row["family"]),
        "matches": matches,
    }


def build_report(snapshot, wanted=None):
    report, service_by_key, key_by_key = analyze_snapshot(snapshot)
    if wanted is not None:
        matched = match_wanted(wanted, service_by_key, key_by_key)
        report["wanted"] = matched
        report["summary"]["wanted_imports"] = matched["distinct_imports"]
        report["summary"]["wanted_missing"] = matched["status_counts"]["missing"]
    return report


def render_json(report):
    return (json.dumps(report, ensure_ascii=True, indent=2, sort_keys=True) + "\n").encode("utf-8")


class SafeHtml(str):
    """HTML that `code` has already escaped for inclusion in a Markdown cell."""


def md_cell(value):
    text = str(value)
    if not isinstance(value, SafeHtml):
        text = html.escape(text, quote=False)
    return text.replace("|", r"\|")


def md_table(headers, rows):
    def line(cells):
        return "| " + " | ".join(md_cell(cell) for cell in cells) + " |"

    return [line(headers), line(["---"] * len(headers)), *(line(row) for row in rows)]


def code(text):
    escaped = html.escape(text, quote=False).replace("|", "&#124;")
    return SafeHtml(f"<code>{escaped}</code>")


def key_text(row):
    text = f"{row['library']}!{row['symbol']}"
    if "kind" in row:
        text += f" ({row['kind']})"
    return text


def render_markdown(report):
    summary = report["summary"]
    lines = [
        "# Runtime service inventory",
        "",
        "Generated by `scripts/audit/runtime_inventory.py`. Do not edit.",
        "",
        "The data comes from constructing the standard live `ImportRegistry` through",
        "`crates/loader/examples/runtime_inventory.rs`. No PE image is loaded, no guest",
        "instruction runs and no service handler is called. `implemented`, `partial` and",
        "`stub` describe implementation scope for the synthetic guest; they are not",
        "independent acceptance or compatibility claims.",
        "",
        "## Summary",
        "",
    ]
    rows = [
        ("Canonical services", summary["canonical_services"]),
        ("Typed registry services", summary["typed_services"]),
        ("Manual code registrations", summary["manual_services"]),
        ("Canonical libraries", summary["libraries"]),
        ("Implemented", summary["implemented"]),
        ("Partial", summary["partial"]),
        ("Stub", summary["stub"]),
        ("Data-import services", summary["data_import_services"]),
        ("Aliases of every kind", summary["aliases"]),
        ("API-set aliases", summary["api_set_aliases"]),
        ("Ordinal aliases", summary["ordinal_aliases"]),
        ("Statuses never explicitly set", summary["implicit_status_services"]),
        ("Duplicate canonical registrations", summary["duplicate_registrations"]),
        ("Structural consistency findings", summary["structural_consistency_findings"]),
    ]
    if report["wanted"] is not None:
        rows.extend(
            [
                ("Wanted imports", summary["wanted_imports"]),
                ("Wanted imports without a service", summary["wanted_missing"]),
            ]
        )
    lines.extend(md_table(("Measure", "Count"), rows))

    by_library = {}
    for row in report["services"]:
        library = by_library.setdefault(
            row["library"],
            {
                "family": row["family"],
                "total": 0,
                "typed": 0,
                "manual": 0,
                "data": 0,
                "implicit": 0,
                **{status: 0 for status in STATUSES},
            },
        )
        library["total"] += 1
        library[row["source"]] += 1
        library[row["status"]] += 1
        library["data"] += row["data_import"]
        library["implicit"] += not row["status_explicit"]

    lines.extend(["", "## Canonical services by library", ""])
    lines.extend(
        md_table(
            (
                "Library",
                "Family",
                "Total",
                "Typed",
                "Manual",
                "Data",
                "Implemented",
                "Partial",
                "Stub",
                "Implicit status",
            ),
            [
                (
                    code(name),
                    code(row["family"]),
                    row["total"],
                    row["typed"],
                    row["manual"],
                    row["data"],
                    row["implemented"],
                    row["partial"],
                    row["stub"],
                    row["implicit"],
                )
                for name, row in sorted(by_library.items())
            ],
        )
    )
    lines.extend(
        [
            "",
            "The full canonical rows, including kind, data-import, source and explicit-status",
            "metadata, are in `inventory.json`.",
            "",
            "## Aliases",
            "",
        ]
    )
    alias_kinds = {
        "API-set": sum(row["api_set"] and row["kind"] == "name" for row in report["aliases"]),
        "Other module": sum(not row["api_set"] and row["kind"] == "name" for row in report["aliases"]),
        "Ordinal": sum(row["kind"] == "ordinal" for row in report["aliases"]),
    }
    lines.extend(md_table(("Alias kind", "Count"), sorted(alias_kinds.items())))
    lines.extend(
        [
            "",
            "Every API-set, module and ordinal alias is listed in `inventory.json`.",
            "",
            "## Findings",
            "",
            "The registry rejects a duplicate key while it is being constructed, so a healthy",
            "live inventory has no duplicate or conflict rows. Malformed snapshots are rejected",
            "before analysis; structurally valid discrepancies remain visible here.",
            "",
            "### Status never explicitly set",
            "",
        ]
    )
    implicit = report["findings"]["implicit_status_services"]
    if implicit:
        lines.extend(
            md_table(
                ("Service", "Status", "Source"),
                [(code(key_text(row)), row["status"], row["source"]) for row in implicit],
            )
        )
    else:
        lines.append("None.")

    finding_sections = (
        ("duplicate_registrations", "Duplicate canonical registrations"),
        ("duplicate_resolution_keys", "Duplicate resolution keys"),
        ("conflicting_resolution_keys", "Conflicting resolution keys"),
        ("duplicate_typed_entries", "Duplicate typed-registry entries"),
        ("aliases_missing_target", "Aliases whose canonical target is absent"),
        ("typed_entries_missing_live", "Typed entries absent from the live registry"),
        ("typed_status_mismatches", "Typed/live status mismatches"),
        ("typed_aliases_missing", "Typed aliases absent from the live registry"),
        ("source_mismatches", "Typed/manual source mismatches"),
    )
    for field, title in finding_sections:
        lines.extend(["", f"### {title}", ""])
        rows = report["findings"][field]
        if not rows:
            lines.append("None.")
        else:
            lines.extend(
                md_table(
                    ("Row", "Detail"),
                    [
                        (
                            index,
                            code(json.dumps(row, ensure_ascii=True, sort_keys=True)),
                        )
                        for index, row in enumerate(rows, 1)
                    ],
                )
            )

    lines.extend(["", "## Wanted imports", ""])
    if report["wanted"] is None:
        lines.extend(
            [
                "No wanted-imports file was used for this committed report. The input format is",
                "documented in [registry.md](registry.md#wanted-imports-for-the-inventory).",
            ]
        )
    else:
        wanted = report["wanted"]
        lines.extend(
            [
                f"Input rows: {wanted['input_rows']}; distinct imports: {wanted['distinct_imports']}.",
                "Rows are grouped by the family supplied in the input, by a probe report's",
                "category, or by the documented module-family fallback when neither is present.",
                "",
            ]
        )
        lines.extend(
            md_table(
                ("Family", "Total", "Implemented", "Partial", "Stub", "Missing"),
                [
                    (
                        code(row["family"]),
                        row["total"],
                        row["implemented"],
                        row["partial"],
                        row["stub"],
                        row["missing"],
                    )
                    for row in wanted["families"]
                ],
            )
        )
        lines.extend(
            [
                "",
                "Every match, its resolution kind and its canonical target are listed in `inventory.json`.",
            ]
        )
    return ("\n".join(lines) + "\n").encode("utf-8")


def render_outputs(report):
    return {
        JSON_NAME: render_json(report),
        MARKDOWN_NAME: render_markdown(report),
    }


def write_or_check(outputs, output_dir, check):
    differences = []
    if not check:
        output_dir.mkdir(parents=True, exist_ok=True)
    for name, data in sorted(outputs.items()):
        path = output_dir / name
        if check:
            committed = path.read_bytes() if path.exists() else b""
            if committed != data:
                differences.append((path, committed, data))
        else:
            with tempfile.NamedTemporaryFile(dir=output_dir, delete=False) as temporary:
                temporary.write(data)
                temporary_path = Path(temporary.name)
            temporary_path.chmod(0o644)
            temporary_path.replace(path)

    if not check:
        print(f"Wrote {len(outputs)} runtime inventory reports to {output_dir}")
        return True
    if not differences:
        print(f"Runtime inventory reports in {output_dir} are up to date")
        return True
    for path, before, after in differences:
        print(f"{path}: runtime inventory report is stale", file=sys.stderr)
        before_lines = before.decode("utf-8", errors="replace").splitlines(keepends=True)
        after_lines = after.decode("utf-8", errors="replace").splitlines(keepends=True)
        diff = list(difflib.unified_diff(before_lines, after_lines, fromfile=str(path), tofile="regenerated", n=0))
        for line in diff[:10]:
            print(line.rstrip("\n"), file=sys.stderr)
        if len(diff) > 10:
            print("…", file=sys.stderr)
    return False


def live_snapshot():
    command = [
        "cargo",
        "run",
        "--locked",
        "--quiet",
        "-p",
        "xenora-loader",
        "--example",
        "runtime_inventory",
    ]
    environment = dict(os.environ)
    environment["CARGO_TERM_COLOR"] = "never"
    try:
        result = subprocess.run(
            command,
            cwd=ROOT,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=300,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise InventoryError(f"could not run the live registry snapshot: {error}") from error
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip().splitlines()[-12:]
        raise InventoryError(
            "cargo example runtime_inventory exited "
            f"{result.returncode}\n" + "\n".join(detail)
        )
    return result.stdout


def output_path(path):
    return path if path.is_absolute() else ROOT / path


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare committed reports without writing")
    parser.add_argument("--wanted", type=Path, help="probe imports.csv/missing_imports.csv or simple wanted-import CSV")
    parser.add_argument("--snapshot", type=Path, help="read a saved live-registry JSON snapshot instead of running Cargo")
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_DIR, help="report directory, relative to the repository root unless absolute")
    args = parser.parse_args(argv)

    try:
        if args.snapshot:
            snapshot_data = read_bounded(args.snapshot, MAX_SNAPSHOT_BYTES, "live registry snapshot")
        else:
            snapshot_data = live_snapshot()
        snapshot = parse_snapshot(snapshot_data)
        wanted = read_wanted(args.wanted) if args.wanted else None
        report = build_report(snapshot, wanted)
        outputs = render_outputs(report)
        return 0 if write_or_check(outputs, output_path(args.output_dir), args.check) else 1
    except InventoryError as error:
        print(f"runtime inventory: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
