// SPDX-License-Identifier: GPL-3.0-or-later

//! The formatted-input engine of the CRT `scanf` family.
//!
//! Documented: the conversion syntax, the return count and the `EOF` rule
//! are Microsoft's `scanf` and `sscanf` pages. The sign and base rules of
//! the integer conversions are the C standard ones the pages defer to.
//!
//! Xenora's limits, honestly stopped rather than misread: the floating
//! conversions (`%e`, `%f`, `%g` and kin) are not implemented, as their
//! rounding contract is a document of its own; nor are the wide forms. A
//! format that uses one stops the guest. Every other conversion the narrow
//! family documents is here: the integers with their length modifiers, the
//! characters, the strings, the scansets, `%n`, `%%`, the field width and
//! assignment suppression.

use super::crt_streams::c_string;
use crate::cpu::ExitReason;
use crate::memory::{GuestMemory, MemoryError};

pub(super) const MAX_SCAN_INPUT: u64 = 1 << 20;

/// How a scan the engine does not model stops the guest, as every service
/// does: the runtime-limit fault, never a silent misread.
const RUNTIME_LIMIT: ExitReason = ExitReason::IllegalInstruction(0xFE);

fn fault(error: MemoryError) -> ExitReason {
    ExitReason::MemoryFault(error)
}

/// Why the scan stops the guest rather than answer.
#[derive(Debug)]
pub(super) enum ScanError {
    /// The format uses a conversion or modifier the engine does not model.
    Unsupported,
    /// Guest memory could not be read or written.
    Memory(MemoryError),
}

/// The input a scan consumes, one byte at a time, with a one-byte pushback
/// so a conversion can un-read the byte that ended it, as the CRT does.
struct Input {
    bytes: Vec<u8>,
    at: usize,
}

impl Input {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.at += 1;
        Some(byte)
    }

    fn back(&mut self) {
        self.at = self.at.saturating_sub(1);
    }
}

/// How far an integer conversion writes: the length modifiers of the
/// family, with the plain `int` the default.
#[derive(Clone, Copy, PartialEq)]
enum Length {
    Char,
    Short,
    Int,
    Long,
    LongLong,
}

/// One parsed conversion specification.
struct Conversion {
    suppress: bool,
    width: Option<usize>,
    length: Length,
    kind: u8,
}

fn whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)
}

/// Parse one conversion specification after the `%` of `format` at `at`.
/// Returns the specification and the index just past it.
fn parse_conversion(format: &[u8], mut at: usize) -> Result<(Conversion, usize), ScanError> {
    let suppress = format.get(at) == Some(&b'*');
    if suppress {
        at += 1;
    }
    let mut width: Option<usize> = None;
    while let Some(digit) = format.get(at).filter(|byte| byte.is_ascii_digit()) {
        width = Some(width.unwrap_or(0) * 10 + usize::from(digit - b'0'));
        at += 1;
    }
    let mut length = Length::Int;
    if format.get(at) == Some(&b'h') {
        at += 1;
        if format.get(at) == Some(&b'h') {
            length = Length::Char;
            at += 1;
        } else {
            length = Length::Short;
        }
    } else if format.get(at) == Some(&b'l') {
        at += 1;
        if format.get(at) == Some(&b'l') {
            length = Length::LongLong;
            at += 1;
        } else {
            length = Length::Long;
        }
    } else if matches!(format.get(at), Some(&b'j') | Some(&b'z') | Some(&b't')) {
        // intmax_t, size_t and ptrdiff_t are all 64-bit here, as they are
        // on any Win64 CRT.
        length = Length::LongLong;
        at += 1;
    } else if format.get(at) == Some(&b'L') {
        // long double, meaningful only for the unimplemented floating
        // conversions — parse it so the error names the conversion.
        at += 1;
    }
    let Some(&kind) = format.get(at) else {
        return Err(ScanError::Unsupported);
    };
    Ok((
        Conversion {
            suppress,
            width,
            length,
            kind,
        },
        at + 1,
    ))
}

/// Write an integer conversion's value through the pointer the va_list
/// holds, sized as the length modifier says. Win64 longs are 32 bits.
fn write_integer(
    memory: &mut GuestMemory,
    pointer: u64,
    length: Length,
    value: u64,
) -> Result<(), ScanError> {
    let result = match length {
        Length::Char => memory.write_u8(pointer, value as u8),
        Length::Short => memory.write_u16(pointer, value as u16),
        // int and long are both 32-bit on Win64.
        Length::Int | Length::Long => memory.write_u32(pointer, value as u32),
        Length::LongLong => memory.write_u64(pointer, value),
    };
    result.map_err(ScanError::Memory)
}

/// The digits one integer conversion consumes: sign, base prefix and the
/// digits themselves, width-bounded. Returns the value zero-extended, the
/// count consumed and whether the digits ran at all — a bare sign or a
/// bare prefix is not a number.
fn scan_integer(input: &mut Input, width: Option<usize>, base_rule: u8) -> Option<(u64, bool)> {
    let mut consumed = 0usize;
    let room = width.unwrap_or(usize::MAX);
    let mut negative = false;
    let mut value: u64 = 0;
    let mut digits = 0usize;
    if matches!(input.peek(), Some(b'+') | Some(b'-')) && consumed < room {
        negative = input.peek() == Some(b'-');
        input.take();
        consumed += 1;
    }
    let mut base = match base_rule {
        b'x' | b'X' => 16,
        b'o' => 8,
        b'u' | b'd' => 10,
        // %i reads the base from the input itself.
        _ => 0,
    };
    let mut prefixed = false;
    if (base == 0 || base == 16) && input.peek() == Some(b'0') {
        // Peek deeper: 0x or 0X starts a hexadecimal number for %i and %x;
        // a lone 0 is already a valid zero in any base.
        input.take();
        consumed += 1;
        digits += 1;
        value = 0;
        if consumed < room && matches!(input.peek(), Some(b'x') | Some(b'X')) {
            input.take();
            consumed += 1;
            base = 16;
            prefixed = true;
            // The digits so far are the prefix, not the number: a bare
            // "0x" still yields the zero the standard promises.
            digits = 0;
        } else if base == 0 {
            base = 8;
        }
    }
    if base == 0 {
        base = 10;
    }
    loop {
        if consumed >= room {
            break;
        }
        let Some(byte) = input.peek() else { break };
        let digit = match byte {
            b'0'..=b'9' => u64::from(byte - b'0'),
            b'a'..=b'f' => u64::from(byte - b'a') + 10,
            b'A'..=b'F' => u64::from(byte - b'A') + 10,
            _ => break,
        };
        if digit >= base {
            break;
        }
        input.take();
        consumed += 1;
        digits += 1;
        value = value.wrapping_mul(base).wrapping_add(digit);
    }
    if digits == 0 && !prefixed {
        // No digit ran at all: the conversion fails and the input is left
        // where it started, as the CRT leaves it for the next directive.
        for _ in 0..consumed {
            input.back();
        }
        return None;
    }
    let value = if negative {
        value.wrapping_neg()
    } else {
        value
    };
    Some((value, true))
}

/// Run the scan of `format` against `input`, writing conversions through
/// the argument pointers `slots` yields in order. Returns the assignment
/// count, or -1 when input ended before the first conversion completed.
pub(super) fn scan(
    memory: &mut GuestMemory,
    input_bytes: Vec<u8>,
    format: &[u8],
    mut slot: impl FnMut() -> Result<u64, MemoryError>,
) -> Result<i32, ScanError> {
    let mut input = Input {
        bytes: input_bytes,
        at: 0,
    };
    let mut assigned = 0i32;
    let mut matched = false;
    let mut at = 0usize;
    while at < format.len() {
        let byte = format[at];
        if whitespace(byte) {
            // Whitespace in the format skips whitespace in the input,
            // including none at all.
            while input.peek().is_some_and(whitespace) {
                input.take();
            }
            at += 1;
            continue;
        }
        if byte != b'%' {
            if input.peek() != Some(byte) {
                break;
            }
            input.take();
            at += 1;
            continue;
        }
        if format.get(at + 1) == Some(&b'%') {
            if input.peek() != Some(b'%') {
                break;
            }
            input.take();
            at += 2;
            continue;
        }
        let (conversion, next) = parse_conversion(format, at + 1)?;
        at = next;
        let width = conversion.width;
        match conversion.kind {
            kind @ (b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'p') => {
                while input.peek().is_some_and(whitespace) {
                    input.take();
                }
                let rule = if kind == b'p' { b'x' } else { kind };
                let Some((value, _)) = scan_integer(&mut input, width, rule) else {
                    break;
                };
                if input.at >= input.bytes.len() && !matched && assigned == 0 {
                    // Reaching the end exactly can still be a success; the
                    // EOF rule is only for a conversion that cannot start.
                }
                if !conversion.suppress {
                    let pointer = slot().map_err(ScanError::Memory)?;
                    write_integer(memory, pointer, conversion.length, value)?;
                    assigned += 1;
                }
                matched = true;
            }
            b'c' => {
                let count = width.unwrap_or(1);
                let mut taken = 0usize;
                if !conversion.suppress {
                    let pointer = slot().map_err(ScanError::Memory)?;
                    for index in 0..count {
                        let Some(unit) = input.take() else { break };
                        memory
                            .write_u8(pointer + index as u64, unit)
                            .map_err(ScanError::Memory)?;
                        taken += 1;
                    }
                    if taken == 0 {
                        break;
                    }
                    assigned += 1;
                } else {
                    for _ in 0..count {
                        if input.take().is_none() {
                            break;
                        }
                        taken += 1;
                    }
                    if taken == 0 {
                        break;
                    }
                }
                matched = true;
            }
            b's' => {
                while input.peek().is_some_and(whitespace) {
                    input.take();
                }
                let mut text = Vec::new();
                let room = width.unwrap_or(usize::MAX);
                while text.len() < room {
                    let Some(unit) = input.peek() else { break };
                    if whitespace(unit) {
                        break;
                    }
                    input.take();
                    text.push(unit);
                }
                if text.is_empty() {
                    break;
                }
                if !conversion.suppress {
                    let pointer = slot().map_err(ScanError::Memory)?;
                    for (index, unit) in text.iter().enumerate() {
                        memory
                            .write_u8(pointer + index as u64, *unit)
                            .map_err(ScanError::Memory)?;
                    }
                    memory
                        .write_u8(pointer + text.len() as u64, 0)
                        .map_err(ScanError::Memory)?;
                    assigned += 1;
                }
                matched = true;
            }
            b'[' => {
                // The scanset: `%[...]` with an immediate `^` inverting it,
                // and `]` a member when it comes first.
                let mut members = [false; 256];
                let mut index = at;
                let invert = format.get(index) == Some(&b'^');
                if invert {
                    index += 1;
                }
                let mut first = true;
                let mut end = index;
                while let Some(&unit) = format.get(end) {
                    if unit == b']' && !first {
                        break;
                    }
                    first = false;
                    end += 1;
                }
                if format.get(end) != Some(&b']') {
                    return Err(ScanError::Unsupported);
                }
                let mut run = index;
                while run < end {
                    let unit = format[run];
                    if unit == b'-' && run > index && run + 1 < end {
                        let start = format[run - 1];
                        let stop = format[run + 1];
                        if start <= stop {
                            for member in start..=stop {
                                members[usize::from(member)] = true;
                            }
                        }
                        run += 1;
                    } else {
                        members[usize::from(unit)] = true;
                    }
                    run += 1;
                }
                at = end + 1;
                let mut text = Vec::new();
                let room = width.unwrap_or(usize::MAX);
                while text.len() < room {
                    let Some(unit) = input.peek() else { break };
                    let inside = members[usize::from(unit)];
                    if inside == invert {
                        break;
                    }
                    input.take();
                    text.push(unit);
                }
                if text.is_empty() {
                    break;
                }
                if !conversion.suppress {
                    let pointer = slot().map_err(ScanError::Memory)?;
                    for (index, unit) in text.iter().enumerate() {
                        memory
                            .write_u8(pointer + index as u64, *unit)
                            .map_err(ScanError::Memory)?;
                    }
                    memory
                        .write_u8(pointer + text.len() as u64, 0)
                        .map_err(ScanError::Memory)?;
                    assigned += 1;
                }
                matched = true;
            }
            b'n' => {
                if !conversion.suppress {
                    let pointer = slot().map_err(ScanError::Memory)?;
                    write_integer(memory, pointer, conversion.length, input.at as u64)?;
                    // %n is not counted, as the documentation says.
                }
            }
            b'e' | b'f' | b'g' | b'E' | b'F' | b'G' | b'a' | b'A' => {
                return Err(ScanError::Unsupported);
            }
            _ => return Err(ScanError::Unsupported),
        }
    }
    if assigned == 0 && !matched && input.bytes.is_empty() {
        return Ok(-1);
    }
    if assigned == 0 && !matched {
        // Distinguish end-of-input from a mismatch: the EOF rule applies
        // only when the input itself ran out before any conversion.
        if input.at >= input.bytes.len() {
            return Ok(-1);
        }
        return Ok(0);
    }
    Ok(assigned)
}

/// How many argument pointers a format consumes: every conversion that is
/// not suppressed counts one, `%%` none. Used to read the variadic slots
/// before the scan borrows memory for its own writes.
pub(super) fn count_assignments(format: &[u8]) -> usize {
    let mut count = 0usize;
    let mut at = 0usize;
    while at < format.len() {
        if format[at] != b'%' {
            at += 1;
            continue;
        }
        if format.get(at + 1) == Some(&b'%') {
            at += 2;
            continue;
        }
        let mut index = at + 1;
        let suppress = format.get(index) == Some(&b'*');
        while index < format.len()
            && !matches!(
                format[index],
                b'c' | b'd'
                    | b'i'
                    | b'u'
                    | b'o'
                    | b'x'
                    | b'X'
                    | b'p'
                    | b's'
                    | b'n'
                    | b'['
                    | b'e'
                    | b'f'
                    | b'g'
                    | b'E'
                    | b'F'
                    | b'G'
                    | b'a'
                    | b'A'
            )
        {
            index += 1;
        }
        if index < format.len() && !suppress {
            count += 1;
        }
        at = index + 1;
    }
    count
}

/// `__stdio_common_vsscanf` (#605): the scan of the buffer through the
/// va_list the caller passes. The options of the modern scan family are the
/// wide-specifier and exponent ones the printf family shares; no locale is
/// modelled, as with the output forms.
pub(super) fn vsscanf(
    memory: &mut GuestMemory,
    options: u64,
    buffer: u64,
    count: u64,
    format: u64,
    locale: u64,
    arguments: u64,
) -> Result<i32, ExitReason> {
    // The behaviour bits of the scan family, from the mingw-w64
    // corecrt_stdio_config.h on the host: secure CRT 0x01, legacy wide
    // specifiers 0x02 and legacy msvcrt compatibility 0x04. The distinctions
    // they select are not modelled yet; a null buffer or format is the
    // invalid call the CRT refuses.
    const KNOWN_SCAN_OPTIONS: u64 = 0x07;
    if options & !KNOWN_SCAN_OPTIONS != 0 || locale != 0 {
        return Err(RUNTIME_LIMIT);
    }
    if buffer == 0 || format == 0 {
        return Ok(-1);
    }
    // The count bounds what of the buffer the scan may read: the string
    // forms pass -1 for an unbounded, terminator-ended input, and a
    // bounded count reads at most that many bytes, terminator or not.
    let input = if count == u64::MAX {
        scan_input_string(memory, buffer)?
    } else {
        let mut bytes = Vec::new();
        for offset in 0..count.min(MAX_SCAN_INPUT) {
            match memory.read_u8(buffer + offset).map_err(fault)? {
                0 => break,
                byte => bytes.push(byte),
            }
        }
        bytes
    };
    let template = scan_input_string(memory, format)?;
    let needed = count_assignments(&template);
    let mut slots = Vec::with_capacity(needed);
    for index in 0..needed {
        slots.push(
            memory
                .read_u64(arguments + 8 * index as u64)
                .map_err(fault)?,
        );
    }
    let mut next_slot = slots.iter();
    match scan(memory, input, &template, || {
        Ok(*next_slot.next().unwrap_or(&0))
    }) {
        Ok(count) => Ok(count),
        Err(ScanError::Unsupported) => Err(RUNTIME_LIMIT),
        Err(ScanError::Memory(error)) => Err(fault(error)),
    }
}

/// Read the narrow string the scan formats from, bounded as any transfer.
pub(super) fn scan_input_string(memory: &GuestMemory, address: u64) -> Result<Vec<u8>, ExitReason> {
    c_string(memory, address, MAX_SCAN_INPUT + 1).map_err(fault)
}
