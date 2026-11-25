"""Validation and freshness tests for crates/loader/generate_x86.py."""

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("generate_x86", ROOT / "crates/loader/generate_x86.py")
generate_x86 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generate_x86)

REGISTRY = (ROOT / "crates/loader/x86/instructions.toml").read_text()


class GenerateX86(unittest.TestCase):
    def rejects(self, text, message):
        with self.assertRaisesRegex(generate_x86.RegistryError, message):
            generate_x86.load(text)

    def form(self, body):
        return REGISTRY + "\n[[form]]\n" + body

    def test_committed_table_is_fresh_and_deterministic(self):
        first = generate_x86.generate(*generate_x86.load(REGISTRY))
        self.assertEqual(first, generate_x86.generate(*generate_x86.load(REGISTRY)))
        self.assertEqual(first, (ROOT / "crates/loader/src/x86_table.rs").read_text())

    def test_sign_extension_cannot_claim_mov_immediate_opcodes(self):
        # The defect that started this work: BE/BF in the one-byte map.
        for opcode in (0xBE, 0xBF):
            self.rejects(
                self.form(f'op = "movsx"\nopcode = {opcode}\nenc = "reg_rm"\nsize = "v"\nsrc = "b"'),
                "overlapping encoding")

    def test_every_mov_immediate_opcode_is_a_mov(self):
        _, forms = generate_x86.load(REGISTRY)
        for opcode in range(0xB0, 0xC0):
            owners = [f["op"] for f in forms if f["map"] == "1"
                      and f["opcode"] <= opcode < f["opcode"] + f["count"]]
            self.assertEqual(owners, ["mov"])

    def test_overlaps_are_rejected(self):
        self.rejects(self.form('op = "nop"\nopcode = 0x90\nenc = "none"\nsize = "n"'), "overlapping")
        self.rejects(self.form('op = "nop"\nopcode = 0x57\nenc = "none"\nsize = "n"'), "overlapping")
        self.rejects(self.form('op = "not"\nopcode = 0xF7\ndigit = 2\nenc = "rm"\nsize = "v"'), "overlapping")
        self.rejects(self.form('op = "not"\nopcode = 0xF7\nenc = "rm"\nsize = "v"'), "overlapping")

    def test_prefixes_and_escapes_are_not_instructions(self):
        for opcode in (0x0F, 0x48, 0x66, 0xF0):
            self.rejects(self.form(f'op = "nop"\nopcode = {opcode}\nenc = "none"\nsize = "n"'), "prefix or escape")

    def test_contradictory_operands_are_rejected(self):
        self.rejects(self.form('op = "lea"\nopcode = 0xD7\nenc = "reg_rm"\nsize = "v"\nmemory_only = true\nregister_only = true'), "contradict")
        self.rejects(self.form('op = "nop"\nopcode = 0xD7\nenc = "none"\nsize = "n"\nmemory_only = true'), "without ModRM")
        self.rejects(self.form('op = "nop"\nopcode = 0xD7\nenc = "none"\nsize = "n"\nimm = "ib"'), "not valid for none")
        self.rejects(self.form('op = "nop"\nopcode = 0xD7\ndigit = 1\nenc = "none"\nsize = "n"'), "digit needs")
        self.rejects(self.form('op = "nop"\nopcode = 0xD4\ncount = 8\nenc = "none"\nsize = "n"'), "aligned")

    def test_mandatory_prefix_cannot_shadow_an_operand_size(self):
        # 66 0F AF is a 16-bit IMUL, and 66 0F 6E is a MOVD; neither may be both.
        self.rejects(self.form('op = "paddb"\nmap = "0f"\nprefix = "66"\nopcode = 0xAF\nenc = "xmm_rm"\nsize = "x"\nsrc = "x"'), "both an operand size")
        self.rejects(self.form('op = "imul2"\nmap = "0f"\nopcode = 0x6E\nenc = "reg_rm"\nsize = "v"'), "both an operand size")

    def test_vector_operand_rules(self):
        self.rejects(self.form('op = "paddb"\nmap = "0f"\nprefix = "66"\nopcode = 0xF0\nenc = "xmm_rm"\nsize = "x"'), "source size")
        self.rejects(self.form('op = "paddb"\nmap = "0f"\nprefix = "66"\nopcode = 0xF0\nenc = "xmm_rm"\nsize = "v"\nsrc = "x"'), "XMM size")
        self.rejects(self.form('op = "movq"\nmap = "0f"\nprefix = "66"\nopcode = 0xF0\nenc = "xmm_rm"\nsize = "x"\nsrc = "q"\nunaligned = true'), "alignment applies")
        self.rejects(self.form('op = "nop"\nprefix = "66"\nopcode = 0xD7\nenc = "none"\nsize = "n"'), "map 0f")

    def test_lock_needs_a_memory_destination(self):
        # Not a form without ModRM, not one whose ModRM operand is the source,
        # and not one that takes only a register.
        message = "LOCK needs a memory destination"
        self.rejects(self.form('op = "nop"\nopcode = 0xD7\nenc = "none"\nsize = "n"\nlock = true'), message)
        self.rejects(self.form('op = "mov"\nopcode = 0xD7\nenc = "reg_rm"\nsize = "v"\nlock = true'), message)
        self.rejects(self.form('op = "xadd"\nopcode = 0xD7\nenc = "rm_reg"\nsize = "v"\nregister_only = true\nlock = true'), message)

    def test_a_fixed_rm_field_needs_a_register_only_digit_form(self):
        base = 'op = "nop"\nopcode = 0xD9\nenc = "rm_unread"\nsize = "n"\n'
        message = "needs register_only and a digit"
        self.rejects(self.form(base + "digit = 0\nrm = 3"), message)
        self.rejects(self.form(base + "digit = 0\nmemory_only = true\nrm = 3"), message)
        self.rejects(self.form(base + "register_only = true\nrm = 3"), message)
        for value in ("8", "-1", "true", '"3"'):
            self.rejects(self.form(base + f"digit = 0\nregister_only = true\nrm = {value}"), "r/m field value 0-7")
        # A second form on the same opcode and digit is an overlap, whatever its r/m field.
        self.rejects(self.form('op = "nop"\nopcode = 0xDB\ndigit = 4\nenc = "rm_unread"\nsize = "n"\n'
                               "register_only = true\nrm = 2"), "overlapping")
        # The valid shape loads, and the key reaches the generated table.
        operations, forms = generate_x86.load(self.form(base + "digit = 0\nregister_only = true\nrm = 7"))
        self.assertEqual([f["rm"] for f in forms if f["map"] == "1" and f["opcode"] == 0xD9], [7])
        self.assertIn("register_only: true, fixed_rm: Some(7),", generate_x86.generate(operations, forms))

    def test_fninit_is_the_only_form_with_a_fixed_rm_field_and_the_only_x87_form(self):
        _, forms = generate_x86.load(REGISTRY)
        fixed = [(f["op"], f["map"], f["opcode"], f["digit"], f["rm"]) for f in forms if f["rm"] is not None]
        self.assertEqual(fixed, [("fninit", "1", 0xDB, 4, 3)])
        x87 = [f["op"] for f in forms if f["map"] == "1" and 0xD8 <= f["opcode"] <= 0xDF]
        self.assertEqual(x87, ["fninit"])

    def test_only_xadd_and_cmpxchg_are_lockable(self):
        _, forms = generate_x86.load(REGISTRY)
        self.assertEqual({form["op"] for form in forms if form.get("lock")}, {"cmpxchg", "xadd"})

    def test_string_forms_are_movs_and_stos_without_operands(self):
        _, forms = generate_x86.load(REGISTRY)
        self.assertEqual({form["op"] for form in forms if form["enc"] == "string"}, {"movs", "stos"})
        self.rejects(self.form('op = "movs"\nopcode = 0xD7\nenc = "string"\nsize = "b"\nimm = "ib"'), "not valid for string")
        self.rejects(self.form('op = "movs"\nopcode = 0xD7\nenc = "string"\nsize = "b"\nlock = true'), "LOCK needs a memory destination")

    def test_unknown_vocabulary_is_rejected(self):
        self.rejects(self.form('op = "frob"\nopcode = 0xD7\nenc = "none"\nsize = "n"'), "unknown op")
        self.rejects(self.form('op = "nop"\nopcode = 0xD7\nenc = "none"\nsize = "z"'), "unknown size")
        self.rejects(self.form('op = "nop"\nopcode = 0xD7\nenc = "none"\nsize = "n"\nextra = 1'), "unknown keys")

    def test_contradictory_effects_are_rejected(self):
        self.rejects(REGISTRY.replace('bt = { writes = "cf",', 'bt = { writes = "cf sf",', 1), "both written and undefined")
        self.rejects(REGISTRY.replace('clc = { writes = "cf"', 'clc = { writes = "cx"', 1), "unknown flag")
        self.rejects(REGISTRY.replace('ref = "sdm CLC"', 'ref = "blog CLC"', 1), "listed document")


if __name__ == "__main__":
    unittest.main()
