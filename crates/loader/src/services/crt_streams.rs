// SPDX-License-Identifier: GPL-3.0-or-later

//! The streams of a hosted C program, and where their output goes.
//!
//! `__acrt_iob_func` hands out the `FILE` objects for `stdin`, `stdout` and
//! `stderr`. What the guest writes to `stdout` and to `stderr` is kept in two
//! host-side buffers that a host or a test reads through `ServicesState`.
//!
//! The standard streams "always open in text mode by default", and in text
//! mode "each line feed is replaced with a carriage return-line feed pair"
//! that "has no effect on the return value" (Microsoft Learn, "Text and
//! binary mode file I/O" and "fwrite"). The buffers hold the bytes after that
//! replacement, which is what the operating system would be handed.
//!
//! A stream that `fopen` returned is a file on the in-memory drive; its rules
//! are in `crt_files.rs`.
//!
//! Output is never held back: there is no buffering to flush or to configure.
//! A pointer that is no stream, reading `stdin` and closing a standard stream
//! are not modelled and stop the guest with the runtime-limit fault.

use super::ObjectState;
use super::crt_files::{self, EBADF, EINVAL, FileStreams};
use super::crt_format::{self, FormatError, Source};
use crate::cpu::ExitReason;
use crate::memory::{GuestMemory, MemoryError, PROCESS_HEAP};

/// The exit for a request the runtime does not model.
const RUNTIME_LIMIT: ExitReason = ExitReason::IllegalInstruction(0xFE);
/// `typedef struct _iobuf { void* _Placeholder; } FILE;` (corecrt_wstdio.h).
const FILE_BYTES: u64 = 8;
/// The most a format string or one `fwrite` or `fputs` may hold.
const MAX_TRANSFER: u64 = 1 << 20;
/// The longest file name or mode string `fopen` reads, in bytes.
const MAX_NAME: u64 = 260;
/// The most each captured stream keeps. Writing past it stops the guest.
pub const MAX_CAPTURED_OUTPUT: usize = 16 << 20;

/// Option bits of `__stdio_common_vfprintf` (corecrt_stdio_config.h) that do
/// not change anything this engine formats: the wide-specifier bit concerns
/// the wide functions, and the exponent and rounding bits concern floating
/// point. Any other bit stops the guest.
const IGNORED_OPTIONS: u64 = 0x04 | 0x10 | 0x20;

/// `_IOFBF`, `_IOLBF` and `_IONBF` (stdio.h).
const IOFBF: i32 = 0x0000;
const IOLBF: i32 = 0x0040;
const IONBF: i32 = 0x0004;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Input,
    Output,
    Error,
    /// A stream `fopen` returned, by the address of its `FILE` object.
    File(u64),
}

#[derive(Default)]
pub(super) struct StandardStreams {
    /// Guest address of the three `FILE` objects, `stdin` first.
    files: Option<u64>,
    output: Vec<u8>,
    error: Vec<u8>,
    /// The streams `fopen` has open.
    opened: FileStreams,
}

/// What a service returns when it fails with a result and an `errno` value:
/// the handler sets `errno` to the second field and returns the first.
pub(super) struct Failed<T>(pub T, pub u32);

pub(super) type Outcome<T> = Result<Result<T, Failed<T>>, ExitReason>;

fn fault(error: MemoryError) -> ExitReason {
    ExitReason::MemoryFault(error)
}

/// The bytes of the C string at `address`, without its terminator.
fn c_string(memory: &GuestMemory, address: u64, limit: u64) -> Result<Vec<u8>, MemoryError> {
    let mut bytes = Vec::new();
    for offset in 0..limit {
        let at = address.checked_add(offset).ok_or(MemoryError::Overflow)?;
        match memory.read_u8(at)? {
            0 => break,
            byte => bytes.push(byte),
        }
    }
    Ok(bytes)
}

/// Arguments from the guest's `va_list`: one 8-byte slot each, in order.
struct VaList<'a> {
    memory: &'a GuestMemory,
    next: u64,
}

impl Source for VaList<'_> {
    fn argument(&mut self) -> Result<u64, MemoryError> {
        let slot = self.memory.read_u64(self.next)?;
        self.next = self.next.checked_add(8).ok_or(MemoryError::Overflow)?;
        Ok(slot)
    }

    fn string(&mut self, address: u64, limit: usize) -> Result<Vec<u8>, MemoryError> {
        c_string(self.memory, address, limit as u64)
    }
}

impl StandardStreams {
    pub(super) fn output(&self) -> &[u8] {
        &self.output
    }

    pub(super) fn error(&self) -> &[u8] {
        &self.error
    }

    /// `__acrt_iob_func`: the `FILE` object of `stdin` (0), `stdout` (1) or
    /// `stderr` (2). The three are allocated together from the process heap
    /// on first use.
    pub(super) fn file(&mut self, memory: &mut GuestMemory, index: u32) -> Result<u64, ExitReason> {
        if index > 2 {
            return Err(RUNTIME_LIMIT);
        }
        let base = match self.files {
            Some(base) => base,
            None => {
                let base = memory
                    .map_heap_allocation(PROCESS_HEAP, (3 * FILE_BYTES) as usize)
                    .map_err(fault)?;
                self.files = Some(base);
                base
            }
        };
        Ok(base + FILE_BYTES * u64::from(index))
    }

    /// Which stream a non-null `FILE*` is: a standard stream, or one `fopen`
    /// returned. Any other pointer is no stream.
    fn stream(&self, file: u64) -> Result<Stream, ExitReason> {
        match self.files.map(|base| file.wrapping_sub(base)) {
            Some(0) => Ok(Stream::Input),
            Some(8) => Ok(Stream::Output),
            Some(16) => Ok(Stream::Error),
            _ if self.opened.contains(file) => Ok(Stream::File(file)),
            _ => Err(RUNTIME_LIMIT),
        }
    }

    /// A stream that may be written. Writing `stdin` is not modelled.
    fn writable(&self, file: u64) -> Result<Stream, ExitReason> {
        match self.stream(file)? {
            Stream::Input => Err(RUNTIME_LIMIT),
            stream => Ok(stream),
        }
    }

    /// Write to a stream. A standard stream is captured in text mode: each
    /// line feed becomes a carriage return and a line feed. A file stream can
    /// fail, with the `errno` value in the inner result and nothing written.
    fn write(
        &mut self,
        drive: &mut ObjectState,
        stream: Stream,
        bytes: &[u8],
    ) -> Result<Result<(), u32>, ExitReason> {
        let captured = match stream {
            Stream::Output => &mut self.output,
            Stream::Error => &mut self.error,
            Stream::Input => return Err(RUNTIME_LIMIT),
            Stream::File(file) => return Ok(self.opened.write(drive, file, bytes)),
        };
        let feeds = bytes.iter().filter(|byte| **byte == b'\n').count();
        if bytes.len() + feeds > MAX_CAPTURED_OUTPUT - captured.len() {
            return Err(RUNTIME_LIMIT);
        }
        for &byte in bytes {
            if byte == b'\n' {
                captured.push(b'\r');
            }
            captured.push(byte);
        }
        Ok(Ok(()))
    }

    /// `__stdio_common_vfprintf`. Returns the number of characters written,
    /// before the text-mode replacement.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn print(
        &mut self,
        memory: &GuestMemory,
        drive: &mut ObjectState,
        options: u64,
        file: u64,
        format: u64,
        locale: u64,
        arguments: u64,
    ) -> Outcome<i32> {
        // A null stream or format: the function returns -1 and sets errno.
        if file == 0 || format == 0 {
            return Ok(Err(Failed(-1, EINVAL)));
        }
        if options & !IGNORED_OPTIONS != 0 || locale != 0 {
            return Err(RUNTIME_LIMIT);
        }
        let stream = self.writable(file)?;
        let template = c_string(memory, format, MAX_TRANSFER + 1).map_err(fault)?;
        if template.len() as u64 > MAX_TRANSFER {
            return Err(RUNTIME_LIMIT);
        }
        let mut source = VaList {
            memory,
            next: arguments,
        };
        let text = match crt_format::format(&template, &mut source) {
            Ok(text) => text,
            // Windows calls the invalid parameter handler for an invalid
            // format. Xenora's choice for when execution continues: nothing
            // is written and the result is that of a null format.
            Err(FormatError::Invalid) => return Ok(Err(Failed(-1, EINVAL))),
            Err(FormatError::Unsupported | FormatError::TooLong) => return Err(RUNTIME_LIMIT),
            Err(FormatError::Fault(error)) => return Err(fault(error)),
        };
        // "A negative value if an output error occurs."
        Ok(match self.write(drive, stream, &text)? {
            Ok(()) => Ok(text.len() as i32),
            Err(errno) => Err(Failed(-1, errno)),
        })
    }

    /// `fwrite`. Returns the number of whole items written.
    pub(super) fn write_items(
        &mut self,
        memory: &GuestMemory,
        drive: &mut ObjectState,
        buffer: u64,
        size: u64,
        count: u64,
        file: u64,
    ) -> Outcome<u64> {
        if buffer == 0 || file == 0 {
            return Ok(Err(Failed(0, EINVAL)));
        }
        let stream = self.writable(file)?;
        let Some(length) = size
            .checked_mul(count)
            .filter(|bytes| *bytes <= MAX_TRANSFER)
        else {
            return Err(RUNTIME_LIMIT);
        };
        if length == 0 {
            return Ok(Ok(0));
        }
        let mut bytes = vec![0; length as usize];
        memory.read_bytes(buffer, &mut bytes).map_err(fault)?;
        // Fewer items than `count` means an error. A failed write here
        // writes none of them.
        Ok(match self.write(drive, stream, &bytes)? {
            Ok(()) => Ok(count),
            Err(errno) => Err(Failed(0, errno)),
        })
    }

    /// `fputs`. Writes the string without its terminator and returns a
    /// nonnegative value, here 0; `EOF` is -1.
    pub(super) fn put_string(
        &mut self,
        memory: &GuestMemory,
        drive: &mut ObjectState,
        text: u64,
        file: u64,
    ) -> Outcome<i32> {
        if text == 0 || file == 0 {
            return Ok(Err(Failed(-1, EINVAL)));
        }
        let stream = self.writable(file)?;
        let bytes = c_string(memory, text, MAX_TRANSFER + 1).map_err(fault)?;
        if bytes.len() as u64 > MAX_TRANSFER {
            return Err(RUNTIME_LIMIT);
        }
        Ok(match self.write(drive, stream, &bytes)? {
            Ok(()) => Ok(0),
            Err(errno) => Err(Failed(-1, errno)),
        })
    }

    /// `fflush`. A null stream means every open stream. Nothing is ever held
    /// back, so there is nothing to do.
    pub(super) fn flush(&self, file: u64) -> Result<i32, ExitReason> {
        if file != 0 {
            self.stream(file)?;
        }
        Ok(0)
    }

    /// `setvbuf`. The mode and size are checked as documented and then have
    /// no effect.
    pub(super) fn set_buffer(&self, file: u64, mode: i32, size: u64) -> Outcome<i32> {
        if file == 0 {
            return Ok(Err(Failed(-1, EINVAL)));
        }
        self.stream(file)?;
        let valid = match mode {
            // "No buffer is used, regardless of buffer or size."
            IONBF => true,
            // "Allowable range: 2 <= size <= INT_MAX".
            IOFBF | IOLBF => (2..=i32::MAX as u64).contains(&size),
            _ => false,
        };
        Ok(if valid {
            Ok(0)
        } else {
            Err(Failed(-1, EINVAL))
        })
    }

    /// `fopen`. Returns the new `FILE*`, or null with the `errno` value.
    pub(super) fn open(
        &mut self,
        memory: &mut GuestMemory,
        drive: &mut ObjectState,
        name: u64,
        mode: u64,
    ) -> Outcome<u64> {
        // A null name or mode is an invalid parameter, as an empty one is.
        if name == 0 || mode == 0 {
            return Ok(Err(Failed(0, EINVAL)));
        }
        let name = c_string(memory, name, MAX_NAME + 1).map_err(fault)?;
        let mode = c_string(memory, mode, MAX_NAME + 1).map_err(fault)?;
        if name.len() as u64 > MAX_NAME || mode.len() as u64 > MAX_NAME {
            return Ok(Err(Failed(0, EINVAL)));
        }
        Ok(self
            .opened
            .open(memory, drive, &name, &mode)
            .map_err(|errno| Failed(0, errno)))
    }

    /// `fclose`. Returns 0, or `EOF` with `EINVAL` for a null stream. Closing
    /// a standard stream is not modelled.
    pub(super) fn close(&mut self, memory: &mut GuestMemory, file: u64) -> Outcome<i32> {
        if file == 0 {
            return Ok(Err(Failed(-1, EINVAL)));
        }
        match self.stream(file)? {
            Stream::File(file) => {
                self.opened.close(memory, file);
                Ok(Ok(0))
            }
            _ => Err(RUNTIME_LIMIT),
        }
    }

    /// `fgets`. Returns `buffer`, or null at end of file or on an error.
    pub(super) fn get_line(
        &mut self,
        memory: &mut GuestMemory,
        drive: &ObjectState,
        buffer: u64,
        count: i32,
        file: u64,
    ) -> Outcome<u64> {
        if buffer == 0 || file == 0 || count <= 0 {
            return Ok(Err(Failed(0, EINVAL)));
        }
        let file = match self.stream(file)? {
            Stream::File(file) => file,
            // Reading the standard input is not modelled.
            Stream::Input => return Err(RUNTIME_LIMIT),
            // The output streams cannot be read.
            Stream::Output | Stream::Error => return Ok(Err(Failed(0, EBADF))),
        };
        // Up to count - 1 characters, and then the terminator.
        let limit = ((count - 1) as u64).min(MAX_TRANSFER) as usize;
        let mut line = match self.opened.read_line(drive, file, limit) {
            Ok(Some(line)) => line,
            // End of file with nothing read: null, and the buffer is left
            // alone.
            Ok(None) => return Ok(Ok(0)),
            Err(errno) => return Ok(Err(Failed(0, errno))),
        };
        line.push(0);
        memory.write_bytes(buffer, &line).map_err(fault)?;
        Ok(Ok(buffer))
    }

    /// `perror`: the message, a colon and a space when there is a message,
    /// then the text for `errno` and a newline, to `stderr`.
    pub(super) fn report(
        &mut self,
        memory: &GuestMemory,
        drive: &mut ObjectState,
        message: u64,
        errno: u32,
    ) -> Result<(), ExitReason> {
        let mut text = Vec::new();
        if message != 0 {
            text = c_string(memory, message, MAX_TRANSFER + 1).map_err(fault)?;
            if text.len() as u64 > MAX_TRANSFER {
                return Err(RUNTIME_LIMIT);
            }
        }
        if !text.is_empty() {
            text.extend_from_slice(b": ");
        }
        text.extend_from_slice(crt_files::error_text(errno).as_bytes());
        text.push(b'\n');
        self.write(drive, Stream::Error, &text).map(|_| ())
    }
}
