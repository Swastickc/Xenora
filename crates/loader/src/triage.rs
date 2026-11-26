// SPDX-License-Identifier: GPL-3.0-or-later

//! Read-only PE32+ header and triage inspection.
//!
//! Provides diagnostic reports for executable binaries without loading or executing them.

use crate::pe::{ParsedPe, PeError, PeSection};
use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// High-level diagnostic report from inspecting a PE32+ binary.
#[derive(Clone, Debug)]
pub struct TriageReport {
    pub file_size: usize,
    pub machine: u16,
    pub is_amd64: bool,
    pub image_base: u64,
    pub entry_point_rva: u32,
    pub size_of_image: u32,
    pub sections: Vec<PeSection>,
}

impl fmt::Display for TriageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "PE32+ Binary Triage Report:")?;
        writeln!(f, "  File Size:         {} bytes", self.file_size)?;
        writeln!(
            f,
            "  Machine:           0x{:04x} (AMD64: {})",
            self.machine, self.is_amd64
        )?;
        writeln!(f, "  Image Base:        0x{:016x}", self.image_base)?;
        writeln!(f, "  Entry Point RVA:   0x{:08x}", self.entry_point_rva)?;
        writeln!(f, "  Size of Image:     0x{:08x}", self.size_of_image)?;
        writeln!(f, "  Sections ({}):", self.sections.len())?;
        for sec in &self.sections {
            let perms = format!(
                "{}{}{}",
                if sec.permissions.read { "R" } else { "-" },
                if sec.permissions.write { "W" } else { "-" },
                if sec.permissions.execute { "X" } else { "-" }
            );
            writeln!(
                f,
                "    - {:<8} RVA: 0x{:08x} Size: 0x{:08x} Raw: 0x{:08x} Perms: [{}]",
                sec.name, sec.virtual_address, sec.virtual_size, sec.raw_data_size, perms
            )?;
        }
        Ok(())
    }
}

/// Why inspection of an executable file failed.
#[derive(Debug)]
pub enum TriageFileError {
    /// The file could not be read.
    Read(io::Error),
    /// The bytes were read but could not be parsed as a PE32+ image.
    Parse(PeError),
}

impl fmt::Display for TriageFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "could not read executable: {error}"),
            Self::Parse(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for TriageFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Parse(error) => Some(error),
        }
    }
}

impl From<PeError> for TriageFileError {
    fn from(error: PeError) -> Self {
        Self::Parse(error)
    }
}

fn require_regular_file(metadata: fs::Metadata) -> io::Result<()> {
    if metadata.is_file() {
        Ok(())
    } else if metadata.is_dir() {
        Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "Is a directory",
        ))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file",
        ))
    }
}

fn open_executable(path: &Path) -> io::Result<(fs::File, u64)> {
    // Follow symlinks, but avoid opening an input already known to be special.
    require_regular_file(fs::metadata(path)?)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        // A path may be replaced with a FIFO after the metadata check.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    let len = metadata.len();
    require_regular_file(metadata)?;
    Ok((file, len))
}

/// Inspect the headers of a regular PE32+ binary file in read-only mode.
pub fn triage_file<P: AsRef<Path>>(path: P) -> Result<TriageReport, TriageFileError> {
    let (mut file, len) = open_executable(path.as_ref()).map_err(TriageFileError::Read)?;
    let file_size = usize::try_from(len).map_err(|_| {
        TriageFileError::Read(io::Error::new(
            io::ErrorKind::InvalidData,
            "executable length does not fit in a file-size report",
        ))
    })?;
    let pe = ParsedPe::parse_headers(len, |offset, size| -> Result<_, TriageFileError> {
        let mut bytes = vec![0; size];
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut bytes))
            .map_err(TriageFileError::Read)?;
        Ok(Cow::Owned(bytes))
    })?;
    Ok(report_from_pe(pe, file_size))
}

/// Inspect in-memory PE32+ bytes in read-only mode.
pub fn triage_bytes(bytes: &[u8]) -> Result<TriageReport, PeError> {
    let pe = ParsedPe::parse(bytes)?;
    Ok(report_from_pe(pe, bytes.len()))
}

fn report_from_pe(pe: ParsedPe, file_size: usize) -> TriageReport {
    TriageReport {
        file_size,
        machine: pe.machine,
        is_amd64: pe.machine == crate::pe::IMAGE_FILE_MACHINE_AMD64,
        image_base: pe.image_base,
        entry_point_rva: pe.entry_point_rva,
        size_of_image: pe.size_of_image,
        sections: pe.sections,
    }
}
