use xenora_loader::PeError;
use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};

/// The static probe must never panic on a truncated PE.
///
/// The fixture is truncated across its whole length, which covers the range
/// where the import directory and the import names overlap the end of the
/// file, and every other range where a name or thunk read computed from the
/// section header can run past the buffer.
#[test]
fn probe_never_panics_on_truncated_input() {
    let pe = include_bytes!("../../../fixtures/pe/trusted-runner.exe");
    for len in 1..=pe.len() {
        let truncated = &pe[..len];
        // The probe may reject the file or report it with unreadable
        // imports, but it must not panic.
        let _ = probe_pe_bytes(truncated, &ProbeConfig::default());
    }
}

#[test]
fn probe_rejects_optional_headers_shorter_than_the_fixed_pe32_plus_fields() {
    let pe = include_bytes!("../../../fixtures/pe/trusted-runner.exe");
    let coff = u32::from_le_bytes(pe[0x3C..0x40].try_into().unwrap()) as usize + 4;
    let optional = coff + 20;

    for declared in [0u16, 1, 2, 63, 64, 111] {
        let mut changed = pe.to_vec();
        changed[coff + 16..coff + 18].copy_from_slice(&declared.to_le_bytes());

        // A readable file suffix cannot stand in for undeclared fields.
        assert!(
            matches!(
                probe_pe_bytes(&changed, &ProbeConfig::default()),
                Err(PeError::TooSmall)
            ),
            "declared size: {declared}"
        );

        // In particular, the zero-size case ends before optional magic and
        // must return an error instead of indexing outside the file.
        changed.truncate(optional + usize::from(declared));
        assert!(
            matches!(
                probe_pe_bytes(&changed, &ProbeConfig::default()),
                Err(PeError::TooSmall)
            ),
            "truncated at declared size: {declared}"
        );
    }
}
