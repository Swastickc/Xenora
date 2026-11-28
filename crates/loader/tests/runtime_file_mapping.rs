// SPDX-License-Identifier: GPL-3.0-or-later

//! Paging-file-backed memory mappings through the guest import table.

mod support;

use support::*;

const KERNEL32: &str = "kernel32.dll";
const MAPPING_API: &str = "api-ms-win-core-memory-l1-1-0.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "CreateFileMappingA"),
    (KERNEL32, "MapViewOfFile"),
    (KERNEL32, "UnmapViewOfFile"),
    (KERNEL32, "FlushViewOfFile"),
    (KERNEL32, "CreateProcessA"),
    (KERNEL32, "CloseHandle"),
    (KERNEL32, "GetLastError"),
];

const VIEW_IMPORTS: [&[(&str, &str)]; 3] = [
    &imports_for_view(KERNEL32, "MapViewOfFile"),
    &imports_for_view(KERNEL32, "MapViewOfFileEx"),
    &imports_for_view(MAPPING_API, "MapViewOfFile"),
];

const fn imports_for_view(
    dll: &'static str,
    view: &'static str,
) -> [(&'static str, &'static str); 5] {
    [
        (dll, view),
        (KERNEL32, "CreateFileMappingA"),
        (KERNEL32, "UnmapViewOfFile"),
        (KERNEL32, "CloseHandle"),
        (KERNEL32, "GetLastError"),
    ]
}

const INVALID_FILE: u64 = u64::MAX;

#[test]
fn shared_memory_mappings_are_zeroed_views_the_program_releases() {
    let mut guest = Guest::new(IMPORTS, &[]);

    // A paging-file-backed object opens and its view reads as zero.
    let object = guest.call("CreateFileMappingA", &[INVALID_FILE, 0, 4, 0, 4096, 0]);
    assert_ne!(object, 0);
    let view = guest.call("MapViewOfFile", &[object, 2, 0, 0, 0]);
    assert_ne!(view, 0);
    assert_eq!(guest.u64_at(view), 0);
    // The memory is usable and unmaps once.
    guest.memory.write_u64(view, 42).unwrap();
    assert_eq!(guest.u64_at(view), 42);
    assert_eq!(guest.call("UnmapViewOfFile", &[view]), 1);
    assert_eq!(guest.call("UnmapViewOfFile", &[view]), 0);
    // The object closes through the uniform handle path.
    assert_eq!(guest.call("CloseHandle", &[object]), 1);

    // A real file is honest: not supported.
    assert_eq!(
        guest.call("CreateFileMappingA", &[0x2000, 0, 4, 0, 4096, 0]),
        0
    );
    assert_eq!(guest.error(), 50);
    // A zero size is the documented file-invalid answer.
    assert_eq!(
        guest.call("CreateFileMappingA", &[INVALID_FILE, 0, 4, 0, 0, 0]),
        0
    );
    assert_eq!(guest.error(), 1006);
    // A bogus object cannot map.
    assert_eq!(guest.call("MapViewOfFile", &[0xdead, 2, 0, 0, 0]), 0);
    assert_eq!(guest.error(), 6);
}

#[test]
fn oversized_view_requests_fail_without_disturbing_the_mapping() {
    for imports in VIEW_IMPORTS {
        let (dll, symbol) = imports[0];
        let mut guest = Guest::new(imports, &[]);
        let object = guest.call("CreateFileMappingA", &[INVALID_FILE, 0, 4, 0, 4096, 0]);
        assert_ne!(object, 0);
        assert_eq!(guest.call(symbol, &[0xdead, 2, 0, 0, 4097, 0]), 0);
        assert_eq!(guest.error(), 6);
        assert_eq!(guest.call(symbol, &[object, 2, 0, 1, 4097, 0]), 0);
        assert_eq!(guest.error(), 50);
        if symbol == "MapViewOfFileEx" {
            assert_eq!(guest.call(symbol, &[object, 2, 0, 0, 4097, 0x10000]), 0);
            assert_eq!(guest.error(), 50);
        }

        let whole_view = guest.call(symbol, &[object, 2, 0, 0, 0, 0]);
        assert_ne!(whole_view, 0, "{dll}!{symbol}");
        guest.memory.write_u8(whole_view + 4095, 42).unwrap();

        for requested in [4097, u64::MAX] {
            assert_eq!(
                guest.call(symbol, &[object, 2, 0, 0, requested, 0]),
                0,
                "{dll}!{symbol} accepted length {requested}"
            );
            assert_eq!(guest.error(), 87);
        }

        assert_eq!(guest.memory.read_u8(whole_view + 4095).unwrap(), 42);
        assert_eq!(guest.call("UnmapViewOfFile", &[whole_view]), 1);

        let exact_view = guest.call(symbol, &[object, 2, 0, 0, 4096, 0]);
        assert_ne!(exact_view, 0, "{dll}!{symbol}");
        guest.memory.write_u8(exact_view + 4095, 17).unwrap();
        assert_eq!(guest.call("UnmapViewOfFile", &[exact_view]), 1);
        assert_eq!(guest.call("CloseHandle", &[object]), 1);
    }
}

#[test]
fn flushing_checks_the_range_of_a_live_view() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let object = guest.call("CreateFileMappingA", &[INVALID_FILE, 0, 4, 0, 4096, 0]);
    assert_ne!(object, 0);
    let view = guest.call("MapViewOfFile", &[object, 2, 0, 0, 2048]);
    assert_ne!(view, 0);
    guest.memory.write_u64(view + 32, 42).unwrap();

    assert_eq!(guest.call("FlushViewOfFile", &[view + 32, 0]), 1);
    assert_eq!(guest.call("FlushViewOfFile", &[view + 32, 2016]), 1);
    assert_eq!(guest.u64_at(view + 32), 42);
    for args in [[view + 32, 2017], [view + 2048, 0], [view + 32, u64::MAX]] {
        assert_eq!(guest.call("FlushViewOfFile", &args), 0);
        assert_eq!(guest.error(), 87);
    }

    assert_eq!(guest.call("UnmapViewOfFile", &[view]), 1);
    assert_eq!(guest.call("FlushViewOfFile", &[view + 32, 0]), 0);
    assert_eq!(guest.error(), 87);
    assert_eq!(guest.call("CloseHandle", &[object]), 1);
}

#[test]
fn create_process_fails_honestly_and_zeroes_the_information() {
    let mut guest = Guest::new(IMPORTS, &[]);
    guest.memory.write_u64(SCRATCH + 0x100, u64::MAX).unwrap();
    let info = SCRATCH + 0x100;
    // Ten arguments: the last two are the startup and process information.
    assert_eq!(
        guest.call("CreateProcessA", &[0, 0, 0, 0, 0, 0, 0, 0, 0, info]),
        0
    );
    assert_eq!(guest.error(), 50);
    assert_eq!(guest.u64_at(info), 0);
}
