// SPDX-License-Identifier: GPL-3.0-or-later

//! PE32+ (64-bit Portable Executable) format parser and section loader for Xenora.
//!
//! Parses standard PE32+ headers (DOS header, COFF header, Optional Header 64)
//! and loads the headers and each section into `GuestMemory` with appropriate
//! permissions.

use crate::memory::{GuestMemory, MemoryError, MemoryPermissions};
use std::fmt;

/// Errors that can occur during PE32+ parsing and loading.
#[derive(Debug, PartialEq, Eq)]
pub enum PeError {
    TooSmall,
    InvalidDosSignature,
    InvalidPeSignature,
    UnsupportedMachine(u16),
    InvalidMagic(u16),
    InvalidSectionHeaders,
    SectionMappingError(MemoryError),
    InvalidEntryPoint,
    /// The headers cannot be mapped at the image base: `SizeOfHeaders` is
    /// zero, ends before the section table does, runs past the end of the
    /// file, or its pages reach into a section; or the bytes given to the
    /// loader do not hold the PE32+ headers that were parsed.
    InvalidHeaderSize,
}

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall => write!(f, "executable image is smaller than required headers"),
            Self::InvalidDosSignature => write!(f, "invalid DOS signature (expected 'MZ')"),
            Self::InvalidPeSignature => write!(f, "invalid PE signature (expected 'PE\\0\\0')"),
            Self::UnsupportedMachine(m) => {
                write!(
                    f,
                    "unsupported machine type 0x{m:04x} (expected AMD64 0x8664)"
                )
            }
            Self::InvalidMagic(magic) => {
                write!(
                    f,
                    "invalid optional header magic 0x{magic:04x} (expected PE32+ 0x020b)"
                )
            }
            Self::InvalidSectionHeaders => write!(f, "invalid or corrupt section headers"),
            Self::SectionMappingError(e) => write!(f, "failed to map section: {e}"),
            Self::InvalidEntryPoint => write!(f, "entry point address is outside executable code"),
            Self::InvalidHeaderSize => write!(
                f,
                "the PE headers cannot be mapped: SizeOfHeaders is zero, shorter than the section \
                 table, longer than the file or reaches into a section, or the bytes do not match \
                 the parsed headers"
            ),
        }
    }
}

impl std::error::Error for PeError {}

/// An individual section extracted from a PE32+ header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeSection {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_data_ptr: u32,
    pub raw_data_size: u32,
    pub characteristics: u32,
    pub permissions: MemoryPermissions,
}

/// Parsed metadata from a PE32+ executable.
#[derive(Clone, Debug)]
pub struct ParsedPe {
    pub machine: u16,
    pub number_of_sections: u16,
    pub time_date_stamp: u32,
    pub image_base: u64,
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub entry_point_rva: u32,
    pub sections: Vec<PeSection>,
}

/// Result of loading a PE32+ image into guest memory.
pub struct LoadedImage {
    pub pe: ParsedPe,
    pub memory: GuestMemory,
    pub entry_point: u64,
    /// Guest address of the `ImageBase` field in the mapped optional header.
    /// Startup writes the load base there once relocations have been applied.
    pub(crate) image_base_field: u64,
}

// Windows PE Characteristic Constants
pub const IMAGE_SCN_CNT_CODE: u32 = 0x00000020;
pub const IMAGE_SCN_MEM_EXECUTE: u32 = 0x20000000;
pub const IMAGE_SCN_MEM_READ: u32 = 0x40000000;
pub const IMAGE_SCN_MEM_WRITE: u32 = 0x80000000;

// Standard Constants
pub const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
pub const PE32_PLUS_MAGIC: u16 = 0x020B;
/// PE32+ optional header length up to and including NumberOfRvaAndSizes.
const PE32_PLUS_MIN_OPTIONAL_HEADER: usize = 112;
const SECTION_HEADER_SIZE: usize = 40;
/// Host memory budget for the mapped headers and sections of one image. This is a
/// loader policy limit, not a property of the PE format.
const MAX_MAPPED_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
/// The mapped headers end on a guest page boundary.
const HEADER_PAGE: u64 = 0x1000;
/// `ImageBase` and `SizeOfHeaders` within the PE32+ optional header.
const OPTIONAL_IMAGE_BASE: usize = 24;
const OPTIONAL_SIZE_OF_HEADERS: usize = 60;

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

impl ParsedPe {
    /// Parse PE32+ headers from an in-memory byte slice.
    pub fn parse(bytes: &[u8]) -> Result<Self, PeError> {
        if bytes.len() < 0x40 {
            return Err(PeError::TooSmall);
        }

        // 1. DOS Header
        if &bytes[0..2] != b"MZ" {
            return Err(PeError::InvalidDosSignature);
        }

        let e_lfanew = u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap()) as usize;
        let coff_offset = e_lfanew.checked_add(4).ok_or(PeError::TooSmall)?;
        if bytes.len() < coff_offset.checked_add(20).ok_or(PeError::TooSmall)? {
            return Err(PeError::TooSmall);
        }

        // 2. PE Signature
        if &bytes[e_lfanew..coff_offset] != b"PE\0\0" {
            return Err(PeError::InvalidPeSignature);
        }

        // 3. COFF File Header
        let machine = read_u16(bytes, coff_offset);
        if machine != IMAGE_FILE_MACHINE_AMD64 {
            return Err(PeError::UnsupportedMachine(machine));
        }
        let number_of_sections = read_u16(bytes, coff_offset + 2);
        let time_date_stamp = read_u32(bytes, coff_offset + 4);
        let size_of_optional_header = usize::from(read_u16(bytes, coff_offset + 16));

        // 4. Optional Header (PE32+ is required). The fixed PE32+ fields end at
        // NumberOfRvaAndSizes (offset 108, 4 bytes); a shorter header is malformed.
        let opt_offset = coff_offset + 20;
        let section_table_offset = opt_offset + size_of_optional_header;
        if bytes.len() < opt_offset + 2 {
            return Err(PeError::TooSmall);
        }
        let magic = read_u16(bytes, opt_offset);
        if magic != PE32_PLUS_MAGIC {
            return Err(PeError::InvalidMagic(magic));
        }
        if size_of_optional_header < PE32_PLUS_MIN_OPTIONAL_HEADER
            || bytes.len() < section_table_offset
        {
            return Err(PeError::TooSmall);
        }

        let entry_point_rva = read_u32(bytes, opt_offset + 16);
        let image_base =
            u64::from_le_bytes(bytes[opt_offset + 24..opt_offset + 32].try_into().unwrap());
        let section_alignment = read_u32(bytes, opt_offset + 32);
        let file_alignment = read_u32(bytes, opt_offset + 36);
        let size_of_image = read_u32(bytes, opt_offset + 56);
        let size_of_headers = read_u32(bytes, opt_offset + 60);

        // 5. Section Headers
        let table_end = usize::from(number_of_sections)
            .checked_mul(SECTION_HEADER_SIZE)
            .and_then(|len| section_table_offset.checked_add(len))
            .ok_or(PeError::InvalidSectionHeaders)?;
        if bytes.len() < table_end {
            return Err(PeError::InvalidSectionHeaders);
        }

        let mut sections = Vec::with_capacity(usize::from(number_of_sections));
        for sec_bytes in bytes[section_table_offset..table_end]
            .as_chunks::<SECTION_HEADER_SIZE>()
            .0
        {
            // Section name (up to 8 bytes UTF-8 or null-padded ASCII)
            let name_raw = &sec_bytes[0..8];
            let name_len = name_raw.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_raw[..name_len]).into_owned();

            let virtual_size = read_u32(sec_bytes, 8);
            let virtual_address = read_u32(sec_bytes, 12);
            let raw_data_size = read_u32(sec_bytes, 16);
            let raw_data_ptr = read_u32(sec_bytes, 20);
            let characteristics = read_u32(sec_bytes, 36);

            // The mapped range must fit in the 32-bit RVA space, and raw data must be in the file.
            virtual_address
                .checked_add(virtual_size.max(raw_data_size))
                .ok_or(PeError::InvalidSectionHeaders)?;
            if raw_data_size > 0 {
                let raw_end = u64::from(raw_data_ptr) + u64::from(raw_data_size);
                if raw_end > bytes.len() as u64 {
                    return Err(PeError::InvalidSectionHeaders);
                }
            }

            let permissions = MemoryPermissions {
                read: (characteristics & IMAGE_SCN_MEM_READ) != 0,
                write: (characteristics & IMAGE_SCN_MEM_WRITE) != 0,
                execute: (characteristics & IMAGE_SCN_MEM_EXECUTE) != 0,
            };

            sections.push(PeSection {
                name,
                virtual_address,
                virtual_size,
                raw_data_ptr,
                raw_data_size,
                characteristics,
                permissions,
            });
        }

        // Mapped section ranges must not overlap. Zero-sized sections map nothing.
        let mut ranges: Vec<(u32, u32)> = sections
            .iter()
            .map(|s| {
                (
                    s.virtual_address,
                    s.virtual_address + s.virtual_size.max(s.raw_data_size),
                )
            })
            .filter(|(start, end)| start < end)
            .collect();
        ranges.sort_unstable();
        if ranges.windows(2).any(|w| w[1].0 < w[0].1) {
            return Err(PeError::InvalidSectionHeaders);
        }

        // The entry point must lie inside an executable section.
        let entry_ok = sections.iter().any(|s| {
            s.permissions.execute
                && entry_point_rva >= s.virtual_address
                && entry_point_rva - s.virtual_address < s.virtual_size.max(s.raw_data_size)
        });
        if !entry_ok {
            return Err(PeError::InvalidEntryPoint);
        }

        Ok(ParsedPe {
            machine,
            number_of_sections,
            time_date_stamp,
            image_base,
            section_alignment,
            file_alignment,
            size_of_image,
            size_of_headers,
            entry_point_rva,
            sections,
        })
    }

    /// Translate a Relative Virtual Address (RVA) to a raw file offset.
    pub fn rva_to_file_offset(&self, rva: u32) -> Option<usize> {
        for sec in &self.sections {
            // Sections are public and may not come from `parse`, so stay checked here.
            let Some(diff) = rva.checked_sub(sec.virtual_address) else {
                continue;
            };
            if diff < sec.virtual_size.max(sec.raw_data_size) && diff < sec.raw_data_size {
                return (sec.raw_data_ptr as usize).checked_add(diff as usize);
            }
        }
        None
    }

    /// Where the headers are and how much to map for them: the file length of
    /// the headers, the page-rounded length of the mapping, and the file offset
    /// of the optional header's `ImageBase` field. Nothing is copied or
    /// allocated here.
    ///
    /// A caller may pass bytes other than the ones parsed, so the header
    /// layout is read again from `bytes` with every access checked, and must
    /// be a PE32+ optional header that agrees with the parsed metadata.
    fn headers_for(&self, bytes: &[u8]) -> Result<(usize, u64, usize), PeError> {
        let field = |at: usize, len: usize| at.checked_add(len).and_then(|end| bytes.get(at..end));
        let u16_at = |at: usize| Some(u16::from_le_bytes(field(at, 2)?.try_into().ok()?));
        let u32_at = |at: usize| Some(u32::from_le_bytes(field(at, 4)?.try_into().ok()?));
        let layout = || {
            let coff = (u32_at(0x3C)? as usize).checked_add(4)?;
            let sections = u16_at(coff.checked_add(2)?)?;
            let optional_size = usize::from(u16_at(coff.checked_add(16)?)?);
            let optional = coff.checked_add(20)?;
            let image_base = optional.checked_add(OPTIONAL_IMAGE_BASE)?;
            let agrees = u16_at(optional)? == PE32_PLUS_MAGIC
                && optional_size >= PE32_PLUS_MIN_OPTIONAL_HEADER
                && sections == self.number_of_sections
                && u64::from_le_bytes(field(image_base, 8)?.try_into().ok()?) == self.image_base
                && u32_at(optional.checked_add(OPTIONAL_SIZE_OF_HEADERS)?)? == self.size_of_headers;
            let table_end = optional
                .checked_add(optional_size)?
                .checked_add(usize::from(sections).checked_mul(SECTION_HEADER_SIZE)?)?;
            agrees.then_some((image_base, table_end))
        };
        let (image_base, table_end) = layout().ok_or(PeError::InvalidHeaderSize)?;

        // A size of zero is shorter than any section table.
        let len = self.size_of_headers as usize;
        if len < table_end || len > bytes.len() {
            return Err(PeError::InvalidHeaderSize);
        }
        let mapped = u64::from(self.size_of_headers).next_multiple_of(HEADER_PAGE);
        let reaches_a_section = self.sections.iter().any(|sec| {
            sec.virtual_size.max(sec.raw_data_size) > 0 && u64::from(sec.virtual_address) < mapped
        });
        if reaches_a_section {
            return Err(PeError::InvalidHeaderSize);
        }
        Ok((len, mapped, image_base))
    }

    /// Load this parsed PE into a fresh `GuestMemory` instance, mapping the
    /// headers and all sections.
    pub fn load_into_memory(&self, bytes: &[u8]) -> Result<LoadedImage, PeError> {
        self.load_at_base(bytes, self.image_base)
    }

    pub(crate) fn load_at_base(&self, bytes: &[u8], base: u64) -> Result<LoadedImage, PeError> {
        let (headers_len, headers_mapped, image_base_field) = self.headers_for(bytes)?;

        // The headers and each section become zero-filled host allocations, so bound
        // the total before allocating anything. Sections may not overlap, so the sum
        // is the mapped size.
        let mapped = self
            .sections
            .iter()
            .map(|sec| u64::from(sec.virtual_size.max(sec.raw_data_size)))
            .sum::<u64>()
            + headers_mapped;
        if mapped > MAX_MAPPED_IMAGE_BYTES {
            return Err(PeError::SectionMappingError(MemoryError::AllocationFailed));
        }

        let mut memory = GuestMemory::new();

        // Windows maps the headers read-only at the image base, and programs read
        // them: a C runtime finds its own sections this way before `main`. The
        // budget above is checked first, and the file bytes are the initial data
        // of the mapping, so nothing else is copied.
        memory
            .map_region_with_data(
                base,
                headers_mapped as usize,
                MemoryPermissions::READ_ONLY,
                &bytes[..headers_len],
            )
            .map_err(PeError::SectionMappingError)?;
        let image_base_field = base
            .checked_add(image_base_field as u64)
            .ok_or(PeError::InvalidHeaderSize)?;

        for sec in &self.sections {
            let base_addr = base
                .checked_add(u64::from(sec.virtual_address))
                .ok_or(PeError::InvalidSectionHeaders)?;
            let map_size = std::cmp::max(sec.virtual_size, sec.raw_data_size) as usize;
            if map_size == 0 {
                continue;
            }

            let initial_data = if sec.raw_data_size > 0 && sec.raw_data_ptr > 0 {
                let start = sec.raw_data_ptr as usize;
                let end = start
                    .checked_add(sec.raw_data_size as usize)
                    .ok_or(PeError::InvalidSectionHeaders)?;
                bytes
                    .get(start..end)
                    .ok_or(PeError::InvalidSectionHeaders)?
            } else {
                &[]
            };

            memory
                .map_region_with_data(base_addr, map_size, sec.permissions, initial_data)
                .map_err(PeError::SectionMappingError)?;
        }

        let entry_point = base
            .checked_add(u64::from(self.entry_point_rva))
            .ok_or(PeError::InvalidEntryPoint)?;

        Ok(LoadedImage {
            pe: self.clone(),
            memory,
            entry_point,
            image_base_field,
        })
    }
}
