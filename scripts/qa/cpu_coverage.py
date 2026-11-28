# SPDX-License-Identifier: GPL-3.0-or-later
"""Read-only CPU and runtime coverage intelligence.

Consumes existing source inventories and reports without modifying CPU/runtime
implementation or executable authorization policies.
"""
from __future__ import annotations

import json
import re
import tomllib
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class OpcodeEntry:
    opcode: str
    status: str  # decodable | unsupported | unknown
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
    """Read decodable opcode identities from the declarative x86 registry.

    Hexadecimal bytes in implementation or fixture code are not evidence. The
    identity includes the encoding space, map and mandatory prefix because the
    same byte can denote different instructions in those contexts.
    """
    data = tomllib.loads(text)
    found: dict[str, OpcodeEntry] = {}

    def key(space: str, map_name: str, prefix: str, opcode: int) -> str:
        mandatory = f"+{prefix}" if prefix else ""
        return f"{space}:{map_name}{mandatory}:{opcode:02x}"

    def add(space: str, form: dict, opcode: int | None = None) -> None:
        value = form["opcode"] if opcode is None else opcode
        identity = key(
            space,
            form.get("map", "1"),
            form.get("prefix", ""),
            value,
        )
        found.setdefault(identity, OpcodeEntry(identity, "decodable", source))

    for form in data.get("form", []):
        for opcode in range(form["opcode"], form["opcode"] + form.get("count", 1)):
            add("legacy", form, opcode)

    # These expansions mirror the declarative registry's documented compression:
    # six forms at operation_index * 8 plus the immediate groups 80, 81 and 83.
    for index, _operation in enumerate(data.get("alu", {}).get("operations", [])):
        base = index * 8
        for opcode in (*range(base, base + 6), 0x80, 0x81, 0x83):
            add("legacy", {"opcode": opcode}, opcode)

    for group in data.get("group", []):
        for opcode, _size, _imm, _enc in group.get("opcodes", []):
            form = {"opcode": opcode, "map": group.get("map", "1"), "prefix": group.get("prefix", "")}
            add("legacy", form, opcode)

    for form in data.get("vex", []):
        for opcode in range(form["opcode"], form["opcode"] + form.get("count", 1)):
            add("vex", form, opcode)

    return [found[k] for k in sorted(found)]



def _blank_comments_and_strings(text: str) -> str:
    """A same-length copy of the Rust source with the insides of comments,
    strings, raw strings and char literals replaced by spaces (newlines
    preserved). Structure scanning over the copy is safe from punctuation
    inside them; block comments nest in Rust and lifetimes are not char
    literals, so this small lexer is required rather than a regex.
    """
    out = list(text)
    pos = 0
    size = len(text)

    def blank(start: int, end: int) -> None:
        for index in range(start, end):
            if out[index] != "\n":
                out[index] = " "

    while pos < size:
        char = text[pos]
        rest = text[pos:]
        if rest.startswith("//"):
            end = text.find("\n", pos)
            end = size if end < 0 else end
            blank(pos, end)
            pos = end
        elif rest.startswith("/*"):
            depth = 0
            end = pos
            while end < size:
                if text.startswith("/*", end):
                    depth += 1
                    end += 2
                elif text.startswith("*/", end):
                    depth -= 1
                    end += 2
                    if depth == 0:
                        break
                else:
                    end += 1
            blank(pos, end)
            pos = end
        elif char == 'r' or (char == 'b' and rest[1:2] == 'r'):
            raw = re.match(r"(?:br|r)(#*)\"", rest)
            if raw:
                hashes = raw.group(1)
                close = '"' + hashes
                start = pos + len(raw.group(0))
                end = text.find(close, start)
                end = size if end < 0 else end + len(close)
                blank(pos, end)
                pos = end
            elif char == 'b' and rest[1:2] == '"':
                end = pos + 2
                while end < size:
                    if text[end] == "\\":
                        end += 2
                    elif text[end] == '"':
                        end += 1
                        break
                    else:
                        end += 1
                blank(pos, end)
                pos = end
            else:
                pos += 1
        elif char == '"':
            end = pos + 1
            while end < size:
                if text[end] == "\\":
                    end += 2
                elif text[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
            blank(pos, end)
            pos = end
        elif char == "'":
            literal = re.match(r"'(?:\\(?:[^u]|u\{[0-9a-fA-F]+\})|[^\\'])'", rest)
            if literal:
                blank(pos, pos + len(literal.group(0)))
                pos += len(literal.group(0))
            else:
                pos += 1
        else:
            pos += 1
    return "".join(out)


_TEST_ATTRIBUTE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")


def strip_test_gated_items(text: str) -> str:
    """Production Rust source: the items a `#[cfg(test)]` attribute gates are
    removed - and nothing else. Splitting the file at the first attribute
    discards every production registration after a test module or a single
    test-only function, so the item each attribute annotates is found by its
    own shape: a brace body matched over the comment- and string-blanked
    copy, or a semicolon declaration.
    """
    structure = _blank_comments_and_strings(text)
    spans: list[tuple[int, int]] = []
    for attribute in _TEST_ATTRIBUTE.finditer(structure):
        pos = attribute.end()
        while True:
            while pos < len(structure) and structure[pos].isspace():
                pos += 1
            chained = re.match(r"#\s*\[", structure[pos:])
            if chained:
                close = structure.find("]", pos)
                if close < 0:
                    break
                pos = close + 1
            else:
                break
        # The item boundary is the first semicolon or opening brace at
        # delimiter depth zero: a semicolon inside a type such as `[u8; 1]`
        # or a parameter list does not end the item, and a brace inside
        # parentheses is not the body.
        depth_paren = 0
        depth_bracket = 0
        brace = -1
        semicolon = -1
        cursor = pos
        while cursor < len(structure):
            character = structure[cursor]
            if character == "(":
                depth_paren += 1
            elif character == ")":
                depth_paren -= 1
            elif character == "[":
                depth_bracket += 1
            elif character == "]":
                depth_bracket -= 1
            elif depth_paren == 0 and depth_bracket == 0:
                if character == ";":
                    semicolon = cursor
                    break
                if character == "{":
                    brace = cursor
                    break
            cursor += 1
        if semicolon >= 0:
            spans.append((attribute.start(), semicolon + 1))
            continue
        if brace < 0:
            spans.append((attribute.start(), len(structure)))
            continue
        depth = 0
        end = brace
        while end < len(structure):
            if structure[end] == "{":
                depth += 1
            elif structure[end] == "}":
                depth -= 1
                if depth == 0:
                    end += 1
                    break
            end += 1
        spans.append((attribute.start(), end))

    merged: list[tuple[int, int]] = []
    for start, end in sorted(spans):
        if merged and start <= merged[-1][1]:
            merged[-1] = (merged[-1][0], max(merged[-1][1], end))
        else:
            merged.append((start, end))
    pieces: list[str] = []
    pos = 0
    for start, end in merged:
        pieces.append(text[pos:start])
        pos = end
    pieces.append(text[pos:])
    return "".join(pieces)


def scan_imports(text: str, *, source: str, default_dll: str = "unknown") -> list[ImportEntry]:
    """Extract explicit service registrations, not every quoted Rust string.

    The report is heuristic, but an import entry must still come from a
    registration-shaped construct. Unit-test modules are ignored because their
    synthetic services are not runtime surface area.
    """
    production = strip_test_gated_items(text)
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


def _rust_production(text: str) -> str:
    """Production Rust source with comments removed enough for token scanning."""
    text = strip_test_gated_items(text)
    text = re.sub(r"/\*[\s\S]*?\*/", " ", text)
    return re.sub(r"//[^\n]*", " ", text)


def _operation_tokens(text: str) -> list[str]:
    return sorted({f"Op::{name}" for name in re.findall(r"\bOp::([A-Z][A-Za-z0-9]*)\b", text)})


def scan_interpreter_operations(texts: list[str]) -> list[str]:
    """Operations named by the shared semantic implementation modules.

    This is intentionally operation-oriented evidence, not proof that every
    encoding of an operation is correct or even accepted. It is still much more
    meaningful than helper-function names because these modules are the only
    semantic execution path.
    """
    tokens: set[str] = set()
    for text in texts:
        tokens.update(_operation_tokens(_rust_production(text)))
    return sorted(tokens)


def _rust_function_body(text: str, name: str) -> str:
    match = re.search(rf"\b(?:pub\s+)?fn\s+{re.escape(name)}\s*\(", text)
    if not match:
        return ""
    start = text.find("{", match.end())
    if start < 0:
        return ""
    depth = 0
    for index in range(start, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[start:index + 1]
    return ""


def scan_dbt_native_operations(text: str) -> list[str]:
    """Operations the DBT native-eligibility path can lower to ARM64.

    `is_native` is the gate. `neon_operation` and `neon_widening` are included
    because the gate delegates SIMD eligibility to those mappings. Conditions
    such as operand width or encoding are intentionally collapsed to operation
    coverage; the report remains a heuristic summary.
    """
    production = _rust_production(text)
    tokens: set[str] = set()
    for name in ("is_native", "neon_operation", "neon_widening"):
        tokens.update(_operation_tokens(_rust_function_body(production, name)))
    return sorted(tokens)


def build_report(root: Path) -> CoverageReport:
    report = CoverageReport(notes=[
        "Read-only scan of existing sources; not an architectural claim of completeness.",
        "Operation coverage is a source-text heuristic, not proof of per-encoding semantic completeness.",
    ])
    semantic_names = ["x86_exec.rs", "x86_simd.rs", "x86_float.rs", "x86_avx.rs", "x86_strings.rs"]
    semantic_texts = []
    for name in semantic_names:
        path = root / "crates" / "loader" / "src" / name
        if path.exists():
            semantic_texts.append(_read(path))
    report.interpreter_symbols = scan_interpreter_operations(semantic_texts)

    dbt = root / "crates" / "loader" / "src" / "dbt.rs"
    if dbt.exists():
        report.dbt_symbols = scan_dbt_native_operations(_read(dbt))
    registry = root / "crates" / "loader" / "x86" / "instructions.toml"
    if registry.exists():
        report.opcodes.extend(scan_opcodes(_read(registry), source=str(registry.relative_to(root))))

    # Import generation / service registries when present.
    for path in sorted((root / "crates").rglob("*.rs")):
        inventory_file = {
            "imports.rs", "services.rs", "win32.rs", "registry.rs", "runtime_table.rs",
        }
        if path.name in inventory_file or "import" in path.name:
            rel = str(path.relative_to(root))
            report.imports.extend(scan_imports(_read(path), source=rel, default_dll=path.stem))

    # Dedup opcodes preferring supported over unsupported over unknown
    rank = {"decodable": 2, "supported": 2, "unsupported": 1, "unknown": 0}
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
    positive = {"supported", "decodable"}
    newly_supported = sorted(
        op for op, st in after_ops.items()
        if st in positive and before_ops.get(op) not in positive
    )
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
