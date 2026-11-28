// SPDX-License-Identifier: GPL-3.0-or-later

//! Keep potentially blocking writes to a host pipe off the supervisor thread.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::os::fd::AsFd;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Instant;

use sha2::{Digest, Sha256};

use super::supervisor::Failure;

pub(super) enum Report {
    Ready,
    Frame {
        sequence: usize,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        pixels: bool,
    },
    Done(usize),
}

pub(super) struct HostOutput {
    reports: SyncSender<Report>,
    written: Receiver<io::Result<()>>,
}

impl HostOutput {
    pub(super) fn start() -> io::Result<Self> {
        // Use an owned duplicate so a stalled write holds no global stdout lock.
        let output = File::from(io::stdout().as_fd().try_clone_to_owned()?);
        let (reports, pending) = sync_channel(1);
        let (completed, written) = sync_channel(1);
        std::thread::Builder::new().spawn(move || {
            let mut output = BufWriter::new(output);
            for report in pending {
                let result = write_report(&mut output, report).and_then(|()| output.flush());
                let failed = result.is_err();
                if completed.send(result).is_err() || failed {
                    break;
                }
            }
        })?;
        Ok(Self { reports, written })
    }

    pub(super) fn publish(
        &self,
        report: Report,
        deadline: Instant,
        phase: &'static str,
    ) -> Result<(), Failure> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(Failure::Deadline(phase))?;
        self.reports
            .try_send(report)
            .map_err(|_| Failure::Session("host output channel closed".into()))?;
        match self.written.recv_timeout(remaining) {
            Ok(result) => Ok(result?),
            Err(RecvTimeoutError::Timeout) => Err(Failure::Deadline(phase)),
            Err(RecvTimeoutError::Disconnected) => {
                Err(Failure::Session("host output channel closed".into()))
            }
        }
    }
}

fn write_report(output: &mut impl Write, report: Report) -> io::Result<()> {
    match report {
        Report::Ready => writeln!(output, "ready"),
        Report::Done(frames) => writeln!(output, "done frames={frames}"),
        Report::Frame {
            sequence,
            width,
            height,
            rgba,
            pixels,
        } => {
            let digest = Sha256::digest(&rgba);
            write!(
                output,
                "frame {sequence} width={width} height={height} sha256={digest:x}"
            )?;
            if pixels {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                output.write_all(b" rgba=")?;
                for byte in rgba {
                    output
                        .write_all(&[HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 15)]])?;
                }
            }
            writeln!(output)
        }
    }
}

/// A closed pipe can be detected on Darwin even when no frame is being sent.
pub(super) fn check_connection() -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::{c_int, c_short, c_uint};
        use std::os::fd::AsRawFd;

        // Layout and flags from Darwin <sys/poll.h>. POLLOUT is required for a
        // pipe's write end to report loss of its reader.
        const POLLOUT: c_short = 0x04;
        const POLLERR: c_short = 0x08;
        const POLLHUP: c_short = 0x10;
        const POLLNVAL: c_short = 0x20;

        #[repr(C)]
        struct PollFd {
            fd: c_int,
            events: c_short,
            revents: c_short,
        }
        unsafe extern "C" {
            fn poll(fds: *mut PollFd, count: c_uint, timeout_ms: c_int) -> c_int;
        }

        let mut output = PollFd {
            fd: io::stdout().as_raw_fd(),
            events: POLLOUT,
            revents: 0,
        };
        // SAFETY: output is one writable pollfd and a zero timeout never waits.
        let result = unsafe { poll(&mut output, 1, 0) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        } else if (output.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "host output disconnected",
            ));
        }
    }
    Ok(())
}
