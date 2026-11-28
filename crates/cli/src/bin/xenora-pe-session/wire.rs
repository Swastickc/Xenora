// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded messages on the private supervisor and worker pipes.

use std::io::{self, Read, Write};

pub(super) const MAX_FRAMES: usize = 64;
const MAX_DIMENSION: u32 = 256;

#[derive(Clone, Copy)]
pub(super) enum Request {
    Input(u32),
    Stop,
}

pub(super) enum Reply {
    Ready,
    Frame {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Done,
}

pub(super) fn frame_len(width: u32, height: u32) -> io::Result<usize> {
    if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
        return Err(invalid("frame dimensions must be between 1 and 256"));
    }
    Ok((width * height * 4) as usize)
}

pub(super) fn read_request(mut reader: impl Read) -> io::Result<Request> {
    match read_tag(&mut reader)? {
        Some(b'I') => Ok(Request::Input(read_u32(&mut reader)?)),
        Some(b'Q') => Ok(Request::Stop),
        Some(_) => Err(invalid("invalid session request tag")),
        None => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the supervisor closed the request channel",
        )),
    }
}

pub(super) fn write_request(mut writer: impl Write, request: Request) -> io::Result<()> {
    match request {
        Request::Input(value) => {
            writer.write_all(b"I")?;
            writer.write_all(&value.to_le_bytes())?;
        }
        Request::Stop => writer.write_all(b"Q")?,
    }
    writer.flush()
}

pub(super) fn read_reply(mut reader: impl Read) -> io::Result<Option<Reply>> {
    let reply = match read_tag(&mut reader)? {
        Some(b'R') => Reply::Ready,
        Some(b'F') => {
            let width = read_u32(&mut reader)?;
            let height = read_u32(&mut reader)?;
            let mut rgba = vec![0; frame_len(width, height)?];
            reader.read_exact(&mut rgba)?;
            Reply::Frame {
                width,
                height,
                rgba,
            }
        }
        Some(b'D') => Reply::Done,
        Some(_) => return Err(invalid("invalid session reply tag")),
        None => return Ok(None),
    };
    Ok(Some(reply))
}

pub(super) fn write_reply(mut writer: impl Write, reply: &Reply) -> io::Result<()> {
    match reply {
        Reply::Ready => writer.write_all(b"R")?,
        Reply::Frame {
            width,
            height,
            rgba,
        } => {
            if rgba.len() != frame_len(*width, *height)? {
                return Err(invalid("invalid RGBA byte count"));
            }
            writer.write_all(b"F")?;
            writer.write_all(&width.to_le_bytes())?;
            writer.write_all(&height.to_le_bytes())?;
            writer.write_all(rgba)?;
        }
        Reply::Done => writer.write_all(b"D")?,
    }
    writer.flush()
}

fn read_tag(mut reader: impl Read) -> io::Result<Option<u8>> {
    let mut tag = [0];
    match reader.read_exact(&mut tag) {
        Ok(()) => Ok(Some(tag[0])),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_u32(mut reader: impl Read) -> io::Result<u32> {
    let mut value = [0; 4];
    reader.read_exact(&mut value)?;
    Ok(u32::from_le_bytes(value))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
