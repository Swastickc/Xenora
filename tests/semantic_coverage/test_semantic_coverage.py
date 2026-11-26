import tempfile
import unittest
from pathlib import Path

from audit import semantic_coverage as coverage


class SemanticCoverageTests(unittest.TestCase):
    def setUp(self):
        self.identified = {("SOP2", "S_ADD_U32"): 0, ("SOP2", "S_SUB_U32"): 1}
        self.registry = {
            "schema": 1,
            "form": [{
                "family": "SOP2", "name": "S_ADD_U32", "opcode": 0,
                "operands": "scalar", "verify": ["metal", "reference", "llvm_encoding"],
            }],
            "operand_rules": {
                "scalar": {"family": "SOP2", "unsupported": ["two_literals"]},
            },
        }

    def counts(self, *rows, decoder=None):
        if decoder is None:
            decoder = {("SOP2", "S_SUB_U32")}
        counts, _ = coverage.collect_coverage(
            self.identified, self.registry, decoder, {"instruction": list(rows)},
        )
        return dict(zip(coverage.COLUMNS, counts["SOP2"]))

    def test_declared_without_metal_evidence_is_not_metal_verified(self):
        counts = self.counts()
        self.assertEqual(counts["Declared"], 1)
        self.assertEqual(counts["Metal-verified"], 0)
        self.assertEqual(counts["Reference-verified"], 0)
        self.assertEqual(counts["Encoding-validated"], 0)

    def test_status_evidence_counts_for_handwritten_form(self):
        counts = self.counts({
            "form": "s_sub_u32", "encoding": "SOP2, scalar operands",
            "llvm": "three-way passed", "lowering": "resolved scalar state and SCC",
            "reference": "implemented", "cpu": "fixture verified",
            "gpu": "fixture verified on Apple M5", "decoded": True,
        })
        self.assertEqual(counts, {
            "Identified": 2, "Encoding-validated": 1, "Declared": 1,
            "Decoder-supported": 1, "IR": 1, "MSL": 1, "Reference-verified": 1,
            "Metal-verified": 1, "Unsupported operand variants": 1,
        })

    def test_pending_readback_does_not_count(self):
        counts = self.counts({
            "form": "s_add_u32", "encoding": "SOP2", "lowering": "resolved scalar state and SCC",
            "reference": "implemented", "cpu": "local known-answer test passed",
            "gpu": "local Apple M5 readback passed; acceptance pending",
        })
        self.assertEqual(counts["Metal-verified"], 0)
        self.assertEqual(counts["Reference-verified"], 0)
        self.assertEqual(counts["IR"], 1)

    def test_verified_evidence_needs_a_decoding_row(self):
        counts = self.counts({
            "form": "s_sub_u32", "encoding": "SOP2", "reference": "implemented",
            "cpu": "fixture verified", "gpu": "fixture verified", "decoded": False,
        })
        self.assertEqual(counts["Reference-verified"], 0)
        self.assertEqual(counts["Metal-verified"], 0)

    def test_declared_but_not_compiled_is_not_decoder_supported(self):
        # S_ADD_U32 is declared in the registry but absent from the compiled tables.
        self.assertEqual(self.counts()["Decoder-supported"], 1)

    def test_stale_decoded_claim_requires_a_compiled_form(self):
        row = {
            "form": "s_add_u32", "encoding": "SOP2", "decoded": True,
            "reference": "implemented", "cpu": "fixture verified", "gpu": "fixture verified",
        }
        with self.assertRaisesRegex(ValueError, "S_ADD_U32.*decoder inventory"):
            self.counts(row)

        generated = coverage.parse_generated(
            "pub const FORM_IDS: [(&str, EncodingFamily); 1] = [\n"
            '    ("S_ADD_U32", EncodingFamily::Sop2),\n];'
        )
        counts = self.counts(row, decoder=generated | {("SOP2", "S_SUB_U32")})
        self.assertEqual(counts["Decoder-supported"], 2)
        self.assertEqual(counts["Reference-verified"], 1)
        self.assertEqual(counts["Metal-verified"], 1)

    def test_refused_form_can_remain_outside_the_decoder_inventory(self):
        counts = self.counts({
            "form": "s_add_u32", "encoding": "SOP2", "decoded": False,
            "reference": "unsupported", "cpu": "refused", "gpu": "acceptance pending",
        })
        self.assertEqual(counts["Decoder-supported"], 1)
        self.assertEqual(counts["Reference-verified"], 0)
        self.assertEqual(counts["Metal-verified"], 0)

    def test_a_variant_without_its_form_is_an_error_not_a_traceback(self):
        self.registry["encoding_variant"] = [{"family": "VOP3", "opcode": 1, "operands": "x"}]
        with self.assertRaisesRegex(ValueError, "every encoding_variant needs"):
            coverage.read_declarations(self.registry, self.identified)

    def test_generated_table_parser(self):
        source = (
            "pub const FORM_IDS: [(&str, EncodingFamily); 1] = [\n"
            '    ("S_ADD_U32", EncodingFamily::Sop2),\n];'
        )
        self.assertEqual(coverage.parse_generated(source), {("SOP2", "S_ADD_U32")})
        with self.assertRaisesRegex(ValueError, "length mismatch"):
            coverage.parse_generated(source.replace("; 1]", "; 2]"))
        duplicated = source.replace("; 1]", "; 2]").replace(
            '    ("S_ADD_U32", EncodingFamily::Sop2),\n',
            '    ("S_ADD_U32", EncodingFamily::Sop2),\n' * 2,
        )
        with self.assertRaisesRegex(ValueError, "duplicate FORM_IDS entry"):
            coverage.parse_generated(duplicated)

    def test_stub_or_unrecognized_lowering_fails(self):
        for lowering in ("stub", "MSL placeholder add", "MSL unimplemented", "something new"):
            with self.assertRaisesRegex(ValueError, "unrecognized lowering"):
                self.counts({"form": "s_add_u32", "encoding": "SOP2", "lowering": lowering})

    def test_status_suffix_disambiguates_encoding(self):
        forms = {("VOP2", "V_AND_B32"): 1, ("VOP3", "V_AND_B32"): 2}
        self.assertEqual(coverage.status_form(
            {"form": "v_and_b32_e32", "encoding": "Two VGPR sources"}, forms,
        ), ("VOP2", "V_AND_B32"))
        with self.assertRaisesRegex(ValueError, "cannot uniquely map"):
            coverage.status_form(
                {"form": "v_and_b32", "encoding": "Two VGPR sources"}, forms,
            )

    def test_unknown_status_form_fails(self):
        with self.assertRaisesRegex(ValueError, "cannot uniquely map.*s_typo"):
            self.counts({"form": "s_typo", "encoding": "SOP2"})

    def test_decoder_parser_rejects_unrecognized_entry(self):
        header = (
            "pub const SEMANTIC_FORMS: [(&str, EncodingFamily); "
            "HANDWRITTEN_FORMS.len() + FORM_IDS.len()] =\n"
            "const HANDWRITTEN_FORMS: [(&str, EncodingFamily); 1] = ["
        )
        self.assertEqual(coverage.parse_decoder(
            f'{header}\n    ("S_ADD_U32", EncodingFamily::Sop2),\n];',
        ), {("SOP2", "S_ADD_U32")})
        with self.assertRaisesRegex(ValueError, "unrecognized SEMANTIC_FORMS entry"):
            coverage.parse_decoder(f"{header}\n    unknown!(),\n];")


class RegistryLoading(unittest.TestCase):
    def test_family_file_guards_apply_to_the_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "rdna2.toml"
            (Path(tmp) / "rdna2").mkdir()
            root.write_text('schema = 1\ninclude = ["rdna2/sop2.toml"]\n', encoding="utf-8")
            (Path(tmp) / "rdna2" / "sop2.toml").write_text(
                '[[form]]\nname = "V_X"\nfamily = "VOP2"\n', encoding="utf-8"
            )
            with self.assertRaisesRegex(ValueError, "form V_X is not a SOP2 form"):
                coverage.load_registry(root)

    def test_loads_the_real_split_registry(self):
        registry = coverage.load_registry(coverage.ROOT / coverage.REGISTRY)
        self.assertNotIn("include", registry)
        self.assertGreater(len(registry["form"]), 0)


if __name__ == "__main__":
    unittest.main()
