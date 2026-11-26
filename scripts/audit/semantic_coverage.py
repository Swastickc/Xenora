"""Generate the gfx1030 semantic coverage report from the checked-in inventories."""

import argparse
from collections import Counter, defaultdict
import difflib
import json
from pathlib import Path
import re
import runpy
import sys
try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - depends on the interpreter
    sys.exit("semantic_coverage.py needs Python 3.11 or newer (tomllib)")

ROOT = Path(__file__).resolve().parents[2]
SNAPSHOT = Path("crates/isa/data/rdna2.jsonl")
REGISTRY = Path("crates/isa/semantics/rdna2.toml")
DECODER = Path("crates/decoder/src/lib.rs")
GENERATED = Path("crates/isa/src/semantics_gen.rs")
STATUS = Path("docs/status.toml")
REPORT = Path("docs/shaders/semantic-coverage.md")

DECODER_HEADER = re.compile(
    r'const HANDWRITTEN_FORMS: \[\(&str, EncodingFamily\); (\d+)\] = \['
)
# The decoder's full list is its hand-written forms plus every registry form.
DECODER_TOTAL = re.compile(
    r'pub const SEMANTIC_FORMS: \[\(&str, EncodingFamily\); '
    r'HANDWRITTEN_FORMS\.len\(\) \+ FORM_IDS\.len\(\)\] ='
)
DECODER_ENTRY = re.compile(r'\("([A-Z0-9_]+)", EncodingFamily::([A-Za-z0-9]+)\),')
STATUS_ALIASES = {
    "s_waitcnt vmcnt(0)": ("SOPP", "S_WAITCNT"),
    "s_waitcnt_vscnt null, 0": ("SOPK", "S_WAITCNT_VSCNT"),
}
COLUMNS = (
    "Identified", "Encoding-validated", "Declared", "Decoder-supported",
    "IR", "MSL", "Reference-verified", "Metal-verified", "Unsupported operand variants",
)


def load_snapshot(root):
    # Reuse the metadata generator's base-form rules, including literal-only
    # forms and the relative moves whose base condition is not "default".
    metadata = runpy.run_path(str(root / "crates/isa/generate_rdna2_meta.py"))
    encodings = metadata["BASE_ENCODINGS"] | metadata["EXTENSIONS"]
    snapshot = (root / SNAPSHOT).read_text(encoding="utf-8")
    variants = []
    for number, line in enumerate(snapshot.splitlines(), 1):
        location = f"{SNAPSHOT}:{number}"
        try:
            row = json.loads(line)
        except ValueError as error:
            raise ValueError(f"{location}: {error}") from error
        if not isinstance(row, dict):
            raise ValueError(f"{location}: expected an object")
        kind = row.get("kind")
        if kind == "variant":
            if (
                not isinstance(row.get("encoding"), str) or row["encoding"] not in encodings
                or any(not isinstance(row.get(key), str) or not row[key]
                       for key in ("instruction", "condition"))
                or type(row.get("opcode")) is not int
            ):
                raise ValueError(f"{location}: invalid variant {row!r}")
            variants.append((location, row))
        elif kind not in ("source", "encoding", "operand_type", "data_format"):
            raise ValueError(f"{location}: unknown row kind {kind!r}")
    try:
        _, _, forms = metadata["forms_from_snapshot"](snapshot)
    except (SystemExit, ValueError, KeyError, IndexError, TypeError) as error:
        raise ValueError(f"{SNAPSHOT}: {error}") from error
    identified = {(form["encoding"].upper(), form["name"]): form["opcode"] for form in forms}
    if not identified or len(identified) != len(forms):
        raise ValueError(f"{SNAPSHOT}: expected unique, nonempty base forms")
    for location, row in variants:
        encoding = row["encoding"]
        base, _ = metadata["EXTENSIONS"].get(encoding, (encoding, None))
        key = (metadata["BASE_ENCODINGS"][base].upper(), row["instruction"])
        if key not in identified:
            raise ValueError(f"{location}: cannot map {encoding} variant to base form {key}")
    return identified


def parse_decoder(source):
    lines = source.splitlines()
    if sum(bool(DECODER_TOTAL.fullmatch(line.strip())) for line in lines) != 1:
        raise ValueError(f"{DECODER}: expected SEMANTIC_FORMS = hand-written + registry forms")
    starts = [i for i, line in enumerate(lines)
              if re.match(r"\s*const HANDWRITTEN_FORMS\b", line)]
    if len(starts) != 1:
        raise ValueError(f"{DECODER}: expected one HANDWRITTEN_FORMS declaration")
    start = starts[0]
    header = DECODER_HEADER.fullmatch(lines[start].strip())
    if not header:
        raise ValueError(f"{DECODER}:{start + 1}: unrecognized SEMANTIC_FORMS declaration")

    forms = set()
    for number, line in enumerate(lines[start + 1:], start + 2):
        line = line.partition("//")[0].strip()
        if line == "];":
            if len(forms) != int(header[1]):
                raise ValueError(f"{DECODER}:{number}: SEMANTIC_FORMS length mismatch")
            return forms
        if not line:
            continue
        entry = DECODER_ENTRY.fullmatch(line)
        if not entry:
            raise ValueError(f"{DECODER}:{number}: unrecognized SEMANTIC_FORMS entry {line!r}")
        key = (entry[2].upper(), entry[1])
        if key in forms:
            raise ValueError(f"{DECODER}:{number}: duplicate form {key}")
        forms.add(key)
    raise ValueError(f"{DECODER}: unterminated SEMANTIC_FORMS")


GENERATED_HEADER = re.compile(r"pub const FORM_IDS: \[\(&str, EncodingFamily\); (\d+)\] = \[")


def parse_generated(source):
    """Registry forms compiled into the decoder, read from the generated FORM_IDS table."""
    lines = source.splitlines()
    starts = [i for i, line in enumerate(lines) if GENERATED_HEADER.fullmatch(line.strip())]
    if len(starts) != 1:
        raise ValueError(f"{GENERATED}: expected one FORM_IDS declaration")
    forms = set()
    rows = 0
    for number, line in enumerate(lines[starts[0] + 1:], starts[0] + 2):
        line = line.strip()
        if line == "];":
            if rows != len(forms):
                raise ValueError(f"{GENERATED}:{number}: duplicate FORM_IDS entry")
            if len(forms) != int(GENERATED_HEADER.fullmatch(lines[starts[0]].strip())[1]):
                raise ValueError(f"{GENERATED}:{number}: FORM_IDS length mismatch")
            return forms
        entry = DECODER_ENTRY.fullmatch(line)
        if not entry:
            raise ValueError(f"{GENERATED}:{number}: unrecognized FORM_IDS entry {line!r}")
        forms.add((entry[2].upper(), entry[1]))
        rows += 1
    raise ValueError(f"{GENERATED}: unterminated FORM_IDS")


def load_registry(path):
    """The root registry with its family files merged, by the generator's loader.

    One loader for every consumer, so the family-file guards (one family per
    file, include paths, duplicates) apply here too.
    """
    import importlib.util

    script = ROOT / "crates/isa/generate_semantics.py"
    spec = importlib.util.spec_from_file_location("generate_semantics", script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    try:
        return module.load_registry(path)
    except module.RegistryError as error:
        raise ValueError(f"{REGISTRY}: {error}") from error


def read_declarations(registry, identified):
    if type(registry.get("schema")) is not int or registry["schema"] != 1:
        raise ValueError(f"{REGISTRY}: expected schema 1")
    families = {family for family, _ in identified}
    rules = {}
    for name, rule in registry["operand_rules"].items():
        location = f"{REGISTRY}: operand_rules.{name}"
        family = rule["family"]
        unsupported = rule["unsupported"]
        if family not in families:
            raise ValueError(f"{location}: unknown family {family!r}")
        if not isinstance(unsupported, list) or any(
            not isinstance(variant, str) or not re.fullmatch(r"[a-zA-Z0-9_]+", variant)
            for variant in unsupported
        ):
            raise ValueError(f"{location}: invalid unsupported list")
        rules[name] = (family, set(unsupported))

    declared = set()
    # An encoding variant declares its base form in another encoding family.
    raw_variants = registry.get("encoding_variant", [])
    if not isinstance(raw_variants, list) or not all(
        isinstance(variant, dict)
        and {"family", "of", "opcode", "operands"} <= set(variant)
        and all(isinstance(variant[key], str) for key in ("family", "of", "operands"))
        and type(variant["opcode"]) is int
        for variant in raw_variants
    ):
        raise ValueError(
            f"{REGISTRY}: every encoding_variant needs string family, of and operands"
            " and an integer opcode"
        )
    variants = [
        {"family": variant["family"], "name": variant["of"], "opcode": variant["opcode"],
         "operands": variant["operands"]}
        for variant in raw_variants
    ]
    for form in registry["form"] + variants:
        key = (form["family"], form["name"])
        if key not in identified:
            raise ValueError(f"{REGISTRY}: unknown form {key}")
        if type(form["opcode"]) is not int or form["opcode"] != identified[key]:
            raise ValueError(f"{REGISTRY}: opcode mismatch for {key}")
        if key in declared:
            raise ValueError(f"{REGISTRY}: duplicate form {key}")
        rule = form["operands"]
        if rule not in rules or rules[rule][0] != key[0]:
            raise ValueError(f"{REGISTRY}: invalid operand rule {rule!r} for {key}")
        declared.add(key)
    return declared, rules


def status_form(row, identified):
    name = row["form"]
    if not isinstance(name, str) or not isinstance(row["encoding"], str):
        raise ValueError(f"{STATUS}: invalid form or encoding in {row!r}")
    families = {family for family, _ in identified}
    family_hint = row["encoding"].split(",", 1)[0].strip().upper()
    if name in STATUS_ALIASES:
        candidates = [STATUS_ALIASES[name]]
    else:
        mnemonic = name.upper()
        eligible = families
        if mnemonic.endswith("_E32"):
            mnemonic = mnemonic.removesuffix("_E32")
            eligible = families & {"VOP1", "VOP2", "VOPC"}
        elif mnemonic.endswith("_E64"):
            mnemonic = mnemonic.removesuffix("_E64")
            eligible = families & {"VOP3", "VOP3SDST"}
        candidates = sorted((family, mnemonic) for family in eligible
                            if (family, mnemonic) in identified)
    if family_hint in families:
        candidates = [key for key in candidates if key[0] == family_hint]
    if len(candidates) != 1 or candidates[0] not in identified:
        raise ValueError(f"{STATUS}: cannot uniquely map {name!r} ({candidates})")
    return candidates[0]


def collect_coverage(identified, registry, decoder, status):
    declared, rules = read_declarations(registry, identified)
    unknown = decoder.difference(identified)
    if unknown:
        raise ValueError(f"{DECODER}: unknown forms {sorted(unknown)}")

    encoded, lowered, reference, metal = set(), set(), set(), set()
    seen = set()
    for row in status["instruction"]:
        key = status_form(row, identified)
        if key in seen:
            raise ValueError(f"{STATUS}: duplicate evidence for {key}")
        seen.add(key)
        decodes = row.get("decoded") is True
        if decodes and key not in decoder:
            raise ValueError(
                f"{STATUS}: {key} claims decoding but is absent from the decoder inventory"
            )
        for field in ("llvm", "lowering", "reference", "cpu", "gpu"):
            if field in row and not isinstance(row[field], str):
                raise ValueError(f"{STATUS}: {key} has invalid {field} evidence")
        if row.get("llvm") == "three-way passed":
            encoded.add(key)
        lowering = row.get("lowering", "").strip().lower()
        # This status schema uses one field for the IR-to-MSL path. A
        # compile-time resolution also implements that path for the form.
        if (
            lowering.startswith(("msl ", "resolved ", "verified ", "constant or ", "per-thread "))
            and not re.search(r"\b(?:stub|placeholder|planned|unsupported|pending|todo|unimplemented|not implemented|not yet)\b", lowering)
        ):
            lowered.add(key)
        elif lowering:
            raise ValueError(f"{STATUS}: {key} has unrecognized lowering {row['lowering']!r}")
        # Verified evidence also requires the status row to say the form decodes.
        if decodes and row.get("reference") == "implemented" and row.get("cpu") == "fixture verified":
            reference.add(key)
        if decodes and row.get("gpu") in {"fixture verified", "fixture verified on Apple M5"}:
            metal.add(key)

    counts = [Counter(family for family, _ in forms) for forms in (
        identified, encoded, declared, decoder, lowered, lowered, reference, metal,
    )]
    unsupported = defaultdict(set)
    for family, variants in rules.values():
        unsupported[family].update(variants)
    return {
        family: (*[count[family] for count in counts], len(unsupported[family]))
        for family in sorted(counts[0])
    }, rules


def render_report(counts, rules):
    lines = [
        "# gfx1030 semantic coverage",
        "",
        "<!-- Generated by scripts/audit/semantic_coverage.py. -->",
        "",
        "Identified forms come from the AMD RDNA 2 instruction snapshot. Declared forms",
        "come from the semantic registry. Decoder support is the decoder's hand-written",
        "list plus the registry forms compiled into crates/isa/src/semantics_gen.rs.",
        "",
        "The status inventory records LLVM three-way encoding checks, intermediate",
        "representation (IR) and Metal Shading Language (MSL) lowering, and fixture",
        "verification. IR and MSL share the inventory's lowering field. Reference",
        "verification requires an implemented reference and verified CPU fixture.",
        "Metal verification requires a verified GPU fixture. Both also require a decoding row. Local readbacks pending",
        "acceptance remain outside the verified count. All results concern synthetic fixtures.",
        "",
        "| Family | " + " | ".join(COLUMNS) + " |",
        "| --- |" + " ---: |" * len(COLUMNS),
    ]
    for family, values in counts.items():
        lines.append(f"| {family} | " + " | ".join(map(str, values)) + " |")
    lines.append("| **Total** | " + " | ".join(map(str, map(sum, zip(*counts.values())))) + " |")
    lines.extend([
        "",
        "## Unsupported operand variants",
        "",
        "The table counts distinct variant names per family across registry operand rules.",
        "",
        "| Family | Operand rule | Unsupported variants |",
        "| --- | --- | --- |",
    ])
    for family, name, variants in sorted((family, name, variants)
                                          for name, (family, variants) in rules.items()):
        names = ", ".join(f"`{variant}`" for variant in sorted(variants))
        lines.append(f"| {family} | `{name}` | {names or 'None listed'} |")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare without writing")
    args = parser.parse_args()
    try:
        identified = load_snapshot(ROOT)
        registry = load_registry(ROOT / REGISTRY)
        decoder = parse_decoder((ROOT / DECODER).read_text(encoding="utf-8")) | parse_generated(
            (ROOT / GENERATED).read_text(encoding="utf-8"))
        status = tomllib.loads((ROOT / STATUS).read_text(encoding="utf-8"))
        report = render_report(*collect_coverage(identified, registry, decoder, status)).encode()
        path = ROOT / REPORT
        if not args.check:
            path.write_bytes(report)
            print(f"Wrote {REPORT}")
            return 0
        committed = path.read_bytes() if path.exists() else b""
        if committed == report:
            print(f"{REPORT} is up to date")
            return 0
        before = committed.decode("utf-8", errors="replace").splitlines(keepends=True)
        after = report.decode().splitlines(keepends=True)
        diff = list(difflib.unified_diff(before, after, fromfile=str(REPORT),
                                         tofile="regenerated", n=0))
        print(f"{REPORT} differs from the regenerated report", file=sys.stderr)
        for line in diff[:12]:
            print(line.rstrip("\n"), file=sys.stderr)
        if len(diff) > 12:
            print("…", file=sys.stderr)
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        print(f"semantic_coverage: {error}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
