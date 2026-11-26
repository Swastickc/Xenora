// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded parser for the PE32+ export directory (Microsoft PE format,
//! "The .edata Section").
//!
//! The parser inventories exports only. It does not bind them, resolve
//! forwarders, or decide whether a non-forwarder RVA names executable code or
//! data. Its address-table, name-table and string limits are Xenora policy
//! bounds for hostile input, not limits of the PE format. Reserved export
//! flags are ignored because the PE format assigns that behavior to consumers.
//! Unsorted export names are format errors. Distinct names may share an
//! address-table index, but reusing one name is rejected as a Xenora
//! validation policy because duplicate associations are ambiguous.
//!
//! Parsing stops at the first problem. Once the fixed directory, counts,
//! required table RVAs and module name have been accepted, the returned table
//! retains every export address-table entry and name association completed
//! before that problem. A caller that executes an image must refuse it when
//! the error is present; inspection may
//! report the partial table.

use std::fmt;

/// Most export address table entries accepted in one image.
pub const MAX_ADDRESS_TABLE_ENTRIES: usize = 65_536;
/// Most export name pointers accepted in one image.
pub const MAX_NAME_POINTERS: usize = 65_536;
/// Longest module, export or forwarder string accepted, in bytes.
pub const MAX_NAME_LEN: usize = 4096;

const DIRECTORY_LEN: u32 = 40;

/// What an export address table entry means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportTarget {
    /// A zero-valued address-table slot. Linkers use these slots as filler in
    /// sparse ordinal layouts; they do not resolve to an exported symbol.
    Empty,
    /// A nonzero exported symbol RVA outside the export directory. The parser
    /// does not validate which section or kind of bytes it names.
    ExportRva(u32),
    /// An RVA inside the export directory. By the PE format this is a
    /// null-terminated string naming the exported symbol in another image.
    /// The parser bounds and inventories the string but intentionally does not
    /// validate or resolve its DLL/symbol grammar.
    Forwarder {
        /// RVA of the forwarder string.
        rva: u32,
        /// Printable bounded contents of the forwarder string.
        target: String,
    },
}

/// One export address table slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportEntry {
    /// Biased ordinal: `ordinal_base + index`, checked against overflow.
    pub ordinal: u32,
    /// Zero-based index into the export address table.
    pub index: u32,
    /// Public names associated with this index. Distinct names may share an
    /// index; the PE format does not prohibit such aliases. A name can also be
    /// associated with an empty ordinal filler slot; consumers must resolve
    /// that association as absent rather than treating RVA zero as code.
    pub names: Vec<String>,
    /// Meaning of the entry's RVA.
    pub target: ExportTarget,
    /// RVA of this export address table slot.
    pub address_table_rva: u32,
}

/// Fixed export-directory metadata plus the entries read from its tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportTable {
    pub time_date_stamp: u32,
    pub major_version: u16,
    pub minor_version: u16,
    /// Module name as spelled in the image.
    pub module: String,
    /// Starting ordinal number for the export address table.
    pub ordinal_base: u32,
    /// Declared number of export address table entries. On an error,
    /// `entries.len()` can be smaller.
    pub address_table_entries: u32,
    /// Declared number of entries in each of the parallel name pointer and
    /// ordinal tables.
    pub name_pointers: u32,
    /// Entries read before the first error, or all entries on success.
    pub entries: Vec<ExportEntry>,
}

/// A required export-directory field whose RVA is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportField {
    ModuleName,
    AddressTable,
    NamePointerTable,
    OrdinalTable,
}

impl fmt::Display for ExportField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ModuleName => "module name",
            Self::AddressTable => "export address table",
            Self::NamePointerTable => "export name pointer table",
            Self::OrdinalTable => "export ordinal table",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportTableError {
    /// A fixed-width structure at this RVA is not fully backed by section data,
    /// or its table arithmetic overflows the RVA space.
    OutOfBounds {
        rva: u32,
    },
    /// The declared export directory ends at this RVA before the fixed
    /// 40-byte export directory is complete.
    ExportDirectoryEnd {
        rva: u32,
    },
    /// A count requires a table or name whose RVA is zero.
    FieldMissing {
        rva: u32,
        field: ExportField,
    },
    /// The module name is empty, unterminated, too long or not printable ASCII.
    MalformedModuleName {
        rva: u32,
    },
    /// An export name is empty, unterminated, too long or not printable ASCII.
    MalformedExportName {
        rva: u32,
    },
    /// A forwarder string is empty, unterminated, too long, not printable
    /// ASCII, or does not end inside the declared export directory.
    MalformedForwarder {
        rva: u32,
    },
    TooManyAddressTableEntries,
    TooManyNamePointers,
    /// Adding the ordinal base and an export address table index overflows.
    OrdinalOverflow {
        rva: u32,
        base: u32,
        index: u32,
    },
    /// A name's ordinal-table index does not select an export address table
    /// entry.
    NameOrdinalOutOfRange {
        rva: u32,
        index: u16,
        address_table_entries: u32,
    },
    /// The name pointer table is not in ascending lexical order.
    UnsortedNames {
        rva: u32,
    },
    /// The same public name is associated twice with the same index.
    DuplicateName {
        rva: u32,
        index: u16,
    },
    /// The same public name is associated with two different indexes.
    InconsistentName {
        rva: u32,
        first_index: u16,
        index: u16,
    },
}

impl fmt::Display for ExportTableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds { rva } => write!(
                f,
                "export data at RVA 0x{rva:08x} is truncated or outside the section data"
            ),
            Self::ExportDirectoryEnd { rva } => write!(
                f,
                "export directory ends at RVA 0x{rva:08x} before its 40-byte directory"
            ),
            Self::FieldMissing { rva, field } => {
                write!(f, "export directory at RVA 0x{rva:08x} has no {field}")
            }
            Self::MalformedModuleName { rva } => {
                write!(f, "malformed export module name at RVA 0x{rva:08x}")
            }
            Self::MalformedExportName { rva } => {
                write!(f, "malformed export name at RVA 0x{rva:08x}")
            }
            Self::MalformedForwarder { rva } => {
                write!(f, "malformed forwarder string at RVA 0x{rva:08x}")
            }
            Self::TooManyAddressTableEntries => write!(
                f,
                "more than {MAX_ADDRESS_TABLE_ENTRIES} export address table entries"
            ),
            Self::TooManyNamePointers => {
                write!(f, "more than {MAX_NAME_POINTERS} export name pointers")
            }
            Self::OrdinalOverflow { rva, base, index } => write!(
                f,
                "ordinal base {base} plus export address table index {index} overflows at RVA 0x{rva:08x}"
            ),
            Self::NameOrdinalOutOfRange {
                rva,
                index,
                address_table_entries,
            } => write!(
                f,
                "export ordinal table entry at RVA 0x{rva:08x} holds index {index}, outside {address_table_entries} address table entries"
            ),
            Self::UnsortedNames { rva } => write!(
                f,
                "export name pointer table entry at RVA 0x{rva:08x} is out of lexical order"
            ),
            Self::DuplicateName { rva, index } => write!(
                f,
                "duplicate export name at RVA 0x{rva:08x} for address table index {index}"
            ),
            Self::InconsistentName {
                rva,
                first_index,
                index,
            } => write!(
                f,
                "export name at RVA 0x{rva:08x} is associated with both address table indexes {first_index} and {index}"
            ),
        }
    }
}

impl std::error::Error for ExportTableError {}

/// Reads image bytes by RVA, requiring every fixed-width structure byte to be
/// backed by the file.
struct Reader<'a, F> {
    bytes: &'a [u8],
    rva_to_offset: F,
}

#[derive(Clone, Copy)]
enum TextKind {
    Module,
    Export,
    Forwarder,
}

impl TextKind {
    fn error(self, rva: u32) -> ExportTableError {
        match self {
            Self::Module => ExportTableError::MalformedModuleName { rva },
            Self::Export => ExportTableError::MalformedExportName { rva },
            Self::Forwarder => ExportTableError::MalformedForwarder { rva },
        }
    }
}

impl<F: Fn(u32) -> Option<usize>> Reader<'_, F> {
    fn byte(&self, rva: u32) -> Option<u8> {
        self.bytes.get((self.rva_to_offset)(rva)?).copied()
    }

    fn array<const N: usize>(&self, rva: u32) -> Result<[u8; N], ExportTableError> {
        let mut bytes = [0; N];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let at = rva
                .checked_add(index as u32)
                .ok_or(ExportTableError::OutOfBounds { rva })?;
            *byte = self.byte(at).ok_or(ExportTableError::OutOfBounds { rva })?;
        }
        Ok(bytes)
    }

    fn u32(&self, rva: u32) -> Result<u32, ExportTableError> {
        self.array(rva).map(u32::from_le_bytes)
    }

    fn u16(&self, rva: u32) -> Result<u16, ExportTableError> {
        self.array(rva).map(u16::from_le_bytes)
    }

    /// Read a printable NUL-terminated ASCII string. When `end_exclusive` is
    /// present, both the contents and terminator must end before that RVA.
    fn text(
        &self,
        rva: u32,
        end_exclusive: Option<u32>,
        kind: TextKind,
    ) -> Result<String, ExportTableError> {
        let malformed = kind.error(rva);
        let mut text = String::new();
        for index in 0..=MAX_NAME_LEN as u32 {
            let Some(at) = rva.checked_add(index) else {
                return Err(malformed);
            };
            if end_exclusive.is_some_and(|end| at >= end) {
                return Err(malformed);
            }
            let Some(byte) = self.byte(at) else {
                return Err(malformed);
            };
            match byte {
                0 if text.is_empty() => return Err(malformed),
                0 => return Ok(text),
                b' '..=b'~' => text.push(char::from(byte)),
                _ => return Err(malformed),
            }
        }
        Err(malformed)
    }
}

/// Parse the export directory of `directory_size` bytes at `directory_rva`.
///
/// The declared directory extent must contain the fixed 40-byte directory and
/// is the range used to recognize forwarder RVAs. Fixed-width export tables
/// and names must be fully backed by section data. Returns the table once its
/// fixed directory, counts, required table RVAs and module name are valid,
/// plus the first problem found.
pub fn parse_exports(
    bytes: &[u8],
    directory_rva: u32,
    directory_size: u32,
    rva_to_offset: impl Fn(u32) -> Option<usize>,
) -> (Option<ExportTable>, Option<ExportTableError>) {
    let reader = Reader {
        bytes,
        rva_to_offset,
    };
    let mut table = None;
    let error = collect(&reader, directory_rva, directory_size, &mut table).err();
    (table, error)
}

fn collect<F: Fn(u32) -> Option<usize>>(
    reader: &Reader<'_, F>,
    directory_rva: u32,
    directory_size: u32,
    output: &mut Option<ExportTable>,
) -> Result<(), ExportTableError> {
    let directory_end = directory_rva
        .checked_add(directory_size)
        .ok_or(ExportTableError::OutOfBounds { rva: directory_rva })?;
    if directory_size < DIRECTORY_LEN {
        return Err(ExportTableError::ExportDirectoryEnd { rva: directory_end });
    }

    let fields = reader.array::<{ DIRECTORY_LEN as usize }>(directory_rva)?;
    let time_date_stamp = u32::from_le_bytes(fields[4..8].try_into().unwrap());
    let major_version = u16::from_le_bytes(fields[8..10].try_into().unwrap());
    let minor_version = u16::from_le_bytes(fields[10..12].try_into().unwrap());
    let module_rva = u32::from_le_bytes(fields[12..16].try_into().unwrap());
    let ordinal_base = u32::from_le_bytes(fields[16..20].try_into().unwrap());
    let address_table_entries = u32::from_le_bytes(fields[20..24].try_into().unwrap());
    let name_pointers = u32::from_le_bytes(fields[24..28].try_into().unwrap());
    let address_table_rva = u32::from_le_bytes(fields[28..32].try_into().unwrap());
    let name_pointer_rva = u32::from_le_bytes(fields[32..36].try_into().unwrap());
    let ordinal_table_rva = u32::from_le_bytes(fields[36..40].try_into().unwrap());

    if address_table_entries as usize > MAX_ADDRESS_TABLE_ENTRIES {
        return Err(ExportTableError::TooManyAddressTableEntries);
    }
    if name_pointers as usize > MAX_NAME_POINTERS {
        return Err(ExportTableError::TooManyNamePointers);
    }
    if module_rva == 0 {
        return Err(ExportTableError::FieldMissing {
            rva: directory_rva,
            field: ExportField::ModuleName,
        });
    }
    if address_table_entries != 0 && address_table_rva == 0 {
        return Err(ExportTableError::FieldMissing {
            rva: directory_rva,
            field: ExportField::AddressTable,
        });
    }
    if name_pointers != 0 {
        for (field, rva) in [
            (ExportField::NamePointerTable, name_pointer_rva),
            (ExportField::OrdinalTable, ordinal_table_rva),
        ] {
            if rva == 0 {
                return Err(ExportTableError::FieldMissing {
                    rva: directory_rva,
                    field,
                });
            }
        }
    }

    let module = reader.text(module_rva, None, TextKind::Module)?;
    *output = Some(ExportTable {
        time_date_stamp,
        major_version,
        minor_version,
        module,
        ordinal_base,
        address_table_entries,
        name_pointers,
        entries: Vec::new(),
    });
    let table = output.as_mut().unwrap();

    collect_address_table(
        reader,
        table,
        address_table_rva,
        directory_rva,
        directory_end,
    )?;
    collect_names(reader, table, name_pointer_rva, ordinal_table_rva)?;
    Ok(())
}

fn table_entry_rva(base: u32, index: u32, width: u32) -> Result<u32, ExportTableError> {
    let offset = index
        .checked_mul(width)
        .ok_or(ExportTableError::OutOfBounds { rva: base })?;
    base.checked_add(offset)
        .ok_or(ExportTableError::OutOfBounds { rva: base })
}

fn collect_address_table<F: Fn(u32) -> Option<usize>>(
    reader: &Reader<'_, F>,
    table: &mut ExportTable,
    address_table_rva: u32,
    directory_rva: u32,
    directory_end: u32,
) -> Result<(), ExportTableError> {
    for index in 0..table.address_table_entries {
        let slot = table_entry_rva(address_table_rva, index, 4)?;
        let ordinal =
            table
                .ordinal_base
                .checked_add(index)
                .ok_or(ExportTableError::OrdinalOverflow {
                    rva: slot,
                    base: table.ordinal_base,
                    index,
                })?;
        let value = reader.u32(slot)?;
        let target = if value == 0 {
            ExportTarget::Empty
        } else if value >= directory_rva && value < directory_end {
            ExportTarget::Forwarder {
                rva: value,
                target: reader.text(value, Some(directory_end), TextKind::Forwarder)?,
            }
        } else {
            ExportTarget::ExportRva(value)
        };
        table.entries.push(ExportEntry {
            ordinal,
            index,
            names: Vec::new(),
            target,
            address_table_rva: slot,
        });
    }
    Ok(())
}

fn collect_names<F: Fn(u32) -> Option<usize>>(
    reader: &Reader<'_, F>,
    table: &mut ExportTable,
    name_pointer_rva: u32,
    ordinal_table_rva: u32,
) -> Result<(), ExportTableError> {
    let mut previous: Option<(String, u16)> = None;
    for position in 0..table.name_pointers {
        let name_slot = table_entry_rva(name_pointer_rva, position, 4)?;
        let ordinal_slot = table_entry_rva(ordinal_table_rva, position, 2)?;
        let name_rva = reader.u32(name_slot)?;
        let index = reader.u16(ordinal_slot)?;
        if u32::from(index) >= table.address_table_entries {
            return Err(ExportTableError::NameOrdinalOutOfRange {
                rva: ordinal_slot,
                index,
                address_table_entries: table.address_table_entries,
            });
        }
        let name = reader.text(name_rva, None, TextKind::Export)?;
        if let Some((previous_name, previous_index)) = &previous {
            if name.as_str() == previous_name.as_str() {
                if index == *previous_index {
                    return Err(ExportTableError::DuplicateName {
                        rva: name_slot,
                        index,
                    });
                }
                return Err(ExportTableError::InconsistentName {
                    rva: name_slot,
                    first_index: *previous_index,
                    index,
                });
            }
            if name.as_str() < previous_name.as_str() {
                return Err(ExportTableError::UnsortedNames { rva: name_slot });
            }
        }
        let Some(entry) = table.entries.get_mut(usize::from(index)) else {
            return Err(ExportTableError::NameOrdinalOutOfRange {
                rva: ordinal_slot,
                index,
                address_table_entries: table.address_table_entries,
            });
        };
        entry.names.push(name.clone());
        previous = Some((name, index));
    }
    Ok(())
}
