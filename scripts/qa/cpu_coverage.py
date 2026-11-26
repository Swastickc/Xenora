# SPDX-License-Identifier: GPL-3.0-or-later
"""Read-only CPU and runtime coverage intelligence.

Consumes existing source inventories and reports without modifying CPU/runtime
implementation or executable authorization policies.
"""
from __future__ import annotations

import json
import re
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Iterable


# Opcode tables are discovered from existing loader sources when present.
OPCODE_MATCH = re.compile(
    r"(?:match\s+opcode|Op::|opcode\s*=>|0x([0-9A-Fa-f]{2}))"
)
IMPORT_MATCH = re.compile(
    r'^\s*(?:pub\s+)?(?:const|static)?\s*(?:NAME|DLL|dll|module)?[^\"\n]*\"([A-Za-z0-9_.]+)\"',
    re.MULTILINE,
)
HANDLER_KIND_MATCH = re.compile(
    r"(Unsupported|Todo|todo!|unimplemented!|alias|Alias|forward|Forward)",
)


@dataclass
class OpcodeEntry:
    opcode: str
    status: str  # supported | unsupported | unknown
    source: str


@dataclass
class ImportEntry:
    name: str
    dll: str
    kind: str  # functional | alias | partial | stub | missing | unknown
    source: str


@dataclass
class CoverageReport:
    opcodes: list[OpcodeEntry] = field(default_factory=list)
    imports: list[ImportEntry] = field(default_factory=list)
    interpreter_symbols: list[str] = field(default_factory=list)
    dbt_symbols: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "opcodes": [asdict(o) for o in self.opcodes],
            "imports": [asdict(i) for i in self.imports],
            "interpreter_symbols": self.interpreter_symbols,
            "dbt_symbols": self.dbt_symbols,
            "notes": self.notes,
            "counts": {
                "opcodes": len(self.opcodes),
                "imports": len(self.imports),
                "interpreter_only": sorted(set(self.interpreter_symbols) - set(self.dbt_symbols)),
                "dbt_only": sorted(set(self.dbt_symbols) - set(self.interpreter_symbols)),
            },
        }


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8") if path.exists() else ""


def scan_opcodes(text: str, *, source: str) -> list[OpcodeEntry]:
    found: dict[str, OpcodeEntry] = {}
    for match in re.finditer(r"0x([0-9A-Fa-f]{2})", text):
        op = match.group(1).lower()
        line_start = text.rfind("\n", 0, match.start()) + 1
        line_end = text.find("\n", match.end())
        if line_end < 0:
            line_end = len(text)
        line = text[line_start:line_end]
        if re.search(r"unsupported|illegal|todo!|unimplemented!", line, re.I):
            status = "unsupported"
        elif re.search(r"match|Op::|opcode|=>", line, re.I):
            status = "supported"
        else:
            status = "unknown"
        prev = found.get(op)
        # Prefer explicit unsupported/supported over unknown; do not let a later
        # unsupported line rewrite an earlier supported opcode with the same hex.
        if prev is None:
            found[op] = OpcodeEntry(opcode=op, status=status, source=source)
        elif prev.status == "unknown" and status != "unknown":
            found[op] = OpcodeEntry(opcode=op, status=status, source=source)
    return [found[k] for k in sorted(found)]


def scan_imports(text: str, *, source: str, default_dll: str = "unknown") -> list[ImportEntry]:
    """Extract explicit service registrations, not every quoted Rust string.

    The report is heuristic, but an import entry must still come from a
    registration-shaped construct. Unit-test modules are ignored because their
    synthetic services are not runtime surface area.
    """
    production = text.split("#[cfg(test)]", 1)[0]
    constants = dict(re.findall(
        r'\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+\.dll)"',
        production,
    ))

    def module(expression: str) -> str:
        expression = expression.strip()
        literal = re.fullmatch(r'"([^"]+\.dll)"', expression)
        if literal:
            return literal.group(1)
        return constants.get(expression, default_dll)

    entries: list[ImportEntry] = []

    metadata = re.compile(
        r'library:\s*"([^"]+\.dll)"\s*,\s*symbol:\s*"([^"]+)"'
        r"[\s\S]{0,1200}?status:\s*ServiceStatus::(Implemented|Partial|Stub)"
    )
    for match in metadata.finditer(production):
        status = match.group(3)
        entries.append(ImportEntry(
            name=match.group(2),
            dll=match.group(1),
            kind="functional" if status == "Implemented" else status.lower(),
            source=source,
        ))

    registration = re.compile(
        r"\b(?P<method>(?:try_)?register(?:_[A-Za-z]+)*)"
        r'\s*\(\s*(?P<module>"[^"]+\.dll"|[A-Z][A-Z0-9_]*)'
        r'\s*,\s*(?:ImportName::Name\s*\(\s*)?"(?P<symbol>[^"]+)"'
    )
    for match in registration.finditer(production):
        tail = production[match.end():match.end() + 160]
        status = re.match(
            r"\s*(?:\.to_owned\s*\(\s*\)\s*)?\s*\)*\s*,"
            r"\s*ServiceStatus::(Implemented|Partial|Stub)",
            tail,
        )
        if status:
            kind = "functional" if status.group(1) == "Implemented" else status.group(1).lower()
        elif match.group("method") == "register":
            kind = "functional"
        else:
            kind = "unknown"
        entries.append(ImportEntry(
            name=match.group("symbol"),
            dll=module(match.group("module")),
            kind=kind,
            source=source,
        ))

    alias = re.compile(
        r"\b(?:try_)?(?:ordinal_)?alias(?:_key)?\s*\(\s*"
        r'(?P<module>"[^"]+\.dll"|[A-Z][A-Z0-9_]*)'
        r'\s*,\s*(?:"[^"]+\.dll"|[A-Z][A-Z0-9_]*)'
        r'\s*,\s*(?:ImportName::Name\s*\(\s*)?"(?P<symbol>[^"]+)"'
    )
    for match in alias.finditer(production):
        entries.append(ImportEntry(
            name=match.group("symbol"),
            dll=module(match.group("module")),
            kind="alias",
            source=source,
        ))

    # Deduplicate by module and symbol, keeping the strongest kind. Windows
    # module and exported-symbol lookup is case-insensitive.
    rank = {"functional": 4, "partial": 3, "alias": 2, "stub": 1, "missing": 0, "unknown": 0}
    best: dict[tuple[str, str], ImportEntry] = {}
    for entry in entries:
        key = (entry.dll.lower(), entry.name.lower())
        cur = best.get(key)
        if cur is None or rank[entry.kind] > rank[cur.kind]:
            best[key] = entry
    return [best[k] for k in sorted(best)]


def scan_symbols(text: str, pattern: str) -> list[str]:
    return sorted(set(re.findall(pattern, text)))


def build_report(root: Path) -> CoverageReport:
    report = CoverageReport(notes=[
        "Read-only scan of existing sources; not an architectural claim of completeness.",
        "Statuses are heuristic over source text where no authoritative inventory file exists.",
    ])
    cpu = root / "crates" / "loader" / "src" / "cpu.rs"
    dbt = root / "crates" / "loader" / "src" / "dbt.rs"
    probe = root / "crates" / "loader" / "src" / "probe.rs"
    if cpu.exists():
        text = _read(cpu)
        report.opcodes.extend(scan_opcodes(text, source=str(cpu.relative_to(root))))
        report.interpreter_symbols = scan_symbols(text, r"\bfn\s+(execute_[a-z0-9_]+)\b")
    if dbt.exists():
        text = _read(dbt)
        report.dbt_symbols = scan_symbols(text, r"\bfn\s+(lower_[a-z0-9_]+|emit_[a-z0-9_]+)\b")
        report.opcodes.extend(scan_opcodes(text, source=str(dbt.relative_to(root))))
    if probe.exists():
        report.opcodes.extend(scan_opcodes(_read(probe), source=str(probe.relative_to(root))))

    # Import generation / service registries when present.
    for path in sorted((root / "crates").rglob("*.rs")):
        inventory_file = {
            "imports.rs", "services.rs", "win32.rs", "registry.rs", "runtime_table.rs",
        }
        if path.name in inventory_file or "import" in path.name:
            rel = str(path.relative_to(root))
            report.imports.extend(scan_imports(_read(path), source=rel, default_dll=path.stem))

    # Dedup opcodes preferring supported over unsupported over unknown
    rank = {"supported": 2, "unsupported": 1, "unknown": 0}
    best: dict[str, OpcodeEntry] = {}
    for entry in report.opcodes:
        cur = best.get(entry.opcode)
        if cur is None or rank[entry.status] > rank[cur.status]:
            best[entry.opcode] = entry
    report.opcodes = [best[k] for k in sorted(best)]
    return report


def compare_reports(before: dict[str, Any], after: dict[str, Any]) -> dict[str, Any]:
    before_ops = {o["opcode"]: o["status"] for o in before.get("opcodes", [])}
    after_ops = {o["opcode"]: o["status"] for o in after.get("opcodes", [])}
    newly_supported = sorted(op for op, st in after_ops.items() if st == "supported" and before_ops.get(op) != "supported")
    newly_unsupported = sorted(op for op, st in after_ops.items() if st == "unsupported" and before_ops.get(op) != "unsupported")
    missing = sorted(set(before_ops) - set(after_ops))
    return {
        "newly_supported": newly_supported,
        "newly_unsupported": newly_unsupported,
        "missing_opcodes": missing,
        "interpreter_only_delta": sorted(
            set(after.get("counts", {}).get("interpreter_only", []))
            - set(before.get("counts", {}).get("interpreter_only", []))
        ),
    }


def blocker_manifest(report: CoverageReport, *, limit: int = 50) -> list[dict[str, Any]]:
    blockers = []
    for entry in report.opcodes:
        if entry.status == "unsupported":
            blockers.append({
                "kind": "opcode",
                "id": entry.opcode,
                "priority": "high",
                "source": entry.source,
                "task": f"Implement or document x86 opcode 0x{entry.opcode}",
            })
    for entry in report.imports:
        if entry.kind in {"partial", "stub", "missing"}:
            blockers.append({
                "kind": "import",
                "id": entry.name,
                "priority": "medium",
                "source": entry.source,
                "task": f"Replace {entry.kind} import handler for {entry.name}",
            })
    for sym in sorted(set(report.interpreter_symbols) - set(report.dbt_symbols))[:limit]:
        blockers.append({
            "kind": "dbt-gap",
            "id": sym,
            "priority": "medium",
            "source": "interpreter-vs-dbt",
            "task": f"Consider native DBT coverage for {sym}",
        })
    return blockers[:limit]


def dumps_report(report: CoverageReport) -> str:
    return json.dumps(report.to_dict(), indent=2, sort_keys=True) + "\n"
