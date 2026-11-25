// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest-to-host service dispatch cost, measured with original guest loops
//! that call standard services through a bound import table under the DBT.
//!
//! Every workload checks its result before timing. Each reported figure is
//! the per-iteration time of a loop of `ITERATIONS` calls: one warm-up batch,
//! then `BATCHES` measured batches. The empty loop is reported separately so
//! readers can subtract the loop and translation overhead themselves.
//!
//! Run with `cargo bench --locked -p xenora-loader --bench service_dispatch`.

#[path = "../tests/support/mod.rs"]
mod support;

use std::time::Instant;
use support::*;
use xenora_loader::cpu::{ExitReason, Register};
use xenora_loader::services::sysinfo::HostCapabilities;

const IMPORTS: &[(&str, &str)] = &[
    ("kernel32.dll", "GetLastError"),
    ("kernel32.dll", "CreateEventW"),
    ("kernel32.dll", "WaitForSingleObject"),
    ("ucrtbase.dll", "strlen"),
    ("kernel32.dll", "GetProcessHeap"),
    ("kernel32.dll", "HeapAlloc"),
    ("kernel32.dll", "HeapFree"),
    ("ucrtbase.dll", "qsort"),
];

const ITERATIONS: u32 = 10_000;
const BATCHES: usize = 10;
const CELL_EVENT: u64 = SCRATCH + 0x10;
const CELL_HEAP: u64 = SCRATCH + 0x18;
const STRING: u64 = SCRATCH + 0x100;
const ARRAY: u64 = SCRATCH + 0x200;
const ARRAY_LEN: u32 = 16;

const EMPTY: usize = 0;
const LAST_ERROR: usize = 1;
const WAIT: usize = 2;
const STRLEN: usize = 3;
const HEAP: usize = 4;
const QSORT: usize = 5;
const COMPARE: usize = 6;

/// A tiny x86-64 assembler for the routines below.
struct Asm {
    base: u64,
    code: Vec<u8>,
}

impl Asm {
    fn new(routine_index: usize) -> Self {
        Self {
            base: routine(routine_index),
            code: Vec::new(),
        }
    }

    fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.code.extend_from_slice(bytes);
        self
    }

    fn imm64(&mut self, opcode: &[u8], value: u64) -> &mut Self {
        self.bytes(opcode).bytes(&value.to_le_bytes())
    }

    fn imm32(&mut self, opcode: &[u8], value: u32) -> &mut Self {
        self.bytes(opcode).bytes(&value.to_le_bytes())
    }

    /// `call qword ptr [rip + displacement]` through an import slot.
    fn call_import(&mut self, symbol: &str) -> &mut Self {
        let index = IMPORTS
            .iter()
            .position(|(_, name)| *name == symbol)
            .unwrap();
        let next = self.base + self.code.len() as u64 + 6;
        let displacement = import_slot(IMPORTS, index).wrapping_sub(next) as u32;
        self.imm32(&[0xFF, 0x15], displacement)
    }

    /// `mov ebx, ITERATIONS`, the loop body, `dec ebx; jnz body; hlt`.
    fn counted(routine_index: usize, body: impl FnOnce(&mut Self)) -> Vec<u8> {
        let mut asm = Self::new(routine_index);
        asm.imm32(&[0xBB], ITERATIONS);
        let start = asm.code.len();
        body(&mut asm);
        asm.bytes(&[0xFF, 0xCB]); // dec ebx
        let back = start as i64 - (asm.code.len() as i64 + 2);
        asm.bytes(&[0x75, i8::try_from(back).unwrap() as u8, 0xF4]);
        asm.code
    }
}

fn routines() -> Vec<Vec<u8>> {
    let mut routines = vec![Vec::new(); 7];
    routines[EMPTY] = Asm::counted(EMPTY, |_| {});
    routines[LAST_ERROR] = Asm::counted(LAST_ERROR, |asm| {
        asm.call_import("GetLastError");
    });
    routines[WAIT] = Asm::counted(WAIT, |asm| {
        asm.imm64(&[0x48, 0xB8], CELL_EVENT) // mov rax, cell
            .bytes(&[0x48, 0x8B, 0x08]) // mov rcx, [rax]
            .bytes(&[0x31, 0xD2]) // xor edx, edx
            .call_import("WaitForSingleObject");
    });
    routines[STRLEN] = Asm::counted(STRLEN, |asm| {
        asm.imm64(&[0x48, 0xB9], STRING).call_import("strlen");
    });
    routines[HEAP] = Asm::counted(HEAP, |asm| {
        asm.imm64(&[0x48, 0xB8], CELL_HEAP)
            .bytes(&[0x48, 0x8B, 0x08]) // mov rcx, [rax]
            .bytes(&[0x31, 0xD2])
            .imm32(&[0x41, 0xB8], 64) // mov r8d, 64
            .call_import("HeapAlloc")
            .bytes(&[0x49, 0x89, 0xC0]) // mov r8, rax
            .imm64(&[0x48, 0xB8], CELL_HEAP)
            .bytes(&[0x48, 0x8B, 0x08])
            .bytes(&[0x31, 0xD2])
            .call_import("HeapFree");
    });
    routines[QSORT] = Asm::counted(QSORT, |asm| {
        asm.imm64(&[0x48, 0xB9], ARRAY)
            .imm32(&[0xBA], ARRAY_LEN) // mov edx, count
            .imm32(&[0x41, 0xB8], 4) // mov r8d, width
            .imm64(&[0x49, 0xB9], routine(COMPARE))
            .call_import("qsort");
    });
    // int compare(const int *a, const int *b) { return *a - *b; }
    routines[COMPARE] = vec![0x8B, 0x01, 0x2B, 0x02, 0xC3];
    routines
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn main() {
    let mut guest = Guest::new(IMPORTS, &routines());
    let event = guest.call("CreateEventW", &[0, 1, 1, 0]);
    assert_ne!(event, 0);
    guest.memory.write_u64(CELL_EVENT, event).unwrap();
    let heap = guest.call("GetProcessHeap", &[]);
    guest.memory.write_u64(CELL_HEAP, heap).unwrap();
    guest.memory.write_bytes(STRING, &[b'a'; 63]).unwrap();
    guest.memory.write_u8(STRING + 63, 0).unwrap();
    let unsorted: Vec<i32> = (0..ARRAY_LEN as i32)
        .map(|i| (i * 7919) % 31 - 15)
        .collect();

    let reset_array = |guest: &mut Guest| {
        for (index, value) in unsorted.iter().enumerate() {
            guest
                .memory
                .write_u32(ARRAY + 4 * index as u64, *value as u32)
                .unwrap();
        }
    };

    let host = HostCapabilities::detect();
    let brand = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default();
    println!(
        "host: {brand}, {} logical processors; profile: {}",
        host.logical_processors.unwrap_or(0),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    );
    println!("{ITERATIONS} iterations per batch, 1 warm-up and {BATCHES} measured batches");
    println!("workload,median_ns_per_iteration,min_ns,max_ns");

    for (name, index, check) in [
        ("empty loop", EMPTY, None),
        ("GetLastError", LAST_ERROR, Some(0)),
        (
            "WaitForSingleObject (signaled manual-reset event)",
            WAIT,
            Some(0),
        ),
        ("strlen (63 bytes)", STRLEN, Some(63)),
        ("HeapAlloc + HeapFree (64 bytes)", HEAP, Some(1)),
        ("qsort (16 ints, guest comparator)", QSORT, Some(0)),
    ] {
        let mut samples = Vec::with_capacity(BATCHES);
        for batch in 0..=BATCHES {
            if index == QSORT {
                reset_array(&mut guest);
            }
            let start = Instant::now();
            let (cpu, exit) = guest.execute(routine(index), usize::MAX);
            let elapsed = start.elapsed();
            assert_eq!(exit, ExitReason::Halt, "{name}");
            assert_eq!(cpu.get(Register::Rbx), 0, "{name} loop count");
            if let Some(expected) = check {
                assert_eq!(cpu.get(Register::Rax), expected, "{name} result");
            }
            if index == QSORT {
                let mut expected = unsorted.clone();
                expected.sort();
                let sorted: Vec<i32> = (0..ARRAY_LEN as u64)
                    .map(|i| guest.u32_at(ARRAY + 4 * i) as i32)
                    .collect();
                assert_eq!(sorted, expected, "{name} ordering");
            }
            if batch > 0 {
                samples.push(elapsed.as_nanos() as f64 / f64::from(ITERATIONS));
            }
        }
        let (min, max) = samples.iter().fold((f64::MAX, 0f64), |(low, high), &s| {
            (low.min(s), high.max(s))
        });
        println!("{name},{:.1},{min:.1},{max:.1}", median(&mut samples));
    }
}
