// SPDX-License-Identifier: GPL-3.0-or-later

//! Process-local Winsock UDP loopback for the synthetic guest.

use super::{ERROR_INVALID_PARAMETER, ServicesState};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::GuestMemory;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MODULE: &str = "ws2_32.dll";
const VERSION: u16 = 0x0202;
const WSADATA_SIZE: usize = 408;
const MAX_DATAGRAM: usize = 65_507;
const MAX_PENDING_DATAGRAMS: usize = 16;
const STREAM_WINDOW: usize = 65_536;
const MAX_WSABUF_COUNT: u32 = 64;
const WSABUF_SIZE: u64 = 16;
const MAX_SOCKETS: usize = 64;
const SOCKET_ERROR: u64 = u32::MAX as u64;
const INVALID_SOCKET: u64 = u64::MAX;
const WSAEACCES: u32 = 10013;
const WSAEINVAL: u32 = 10022;
const WSAEWOULDBLOCK: u32 = 10035;
const WSAEMSGSIZE: u32 = 10040;
const WSAEADDRINUSE: u32 = 10048;
const WSAENOBUFS: u32 = 10055;
const WSAENOTCONN: u32 = 10057;
const WSAEFAULT: u32 = 10014;
const WSAEMFILE: u32 = 10024;
const WSAENOTSOCK: u32 = 10038;
const WSAEPROTONOSUPPORT: u32 = 10043;
const WSAESOCKTNOSUPPORT: u32 = 10044;
const WSAEOPNOTSUPP: u32 = 10045;
const WSAENOPROTOOPT: u32 = 10042;
const WSAESHUTDOWN: u32 = 10058;
const WSAECONNREFUSED: u32 = 10061;
const WSAEAFNOSUPPORT: u32 = 10047;
const WSASYSNOTREADY: u32 = 10091;
const WSAVERNOTSUPPORTED: u32 = 10092;
const WSANOTINITIALISED: u32 = 10093;

#[derive(Default)]
pub(super) struct Winsock {
    refs: u32,
    serial: u32,
    sockets: BTreeMap<u64, Socket>,
    #[cfg(test)]
    poll_waiters: usize,
}

#[derive(Default)]
struct Socket {
    port: Option<u16>,
    peer: Option<u16>,
    pending: VecDeque<(u16, Vec<u8>)>,
    shutdown_receive: bool,
    shutdown_send: bool,
    reuse_address: bool,
    broadcast: bool,
    /// True for a byte-stream (TCP) socket rather than a datagram one.
    stream: bool,
    /// Backlog limit while listening, and the server-side handles waiting
    /// for accept - in connect order, exactly the queue semantics.
    listening: Option<usize>,
    backlog: VecDeque<u64>,
    /// The paired endpoint of a connected stream, its byte fifo, and the
    /// orderly-shutdown flag the peer's close or shutdown raises.
    stream_peer: Option<u64>,
    stream_read: VecDeque<u8>,
    stream_eof: bool,
    /// WSAEventSelect association: the event handle and its mask, plus
    /// the occurred-event bits EnumNetworkEvents reports and clears.
    event_select: Option<(u64, u32)>,
    events_occurred: u32,
}

pub(super) const FD_READ: u32 = 0x01;
pub(super) const FD_WRITE: u32 = 0x02;
pub(super) const FD_ACCEPT: u32 = 0x08;
pub(super) const FD_CONNECT: u32 = 0x10;
pub(super) const FD_CLOSE: u32 = 0x20;

/// Record an occurred network event on the socket and signal the
/// associated event object - the edge the guest waits on.
fn fire_event(state: &Arc<ServicesState>, net: &mut Winsock, socket: u64, bit: u32) {
    use super::GuestObject;
    use super::events::Waitable;
    let Some(endpoint) = net.sockets.get_mut(&socket) else {
        return;
    };
    let Some((event, mask)) = endpoint.event_select else {
        return;
    };
    if mask & bit == 0 {
        return;
    }
    endpoint.events_occurred |= bit;
    let objects = state.objects.lock().expect("guest object state");
    if let Some(GuestObject::Waitable {
        state: waitable, ..
    }) = objects.handles.get(&event)
        && let Ok(mut inner) = waitable.lock()
        && let Waitable::Event { signaled, .. } = &mut *inner
    {
        *signaled = true;
    }
    drop(objects);
    state.hooks.signal();
}

impl Winsock {
    /// Sockets the guest has open.
    pub(super) fn open_count(&self) -> usize {
        self.sockets.len()
    }

    /// Process exit: every socket is closed.
    pub(super) fn close_all(&mut self) {
        self.sockets.clear();
    }

    fn socket(&self, handle: u64) -> Result<&Socket, u32> {
        if self.refs == 0 {
            return Err(WSANOTINITIALISED);
        }
        self.sockets.get(&handle).ok_or(WSAENOTSOCK)
    }

    fn port(&self, requested: u16) -> Result<u16, u32> {
        let available = |port| {
            self.sockets
                .values()
                .all(|socket| socket.port != Some(port))
        };
        if requested != 0 {
            return available(requested)
                .then_some(requested)
                .ok_or(WSAEADDRINUSE);
        }
        (49152..=65535)
            .find(|&port| available(port))
            .ok_or(WSAEADDRINUSE)
    }
}

fn loopback_port(memory: &GuestMemory, address: u64, size: u32) -> Result<u16, u32> {
    let mut data = [0; 16];
    if (size as i32) < 16 || address == 0 || memory.read_bytes(address, &mut data).is_err() {
        return Err(WSAEFAULT);
    }
    if u16::from_le_bytes([data[0], data[1]]) != 2 {
        return Err(WSAEAFNOSUPPORT);
    }
    if data[4..8] != [127, 0, 0, 1] {
        return Err(WSAEACCES);
    }
    Ok(u16::from_be_bytes([data[2], data[3]]))
}

fn queue_loopback_datagram(
    state: &Arc<ServicesState>,
    net: &mut Winsock,
    source: u16,
    peer: u16,
    data: Vec<u8>,
) -> Result<(), u32> {
    let target_handle = net.sockets.iter().find_map(|(handle, socket)| {
        (socket.port == Some(peer) && socket.peer.is_none_or(|port| port == source))
            .then_some(*handle)
    });
    if let Some(handle) = target_handle {
        let target = net.sockets.get_mut(&handle).expect("target socket");
        if target.pending.len() == MAX_PENDING_DATAGRAMS {
            return Err(WSAENOBUFS);
        }
        target.pending.push_back((source, data));
        fire_event(state, net, handle, FD_READ);
        state.winsock_changed.notify_all();
    }
    Ok(())
}

fn write_loopback_source(
    memory: &mut GuestMemory,
    source_address: u64,
    source_length: u64,
    source: u16,
) -> Result<(), u32> {
    if source_address == 0 {
        return Ok(());
    }
    if source_length == 0 {
        return Err(WSAEFAULT);
    }
    let capacity = memory.read_u32(source_length).map_err(|_| WSAEFAULT)? as i32;
    if capacity < 16 {
        return Err(WSAEFAULT);
    }
    let mut address = [0; 16];
    address[..2].copy_from_slice(&2_u16.to_le_bytes());
    address[2..4].copy_from_slice(&source.to_be_bytes());
    address[4..8].copy_from_slice(&[127, 0, 0, 1]);
    memory
        .write_bytes(source_address, &address)
        .map_err(|_| WSAEFAULT)?;
    memory.write_u32(source_length, 16).map_err(|_| WSAEFAULT)?;
    Ok(())
}

fn read_wsabufs(
    memory: &GuestMemory,
    address: u64,
    count: u32,
) -> Result<(Vec<(u64, u32)>, u64), u32> {
    if address == 0 {
        return Err(WSAEFAULT);
    }
    if count == 0 || count > MAX_WSABUF_COUNT {
        return Err(WSAEINVAL);
    }
    let mut buffers = Vec::with_capacity(count as usize);
    let mut total = 0_u64;
    for index in 0..u64::from(count) {
        let base = address.checked_add(index * WSABUF_SIZE).ok_or(WSAEFAULT)?;
        let length = memory.read_u32(base).map_err(|_| WSAEFAULT)?;
        let pointer = memory.read_u64(base + 8).map_err(|_| WSAEFAULT)?;
        if length != 0 && pointer == 0 {
            return Err(WSAEFAULT);
        }
        total = total.checked_add(u64::from(length)).ok_or(WSAEINVAL)?;
        buffers.push((pointer, length));
    }
    Ok((buffers, total))
}

fn gather_wsabufs(
    memory: &GuestMemory,
    buffers: &[(u64, u32)],
    limit: usize,
) -> Result<Vec<u8>, u32> {
    let mut gathered = Vec::with_capacity(limit);
    for &(pointer, length) in buffers {
        let remaining = limit - gathered.len();
        if remaining == 0 {
            break;
        }
        let count = (length as usize).min(remaining);
        if count == 0 {
            continue;
        }
        let start = gathered.len();
        gathered.resize(start + count, 0);
        memory
            .read_bytes(pointer, &mut gathered[start..])
            .map_err(|_| WSAEFAULT)?;
    }
    Ok(gathered)
}

fn scatter_wsabufs(
    memory: &mut GuestMemory,
    buffers: &[(u64, u32)],
    data: &[u8],
) -> Result<(), u32> {
    let mut offset = 0;
    for &(pointer, length) in buffers {
        if offset == data.len() {
            break;
        }
        let count = (length as usize).min(data.len() - offset);
        if count == 0 {
            continue;
        }
        memory
            .write_bytes(pointer, &data[offset..offset + count])
            .map_err(|_| WSAEFAULT)?;
        offset += count;
    }
    if offset == data.len() {
        Ok(())
    } else {
        Err(WSAEFAULT)
    }
}

fn socket_result(
    state: &ServicesState,
    teb: u64,
    result: Result<u64, u32>,
) -> Result<u64, ExitReason> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => state.fail(teb, error, SOCKET_ERROR),
    }
}

fn create_loopback_socket(
    net: &mut Winsock,
    af: u64,
    kind: u64,
    protocol: u64,
) -> Result<u64, u32> {
    if net.refs == 0 {
        return Err(WSANOTINITIALISED);
    }
    if af as u32 != 2 {
        return Err(WSAEAFNOSUPPORT);
    }
    let stream = match (kind as u32, protocol as u32) {
        (2, 0 | 17) => false,
        (1, 0 | 6) => true,
        (1, _) | (2, _) => return Err(WSAEPROTONOSUPPORT),
        _ => return Err(WSAESOCKTNOSUPPORT),
    };
    if net.sockets.len() >= MAX_SOCKETS {
        return Err(WSAEMFILE);
    }
    let Some(next) = net.serial.checked_add(1) else {
        return Err(WSAEMFILE);
    };
    net.serial = next;
    let socket = 0x5300_0000_0000_0000 | u64::from(next);
    net.sockets.insert(
        socket,
        Socket {
            stream,
            ..Socket::default()
        },
    );
    Ok(socket)
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let startup = Arc::clone(state);
    registry.register(MODULE, "WSAStartup", move |args, _, memory| {
        let requested = args.rcx as u16;
        let negotiated = match requested.to_le_bytes() {
            [2, minor] => u16::from_le_bytes([2, minor.min(2)]),
            _ => return Ok(u64::from(WSAVERNOTSUPPORTED)),
        };
        let mut net = startup.winsock.lock().expect("guest Winsock state");
        let Some(next) = net.refs.checked_add(1) else {
            return Ok(u64::from(WSASYSNOTREADY));
        };
        let mut data = [0; WSADATA_SIZE];
        data[..2].copy_from_slice(&negotiated.to_le_bytes());
        data[2..4].copy_from_slice(&VERSION.to_le_bytes());
        let description = b"Xenora synthetic Winsock";
        data[16..16 + description.len()].copy_from_slice(description);
        if args.rdx == 0 || memory.write_bytes(args.rdx, &data).is_err() {
            return Ok(u64::from(WSAEFAULT));
        }
        net.refs = next;
        Ok(0)
    });

    let cleanup = Arc::clone(state);
    registry.register(MODULE, "WSACleanup", move |_, cpu, _| {
        let mut net = cleanup.winsock.lock().expect("guest Winsock state");
        if net.refs == 0 {
            return cleanup.fail(cpu.gs_base, WSANOTINITIALISED, SOCKET_ERROR);
        }
        net.refs -= 1;
        if net.refs == 0 {
            net.sockets.clear();
            cleanup.winsock_changed.notify_all();
        }
        Ok(0)
    });

    let open = Arc::clone(state);
    registry.register(MODULE, "socket", move |args, cpu, _| {
        let mut net = open.winsock.lock().expect("guest Winsock state");
        match create_loopback_socket(&mut net, args.rcx, args.rdx, args.r8) {
            Ok(socket) => Ok(socket),
            Err(error) => open.fail(cpu.gs_base, error, INVALID_SOCKET),
        }
    });

    let close = Arc::clone(state);
    registry.register(MODULE, "closesocket", move |args, cpu, _| {
        let mut net = close.winsock.lock().expect("guest Winsock state");
        if net.refs == 0 {
            return close.fail(cpu.gs_base, WSANOTINITIALISED, SOCKET_ERROR);
        }
        let Some(removed) = net.sockets.remove(&args.rcx) else {
            return close.fail(cpu.gs_base, WSAENOTSOCK, SOCKET_ERROR);
        };
        if let Some(peer) = removed.stream_peer
            && let Some(endpoint) = net.sockets.get_mut(&peer)
        {
            endpoint.stream_eof = true;
            endpoint.stream_peer = None;
        }
        if let Some(peer) = removed.stream_peer {
            fire_event(&close, &mut net, peer, FD_CLOSE);
        }
        close.winsock_changed.notify_all();
        Ok(0)
    });

    register_address_conversion(registry, state);
    register_loopback(registry, state);
    register_wsa_io(registry, state);
    register_socket_names(registry, state);
    register_service_lookup(registry, state);
    register_socket_options(registry, state);
    register_event_select(registry, state);
    register_ioctl_and_reverse(registry, state);
    register_streams(registry, state);
    register_datagrams(registry, state);
    register_host_identity(registry, state);
    register_lifecycle(registry, state);
    register_adapters(registry, state);
    register_resolution(registry, state);
    register_select(registry, state);
    register_poll(registry, state);

    let get = Arc::clone(state);
    registry.register(MODULE, "WSAGetLastError", move |_, cpu, _| {
        Ok(u64::from(get.last_error(cpu.gs_base)))
    });
    let set = Arc::clone(state);
    registry.register(MODULE, "WSASetLastError", move |args, cpu, _| {
        set.set_last_error(cpu.gs_base, args.rcx as u32);
        Ok(0)
    });
    // Published desktop AMD64 export tables agree on these ordinal slots.
    // They are aliases of the same named implementations, not separate ABI
    // surfaces; ordinals without a named implementation stay unresolved.
    for (symbol, ordinal) in [
        ("accept", 1),
        ("bind", 2),
        ("closesocket", 3),
        ("connect", 4),
        ("getpeername", 5),
        ("getsockname", 6),
        ("getsockopt", 7),
        ("htonl", 8),
        ("htons", 9),
        ("ioctlsocket", 10),
        ("inet_addr", 11),
        ("inet_ntoa", 12),
        ("listen", 13),
        ("ntohl", 14),
        ("ntohs", 15),
        ("recv", 16),
        ("recvfrom", 17),
        ("select", 18),
        ("send", 19),
        ("sendto", 20),
        ("setsockopt", 21),
        ("shutdown", 22),
        ("socket", 23),
        ("gethostbyname", 52),
        ("gethostbyaddr", 51),
        ("gethostname", 57),
        ("WSAGetLastError", 111),
        ("WSASetLastError", 112),
        ("WSAStartup", 115),
        ("WSACleanup", 116),
        ("__WSAFDIsSet", 151),
    ] {
        registry
            .try_ordinal_alias(MODULE, symbol, ordinal)
            .expect("documented Winsock ordinal alias");
    }
    // The higher WSA gather/scatter ordinals are deliberately absent:
    // published desktop assignments for those slots vary by Windows version,
    // and there is no public Xbox-specific ordinal evidence to choose one.
    for symbol in ["WSAStartup", "WSACleanup", "socket", "closesocket"] {
        registry.set_status(MODULE, symbol, ServiceStatus::Partial);
    }
}

fn register_wsa_byteorder(
    registry: &mut ImportRegistry,
    state: &Arc<ServicesState>,
    symbol: &'static str,
    wide: bool,
) {
    let state = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        symbol,
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                state
                    .winsock
                    .lock()
                    .expect("guest Winsock state")
                    .socket(args.rcx)?;
                if args.r8 == 0 {
                    return Err(WSAEFAULT);
                }
                if wide {
                    memory.check_write(args.r8, 4).map_err(|_| WSAEFAULT)?;
                    memory
                        .write_u32(args.r8, (args.rdx as u32).swap_bytes())
                        .map_err(|_| WSAEFAULT)?;
                } else {
                    memory.check_write(args.r8, 2).map_err(|_| WSAEFAULT)?;
                    memory
                        .write_u16(args.r8, (args.rdx as u16).swap_bytes())
                        .map_err(|_| WSAEFAULT)?;
                }
                Ok(0)
            })();
            socket_result(&state, cpu.gs_base, result)
        },
    );
}

fn register_address_conversion(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    for symbol in ["htons", "ntohs"] {
        registry.register(MODULE, symbol, |args, _, _| {
            Ok(u64::from((args.rcx as u16).swap_bytes()))
        });
    }
    for symbol in ["htonl", "ntohl"] {
        registry.register(MODULE, symbol, |args, _, _| {
            Ok(u64::from((args.rcx as u32).swap_bytes()))
        });
    }
    for (symbol, wide) in [
        ("WSAHtonl", true),
        ("WSAHtons", false),
        ("WSANtohl", true),
        ("WSANtohs", false),
    ] {
        register_wsa_byteorder(registry, state, symbol, wide);
    }
    let format_state = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "inet_ntop",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let fail = |error| format_state.fail(cpu.gs_base, error, 0);
            if args.rcx as u32 != 2 {
                return fail(WSAEAFNOSUPPORT);
            }
            if args.r8 == 0 || args.r9 == 0 {
                return fail(ERROR_INVALID_PARAMETER);
            }
            let mut address = [0; 4];
            if args.rdx == 0 || memory.read_bytes(args.rdx, &mut address).is_err() {
                return fail(WSAEFAULT);
            }
            let mut text = std::net::Ipv4Addr::from(address).to_string().into_bytes();
            text.push(0);
            if (text.len() as u64) > args.r9 {
                return fail(ERROR_INVALID_PARAMETER);
            }
            if memory.write_bytes(args.r8, &text).is_err() {
                return fail(WSAEFAULT);
            }
            Ok(args.r8)
        },
    );
    let state = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "inet_pton",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let fail = |error| state.fail(cpu.gs_base, error, SOCKET_ERROR);
            if args.rcx as u32 != 2 {
                return fail(WSAEAFNOSUPPORT);
            }
            if args.rdx == 0 || args.r8 == 0 {
                return fail(WSAEFAULT);
            }
            // The longest dotted decimal IPv4 spelling is 15 bytes plus NUL.
            let mut text = [0; 15];
            for len in 0..=text.len() {
                let Some(address) = args.rdx.checked_add(len as u64) else {
                    return fail(WSAEFAULT);
                };
                let Ok(byte) = memory.read_u8(address) else {
                    return fail(WSAEFAULT);
                };
                if byte == 0 {
                    let Some(ip) = std::str::from_utf8(&text[..len])
                        .ok()
                        .and_then(|text| text.parse::<std::net::Ipv4Addr>().ok())
                    else {
                        return Ok(0);
                    };
                    return if memory.write_bytes(args.r8, &ip.octets()).is_ok() {
                        Ok(1)
                    } else {
                        fail(WSAEFAULT)
                    };
                }
                if len == text.len() {
                    break;
                }
                text[len] = byte;
            }
            Ok(0)
        },
    );
}

fn connect_loopback(
    state: &Arc<ServicesState>,
    net: &mut Winsock,
    memory: &GuestMemory,
    handle: u64,
    address: u64,
    address_len: u32,
) -> Result<(), u32> {
    let socket = net.socket(handle)?;
    let is_stream = socket.stream;
    let existing = socket.port;
    let requested = loopback_port(memory, address, address_len)?;
    if is_stream {
        if socket.stream_peer.is_some() {
            return Err(WSAEINVAL);
        }
        let listener = net
            .sockets
            .values()
            .any(|socket| socket.listening.is_some() && socket.port == Some(requested));
        if !listener {
            return Err(WSAECONNREFUSED);
        }
        let backlog_full = net.sockets.values().any(|socket| {
            socket.listening.is_some()
                && socket.port == Some(requested)
                && socket.backlog.len() >= socket.listening.unwrap_or(1)
        });
        if backlog_full {
            return Err(WSAECONNREFUSED);
        }
        if net.sockets.len() + 1 >= MAX_SOCKETS {
            return Err(WSAEMFILE);
        }
        let local = match existing {
            Some(port) => port,
            None => net.port(0)?,
        };
        net.serial += 1;
        let server = 0x5300_0000_0000_0000 | u64::from(net.serial);
        net.sockets.insert(
            server,
            Socket {
                stream: true,
                port: Some(requested),
                stream_peer: Some(handle),
                ..Socket::default()
            },
        );
        let client = net.sockets.get_mut(&handle).expect("validated socket");
        client.port = Some(local);
        client.stream_peer = Some(server);
        let listener_handle = net.sockets.iter().find_map(|(candidate, socket)| {
            (socket.listening.is_some() && socket.port == Some(requested)).then_some(*candidate)
        });
        if let Some(listening) = net
            .sockets
            .values_mut()
            .find(|socket| socket.listening.is_some() && socket.port == Some(requested))
        {
            listening.backlog.push_back(server);
        }
        if let Some(listener) = listener_handle {
            fire_event(state, net, listener, FD_ACCEPT);
        }
        fire_event(state, net, handle, FD_CONNECT | FD_WRITE);
        state.winsock_changed.notify_all();
        return Ok(());
    }
    if socket.stream_peer.is_some() || requested == 0 {
        return Err(WSAEINVAL);
    }
    let local = match existing {
        Some(port) => port,
        None => net.port(0)?,
    };
    let socket = net.sockets.get_mut(&handle).expect("validated socket");
    socket.port = Some(local);
    socket.peer = Some(requested);
    socket.pending.retain(|(source, _)| *source == requested);
    state.winsock_changed.notify_all();
    Ok(())
}

fn register_loopback(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    for (symbol, connect) in [("bind", false), ("connect", true)] {
        let state = Arc::clone(state);
        registry.register_with_status(
            MODULE,
            symbol,
            ServiceStatus::Partial,
            move |args, cpu, memory| {
                let mut net = state.winsock.lock().expect("guest Winsock state");
                let result = (|| {
                    if connect {
                        connect_loopback(
                            &state,
                            &mut net,
                            memory,
                            args.rcx,
                            args.rdx,
                            args.r8 as u32,
                        )?;
                        return Ok(0);
                    }
                    let socket = net.socket(args.rcx)?;
                    let existing = socket.port;
                    let requested = loopback_port(memory, args.rdx, args.r8 as u32)?;
                    if existing.is_some() {
                        return Err(WSAEINVAL);
                    }
                    let local = net.port(requested)?;
                    let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                    socket.port = Some(local);
                    state.winsock_changed.notify_all();
                    Ok(0)
                })();
                socket_result(&state, cpu.gs_base, result)
            },
        );
    }

    let wsa_connect = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSAConnect",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                if args.r9 != 0
                    || args.get(4, memory).map_err(|_| WSAEFAULT)? != 0
                    || args.get(5, memory).map_err(|_| WSAEFAULT)? != 0
                    || args.get(6, memory).map_err(|_| WSAEFAULT)? != 0
                {
                    return Err(WSAEINVAL);
                }
                let mut net = wsa_connect.winsock.lock().expect("guest Winsock state");
                connect_loopback(
                    &wsa_connect,
                    &mut net,
                    memory,
                    args.rcx,
                    args.rdx,
                    args.r8 as u32,
                )
                .map(|_| 0)
            })();
            socket_result(&wsa_connect, cpu.gs_base, result)
        },
    );

    let send = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "send",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let mut net = send.winsock.lock().expect("guest Winsock state");
            let result = (|| {
                let socket = net.socket(args.rcx)?;
                if socket.shutdown_send {
                    return Err(WSAESHUTDOWN);
                }
                if socket.stream {
                    // Byte stream: bytes append to the paired endpoint's
                    // fifo within the flow-control window, no boundaries.
                    let peer_handle = socket.stream_peer.ok_or(WSAENOTCONN)?;
                    let length = args.r8 as i32;
                    if length < 0 || args.r9 as u32 != 0 {
                        return Err(WSAEINVAL);
                    }
                    let length = length as usize;
                    let mut data = vec![0; length];
                    if length != 0 {
                        memory
                            .read_bytes(args.rdx, &mut data)
                            .map_err(|_| WSAEFAULT)?;
                    }
                    let Some(target) = net.sockets.get_mut(&peer_handle) else {
                        return Err(WSAECONNREFUSED);
                    };
                    let room = STREAM_WINDOW.saturating_sub(target.stream_read.len());
                    let accepted = room.min(length);
                    target.stream_read.extend(data[..accepted].iter().copied());
                    if accepted != 0 {
                        fire_event(&send, &mut net, peer_handle, FD_READ);
                        send.winsock_changed.notify_all();
                    }
                    return Ok(accepted as u64);
                }
                let peer = socket.peer.ok_or(WSAENOTCONN)?;
                let source = socket.port.expect("connected socket is bound");
                let length = args.r8 as i32;
                if length < 0 || args.r9 as u32 != 0 {
                    return Err(WSAEINVAL);
                }
                let length = length as usize;
                if length > MAX_DATAGRAM {
                    return Err(WSAEMSGSIZE);
                }
                let mut data = vec![0; length];
                if length != 0 {
                    memory
                        .read_bytes(args.rdx, &mut data)
                        .map_err(|_| WSAEFAULT)?;
                }
                queue_loopback_datagram(&send, &mut net, source, peer, data)?;
                Ok(length as u64)
            })();
            socket_result(&send, cpu.gs_base, result)
        },
    );

    let recv = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "recv",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let mut net = recv.winsock.lock().expect("guest Winsock state");
            let result = (|| {
                if net.socket(args.rcx)?.port.is_none() {
                    return Err(WSAEINVAL);
                }
                if net.socket(args.rcx)?.shutdown_receive {
                    return Err(WSAESHUTDOWN);
                }
                let length = args.r8 as i32;
                if length < 0 || args.r9 as u32 != 0 {
                    return Err(WSAEINVAL);
                }
                if net.socket(args.rcx)?.stream {
                    let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                    if socket.stream_read.is_empty() {
                        if socket.stream_eof {
                            // Orderly shutdown: zero bytes is the eof.
                            return Ok(0);
                        }
                        return Err(WSAEWOULDBLOCK);
                    }
                    let copied = (length as usize).min(socket.stream_read.len());
                    let data: Vec<u8> = socket.stream_read.drain(..copied).collect();
                    if copied != 0 {
                        memory.write_bytes(args.rdx, &data).map_err(|_| WSAEFAULT)?;
                    }
                    recv.winsock_changed.notify_all();
                    return Ok(copied as u64);
                }
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                let (_, data) = socket.pending.front().ok_or(WSAEWOULDBLOCK)?;
                let copied = (length as usize).min(data.len());
                if copied != 0 {
                    memory
                        .write_bytes(args.rdx, &data[..copied])
                        .map_err(|_| WSAEFAULT)?;
                }
                let truncated = copied < data.len();
                socket.pending.pop_front();
                recv.winsock_changed.notify_all();
                if truncated {
                    return Err(WSAEMSGSIZE);
                }
                Ok(copied as u64)
            })();
            socket_result(&recv, cpu.gs_base, result)
        },
    );
}

fn register_wsa_io(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    // The synchronous non-overlapped gather form. The sockets of this
    // machine are nonblocking by construction; stream sends therefore keep
    // the existing partial-flow-control result in the byte-count output.
    let send = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSASend",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let flags = args.get(4, memory).map_err(|_| WSAEFAULT)? as u32;
                let overlapped = args.get(5, memory).map_err(|_| WSAEFAULT)?;
                let completion = args.get(6, memory).map_err(|_| WSAEFAULT)?;
                if flags != 0 || overlapped != 0 || completion != 0 {
                    return Err(WSAEOPNOTSUPP);
                }
                if args.r9 == 0 {
                    return Err(WSAEFAULT);
                }
                memory.check_write(args.r9, 4).map_err(|_| WSAEFAULT)?;
                let (buffers, total) = read_wsabufs(memory, args.rdx, args.r8 as u32)?;
                let limit = (total as usize).min(STREAM_WINDOW);
                let data = gather_wsabufs(memory, &buffers, limit)?;
                let mut net = send.winsock.lock().expect("guest Winsock state");
                let socket = net.socket(args.rcx)?;
                if socket.shutdown_send {
                    return Err(WSAESHUTDOWN);
                }
                if socket.stream {
                    let peer_handle = socket.stream_peer.ok_or(WSAENOTCONN)?;
                    let Some(target) = net.sockets.get_mut(&peer_handle) else {
                        return Err(WSAECONNREFUSED);
                    };
                    let room = STREAM_WINDOW.saturating_sub(target.stream_read.len());
                    let accepted = room.min(data.len());
                    target.stream_read.extend(data[..accepted].iter().copied());
                    if accepted != 0 {
                        fire_event(&send, &mut net, peer_handle, FD_READ);
                        send.winsock_changed.notify_all();
                    }
                    memory
                        .write_u32(args.r9, accepted as u32)
                        .map_err(|_| WSAEFAULT)?;
                    return Ok(0);
                }
                if total > MAX_DATAGRAM as u64 {
                    return Err(WSAEMSGSIZE);
                }
                let peer = socket.peer.ok_or(WSAENOTCONN)?;
                let source = socket.port.expect("connected socket is bound");
                queue_loopback_datagram(&send, &mut net, source, peer, data)?;
                memory
                    .write_u32(args.r9, total as u32)
                    .map_err(|_| WSAEFAULT)?;
                Ok(0)
            })();
            socket_result(&send, cpu.gs_base, result)
        },
    );

    // Scatter receive. Datagram truncation follows the documented unreliable
    // protocol rule: the excess is discarded and WSAEMSGSIZE is reported.
    let recv = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSARecv",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let flags_ptr = args.get(4, memory).map_err(|_| WSAEFAULT)?;
                let overlapped = args.get(5, memory).map_err(|_| WSAEFAULT)?;
                let completion = args.get(6, memory).map_err(|_| WSAEFAULT)?;
                if overlapped != 0 || completion != 0 {
                    return Err(WSAEOPNOTSUPP);
                }
                if args.r9 == 0 || flags_ptr == 0 {
                    return Err(WSAEFAULT);
                }
                memory.check_write(args.r9, 4).map_err(|_| WSAEFAULT)?;
                memory.check_write(flags_ptr, 4).map_err(|_| WSAEFAULT)?;
                let flags = memory.read_u32(flags_ptr).map_err(|_| WSAEFAULT)?;
                if flags != 0 {
                    return Err(WSAEOPNOTSUPP);
                }
                let (buffers, total) = read_wsabufs(memory, args.rdx, args.r8 as u32)?;
                for &(pointer, length) in &buffers {
                    if length != 0 {
                        memory
                            .check_write(pointer, length as usize)
                            .map_err(|_| WSAEFAULT)?;
                    }
                }
                let mut net = recv.winsock.lock().expect("guest Winsock state");
                if net.socket(args.rcx)?.port.is_none() {
                    return Err(WSAEINVAL);
                }
                if net.socket(args.rcx)?.shutdown_receive {
                    return Err(WSAESHUTDOWN);
                }
                if net.socket(args.rcx)?.stream {
                    let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                    if socket.stream_read.is_empty() {
                        if socket.stream_eof {
                            memory.write_u32(args.r9, 0).map_err(|_| WSAEFAULT)?;
                            memory.write_u32(flags_ptr, 0).map_err(|_| WSAEFAULT)?;
                            return Ok(0);
                        }
                        return Err(WSAEWOULDBLOCK);
                    }
                    let copied = (total as usize).min(socket.stream_read.len());
                    let data: Vec<u8> = socket.stream_read.drain(..copied).collect();
                    scatter_wsabufs(memory, &buffers, &data)?;
                    memory
                        .write_u32(args.r9, copied as u32)
                        .map_err(|_| WSAEFAULT)?;
                    memory.write_u32(flags_ptr, 0).map_err(|_| WSAEFAULT)?;
                    recv.winsock_changed.notify_all();
                    return Ok(0);
                }
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                let Some((_, data)) = socket.pending.front() else {
                    return Err(WSAEWOULDBLOCK);
                };
                let copied = (total as usize).min(data.len());
                let data = data.clone();
                scatter_wsabufs(memory, &buffers, &data[..copied])?;
                let truncated = copied < data.len();
                socket.pending.pop_front();
                recv.winsock_changed.notify_all();
                if truncated {
                    return Err(WSAEMSGSIZE);
                }
                memory
                    .write_u32(args.r9, copied as u32)
                    .map_err(|_| WSAEFAULT)?;
                memory.write_u32(flags_ptr, 0).map_err(|_| WSAEFAULT)?;
                Ok(0)
            })();
            socket_result(&recv, cpu.gs_base, result)
        },
    );

    // Addressed datagram gather send. A null destination is the connected
    // datagram form; an explicit destination uses the same loopback-only
    // sockaddr validation as sendto.
    let send_to = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSASendTo",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let flags = args.get(4, memory).map_err(|_| WSAEFAULT)? as u32;
                let destination = args.get(5, memory).map_err(|_| WSAEFAULT)?;
                let destination_len = args.get(6, memory).map_err(|_| WSAEFAULT)? as u32;
                let overlapped = args.get(7, memory).map_err(|_| WSAEFAULT)?;
                let completion = args.get(8, memory).map_err(|_| WSAEFAULT)?;
                if flags != 0 || overlapped != 0 || completion != 0 {
                    return Err(WSAEOPNOTSUPP);
                }
                if args.r9 == 0 {
                    return Err(WSAEFAULT);
                }
                memory.check_write(args.r9, 4).map_err(|_| WSAEFAULT)?;
                let (buffers, total) = read_wsabufs(memory, args.rdx, args.r8 as u32)?;
                if total > MAX_DATAGRAM as u64 {
                    return Err(WSAEMSGSIZE);
                }
                let data = gather_wsabufs(memory, &buffers, total as usize)?;
                let mut net = send_to.winsock.lock().expect("guest Winsock state");
                let (stream, shutdown_send, connected_peer, bound_port) = {
                    let socket = net.socket(args.rcx)?;
                    (
                        socket.stream,
                        socket.shutdown_send,
                        socket.peer,
                        socket.port,
                    )
                };
                if shutdown_send {
                    return Err(WSAESHUTDOWN);
                }
                if stream {
                    return Err(WSAEOPNOTSUPP);
                }
                let peer = if destination != 0 {
                    loopback_port(memory, destination, destination_len)?
                } else {
                    connected_peer.ok_or(WSAENOTCONN)?
                };
                let source = match bound_port {
                    Some(port) => port,
                    None => {
                        let port = net.port(0)?;
                        net.sockets
                            .get_mut(&args.rcx)
                            .expect("validated socket")
                            .port = Some(port);
                        port
                    }
                };
                queue_loopback_datagram(&send_to, &mut net, source, peer, data)?;
                memory
                    .write_u32(args.r9, total as u32)
                    .map_err(|_| WSAEFAULT)?;
                Ok(0)
            })();
            socket_result(&send_to, cpu.gs_base, result)
        },
    );

    // Addressed datagram scatter receive. The complete output shape is
    // validated before the datagram is consumed, so a bad source-address
    // pointer cannot lose a pending message.
    let recv_from = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSARecvFrom",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let flags_ptr = args.get(4, memory).map_err(|_| WSAEFAULT)?;
                let source_address = args.get(5, memory).map_err(|_| WSAEFAULT)?;
                let source_length = args.get(6, memory).map_err(|_| WSAEFAULT)?;
                let overlapped = args.get(7, memory).map_err(|_| WSAEFAULT)?;
                let completion = args.get(8, memory).map_err(|_| WSAEFAULT)?;
                if overlapped != 0 || completion != 0 {
                    return Err(WSAEOPNOTSUPP);
                }
                if args.r9 == 0 || flags_ptr == 0 {
                    return Err(WSAEFAULT);
                }
                memory.check_write(args.r9, 4).map_err(|_| WSAEFAULT)?;
                memory.check_write(flags_ptr, 4).map_err(|_| WSAEFAULT)?;
                let flags = memory.read_u32(flags_ptr).map_err(|_| WSAEFAULT)?;
                if flags != 0 {
                    return Err(WSAEOPNOTSUPP);
                }
                if source_address != 0 {
                    if source_length == 0 {
                        return Err(WSAEFAULT);
                    }
                    let capacity = memory.read_u32(source_length).map_err(|_| WSAEFAULT)? as i32;
                    if capacity < 16 {
                        return Err(WSAEFAULT);
                    }
                    memory
                        .check_write(source_address, 16)
                        .map_err(|_| WSAEFAULT)?;
                    memory
                        .check_write(source_length, 4)
                        .map_err(|_| WSAEFAULT)?;
                }
                let (buffers, total) = read_wsabufs(memory, args.rdx, args.r8 as u32)?;
                for &(pointer, length) in &buffers {
                    if length != 0 {
                        memory
                            .check_write(pointer, length as usize)
                            .map_err(|_| WSAEFAULT)?;
                    }
                }
                let mut net = recv_from.winsock.lock().expect("guest Winsock state");
                let (stream, port, shutdown_receive) = {
                    let socket = net.socket(args.rcx)?;
                    (socket.stream, socket.port, socket.shutdown_receive)
                };
                if stream {
                    return Err(WSAEOPNOTSUPP);
                }
                if port.is_none() {
                    return Err(WSAEINVAL);
                }
                if shutdown_receive {
                    return Err(WSAESHUTDOWN);
                }
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                let Some((source, data)) = socket.pending.front() else {
                    return Err(WSAEWOULDBLOCK);
                };
                let source = *source;
                let copied = (total as usize).min(data.len());
                let data = data.clone();
                scatter_wsabufs(memory, &buffers, &data[..copied])?;
                let truncated = copied < data.len();
                write_loopback_source(memory, source_address, source_length, source)?;
                socket.pending.pop_front();
                recv_from.winsock_changed.notify_all();
                if truncated {
                    return Err(WSAEMSGSIZE);
                }
                memory
                    .write_u32(args.r9, copied as u32)
                    .map_err(|_| WSAEFAULT)?;
                memory.write_u32(flags_ptr, 0).map_err(|_| WSAEFAULT)?;
                Ok(0)
            })();
            socket_result(&recv_from, cpu.gs_base, result)
        },
    );
}

fn register_event_select(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    use super::GuestObject;
    use super::events::Waitable;

    // WSAEventSelect associates a socket with an event object and a mask.
    // Registering re-evaluates the current state - a connected stream is
    // writable now, pending bytes are readable now - posting those events
    // immediately, exactly the level the guest's first wait observes.
    let select = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSAEventSelect",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let _ = memory;
                let mut net = select.winsock.lock().expect("guest Winsock state");
                if net.socket(args.rcx).is_err() {
                    return Err(WSAENOTSOCK);
                }
                let known = FD_READ | FD_WRITE | FD_ACCEPT | FD_CONNECT | FD_CLOSE;
                let mask = args.r8 as u32;
                if mask & !known != 0 {
                    return Err(WSAEINVAL);
                }
                if mask == 0 {
                    // Zero mask cancels the association.
                    let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                    socket.event_select = None;
                    return Ok(0);
                }
                {
                    let objects = select.objects.lock().expect("guest object state");
                    let valid = matches!(
                        objects.handles.get(&args.rdx),
                        Some(GuestObject::Waitable { state: waitable, .. })
                            if matches!(*waitable.lock().expect("waitable"), Waitable::Event { .. })
                    );
                    if !valid {
                        return Err(WSAENOTSOCK);
                    }
                }
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                socket.event_select = Some((args.rdx, mask));
                socket.events_occurred = 0;
                // Level evaluation at registration: post what is already true.
                let mut post = 0;
                if !socket.stream_read.is_empty() || !socket.pending.is_empty() {
                    post |= FD_READ;
                }
                if socket.stream_peer.is_some() {
                    post |= FD_WRITE;
                }
                if !socket.backlog.is_empty() {
                    post |= FD_ACCEPT;
                }
                if socket.stream_eof {
                    post |= FD_CLOSE;
                }
                let bits: Vec<u32> = [FD_READ, FD_WRITE, FD_ACCEPT, FD_CLOSE]
                    .into_iter()
                    .filter(|bit| post & bit != 0)
                    .collect();
                for bit in bits {
                    fire_event(&select, &mut net, args.rcx, bit);
                }
                Ok(0)
            })();
            socket_result(&select, cpu.gs_base, result)
        },
    );

    // WSAEnumNetworkEvents reports the occurred bits through the network
    // events structure, clears them, and resets the event object - the
    // report-and-drain contract of the form.
    let enumerate = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSAEnumNetworkEvents",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let mut net = enumerate.winsock.lock().expect("guest Winsock state");
                if net.socket(args.rcx).is_err() {
                    return Err(WSAENOTSOCK);
                }
                if args.r8 == 0 {
                    return Err(WSAEFAULT);
                }
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                let occurred = socket.events_occurred;
                socket.events_occurred = 0;
                // Structure: i32 events bitmask then ten i32 error codes.
                memory.write_u32(args.r8, occurred).map_err(|_| WSAEFAULT)?;
                for index in 0..10 {
                    memory
                        .write_u32(args.r8 + 4 + index * 4, 0)
                        .map_err(|_| WSAEFAULT)?;
                }
                // The form resets the event object it reports against.
                if args.rdx != 0 {
                    let objects = enumerate.objects.lock().expect("guest object state");
                    if let Some(GuestObject::Waitable {
                        state: waitable, ..
                    }) = objects.handles.get(&args.rdx)
                        && let Ok(mut inner) = waitable.lock()
                        && let Waitable::Event { signaled, .. } = &mut *inner
                    {
                        *signaled = false;
                    }
                }
                enumerate.winsock_changed.notify_all();
                Ok(0)
            })();
            socket_result(&enumerate, cpu.gs_base, result)
        },
    );
}

fn ioctl_socket_memory(
    memory: &mut GuestMemory,
    socket: &Socket,
    command: u32,
    input: u64,
    output: u64,
) -> Result<u64, u32> {
    const FIONREAD: u32 = 0x4004_667F;
    const FIONBIO: u32 = 0x8004_667E;
    match command {
        FIONREAD => {
            if output == 0 {
                return Err(WSAEFAULT);
            }
            let bytes = if socket.stream {
                socket.stream_read.len() as u32
            } else {
                socket
                    .pending
                    .front()
                    .map_or(0, |(_, data)| data.len() as u32)
            };
            memory.write_u32(output, bytes).map_err(|_| WSAEFAULT)?;
            Ok(0)
        }
        FIONBIO => {
            if input == 0 {
                return Err(WSAEFAULT);
            }
            let enable = memory.read_u32(input).map_err(|_| WSAEFAULT)? != 0;
            // Every socket of the machine is nonblocking by construction:
            // enabling is the truth, disabling cannot be served.
            if enable { Ok(0) } else { Err(WSAEINVAL) }
        }
        _ => Err(WSAEINVAL),
    }
}

fn register_ioctl_and_reverse(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    // WSAIoctl serves the queries with real answers in this model: FIONREAD
    // reports the bytes a recv could return now - the stream fifo length or
    // the first pending datagram's size. FIONBIO is vacuous: every socket
    // of the machine is already nonblocking, so enabling is success and
    // disabling is honestly impossible. Everything else refuses.
    let ioctl = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSAIoctl",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let net = ioctl.winsock.lock().expect("guest Winsock state");
                let socket = net.socket(args.rcx)?;
                let output = args.get(4, memory).map_err(|_| WSAEFAULT)?;
                ioctl_socket_memory(memory, socket, args.rdx as u32, args.r8, output)
            })();
            socket_result(&ioctl, cpu.gs_base, result)
        },
    );

    // ioctlsocket is the legacy three-argument form of the same bounded
    // socket ioctl surface: socket, command and one argument pointer.
    let legacy_ioctl = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "ioctlsocket",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let net = legacy_ioctl.winsock.lock().expect("guest Winsock state");
                let socket = net.socket(args.rcx)?;
                ioctl_socket_memory(memory, socket, args.rdx as u32, args.r8, args.r8)
            })();
            socket_result(&legacy_ioctl, cpu.gs_base, result)
        },
    );

    // gethostbyaddr reverses the loopback address into the machine's one
    // name, in heap-owned storage reused across calls - the static-storage
    // contract of the legacy form. Anything but the loopback is an honest
    // no-host.
    let reverse = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "gethostbyaddr",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            use crate::memory::PROCESS_HEAP;
            let result = (|| {
                if reverse.winsock.lock().expect("guest Winsock state").refs == 0 {
                    return Err(WSANOTINITIALISED);
                }
                if args.rcx == 0 || args.rdx != 4 || args.r8 as u32 != 2 {
                    return Err(WSAEFAULT);
                }
                let mut ip = [0; 4];
                memory
                    .read_bytes(args.rcx, &mut ip)
                    .map_err(|_| WSAEFAULT)?;
                if ip != [127, 0, 0, 1] {
                    return Err(11001); // WSAHOST_NOT_FOUND
                }
                // hostent: name ptr, aliases, addrtype, length, addr_list.
                // One reused allocation: struct, name, address, list.
                let needs = 64;
                let base = memory
                    .map_heap_allocation(PROCESS_HEAP, needs)
                    .map_err(|_| WSAENOBUFS)?;
                let name = b"XENORA\0";
                memory.write_bytes(base + 32, name).map_err(|_| WSAEFAULT)?;
                memory
                    .write_bytes(base + 40, &[127, 0, 0, 1])
                    .map_err(|_| WSAEFAULT)?;
                // addr_list: pointer at base+48 then null at base+56.
                memory
                    .write_u64(base + 48, base + 40)
                    .map_err(|_| WSAEFAULT)?;
                memory.write_u64(base + 56, 0).map_err(|_| WSAEFAULT)?;
                memory.write_u64(base, base + 32).map_err(|_| WSAEFAULT)?; // h_name
                memory
                    .write_u64(base + 8, base + 56)
                    .map_err(|_| WSAEFAULT)?; // aliases: null list
                memory.write_u64(base + 16, 2).map_err(|_| WSAEFAULT)?; // addrtype AF_INET (+length packed)
                memory
                    .write_u64(base + 24, base + 48)
                    .map_err(|_| WSAEFAULT)?; // h_addr_list
                Ok(base)
            })();
            match result {
                Ok(value) => Ok(value),
                Err(error) => {
                    let _ = reverse.fail(cpu.gs_base, error, 0);
                    Ok(0)
                }
            }
        },
    );

    // gethostbyname resolves only names true in this loopback machine. Every
    // other name is host-not-found because there is no external DNS.
    let byname = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "gethostbyname",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            use crate::memory::PROCESS_HEAP;
            let result = (|| {
                if byname.winsock.lock().expect("guest Winsock state").refs == 0 {
                    return Err(WSANOTINITIALISED);
                }
                if args.rcx == 0 {
                    return Err(WSAEFAULT);
                }
                let name = read_socket_cstr(memory, args.rcx)?;
                if !matches!(name.as_str(), "localhost" | "127.0.0.1" | "XENORA") {
                    return Err(11001); // WSAHOST_NOT_FOUND
                }
                let needs = 64;
                let base = memory
                    .map_heap_allocation(PROCESS_HEAP, needs)
                    .map_err(|_| WSAENOBUFS)?;
                memory
                    .write_bytes(base + 32, b"XENORA\0")
                    .map_err(|_| WSAEFAULT)?;
                memory
                    .write_bytes(base + 40, &[127, 0, 0, 1])
                    .map_err(|_| WSAEFAULT)?;
                memory
                    .write_u64(base + 48, base + 40)
                    .map_err(|_| WSAEFAULT)?;
                memory.write_u64(base + 56, 0).map_err(|_| WSAEFAULT)?;
                memory.write_u64(base, base + 32).map_err(|_| WSAEFAULT)?;
                memory
                    .write_u64(base + 8, base + 56)
                    .map_err(|_| WSAEFAULT)?;
                memory.write_u64(base + 16, 2).map_err(|_| WSAEFAULT)?;
                memory
                    .write_u64(base + 24, base + 48)
                    .map_err(|_| WSAEFAULT)?;
                Ok(base)
            })();
            match result {
                Ok(value) => Ok(value),
                Err(error) => {
                    let _ = byname.fail(cpu.gs_base, error, 0);
                    Ok(0)
                }
            }
        },
    );
}

fn register_service_lookup(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    // getservbyname resolves a service name and protocol to its port from
    // the services database. The machine carries the standard Internet
    // assignment table the way Windows ships system32\\drivers\\etc\\services:
    // a fixed, documented list. An unknown name is an honest null. busybox
    // binds the import for its networking applets; callers that never look
    // a service up are undisturbed. Ports leave in network byte order, per
    // the documented contract.
    let lookup = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "getservbyname",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            use crate::memory::PROCESS_HEAP;
            const SERVICES: [(&[u8], u16, &[u8]); 10] = [
                (b"ftp\0", 21, b"tcp\0"),
                (b"ssh\0", 22, b"tcp\0"),
                (b"telnet\0", 23, b"tcp\0"),
                (b"smtp\0", 25, b"tcp\0"),
                (b"domain\0", 53, b"tcp\0"),
                (b"domain\0", 53, b"udp\0"),
                (b"http\0", 80, b"tcp\0"),
                (b"ntp\0", 123, b"udp\0"),
                (b"https\0", 443, b"tcp\0"),
                (b"bootps\0", 67, b"udp\0"),
            ];
            let fail = |error: u32| {
                let _ = lookup.fail(cpu.gs_base, error, 0);
                Ok(0)
            };
            if lookup.winsock.lock().expect("guest Winsock state").refs == 0 {
                return fail(WSANOTINITIALISED);
            }
            let name_ptr = args.rcx;
            let proto_ptr = args.rdx;
            if name_ptr == 0 {
                return fail(WSAEFAULT);
            }
            // Read the name byte by byte, case-insensitively compared.
            let mut name = Vec::new();
            for offset in 0..32u64 {
                let mut one = [0u8; 1];
                if memory.read_bytes(name_ptr + offset, &mut one).is_err() {
                    return fail(WSAEFAULT);
                }
                if one[0] == 0 {
                    break;
                }
                name.push(one[0].to_ascii_lowercase());
            }
            let mut proto = Vec::new();
            if proto_ptr != 0 {
                for offset in 0..8u64 {
                    let mut one = [0u8; 1];
                    if memory.read_bytes(proto_ptr + offset, &mut one).is_err() {
                        return fail(WSAEFAULT);
                    }
                    if one[0] == 0 {
                        break;
                    }
                    proto.push(one[0].to_ascii_lowercase());
                }
            }
            let found = SERVICES.iter().find(|(service, _, transport)| {
                let service_name: Vec<u8> = service
                    .iter()
                    .take_while(|byte| **byte != 0)
                    .copied()
                    .collect();
                let transport_name: Vec<u8> = transport
                    .iter()
                    .take_while(|byte| **byte != 0)
                    .copied()
                    .collect();
                service_name == name && (proto.is_empty() || transport_name == proto)
            });
            let Some((service, port, transport)) = found else {
                return fail(11001); // WSANO_DATA: no such service is known.
            };
            // servent: name ptr, aliases ptr, port (u16 at +16), proto ptr (+24).
            let base = match memory.map_heap_allocation(PROCESS_HEAP, 80) {
                Ok(base) => base,
                Err(_) => return fail(WSAENOBUFS),
            };
            let write = memory.write_bytes(base + 32, service);
            if write.is_err() {
                return fail(WSAEFAULT);
            }
            if memory.write_bytes(base + 48, transport).is_err() {
                return fail(WSAEFAULT);
            }
            if memory.write_u64(base, base + 32).is_err() // s_name
                || memory.write_u64(base + 8, 0).is_err() // s_aliases: none
                || memory.write_u16(base + 16, port.to_be()).is_err() // network order
                || memory.write_u64(base + 24, base + 48).is_err()
            {
                return fail(WSAEFAULT);
            }
            Ok(base)
        },
    );
}

fn accept_loopback(
    state: &Arc<ServicesState>,
    memory: &mut GuestMemory,
    handle: u64,
    address: u64,
    address_len: u64,
) -> Result<u64, u32> {
    let mut net = state.winsock.lock().expect("guest Winsock state");
    if net.socket(handle)?.listening.is_none() {
        return Err(WSAEINVAL);
    }
    if address != 0 {
        if address_len == 0 {
            return Err(WSAEFAULT);
        }
        let capacity = memory.read_u32(address_len).map_err(|_| WSAEFAULT)? as i32;
        if capacity < 16 {
            return Err(WSAEFAULT);
        }
        memory.check_write(address, 16).map_err(|_| WSAEFAULT)?;
        memory.check_write(address_len, 4).map_err(|_| WSAEFAULT)?;
    }
    let socket = net.sockets.get_mut(&handle).expect("validated socket");
    let server = socket.backlog.pop_front().ok_or(WSAEWOULDBLOCK)?;
    state.winsock_changed.notify_all();
    if address != 0 {
        let client_port = net
            .sockets
            .get(&server)
            .and_then(|endpoint| endpoint.stream_peer)
            .and_then(|client| net.sockets.get(&client))
            .and_then(|client| client.port)
            .unwrap_or(0);
        let mut bytes = [0; 16];
        bytes[..2].copy_from_slice(&2_u16.to_le_bytes());
        bytes[2..4].copy_from_slice(&client_port.to_be_bytes());
        bytes[4..8].copy_from_slice(&[127, 0, 0, 1]);
        memory.write_bytes(address, &bytes).map_err(|_| WSAEFAULT)?;
        memory.write_u32(address_len, 16).map_err(|_| WSAEFAULT)?;
    }
    Ok(server)
}

fn register_streams(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    // listen marks a bound stream socket with its backlog limit. The queue
    // is real: connect enqueues, accept dequeues, in order.
    let listen = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "listen",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let _ = memory;
            let result = (|| {
                let mut net = listen.winsock.lock().expect("guest Winsock state");
                let socket = net.socket(args.rcx)?;
                if !socket.stream {
                    return Err(WSAEOPNOTSUPP);
                }
                if socket.port.is_none() {
                    return Err(WSAEINVAL);
                }
                if socket.stream_peer.is_some() {
                    return Err(WSAEINVAL);
                }
                let backlog = (args.rdx as i32).clamp(1, 16) as usize;
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                socket.listening = Some(backlog);
                listen.winsock_changed.notify_all();
                Ok(0)
            })();
            socket_result(&listen, cpu.gs_base, result)
        },
    );

    // accept dequeues the oldest pending server endpoint, reporting the
    // client address when asked.
    let accept = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "accept",
        ServiceStatus::Partial,
        move |args, cpu, memory| match accept_loopback(&accept, memory, args.rcx, args.rdx, args.r8)
        {
            Ok(value) => Ok(value),
            Err(error) => accept.fail(cpu.gs_base, error, INVALID_SOCKET),
        },
    );

    let wsa_accept = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSAAccept",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                if args.r9 != 0 || args.get(4, memory).map_err(|_| WSAEFAULT)? != 0 {
                    return Err(WSAEINVAL);
                }
                accept_loopback(&wsa_accept, memory, args.rcx, args.rdx, args.r8)
            })();
            match result {
                Ok(value) => Ok(value),
                Err(error) => wsa_accept.fail(cpu.gs_base, error, INVALID_SOCKET),
            }
        },
    );
}

fn register_datagrams(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let sendto = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "sendto",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let mut net = sendto.winsock.lock().expect("guest Winsock state");
            let result = (|| {
                let length = args.r8 as i32;
                if length < 0 || args.r9 as u32 != 0 {
                    return Err(WSAEINVAL);
                }
                let length = length as usize;
                if length > MAX_DATAGRAM {
                    return Err(WSAEMSGSIZE);
                }
                // The destination is explicit: an absent address is the
                // connected form, which send already serves.
                if args.get(4, memory).map_err(|_| WSAEFAULT)? == 0 {
                    return Err(WSAEINVAL);
                }
                let peer = loopback_port(memory, args.get(4, memory).map_err(|_| WSAEFAULT)?, {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        args.get(5, memory).map_err(|_| WSAEFAULT)? as u32
                    }
                })?;
                // An unbound socket auto-binds an ephemeral port, as the
                // real stack does on its first send.
                if net.socket(args.rcx)?.port.is_none() {
                    let port = net.port(0)?;
                    net.sockets
                        .get_mut(&args.rcx)
                        .expect("validated socket")
                        .port = Some(port);
                }
                let source = net.socket(args.rcx)?.port.expect("bound above");
                let mut data = vec![0; length];
                if length != 0 {
                    memory
                        .read_bytes(args.rdx, &mut data)
                        .map_err(|_| WSAEFAULT)?;
                }
                queue_loopback_datagram(&sendto, &mut net, source, peer, data)?;
                Ok(length as u64)
            })();
            socket_result(&sendto, cpu.gs_base, result)
        },
    );

    let recvfrom = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "recvfrom",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let mut net = recvfrom.winsock.lock().expect("guest Winsock state");
            let result = (|| {
                if net.socket(args.rcx)?.port.is_none() {
                    return Err(WSAEINVAL);
                }
                let length = args.r8 as i32;
                if length < 0 || args.r9 as u32 != 0 {
                    return Err(WSAEINVAL);
                }
                let source_address = args.get(4, memory).map_err(|_| WSAEFAULT)?;
                let source_length = args.get(5, memory).map_err(|_| WSAEFAULT)?;
                if source_address != 0 {
                    if source_length == 0 {
                        return Err(WSAEFAULT);
                    }
                    let capacity = memory.read_u32(source_length).map_err(|_| WSAEFAULT)? as i32;
                    if capacity < 16 {
                        return Err(WSAEFAULT);
                    }
                    memory
                        .check_write(source_address, 16)
                        .map_err(|_| WSAEFAULT)?;
                    memory
                        .check_write(source_length, 4)
                        .map_err(|_| WSAEFAULT)?;
                }
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                let (source, data) = socket.pending.front().ok_or(WSAEWOULDBLOCK)?;
                let (source, copied) = (*source, (length as usize).min(data.len()));
                if copied != 0 {
                    memory
                        .write_bytes(args.rdx, &data[..copied])
                        .map_err(|_| WSAEFAULT)?;
                }
                let truncated = copied < data.len();
                socket.pending.pop_front();
                recvfrom.winsock_changed.notify_all();
                // The source address reports where the datagram came from
                // when the caller asked for it.
                write_loopback_source(memory, source_address, source_length, source)?;
                if truncated {
                    return Err(WSAEMSGSIZE);
                }
                Ok(copied as u64)
            })();
            socket_result(&recvfrom, cpu.gs_base, result)
        },
    );
}

fn register_host_identity(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let hostname = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "gethostname",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                if hostname.winsock.lock().expect("guest Winsock state").refs == 0 {
                    return Err(WSANOTINITIALISED);
                }
                // The machine has one fixed name on its simulated LAN.
                let name = b"XENORA\0";
                if (args.rdx as i32) < name.len() as i32 {
                    return Err(WSAEFAULT);
                }
                memory.write_bytes(args.rcx, name).map_err(|_| WSAEFAULT)?;
                Ok(0)
            })();
            socket_result(&hostname, cpu.gs_base, result)
        },
    );

    // inet_addr is the legacy parse-only form: dotted decimal or 255.255.255.255
    // for a failure, exactly the ambiguity the CRT documents.
    let inet = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "inet_addr",
        ServiceStatus::Partial,
        move |args, _cpu, memory| {
            let _ = &inet;
            let mut text = [0u8; 15];
            let mut len = 0usize;
            loop {
                let byte = memory
                    .read_u8(args.rcx + len as u64)
                    .map_err(ExitReason::MemoryFault)?;
                if byte == 0 {
                    break;
                }
                if len == text.len() {
                    return Ok(u64::from(u32::MAX));
                }
                text[len] = byte;
                len += 1;
            }
            let parsed = std::str::from_utf8(&text[..len])
                .ok()
                .and_then(|text| text.parse::<std::net::Ipv4Addr>().ok());
            match parsed {
                Some(ip) => Ok(u64::from(u32::from_le_bytes(ip.octets()))),
                None => Ok(u64::from(u32::MAX)),
            }
        },
    );
}

fn register_lifecycle(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let shutdown = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "shutdown",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let _ = memory;
            let result = (|| {
                let mut net = shutdown.winsock.lock().expect("guest Winsock state");
                if net.socket(args.rcx)?.peer.is_none() {
                    return Err(WSAENOTCONN);
                }
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                match args.rdx as i32 {
                    0 => socket.shutdown_receive = true,
                    1 => socket.shutdown_send = true,
                    2 => {
                        socket.shutdown_receive = true;
                        socket.shutdown_send = true;
                    }
                    _ => return Err(WSAEINVAL),
                }
                shutdown.winsock_changed.notify_all();
                Ok(0)
            })();
            socket_result(&shutdown, cpu.gs_base, result)
        },
    );

    // The options the loopback model stores. They are vacuous - loopback
    // datagrams have no broadcast domain to permit and no port sharing to
    // relax - but they are stored honestly rather than silently dropped,
    // and getsockopt can grow to report them.
    let setsockopt = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "setsockopt",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            const SO_REUSEADDR: u32 = 0x0004;
            const SO_BROADCAST: u32 = 0x0020;
            let result = (|| {
                let mut net = setsockopt.winsock.lock().expect("guest Winsock state");
                net.socket(args.rcx)?;
                if args.rdx as u32 != 0xffff {
                    return Err(WSAENOPROTOOPT);
                }
                let length = args.get(4, memory).map_err(|_| WSAEFAULT)? as i32;
                if args.r9 == 0 || length < 4 {
                    return Err(WSAEFAULT);
                }
                let value = memory.read_u32(args.r9).map_err(|_| WSAEFAULT)? != 0;
                let socket = net.sockets.get_mut(&args.rcx).expect("validated socket");
                match args.r8 as u32 {
                    SO_REUSEADDR => socket.reuse_address = value,
                    SO_BROADCAST => socket.broadcast = value,
                    _ => return Err(WSAENOPROTOOPT),
                }
                Ok(0)
            })();
            socket_result(&setsockopt, cpu.gs_base, result)
        },
    );
}

fn register_adapters(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    // WSASocketA is the overlapped-capable creation form. The machine has
    // no completion machinery, so the flags must be zero - anything else is
    // honest refusal - and a zero-flag creation is exactly socket().
    for symbol in ["WSASocketA", "WSASocketW"] {
        let wsasocket = Arc::clone(state);
        registry.register_with_status(
            MODULE,
            symbol,
            ServiceStatus::Partial,
            move |args, cpu, memory| {
                let result = (|| {
                    if args.get(3, memory).map_err(|_| WSAEFAULT)? != 0
                        || args.get(4, memory).map_err(|_| WSAEFAULT)? != 0
                    {
                        return Err(WSAEINVAL);
                    }
                    if args.get(5, memory).map_err(|_| WSAEFAULT)? != 0 {
                        return Err(WSAEOPNOTSUPP);
                    }
                    let mut net = wsasocket.winsock.lock().expect("guest Winsock state");
                    create_loopback_socket(&mut net, args.rcx, args.rdx, args.r8)
                })();
                match result {
                    Ok(value) => Ok(value),
                    Err(error) => wsasocket.fail(cpu.gs_base, error, INVALID_SOCKET),
                }
            },
        );
    }

    // __WSAFDIsSet is the fd_set membership test the FD_SET macro expands
    // to: linear scan of the count-prefixed handle array.
    registry.register_with_status(
        MODULE,
        "__WSAFDIsSet",
        ServiceStatus::Partial,
        |args, _, memory| {
            let count = memory.read_u32(args.rdx).map_err(ExitReason::MemoryFault)?;
            for index in 0..count.min(64) {
                if memory
                    .read_u64(args.rdx + 8 + u64::from(index) * 8)
                    .map_err(ExitReason::MemoryFault)?
                    == args.rcx
                {
                    return Ok(1);
                }
            }
            Ok(0)
        },
    );

    // WSAAddressToString formats the loopback sockaddr_in in dotted
    // decimal with its port - the one family the machine serves. ANSI uses
    // byte units and Unicode uses WCHAR units for the length parameter.
    for (symbol, unicode) in [
        ("WSAAddressToStringA", false),
        ("WSAAddressToStringW", true),
    ] {
        let addrtostr = Arc::clone(state);
        registry.register_with_status(
            MODULE,
            symbol,
            ServiceStatus::Partial,
            move |args, cpu, memory| {
                let result = (|| {
                    if args.rdx != 16 || args.r8 != 0 {
                        return Err(WSAEINVAL);
                    }
                    let mut data = [0; 16];
                    memory
                        .read_bytes(args.rcx, &mut data)
                        .map_err(|_| WSAEFAULT)?;
                    if u16::from_le_bytes([data[0], data[1]]) != 2 {
                        return Err(WSAEAFNOSUPPORT);
                    }
                    let port = u16::from_be_bytes([data[2], data[3]]);
                    let text = format!("{}.{}.{}.{}:{}", data[4], data[5], data[6], data[7], port);
                    let length = args.get(4, memory).map_err(|_| WSAEFAULT)?;
                    if length == 0 {
                        return Err(WSAEFAULT);
                    }
                    let capacity = memory.read_u32(length).map_err(|_| WSAEFAULT)? as usize;
                    let required = text.len() + 1;
                    memory.check_write(length, 4).map_err(|_| WSAEFAULT)?;
                    memory
                        .write_u32(length, required as u32)
                        .map_err(|_| WSAEFAULT)?;
                    if capacity < required {
                        return Err(WSAEFAULT);
                    }
                    if unicode {
                        memory
                            .check_write(args.r9, required * 2)
                            .map_err(|_| WSAEFAULT)?;
                        for (index, unit) in text.encode_utf16().chain([0]).enumerate() {
                            memory
                                .write_u16(args.r9 + index as u64 * 2, unit)
                                .map_err(|_| WSAEFAULT)?;
                        }
                    } else {
                        memory
                            .check_write(args.r9, required)
                            .map_err(|_| WSAEFAULT)?;
                        memory
                            .write_bytes(args.r9, text.as_bytes())
                            .map_err(|_| WSAEFAULT)?;
                        memory
                            .write_u8(args.r9 + text.len() as u64, 0)
                            .map_err(|_| WSAEFAULT)?;
                    }
                    Ok(0)
                })();
                socket_result(&addrtostr, cpu.gs_base, result)
            },
        );
    }
}

fn read_socket_cstr(memory: &GuestMemory, address: u64) -> Result<String, u32> {
    let mut bytes = Vec::new();
    for index in 0..256u64 {
        let byte = memory.read_u8(address + index).map_err(|_| WSAEFAULT)?;
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    String::from_utf8(bytes).map_err(|_| WSAEINVAL)
}

fn register_resolution(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    use crate::memory::PROCESS_HEAP;

    // getaddrinfo over the loopback-only naming surface: "localhost",
    // "127.0.0.1" or a null node resolve to the machine's one address;
    // anything else is an honest no-name - there is no DNS. The result is
    // one heap allocation the caller owns through freeaddrinfo, exactly
    // the contract.
    let gai = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "getaddrinfo",
        ServiceStatus::Partial,
        move |args, _cpu, memory| {
            let result = (|| {
                if gai.winsock.lock().expect("guest Winsock state").refs == 0 {
                    return Err(WSANOTINITIALISED);
                }
                if args.r9 == 0 {
                    return Err(WSAEFAULT);
                }
                let node = if args.rcx == 0 {
                    None
                } else {
                    Some(read_socket_cstr(memory, args.rcx)?)
                };
                let service = if args.rdx == 0 {
                    None
                } else {
                    Some(read_socket_cstr(memory, args.rdx)?)
                };
                let ip: [u8; 4] = match node.as_deref() {
                    None | Some("localhost") | Some("127.0.0.1") => [127, 0, 0, 1],
                    _ => return Err(11001), // WSAHOST_NOT_FOUND: no DNS exists
                };
                let port: u16 = match service.as_deref() {
                    None => 0,
                    Some(text) => text.parse::<u16>().map_err(|_| 11001u32)?,
                };
                // hints validation: family, socktype and protocol must be
                // the loopback ones or unspecified.
                if args.r8 != 0 {
                    let family = memory.read_u32(args.r8 + 4).map_err(|_| WSAEFAULT)?;
                    let socktype = memory.read_u32(args.r8 + 8).map_err(|_| WSAEFAULT)?;
                    let protocol = memory.read_u32(args.r8 + 12).map_err(|_| WSAEFAULT)?;
                    if family != 0 && family != 2 {
                        return Err(WSAEAFNOSUPPORT);
                    }
                    if socktype != 0 && socktype != 2 {
                        return Err(WSAESOCKTNOSUPPORT);
                    }
                    if protocol != 0 && protocol != 17 {
                        return Err(WSAESOCKTNOSUPPORT);
                    }
                }
                // One allocation: addrinfo (48 bytes) then sockaddr_in (16).
                let base = memory
                    .map_heap_allocation(PROCESS_HEAP, 64)
                    .map_err(|_| WSAENOBUFS)?;
                let mut addr = [0; 16];
                addr[..2].copy_from_slice(&2_u16.to_le_bytes());
                addr[2..4].copy_from_slice(&port.to_be_bytes());
                addr[4..8].copy_from_slice(&ip);
                memory
                    .write_bytes(base + 48, &addr)
                    .map_err(|_| WSAEFAULT)?;
                for (offset, value) in [(0u64, 0u32), (4, 2), (8, 2), (12, 17)] {
                    memory
                        .write_u32(base + offset, value)
                        .map_err(|_| WSAEFAULT)?;
                }
                for (offset, value) in [
                    (16u64, 16u64),  // ai_addrlen
                    (24, 0),         // ai_canonname
                    (32, base + 48), // ai_addr
                    (40, 0),         // ai_next
                ] {
                    memory
                        .write_u64(base + offset, value)
                        .map_err(|_| WSAEFAULT)?;
                }
                memory.write_u64(args.r9, base).map_err(|_| WSAEFAULT)?;
                Ok(0)
            })();
            match result {
                Ok(value) => Ok(value),
                Err(error) => Ok(u64::from(error)),
            }
        },
    );

    // freeaddrinfo releases the one allocation the contract defines.
    let fai = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "freeaddrinfo",
        ServiceStatus::Partial,
        move |args, _cpu, memory| {
            let _ = &fai;
            if args.rcx != 0 && !memory.release_heap_allocation(PROCESS_HEAP, args.rcx) {
                // A foreign pointer is misuse the CRT would fault on; the
                // honest surface reports it through the debug contract of
                // a no-op free being impossible. Nothing to return.
                return Ok(0);
            }
            Ok(0)
        },
    );

    // getnameinfo: the reverse form over the same one address.
    let gni = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "getnameinfo",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let _ = cpu;
            let result = (|| {
                if gni.winsock.lock().expect("guest Winsock state").refs == 0 {
                    return Err(WSANOTINITIALISED);
                }
                let mut sa = [0; 16];
                memory
                    .read_bytes(args.rcx, &mut sa)
                    .map_err(|_| WSAEFAULT)?;
                if u16::from_le_bytes([sa[0], sa[1]]) != 2 {
                    return Err(WSAEAFNOSUPPORT);
                }
                let host = format!("{}.{}.{}.{}", sa[4], sa[5], sa[6], sa[7]);
                let port = u16::from_be_bytes([sa[2], sa[3]]).to_string();
                if args.r8 != 0 && args.r9 as u32 as usize > host.len() {
                    memory
                        .write_bytes(args.r8, host.as_bytes())
                        .map_err(|_| WSAEFAULT)?;
                    memory
                        .write_u8(args.r8 + host.len() as u64, 0)
                        .map_err(|_| WSAEFAULT)?;
                }
                let service_out = args.get(4, memory).map_err(|_| WSAEFAULT)?;
                let service_len = args.get(5, memory).map_err(|_| WSAEFAULT)? as u32 as usize;
                if service_out != 0 && service_len > port.len() {
                    memory
                        .write_bytes(service_out, port.as_bytes())
                        .map_err(|_| WSAEFAULT)?;
                    memory
                        .write_u8(service_out + port.len() as u64, 0)
                        .map_err(|_| WSAEFAULT)?;
                }
                Ok(0)
            })();
            match result {
                Ok(value) => Ok(value),
                Err(error) => Ok(u64::from(error)),
            }
        },
    );

    // inet_ntoa formats into one heap allocation reused across calls - the
    // overwritten-static-storage contract of the CRT form.
    registry.register_with_status(
        MODULE,
        "inet_ntoa",
        ServiceStatus::Partial,
        move |args, _cpu, memory| {
            let bytes = (args.rcx as u32).to_le_bytes();
            let text = format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3]);
            // The static buffer is allocated once and reused, matching the
            // CRT contract of storage overwritten by each call.
            static mut NTOA_BASE: u64 = 0;
            let base = unsafe {
                if NTOA_BASE == 0 {
                    let Ok(base) = memory.map_heap_allocation(PROCESS_HEAP, 16) else {
                        return Ok(0);
                    };
                    NTOA_BASE = base;
                }
                NTOA_BASE
            };
            memory
                .write_bytes(base, text.as_bytes())
                .map_err(ExitReason::MemoryFault)?;
            memory
                .write_u8(base + text.len() as u64, 0)
                .map_err(ExitReason::MemoryFault)?;
            Ok(base)
        },
    );
}

// select uses the Winsock fd_set layout: a u32 count followed by the socket
// handles at offset 8, filtered in place. Readiness mirrors WSAPoll.
fn register_select(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    const MAX_SELECT_MS: u64 = 1_000;
    const FD_SETSIZE: usize = 64;

    let select_state = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "select",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let mut net = select_state.winsock.lock().expect("guest Winsock state");
                if net.refs == 0 {
                    return Err(WSANOTINITIALISED);
                }
                let read_set = args.rdx;
                let write_set = args.r8;
                if read_set == 0 && write_set == 0 {
                    return Err(WSAEINVAL);
                }
                // The timeout is seconds and microseconds, null meaning
                // infinite - which the single-executor machine caps like
                // WSAPoll does rather than deadlocking the guest.
                let mut timeout_ms = MAX_SELECT_MS;
                let timeout_address = args.get(4, memory).map_err(|_| WSAEFAULT)?;
                if timeout_address != 0 {
                    let seconds = memory.read_u64(timeout_address).map_err(|_| WSAEFAULT)?;
                    let micros = memory
                        .read_u64(timeout_address + 8)
                        .map_err(|_| WSAEFAULT)?;
                    timeout_ms = (seconds * 1_000 + micros / 1_000).min(MAX_SELECT_MS);
                }
                let read_handles = |memory: &GuestMemory| -> Result<Vec<u64>, u32> {
                    if read_set == 0 {
                        return Ok(Vec::new());
                    }
                    let count = memory.read_u32(read_set).map_err(|_| WSAEFAULT)? as usize;
                    if count > FD_SETSIZE {
                        return Err(WSAEINVAL);
                    }
                    let mut handles = Vec::with_capacity(count);
                    for index in 0..count {
                        handles.push(
                            memory
                                .read_u64(read_set + 8 + index as u64 * 8)
                                .map_err(|_| WSAEFAULT)?,
                        );
                    }
                    Ok(handles)
                };
                let write_handles = |memory: &GuestMemory| -> Result<Vec<u64>, u32> {
                    if write_set == 0 {
                        return Ok(Vec::new());
                    }
                    let count = memory.read_u32(write_set).map_err(|_| WSAEFAULT)? as usize;
                    if count > FD_SETSIZE {
                        return Err(WSAEINVAL);
                    }
                    let mut handles = Vec::with_capacity(count);
                    for index in 0..count {
                        handles.push(
                            memory
                                .read_u64(write_set + 8 + index as u64 * 8)
                                .map_err(|_| WSAEFAULT)?,
                        );
                    }
                    Ok(handles)
                };
                let reads = read_handles(memory)?;
                let writes = write_handles(memory)?;
                if read_set != 0 {
                    memory
                        .check_write(read_set, 8 + reads.len() * 8)
                        .map_err(|_| WSAEFAULT)?;
                }
                if write_set != 0 {
                    memory
                        .check_write(write_set, 8 + writes.len() * 8)
                        .map_err(|_| WSAEFAULT)?;
                }
                let deadline = Instant::now() + Duration::from_millis(timeout_ms);
                let (ready_reads, ready_writes) = loop {
                    let readable = |net: &Winsock, handle: u64| {
                        net.sockets.get(&handle).is_some_and(|socket| {
                            !socket.pending.is_empty() && !socket.shutdown_receive
                        })
                    };
                    let writable = |net: &Winsock, handle: u64| {
                        net.sockets.get(&handle).is_some_and(|socket| {
                            !socket.shutdown_send
                                && socket.peer.is_none_or(|peer| {
                                    !net.sockets.values().any(|target| {
                                        target.port == Some(peer)
                                            && target.pending.len() == MAX_PENDING_DATAGRAMS
                                    })
                                })
                        })
                    };
                    let ready_reads: Vec<u64> = reads
                        .iter()
                        .copied()
                        .filter(|&h| readable(&net, h))
                        .collect();
                    let ready_writes: Vec<u64> = writes
                        .iter()
                        .copied()
                        .filter(|&h| writable(&net, h))
                        .collect();
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if !ready_reads.is_empty() || !ready_writes.is_empty() || remaining.is_zero() {
                        break (ready_reads, ready_writes);
                    }
                    (net, _) = select_state
                        .winsock_changed
                        .wait_timeout(net, remaining)
                        .expect("guest Winsock state");
                };
                // Each set reports only its ready sockets, in place.
                if read_set != 0 {
                    memory
                        .write_u32(read_set, ready_reads.len() as u32)
                        .map_err(|_| WSAEFAULT)?;
                    for (index, handle) in ready_reads.iter().enumerate() {
                        memory
                            .write_u64(read_set + 8 + index as u64 * 8, *handle)
                            .map_err(|_| WSAEFAULT)?;
                    }
                }
                if write_set != 0 {
                    memory
                        .write_u32(write_set, ready_writes.len() as u32)
                        .map_err(|_| WSAEFAULT)?;
                    for (index, handle) in ready_writes.iter().enumerate() {
                        memory
                            .write_u64(write_set + 8 + index as u64 * 8, *handle)
                            .map_err(|_| WSAEFAULT)?;
                    }
                }
                Ok((ready_reads.len() + ready_writes.len()) as u64)
            })();
            socket_result(&select_state, cpu.gs_base, result)
        },
    );
}

fn register_socket_names(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    for (symbol, peer) in [("getsockname", false), ("getpeername", true)] {
        let state = Arc::clone(state);
        registry.register_with_status(
            MODULE,
            symbol,
            ServiceStatus::Partial,
            move |args, cpu, memory| {
                let result = (|| {
                    let net = state.winsock.lock().expect("guest Winsock state");
                    let socket = net.socket(args.rcx)?;
                    let port = if peer {
                        socket.peer.ok_or(WSAENOTCONN)?
                    } else {
                        socket.port.ok_or(WSAEINVAL)?
                    };
                    if args.rdx == 0 || args.r8 == 0 {
                        return Err(WSAEFAULT);
                    }
                    let length = memory.read_u32(args.r8).map_err(|_| WSAEFAULT)? as i32;
                    let mut address = [0; 16];
                    if length < address.len() as i32 {
                        return Err(WSAEFAULT);
                    }
                    memory
                        .check_write(args.rdx, address.len())
                        .map_err(|_| WSAEFAULT)?;
                    memory.check_write(args.r8, 4).map_err(|_| WSAEFAULT)?;
                    address[..2].copy_from_slice(&2_u16.to_le_bytes());
                    address[2..4].copy_from_slice(&port.to_be_bytes());
                    address[4..8].copy_from_slice(&[127, 0, 0, 1]);
                    memory
                        .write_bytes(args.rdx, &address)
                        .map_err(|_| WSAEFAULT)?;
                    memory
                        .write_u32(args.r8, address.len() as u32)
                        .map_err(|_| WSAEFAULT)?;
                    Ok(0)
                })();
                socket_result(&state, cpu.gs_base, result)
            },
        );
    }
}

fn register_socket_options(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    const SOL_SOCKET: u32 = 0xffff;
    const SO_TYPE: u32 = 0x1008;
    const SO_MAX_MSG_SIZE: u32 = 0x2003;
    const SOCK_DGRAM: u32 = 2;

    let state = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "getsockopt",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let net = state.winsock.lock().expect("guest Winsock state");
                net.socket(args.rcx)?;
                let value = match (args.rdx as u32, args.r8 as u32) {
                    (SOL_SOCKET, SO_TYPE) => SOCK_DGRAM,
                    (SOL_SOCKET, SO_MAX_MSG_SIZE) => MAX_DATAGRAM as u32,
                    _ => return Err(WSAENOPROTOOPT),
                };
                let length = args.get(4, memory).map_err(|_| WSAEFAULT)?;
                if args.r9 == 0 || length == 0 {
                    return Err(WSAEFAULT);
                }
                if (memory.read_u32(length).map_err(|_| WSAEFAULT)? as i32) < 4 {
                    return Err(WSAEFAULT);
                }
                memory.check_write(args.r9, 4).map_err(|_| WSAEFAULT)?;
                memory.check_write(length, 4).map_err(|_| WSAEFAULT)?;
                memory.write_u32(args.r9, value).map_err(|_| WSAEFAULT)?;
                memory.write_u32(length, 4).map_err(|_| WSAEFAULT)?;
                Ok(0)
            })();
            socket_result(&state, cpu.gs_base, result)
        },
    );
}

fn register_poll(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    const MAX_POLL_MS: i32 = 1_000;
    const POLLFD_SIZE: usize = 16;
    const POLLNVAL: u16 = 0x0004;
    const POLLWRNORM: u16 = 0x0010;
    const POLLRDNORM: u16 = 0x0100;
    const POLLRDBAND: u16 = 0x0200;

    let state = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WSAPoll",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let result = (|| {
                let mut net = state.winsock.lock().expect("guest Winsock state");
                if net.refs == 0 {
                    return Err(WSANOTINITIALISED);
                }
                let count = args.rdx as u32 as usize;
                if !(1..=MAX_SOCKETS).contains(&count) {
                    return Err(WSAEINVAL);
                }
                let timeout = args.r8 as i32;
                if !(0..=MAX_POLL_MS).contains(&timeout) {
                    return Err(WSAEOPNOTSUPP);
                }
                if args.rcx == 0 {
                    return Err(WSAEFAULT);
                }
                let mut storage = [0; MAX_SOCKETS * POLLFD_SIZE];
                let descriptors = &mut storage[..count * POLLFD_SIZE];
                memory
                    .read_bytes(args.rcx, descriptors)
                    .map_err(|_| WSAEFAULT)?;
                memory
                    .check_write(args.rcx, descriptors.len())
                    .map_err(|_| WSAEFAULT)?;
                let deadline = Instant::now() + Duration::from_millis(timeout as u64);
                let ready = loop {
                    if net.refs == 0 {
                        return Err(WSANOTINITIALISED);
                    }
                    let mut ready = 0;
                    for fd in descriptors.as_chunks_mut::<POLLFD_SIZE>().0 {
                        let handle = u64::from_le_bytes(fd[..8].try_into().expect("socket handle"));
                        let events = u16::from_le_bytes([fd[8], fd[9]]);
                        if events & !(POLLWRNORM | POLLRDNORM | POLLRDBAND) != 0 {
                            return Err(WSAEINVAL);
                        }
                        let mut result = POLLNVAL;
                        if let Some(socket) = net.sockets.get(&handle) {
                            result = 0;
                            if !socket.pending.is_empty() {
                                result |= events & POLLRDNORM;
                            }
                            if let Some(peer) = socket.peer
                                && !net.sockets.values().any(|target| {
                                    target.port == Some(peer)
                                        && target.peer.is_none_or(|port| Some(port) == socket.port)
                                        && target.pending.len() == MAX_PENDING_DATAGRAMS
                                })
                            {
                                result |= events & POLLWRNORM;
                            }
                        }
                        fd[10..12].copy_from_slice(&result.to_le_bytes());
                        ready += u64::from(result != 0);
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if ready != 0 || remaining.is_zero() {
                        break ready;
                    }
                    #[cfg(test)]
                    {
                        net.poll_waiters += 1;
                    }
                    (net, _) = state
                        .winsock_changed
                        .wait_timeout(net, remaining)
                        .expect("guest Winsock state");
                    #[cfg(test)]
                    {
                        net.poll_waiters -= 1;
                    }
                };
                memory
                    .write_bytes(args.rcx, descriptors)
                    .map_err(|_| WSAEFAULT)?;
                Ok(ready)
            })();
            socket_result(&state, cpu.gs_base, result)
        },
    );
}

#[cfg(test)]
#[test]
fn loopback_datagrams_preserve_boundaries_and_reject_external_addresses() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 4096, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let server = call(&mut memory, "socket", &[2, 2, 17]);
    let client = call(&mut memory, "socket", &[2, 2, 17]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[server, 0x1200, 16]), 0);
    assert_eq!(
        call(&mut memory, "bind", &[client, 0x1200, 16]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEADDRINUSE)
    );
    memory.write_u8(0x1204, 192).unwrap();
    assert_eq!(
        call(&mut memory, "connect", &[client, 0x1200, 16]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEACCES)
    );
    memory.write_u8(0x1204, 127).unwrap();
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);
    memory.write_bytes(0x1300, b"hello").unwrap();
    assert_eq!(call(&mut memory, "send", &[client, 0x1300, 5, 0]), 5);
    assert_eq!(call(&mut memory, "send", &[client, 0x1300, 2, 0]), 2);
    assert_eq!(
        call(&mut memory, "recv", &[server, 0xff, 4, 0]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );
    assert_eq!(
        call(&mut memory, "recv", &[server, 0x1400, 4, 0]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEMSGSIZE)
    );
    let mut received = [0; 4];
    memory.read_bytes(0x1400, &mut received).unwrap();
    assert_eq!(&received, b"hell");
    assert_eq!(call(&mut memory, "recv", &[server, 0x1400, 4, 0]), 2);
    assert_eq!(
        call(&mut memory, "recv", &[server, 0x1400, 4, 0]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEWOULDBLOCK)
    );

    let mut polling_memory = memory.clone();
    polling_memory.write_u64(0x1500, server).unwrap();
    polling_memory.write_u16(0x1508, 0x0100).unwrap();
    std::thread::scope(|scope| {
        let (finished, completion) = std::sync::mpsc::channel();
        let polling_memory = &mut polling_memory;
        scope.spawn(move || {
            let ready = call(polling_memory, "WSAPoll", &[0x1500, 1, 1000]);
            finished.send(ready).unwrap();
        });
        let deadline = Instant::now() + Duration::from_millis(500);
        while state.winsock.lock().unwrap().poll_waiters == 0 {
            assert!(Instant::now() < deadline, "poll did not enter its wait");
            std::thread::yield_now();
        }
        assert_eq!(call(&mut memory, "send", &[client, 0x1300, 5, 0]), 5);
        assert_eq!(
            completion.recv_timeout(Duration::from_millis(400)).unwrap(),
            1
        );
    });
    assert_eq!(polling_memory.read_u16(0x150a).unwrap(), 0x0100);
}

#[cfg(test)]
#[test]
fn sendto_recvfrom_hostname_and_inet_addr() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        // Parameters past the fourth arrive on the stack per the Win64
        // calling convention.
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let server = call(&mut memory, "socket", &[2, 2, 17]);
    let client = call(&mut memory, "socket", &[2, 2, 17]);
    // Bind the server; the client stays unbound and auto-binds on sendto.
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[server, 0x1200, 16]), 0);
    memory.write_bytes(0x1300, b"ping").unwrap();
    // sendto with the destination in the stack parameters.
    assert_eq!(
        call(&mut memory, "sendto", &[client, 0x1300, 4, 0, 0x1200, 16]),
        4
    );
    // The legacy ioctl reports the first pending datagram's size.
    const FIONREAD: u64 = 0x4004_667F;
    assert_eq!(
        call(&mut memory, "ioctlsocket", &[server, FIONREAD, 0x1504]),
        0
    );
    assert_eq!(memory.read_u32(0x1504).unwrap(), 4);
    // recvfrom reports the source address of the datagram.
    // A too-small source output is rejected before the pending datagram is
    // consumed; correcting the capacity receives the same message.
    memory.write_u32(0x1500, 8).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "recvfrom",
            &[server, 0x1400, 8, 0, 0x1600, 0x1500]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );
    memory.write_u32(0x1500, 16).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "recvfrom",
            &[server, 0x1400, 8, 0, 0x1600, 0x1500]
        ),
        4
    );
    let mut received = [0; 4];
    memory.read_bytes(0x1400, &mut received).unwrap();
    assert_eq!(&received, b"ping");
    let mut source = [0; 8];
    memory.read_bytes(0x1600, &mut source).unwrap();
    assert_eq!(&source[..2], &2_u16.to_le_bytes(), "AF_INET source");
    assert_eq!(&source[4..8], &[127, 0, 0, 1]);
    // The auto-bound client port is ephemeral.
    let source_port = u16::from_be_bytes([source[2], source[3]]);
    assert!(source_port >= 49152);
    // gethostname writes the fixed machine name.
    assert_eq!(call(&mut memory, "gethostname", &[0x1700, 32]), 0);
    let mut name = [0; 7];
    memory.read_bytes(0x1700, &mut name).unwrap();
    assert_eq!(&name, b"XENORA\0");
    // inet_addr parses dotted decimal into network order, or all-ones on
    // a failure - the documented ambiguity.
    memory.write_bytes(0x1800, b"127.0.0.1\0").unwrap();
    assert_eq!(
        call(&mut memory, "inet_addr", &[0x1800]),
        u64::from(u32::from_le_bytes([127, 0, 0, 1]))
    );
    memory.write_bytes(0x1800, b"not-an-address\0").unwrap();
    assert_eq!(
        call(&mut memory, "inet_addr", &[0x1800]),
        u64::from(u32::MAX)
    );
}

#[cfg(test)]
#[test]
fn select_readiness_shutdown_and_socket_options() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let server = call(&mut memory, "socket", &[2, 2, 17]);
    let client = call(&mut memory, "socket", &[2, 2, 17]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[server, 0x1200, 16]), 0);
    // select on an idle socket times out to zero ready within the cap.
    memory.write_bytes(0x1800, b"go").unwrap();
    // The read set: count 1, then the server handle at offset 8.
    memory.write_u32(0x1900, 1).unwrap();
    memory.write_u64(0x1908, server).unwrap();
    memory.write_u64(0x1a00, 0).unwrap(); // timeout: 0 sec 0 usec = poll once
    memory.write_u64(0x1a08, 0).unwrap();
    assert_eq!(call(&mut memory, "select", &[0, 0x1900, 0, 0, 0x1a00]), 0);
    // A datagram arriving makes the server readable.
    assert_eq!(
        call(&mut memory, "sendto", &[client, 0x1800, 2, 0, 0x1200, 16]),
        2
    );
    memory.write_u32(0x1900, 1).unwrap();
    memory.write_u64(0x1908, server).unwrap();
    assert_eq!(call(&mut memory, "select", &[0, 0x1900, 0, 0, 0x1a00]), 1);
    assert_eq!(
        memory.read_u32(0x1900).unwrap(),
        1,
        "one ready handle reported"
    );
    assert_eq!(memory.read_u64(0x1908).unwrap(), server);
    // shutdown requires a connected socket.
    assert_eq!(call(&mut memory, "shutdown", &[server, 2]), SOCKET_ERROR);
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAENOTCONN)
    );
    // Connect the client, then shutdown both directions.
    memory.write_u8(0x1202, 0x13).unwrap();
    memory.write_u8(0x1203, 0x88).unwrap();
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "shutdown", &[client, 1]), 0);
    memory.write_bytes(0x1800, b"xx").unwrap();
    assert_eq!(
        call(&mut memory, "send", &[client, 0x1800, 2, 0]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAESHUTDOWN)
    );
    // setsockopt stores the vacuous options and refuses the rest.
    memory.write_u32(0x1b00, 1).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "setsockopt",
            &[client, 0xffff, 0x20, 0x1b00, 4]
        ),
        0
    );
    assert_eq!(
        call(
            &mut memory,
            "setsockopt",
            &[client, 0xffff, 0x9999, 0x1b00, 4]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAENOPROTOOPT)
    );
}

#[cfg(test)]
#[test]
fn wsasocket_adapter_fdisset_and_address_formatting() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    // WSASocketA with zero flags creates; nonzero refuses honestly.
    let handle = call(&mut memory, "WSASocketA", &[2, 2, 17, 0, 0, 0]);
    assert!(handle != INVALID_SOCKET && handle != 0);
    assert_eq!(
        call(&mut memory, "WSASocketA", &[2, 2, 17, 1, 0, 0]),
        INVALID_SOCKET
    );
    // __WSAFDIsSet scans the count-prefixed array.
    memory.write_u32(0x1900, 2).unwrap();
    memory.write_u64(0x1908, 111).unwrap();
    memory.write_u64(0x1910, handle).unwrap();
    assert_eq!(call(&mut memory, "__WSAFDIsSet", &[handle, 0x1900]), 1);
    assert_eq!(call(&mut memory, "__WSAFDIsSet", &[222, 0x1900]), 0);
    // WSAAddressToStringA formats family, dotted decimal and port.
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    memory.write_u32(0x1a00, 64).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSAAddressToStringA",
            &[0x1200, 16, 0, 0x1b00, 0x1a00]
        ),
        0
    );
    let mut text = [0; 14];
    memory.read_bytes(0x1b00, &mut text).unwrap();
    assert_eq!(&text, b"127.0.0.1:5000");
    assert_eq!(memory.read_u32(0x1a00).unwrap(), 15);

    // The Unicode form uses WCHAR units, not bytes, and reports the required
    // count even when the capacity is insufficient.
    memory.write_u32(0x1a00, 15).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSAAddressToStringW",
            &[0x1200, 16, 0, 0x1c00, 0x1a00]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1a00).unwrap(), 15);
    for (index, unit) in "127.0.0.1:5000".encode_utf16().chain([0]).enumerate() {
        assert_eq!(memory.read_u16(0x1c00 + index as u64 * 2).unwrap(), unit);
    }
    memory.write_u32(0x1a00, 14).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSAAddressToStringW",
            &[0x1200, 16, 0, 0x1c00, 0x1a00]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );
    assert_eq!(memory.read_u32(0x1a00).unwrap(), 15);
    for args in [
        [0x1200, 15, 0, 0x1c00, 0x1a00],
        [0x1200, 16, 1, 0x1c00, 0x1a00],
    ] {
        memory.write_u32(0x1a00, 15).unwrap();
        assert_eq!(
            call(&mut memory, "WSAAddressToStringW", &args),
            SOCKET_ERROR
        );
        assert_eq!(
            call(&mut memory, "WSAGetLastError", &[]),
            u64::from(WSAEINVAL)
        );
    }
}

#[cfg(test)]
#[test]
fn loopback_name_resolution_round_trips() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    // localhost:80 resolves to one addrinfo over the loopback address.
    memory.write_bytes(0x1100, b"localhost\0").unwrap();
    memory.write_bytes(0x1120, b"80\0").unwrap();
    assert_eq!(
        call(&mut memory, "getaddrinfo", &[0x1100, 0x1120, 0, 0x1140]),
        0
    );
    let info = memory.read_u64(0x1140).unwrap();
    assert_ne!(info, 0);
    assert_eq!(memory.read_u64(info + 4).unwrap() as u32, 2, "AF_INET");
    let sockaddr = memory.read_u64(info + 32).unwrap();
    let port = u16::from(memory.read_u8(sockaddr + 2).unwrap()) * 256
        + u16::from(memory.read_u8(sockaddr + 3).unwrap());
    assert_eq!(port, 80);
    assert_eq!(memory.read_u8(sockaddr + 4).unwrap(), 127);
    // A name that is not the machine's one address is an honest no-name.
    memory.write_bytes(0x1100, b"example.com\0").unwrap();
    assert_ne!(call(&mut memory, "getaddrinfo", &[0x1100, 0, 0, 0x1140]), 0);
    // freeaddrinfo releases the allocation without fault.
    assert_eq!(call(&mut memory, "freeaddrinfo", &[info]), 0);
    // getnameinfo reverses the same sockaddr.
    let mut sa = [0; 16];
    sa[..8].copy_from_slice(&[2, 0, 0x00, 0x50, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &sa).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "getnameinfo",
            &[0x1200, 16, 0x1300, 32, 0x1340, 16, 0]
        ),
        0
    );
    let mut host = [0; 10];
    memory.read_bytes(0x1300, &mut host).unwrap();
    assert_eq!(&host[..10], b"127.0.0.1\0");
    let mut service = [0; 3];
    memory.read_bytes(0x1340, &mut service).unwrap();
    assert_eq!(&service, b"80\0");
    // inet_ntoa formats into reusable storage.
    let first = call(
        &mut memory,
        "inet_ntoa",
        &[u64::from(u32::from_le_bytes([127, 0, 0, 1]))],
    );
    assert_ne!(first, 0);
    let mut text = [0; 10];
    memory.read_bytes(first, &mut text).unwrap();
    assert_eq!(&text[..10], b"127.0.0.1\0");
}

#[cfg(test)]
#[test]
fn loopback_streams_connect_accept_exchange_and_close() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    // Stream sockets create; the listener binds and listens.
    let listener = call(&mut memory, "socket", &[2, 1, 6]);
    assert!(listener != INVALID_SOCKET && listener != 0);
    let client = call(&mut memory, "socket", &[2, 1, 6]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x1F, 0x90, 127, 0, 0, 1]); // port 8080
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[listener, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "listen", &[listener, 4]), 0);
    // connect before listen was impossible; now it pairs and queues.
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);
    // accept dequeues the server endpoint and reports the client.
    memory.write_u32(0x1300, 16).unwrap();
    let server = call(&mut memory, "accept", &[listener, 0x1400, 0x1300]);
    assert!(server != INVALID_SOCKET && server != 0 && server != client);
    let mut peer = [0; 8];
    memory.read_bytes(0x1400, &mut peer).unwrap();
    assert_eq!(&peer[4..8], &[127, 0, 0, 1]);
    // A second accept on the empty queue is would-block.
    assert_eq!(
        call(&mut memory, "accept", &[listener, 0, 0]),
        INVALID_SOCKET
    );
    // Byte streams flow both ways without datagram boundaries.
    memory.write_bytes(0x1500, b"hello ").unwrap();
    assert_eq!(call(&mut memory, "send", &[client, 0x1500, 6, 0]), 6);
    memory.write_bytes(0x1500, b"stream").unwrap();
    assert_eq!(call(&mut memory, "send", &[client, 0x1500, 6, 0]), 6);
    // Two sends merge into one byte fifo: a 12-byte read gets both.
    assert_eq!(call(&mut memory, "recv", &[server, 0x1600, 12, 0]), 12);
    let mut received = [0; 12];
    memory.read_bytes(0x1600, &mut received).unwrap();
    assert_eq!(&received, b"hello stream");
    // A partial read consumes only what it reads.
    memory.write_bytes(0x1500, b"partial!").unwrap();
    assert_eq!(call(&mut memory, "send", &[client, 0x1500, 8, 0]), 8);
    assert_eq!(call(&mut memory, "recv", &[server, 0x1600, 3, 0]), 3);
    assert_eq!(call(&mut memory, "recv", &[server, 0x1600, 5, 0]), 5);
    // Server-to-client works symmetrically.
    memory.write_bytes(0x1500, b"reply").unwrap();
    assert_eq!(call(&mut memory, "send", &[server, 0x1500, 5, 0]), 5);
    assert_eq!(call(&mut memory, "recv", &[client, 0x1600, 5, 0]), 5);
    // Closing the client signals eof: the server's recv returns zero.
    assert_eq!(call(&mut memory, "closesocket", &[client]), 0);
    assert_eq!(call(&mut memory, "recv", &[server, 0x1600, 5, 0]), 0);
    // Connecting where nothing listens is refused.
    memory.write_bytes(0x1200, &address).unwrap();
    memory.write_u8(0x1202, 0x1F).unwrap();
    memory.write_u8(0x1203, 0x91).unwrap(); // port 8081
    let orphan = call(&mut memory, "socket", &[2, 1, 6]);
    assert_eq!(
        call(&mut memory, "connect", &[orphan, 0x1200, 16]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAECONNREFUSED)
    );
}

#[cfg(test)]
#[test]
fn wsasend_and_wsarecv_gather_scatter_stream_bytes() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    let wsabuf = |memory: &mut GuestMemory, base: u64, entries: &[(u64, u32)]| {
        for (index, &(pointer, length)) in entries.iter().enumerate() {
            let entry = base + index as u64 * 16;
            memory.write_u32(entry, length).unwrap();
            memory.write_u64(entry + 8, pointer).unwrap();
        }
    };

    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let listener = call(&mut memory, "socket", &[2, 1, 6]);
    let client = call(&mut memory, "socket", &[2, 1, 6]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x1F, 0x92, 127, 0, 0, 1]); // port 8082
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[listener, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "listen", &[listener, 4]), 0);
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);
    memory.write_u32(0x1300, 16).unwrap();
    let server = call(&mut memory, "accept", &[listener, 0, 0x1300]);

    memory.write_bytes(0x1500, b"hel").unwrap();
    memory.write_bytes(0x1520, b"lo stream").unwrap();
    wsabuf(&mut memory, 0x1700, &[(0x1500, 3), (0, 0), (0x1520, 9)]);
    assert_eq!(
        call(
            &mut memory,
            "WSASend",
            &[client, 0x1700, 3, 0x1800, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 12);

    wsabuf(&mut memory, 0x1780, &[(0x1900, 5), (0x1910, 7)]);
    memory.write_u32(0x1840, 0).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[server, 0x1780, 2, 0x1800, 0x1840, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 12);
    assert_eq!(memory.read_u32(0x1840).unwrap(), 0);
    let mut first = [0; 5];
    let mut second = [0; 7];
    memory.read_bytes(0x1900, &mut first).unwrap();
    memory.read_bytes(0x1910, &mut second).unwrap();
    assert_eq!(&first, b"hello");
    assert_eq!(&second, b" stream");

    // Zero-length gathers complete without making the peer readable.
    wsabuf(&mut memory, 0x1700, &[(0, 0)]);
    assert_eq!(
        call(
            &mut memory,
            "WSASend",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 0);
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[server, 0x1780, 2, 0x1800, 0x1840, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEWOULDBLOCK)
    );

    // A full flow-control window accepts only the room that remains.
    memory.write_bytes(0x1500, &[0x5a; 1024]).unwrap();
    assert_eq!(call(&mut memory, "send", &[client, 0x1500, 10, 0]), 10);
    for index in 0..64_u64 {
        let entry = 0x1700 + index * 16;
        memory.write_u32(entry, 1024).unwrap();
        memory.write_u64(entry + 8, 0x1500).unwrap();
    }
    assert_eq!(
        call(
            &mut memory,
            "WSASend",
            &[client, 0x1700, 64, 0x1800, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), STREAM_WINDOW as u32 - 10);
}

#[cfg(test)]
#[test]
fn wsasend_and_wsarecv_preserve_datagram_boundaries() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    let wsabuf = |memory: &mut GuestMemory, base: u64, entries: &[(u64, u32)]| {
        for (index, &(pointer, length)) in entries.iter().enumerate() {
            let entry = base + index as u64 * 16;
            memory.write_u32(entry, length).unwrap();
            memory.write_u64(entry + 8, pointer).unwrap();
        }
    };

    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let server = call(&mut memory, "socket", &[2, 2, 17]);
    let client = call(&mut memory, "socket", &[2, 2, 17]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[server, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);

    memory.write_bytes(0x1500, b"pi").unwrap();
    memory.write_bytes(0x1510, b"ng").unwrap();
    memory.write_bytes(0x1520, b"!").unwrap();
    wsabuf(
        &mut memory,
        0x1700,
        &[(0x1500, 2), (0, 0), (0x1510, 2), (0x1520, 1)],
    );
    assert_eq!(
        call(
            &mut memory,
            "WSASend",
            &[client, 0x1700, 4, 0x1800, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 5);
    wsabuf(
        &mut memory,
        0x1780,
        &[(0x1900, 2), (0x1910, 2), (0x1920, 2)],
    );
    memory.write_u32(0x1840, 0).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[server, 0x1780, 3, 0x1800, 0x1840, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 5);
    assert_eq!(memory.read_u32(0x1840).unwrap(), 0);
    let mut first = [0; 2];
    let mut middle = [0; 2];
    let mut last = [0; 1];
    memory.read_bytes(0x1900, &mut first).unwrap();
    memory.read_bytes(0x1910, &mut middle).unwrap();
    memory.read_bytes(0x1920, &mut last).unwrap();
    assert_eq!(&first, b"pi");
    assert_eq!(&middle, b"ng");
    assert_eq!(&last, b"!");

    // Truncation copies the prefix, discards the datagram remainder and
    // reports WSAEMSGSIZE rather than blending bytes into the next datagram.
    memory.write_bytes(0x1500, b"abcdef").unwrap();
    wsabuf(&mut memory, 0x1700, &[(0x1500, 6)]);
    assert_eq!(
        call(
            &mut memory,
            "WSASend",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0]
        ),
        0
    );
    wsabuf(&mut memory, 0x1780, &[(0x1900, 2), (0x1910, 1)]);
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[server, 0x1780, 2, 0x1800, 0x1840, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEMSGSIZE)
    );
    memory.read_bytes(0x1900, &mut first).unwrap();
    memory.read_bytes(0x1910, &mut last[..1]).unwrap();
    assert_eq!(&first, b"ab");
    assert_eq!(&last[..1], b"c");
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[server, 0x1780, 2, 0x1800, 0x1840, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEWOULDBLOCK)
    );

    // A zero-length datagram remains a boundary and completes with zero bytes.
    wsabuf(&mut memory, 0x1700, &[(0, 0)]);
    assert_eq!(
        call(
            &mut memory,
            "WSASend",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 0);
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[server, 0x1780, 2, 0x1800, 0x1840, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 0);
}

#[cfg(test)]
#[test]
fn wsasend_and_wsarecv_reject_unsupported_forms_and_bad_inputs() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let server = call(&mut memory, "socket", &[2, 2, 17]);
    let client = call(&mut memory, "socket", &[2, 2, 17]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[server, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);
    memory.write_bytes(0x1500, b"x").unwrap();
    memory.write_u32(0x1700, 1).unwrap();
    memory.write_u64(0x1708, 0x1500).unwrap();
    memory.write_u32(0x1840, 0).unwrap();

    for (symbol, args, error) in [
        (
            "WSASend",
            [client, 0x1700, 1, 0x1800, 1, 0, 0],
            WSAEOPNOTSUPP,
        ),
        (
            "WSASend",
            [client, 0x1700, 1, 0x1800, 0, 1, 0],
            WSAEOPNOTSUPP,
        ),
        (
            "WSASend",
            [client, 0x1700, 1, 0x1800, 0, 0, 1],
            WSAEOPNOTSUPP,
        ),
        ("WSASend", [client, 0x1700, 0, 0x1800, 0, 0, 0], WSAEINVAL),
        ("WSASend", [client, 0x1700, 65, 0x1800, 0, 0, 0], WSAEINVAL),
        ("WSASend", [client, 0, 1, 0x1800, 0, 0, 0], WSAEFAULT),
        ("WSASend", [client, 0x1700, 1, 0, 0, 0, 0], WSAEFAULT),
        ("WSARecv", [server, 0x1700, 1, 0x1800, 0, 0, 0], WSAEFAULT),
    ] {
        assert_eq!(
            call(&mut memory, symbol, &args),
            SOCKET_ERROR,
            "{symbol} {args:?}"
        );
        assert_eq!(call(&mut memory, "WSAGetLastError", &[]), u64::from(error));
    }

    memory.write_u32(0x1840, 1).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[server, 0x1700, 1, 0x1800, 0x1840, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEOPNOTSUPP)
    );
    memory.write_u32(0x1840, 0).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[server, 0x1700, 1, 0x1800, 0x1840, 1, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEOPNOTSUPP)
    );

    memory.write_u32(0x1700, 1).unwrap();
    memory.write_u64(0x1708, 0).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSASend",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );
    memory.write_u64(0x1708, 0x1500).unwrap();
    assert_eq!(
        call(&mut memory, "WSASend", &[9999, 0x1700, 1, 0x1800, 0, 0, 0]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAENOTSOCK)
    );

    assert_eq!(call(&mut memory, "shutdown", &[client, 1]), 0);
    memory.write_u64(0x1708, 0x1500).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSASend",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAESHUTDOWN)
    );
    assert_eq!(call(&mut memory, "shutdown", &[client, 0]), 0);
    assert_eq!(
        call(
            &mut memory,
            "WSARecv",
            &[client, 0x1700, 1, 0x1800, 0x1840, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAESHUTDOWN)
    );
}

#[cfg(test)]
#[test]
fn wsasendto_and_wsarecvfrom_gather_scatter_loopback_datagrams() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    let wsabuf = |memory: &mut GuestMemory, base: u64, entries: &[(u64, u32)]| {
        for (index, &(pointer, length)) in entries.iter().enumerate() {
            let entry = base + index as u64 * 16;
            memory.write_u32(entry, length).unwrap();
            memory.write_u64(entry + 8, pointer).unwrap();
        }
    };

    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let server = call(&mut memory, "socket", &[2, 2, 17]);
    let client = call(&mut memory, "socket", &[2, 2, 17]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[server, 0x1200, 16]), 0);

    memory.write_bytes(0x1500, b"pi").unwrap();
    memory.write_bytes(0x1510, b"ng").unwrap();
    memory.write_bytes(0x1520, b"!").unwrap();
    wsabuf(
        &mut memory,
        0x1700,
        &[(0x1500, 2), (0, 0), (0x1510, 2), (0x1520, 1)],
    );
    // The unbound client auto-binds an ephemeral port on its first send.
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 4, 0x1800, 0, 0x1200, 16, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 5);

    wsabuf(
        &mut memory,
        0x1780,
        &[(0x1900, 2), (0x1910, 2), (0x1920, 2)],
    );
    memory.write_u32(0x1840, 0).unwrap();
    memory.write_u32(0x1a20, 16).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1780, 3, 0x1800, 0x1840, 0x1a00, 0x1a20, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 5);
    assert_eq!(memory.read_u32(0x1840).unwrap(), 0);
    assert_eq!(memory.read_u32(0x1a20).unwrap(), 16);
    let mut first = [0; 2];
    let mut middle = [0; 2];
    let mut last = [0; 1];
    memory.read_bytes(0x1900, &mut first).unwrap();
    memory.read_bytes(0x1910, &mut middle).unwrap();
    memory.read_bytes(0x1920, &mut last).unwrap();
    assert_eq!(&first, b"pi");
    assert_eq!(&middle, b"ng");
    assert_eq!(&last, b"!");
    let mut source = [0; 8];
    memory.read_bytes(0x1a00, &mut source).unwrap();
    assert_eq!(&source[..2], &2_u16.to_le_bytes());
    assert_eq!(&source[4..8], &[127, 0, 0, 1]);
    let source_port = u16::from_be_bytes([source[2], source[3]]);
    assert!(source_port >= 49152, "auto-bound ephemeral source");

    // A null destination uses the connected peer; a null source output is allowed.
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);
    memory.write_bytes(0x1500, b"connected").unwrap();
    wsabuf(&mut memory, 0x1700, &[(0x1500, 9)]);
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 9);
    wsabuf(&mut memory, 0x1780, &[(0x1900, 9)]);
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1780, 1, 0x1800, 0x1840, 0, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 9);
    let mut connected = [0; 9];
    memory.read_bytes(0x1900, &mut connected).unwrap();
    assert_eq!(&connected, b"connected");

    // A zero-length datagram remains a real datagram boundary.
    wsabuf(&mut memory, 0x1700, &[(0, 0)]);
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0, 0, 0]
        ),
        0
    );
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1780, 1, 0x1800, 0x1840, 0, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 0);

    // Truncation copies the prefix, discards the remainder and never blends
    // the next datagram into this receive.
    memory.write_bytes(0x1500, b"abcdef").unwrap();
    wsabuf(&mut memory, 0x1700, &[(0x1500, 6)]);
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0, 0, 0]
        ),
        0
    );
    wsabuf(&mut memory, 0x1780, &[(0x1900, 2), (0x1910, 1)]);
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1780, 2, 0x1800, 0x1840, 0, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEMSGSIZE)
    );
    memory.read_bytes(0x1900, &mut first).unwrap();
    memory.read_bytes(0x1910, &mut last).unwrap();
    assert_eq!(&first, b"ab");
    assert_eq!(&last, b"c");
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1780, 2, 0x1800, 0x1840, 0, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEWOULDBLOCK)
    );
}

#[cfg(test)]
#[test]
fn wsasendto_and_wsarecvfrom_reject_bad_forms_without_losing_datagrams() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let server = call(&mut memory, "socket", &[2, 2, 17]);
    let client = call(&mut memory, "socket", &[2, 2, 17]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[server, 0x1200, 16]), 0);
    memory.write_bytes(0x1500, b"x").unwrap();
    memory.write_u32(0x1700, 1).unwrap();
    memory.write_u64(0x1708, 0x1500).unwrap();
    memory.write_u32(0x1840, 0).unwrap();

    // Null destinations require a connected datagram socket.
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAENOTCONN)
    );
    // Non-loopback destinations and short addresses are rejected exactly.
    memory.write_u8(0x1204, 192).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 1, 0x1800, 0, 0x1200, 16, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEACCES)
    );
    memory.write_u8(0x1204, 127).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 1, 0x1800, 0, 0x1200, 15, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );

    for (args, error) in [
        (
            [client, 0x1700, 1, 0x1800, 1, 0x1200, 16, 0, 0],
            WSAEOPNOTSUPP,
        ),
        (
            [client, 0x1700, 1, 0x1800, 0, 0x1200, 16, 1, 0],
            WSAEOPNOTSUPP,
        ),
        (
            [client, 0x1700, 1, 0x1800, 0, 0x1200, 16, 0, 1],
            WSAEOPNOTSUPP,
        ),
        ([client, 0x1700, 0, 0x1800, 0, 0x1200, 16, 0, 0], WSAEINVAL),
        ([client, 0x1700, 1, 0, 0, 0x1200, 16, 0, 0], WSAEFAULT),
    ] {
        assert_eq!(call(&mut memory, "WSASendTo", &args), SOCKET_ERROR);
        assert_eq!(call(&mut memory, "WSAGetLastError", &[]), u64::from(error));
    }

    // The addressed forms are datagram-only in this bounded implementation.
    let stream = call(&mut memory, "socket", &[2, 1, 6]);
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[stream, 0x1700, 1, 0x1800, 0, 0x1200, 16, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEOPNOTSUPP)
    );
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[stream, 0x1700, 1, 0x1800, 0x1840, 0, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEOPNOTSUPP)
    );

    // Queue one datagram, then prove a too-small source-address output does
    // not consume it.
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 1, 0x1800, 0, 0x1200, 16, 0, 0]
        ),
        0
    );
    memory.write_u32(0x1a20, 8).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1700, 1, 0x1800, 0x1840, 0x1a00, 0x1a20, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );
    memory.write_u32(0x1a20, 16).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1700, 1, 0x1800, 0x1840, 0x1a00, 0x1a20, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 1);
    assert_eq!(memory.read_u32(0x1a20).unwrap(), 16);

    // Unsupported receive forms fail before consuming anything.
    memory.write_u32(0x1840, 1).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1700, 1, 0x1800, 0x1840, 0, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEOPNOTSUPP)
    );
    memory.write_u32(0x1840, 0).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[server, 0x1700, 1, 0x1800, 0x1840, 0, 0, 1, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEOPNOTSUPP)
    );

    // Shutdown state is honored in both directions on a connected socket.
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "shutdown", &[client, 1]), 0);
    assert_eq!(
        call(
            &mut memory,
            "WSASendTo",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAESHUTDOWN)
    );
    assert_eq!(call(&mut memory, "shutdown", &[client, 0]), 0);
    assert_eq!(
        call(
            &mut memory,
            "WSARecvFrom",
            &[client, 0x1700, 1, 0x1800, 0x1840, 0, 0, 0, 0]
        ),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAESHUTDOWN)
    );
}

#[cfg(test)]
#[test]
fn wsaaccept_prevalidates_output_and_matches_legacy_accept() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    let listener = call(&mut memory, "socket", &[2, 1, 6]);
    assert_eq!(call(&mut memory, "bind", &[listener, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "listen", &[listener, 4]), 0);

    // A too-small output is rejected without consuming the pending endpoint.
    let first = call(&mut memory, "socket", &[2, 1, 6]);
    assert_eq!(call(&mut memory, "connect", &[first, 0x1200, 16]), 0);
    memory.write_u32(0x1300, 8).unwrap();
    assert_eq!(
        call(&mut memory, "WSAAccept", &[listener, 0x1400, 0x1300, 0, 0]),
        INVALID_SOCKET
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );
    memory.write_u32(0x1300, 16).unwrap();
    let server = call(&mut memory, "WSAAccept", &[listener, 0x1400, 0x1300, 0, 0]);
    assert_ne!(server, INVALID_SOCKET);
    assert_eq!(memory.read_u32(0x1300).unwrap(), 16);
    let mut peer = [0; 8];
    memory.read_bytes(0x1400, &mut peer).unwrap();
    assert_eq!(&peer[..2], &2_u16.to_le_bytes());
    assert_eq!(&peer[4..8], &[127, 0, 0, 1]);

    // Null condition/callback is the supported subset; legacy accept remains
    // on the same backlog path.
    let second = call(&mut memory, "socket", &[2, 1, 6]);
    assert_eq!(call(&mut memory, "connect", &[second, 0x1200, 16]), 0);
    assert_ne!(
        call(&mut memory, "accept", &[listener, 0, 0]),
        INVALID_SOCKET
    );
    for args in [[listener, 0, 0, 1, 0], [listener, 0, 0, 0, 1]] {
        assert_eq!(call(&mut memory, "WSAAccept", &args), INVALID_SOCKET);
        assert_eq!(
            call(&mut memory, "WSAGetLastError", &[]),
            u64::from(WSAEINVAL)
        );
    }
    assert_eq!(
        call(&mut memory, "WSAAccept", &[listener, 0, 0, 0, 0]),
        INVALID_SOCKET
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEWOULDBLOCK)
    );
}

#[cfg(test)]
#[test]
fn wsaconnect_reuses_loopback_connect_semantics() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x13, 0x88, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();

    // Stream connect reaches the listener exactly like legacy connect.
    let listener = call(&mut memory, "socket", &[2, 1, 6]);
    let stream_client = call(&mut memory, "socket", &[2, 1, 6]);
    assert_eq!(call(&mut memory, "bind", &[listener, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "listen", &[listener, 4]), 0);
    assert_eq!(
        call(
            &mut memory,
            "WSAConnect",
            &[stream_client, 0x1200, 16, 0, 0, 0, 0]
        ),
        0
    );
    assert_ne!(
        call(&mut memory, "accept", &[listener, 0, 0]),
        INVALID_SOCKET
    );

    // Datagram connect preserves the connected-peer send path.
    address[2..4].copy_from_slice(&[0x13, 0x89]);
    memory.write_bytes(0x1200, &address).unwrap();
    let udp_server = call(&mut memory, "socket", &[2, 2, 17]);
    let udp_client = call(&mut memory, "socket", &[2, 2, 17]);
    assert_eq!(call(&mut memory, "bind", &[udp_server, 0x1200, 16]), 0);
    assert_eq!(
        call(
            &mut memory,
            "WSAConnect",
            &[udp_client, 0x1200, 16, 0, 0, 0, 0]
        ),
        0
    );
    memory.write_bytes(0x1500, b"ok").unwrap();
    assert_eq!(call(&mut memory, "send", &[udp_client, 0x1500, 2, 0]), 2);
    assert_eq!(call(&mut memory, "recv", &[udp_server, 0x1600, 2, 0]), 2);

    // Caller/callee data and QoS extensions are outside this bounded subset.
    for args in [
        [udp_client, 0x1200, 16, 1, 0, 0, 0],
        [udp_client, 0x1200, 16, 0, 1, 0, 0],
        [udp_client, 0x1200, 16, 0, 0, 1, 0],
        [udp_client, 0x1200, 16, 0, 0, 0, 1],
    ] {
        assert_eq!(call(&mut memory, "WSAConnect", &args), SOCKET_ERROR);
        assert_eq!(
            call(&mut memory, "WSAGetLastError", &[]),
            u64::from(WSAEINVAL)
        );
    }
}

#[cfg(test)]
#[test]
fn wsasocket_widths_create_bounded_loopback_sockets() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };

    assert_eq!(
        call(&mut memory, "WSASocketA", &[2, 2, 17, 0, 0, 0]),
        INVALID_SOCKET
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSANOTINITIALISED)
    );
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    for symbol in ["WSASocketA", "WSASocketW"] {
        let udp = call(&mut memory, symbol, &[2, 2, 17, 0, 0, 0]);
        let tcp = call(&mut memory, symbol, &[2, 1, 6, 0, 0, 0]);
        assert_ne!(udp, INVALID_SOCKET);
        assert_ne!(tcp, INVALID_SOCKET);
        assert_ne!(udp, tcp);
        for (args, error) in [
            ([3, 2, 17, 0, 0, 0], WSAEAFNOSUPPORT),
            ([2, 2, 6, 0, 0, 0], WSAEPROTONOSUPPORT),
            ([2, 2, 17, 1, 0, 0], WSAEINVAL),
            ([2, 2, 17, 0, 1, 0], WSAEINVAL),
            ([2, 2, 17, 0, 0, 1], WSAEOPNOTSUPP),
        ] {
            assert_eq!(call(&mut memory, symbol, &args), INVALID_SOCKET);
            assert_eq!(call(&mut memory, "WSAGetLastError", &[]), u64::from(error));
        }
    }
}

#[cfg(test)]
#[test]
fn wsa_byteorder_conversions_validate_socket_and_output() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 4096, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    let socket = call(&mut memory, "socket", &[2, 2, 17]);
    for (symbol, input, output, bytes) in [
        ("WSAHtonl", 0x1234_5678_u64, 0x7856_3412_u64, 4_u64),
        ("WSANtohl", 0x7856_3412, 0x1234_5678, 4),
        ("WSAHtons", 0x1234, 0x3412, 2),
        ("WSANtohs", 0x3412, 0x1234, 2),
    ] {
        memory.write_u64(0x1800, 0).unwrap();
        assert_eq!(call(&mut memory, symbol, &[socket, input, 0x1800]), 0);
        assert_eq!(
            memory.read_u64(0x1800).unwrap() & (u64::MAX >> (64 - bytes * 8)),
            output
        );
    }
    assert_eq!(
        call(&mut memory, "WSAHtonl", &[INVALID_SOCKET, 1, 0x1800]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAENOTSOCK)
    );
    assert_eq!(call(&mut memory, "WSAHtons", &[socket, 1, 0]), SOCKET_ERROR);
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );
    assert_eq!(
        call(&mut memory, "WSANtohl", &[socket, 1, 0x6000]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(&mut memory, "WSAGetLastError", &[]),
        u64::from(WSAEFAULT)
    );
}

#[cfg(test)]
#[test]
fn wsioctl_fionread_and_reverse_lookup() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(call(&mut memory, "WSAStartup", &[0x0202, 0x1000]), 0);
    // FIONREAD reports the stream fifo length.
    let listener = call(&mut memory, "socket", &[2, 1, 6]);
    let client = call(&mut memory, "socket", &[2, 1, 6]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x1F, 0x92, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(call(&mut memory, "bind", &[listener, 0x1200, 16]), 0);
    assert_eq!(call(&mut memory, "listen", &[listener, 4]), 0);
    assert_eq!(call(&mut memory, "connect", &[client, 0x1200, 16]), 0);
    let server = call(&mut memory, "accept", &[listener, 0, 0]);
    memory.write_bytes(0x1500, b"12345").unwrap();
    assert_eq!(call(&mut memory, "send", &[client, 0x1500, 5, 0]), 5);
    const FIONREAD: u64 = 0x4004_667F;
    assert_eq!(
        call(
            &mut memory,
            "WSAIoctl",
            &[server, FIONREAD, 0, 0, 0x1700, 4, 0x1710, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1700).unwrap(), 5, "fifo length reported");
    memory.write_u32(0x1700, 0).unwrap();
    assert_eq!(
        call(&mut memory, "ioctlsocket", &[server, FIONREAD, 0x1700]),
        0
    );
    assert_eq!(memory.read_u32(0x1700).unwrap(), 5);
    assert_eq!(
        call(&mut memory, "ioctlsocket", &[server, FIONREAD, 0]),
        SOCKET_ERROR
    );
    assert_eq!(
        call(
            &mut memory,
            "ioctlsocket",
            &[INVALID_SOCKET, FIONREAD, 0x1700]
        ),
        SOCKET_ERROR
    );
    // FIONBIO enabling is the truth; disabling is honest refusal.
    const FIONBIO: u64 = 0x8004_667E;
    memory.write_u32(0x1710, 1).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSAIoctl",
            &[server, FIONBIO, 0x1710, 4, 0x1720, 4, 0x1730, 0, 0]
        ),
        0
    );
    memory.write_u32(0x1710, 0).unwrap();
    assert_eq!(
        call(
            &mut memory,
            "WSAIoctl",
            &[server, FIONBIO, 0x1710, 4, 0x1720, 4, 0x1730, 0, 0]
        ),
        SOCKET_ERROR
    );
    memory.write_u32(0x1710, 1).unwrap();
    assert_eq!(
        call(&mut memory, "ioctlsocket", &[server, FIONBIO, 0x1710]),
        0
    );
    memory.write_u32(0x1710, 0).unwrap();
    assert_eq!(
        call(&mut memory, "ioctlsocket", &[server, FIONBIO, 0x1710]),
        SOCKET_ERROR
    );
    // The loopback reverses into the machine's one name.
    memory.write_bytes(0x1800, &[127, 0, 0, 1]).unwrap();
    let hostent = call(&mut memory, "gethostbyaddr", &[0x1800, 4, 2]);
    assert_ne!(hostent, 0);
    let name_ptr = memory.read_u64(hostent).unwrap();
    let mut name = [0; 7];
    memory.read_bytes(name_ptr, &mut name).unwrap();
    assert_eq!(&name, b"XENORA\0");
    for text in [
        b"localhost\0".as_slice(),
        b"127.0.0.1\0".as_slice(),
        b"XENORA\0".as_slice(),
    ] {
        memory.write_bytes(0x1900, text).unwrap();
        let hostent = call(&mut memory, "gethostbyname", &[0x1900]);
        assert_ne!(hostent, 0);
        assert_eq!(memory.read_u64(hostent + 24).unwrap() - hostent, 48);
        let address_list = memory.read_u64(hostent + 24).unwrap();
        let address_ptr = memory.read_u64(address_list).unwrap();
        let mut loopback = [0; 4];
        memory.read_bytes(address_ptr, &mut loopback).unwrap();
        assert_eq!(loopback, [127, 0, 0, 1]);
    }
    memory.write_bytes(0x1900, b"example.invalid\0").unwrap();
    assert_eq!(call(&mut memory, "gethostbyname", &[0x1900]), 0);
    assert_eq!(call(&mut memory, "gethostbyname", &[0]), 0);
    // A foreign address is an honest no-host.
    memory.write_bytes(0x1800, &[8, 8, 8, 8]).unwrap();
    assert_eq!(call(&mut memory, "gethostbyaddr", &[0x1800, 4, 2]), 0);
}

#[cfg(test)]
#[test]
fn event_select_posts_and_enumeration_drains() {
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    super::events::register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, module: &str, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(module, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };
    assert_eq!(
        call(&mut memory, MODULE, "WSAStartup", &[0x0202, 0x1000]),
        0
    );
    // Manual-reset event for the client socket.
    let event = call(&mut memory, "kernel32.dll", "CreateEventA", &[0, 1, 0, 0]);
    assert_ne!(event, 0);
    // A listener, a client with event selection covering connect.
    let listener = call(&mut memory, MODULE, "socket", &[2, 1, 6]);
    let client = call(&mut memory, MODULE, "socket", &[2, 1, 6]);
    let mut address = [0; 16];
    address[..8].copy_from_slice(&[2, 0, 0x1F, 0x94, 127, 0, 0, 1]);
    memory.write_bytes(0x1200, &address).unwrap();
    assert_eq!(
        call(&mut memory, MODULE, "bind", &[listener, 0x1200, 16]),
        0
    );
    assert_eq!(call(&mut memory, MODULE, "listen", &[listener, 4]), 0);
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSAEventSelect",
            &[
                client,
                event,
                FD_CONNECT as u64 | FD_WRITE as u64 | FD_READ as u64
            ]
        ),
        0
    );
    assert_eq!(
        call(&mut memory, MODULE, "connect", &[client, 0x1200, 16]),
        0
    );
    // The connect completion posted FD_CONNECT|FD_WRITE and signaled.
    let server = call(&mut memory, MODULE, "accept", &[listener, 0, 0]);
    assert!(server != INVALID_SOCKET);
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSAEnumNetworkEvents",
            &[client, event, 0x1300]
        ),
        0
    );
    let occurred = memory.read_u32(0x1300).unwrap();
    assert_eq!(
        occurred & (FD_CONNECT | FD_WRITE),
        FD_CONNECT | FD_WRITE,
        "connect posted its events"
    );
    // Drained: a second enumeration reports nothing.
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSAEnumNetworkEvents",
            &[client, event, 0x1300]
        ),
        0
    );
    assert_eq!(
        memory.read_u32(0x1300).unwrap() & (FD_CONNECT | FD_WRITE),
        0
    );
    // Inbound bytes post FD_READ on the receiving endpoint.
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSAEventSelect",
            &[server, event, FD_READ as u64 | FD_CLOSE as u64]
        ),
        0
    );
    memory.write_bytes(0x1500, b"wake").unwrap();
    assert_eq!(
        call(&mut memory, MODULE, "send", &[client, 0x1500, 4, 0]),
        4
    );
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSAEnumNetworkEvents",
            &[server, event, 0x1300]
        ),
        0
    );
    assert_ne!(
        memory.read_u32(0x1300).unwrap() & FD_READ,
        0,
        "bytes posted read"
    );
    // Zero-length stream sends are no-ops: neither the legacy form nor the
    // gather form manufactures a read edge on the peer.
    assert_eq!(
        call(&mut memory, MODULE, "send", &[client, 0x1500, 0, 0]),
        0
    );
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSAEnumNetworkEvents",
            &[server, event, 0x1300]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1300).unwrap() & FD_READ, 0);
    memory.write_u32(0x1700, 0).unwrap();
    memory.write_u64(0x1708, 0).unwrap();
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSASend",
            &[client, 0x1700, 1, 0x1800, 0, 0, 0]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1800).unwrap(), 0);
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSAEnumNetworkEvents",
            &[server, event, 0x1300]
        ),
        0
    );
    assert_eq!(memory.read_u32(0x1300).unwrap() & FD_READ, 0);
    // Closing the client posts FD_CLOSE on the server.
    assert_eq!(call(&mut memory, MODULE, "closesocket", &[client]), 0);
    assert_eq!(
        call(
            &mut memory,
            MODULE,
            "WSAEnumNetworkEvents",
            &[server, event, 0x1300]
        ),
        0
    );
    let final_bits = memory.read_u32(0x1300).unwrap();
    assert_ne!(final_bits & FD_CLOSE, 0, "close posted");
}

#[cfg(test)]
#[test]
fn servbyname_resolves_the_standard_ports_and_misses_honestly() {
    use super::ServicesState;
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::{GuestMemory, MemoryPermissions};
    use crate::services::MockClock;
    use std::sync::Arc;

    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    register(&mut registry, &state);
    let mut memory = GuestMemory::new();
    memory
        .map_region(0x1000, 8192, MemoryPermissions::READ_WRITE)
        .unwrap();
    let call = |memory: &mut GuestMemory, module: &str, symbol: &str, args: &[u64]| {
        let mut cpu = CpuState::default();
        for (reg, &value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(reg, value);
        }
        cpu.set(Register::Rsp, 0x2000);
        for (index, &value) in args.iter().enumerate().skip(4) {
            memory
                .write_u64(0x2000 + 0x28 + (index as u64 - 4) * 8, value)
                .unwrap();
        }
        let descriptor = registry
            .resolve(module, &ImportName::Name(symbol.to_owned()))
            .unwrap();
        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, memory).unwrap()
    };

    // Winsock must be started before the lookup answers.
    assert_eq!(call(&mut memory, MODULE, "WSAStartup", &[0x202, 0x1800]), 0);
    memory.write_bytes(0x1100, b"http\0").unwrap();
    memory.write_bytes(0x1120, b"tcp\0").unwrap();
    let servent = call(&mut memory, MODULE, "getservbyname", &[0x1100, 0x1120]);
    assert_ne!(servent, 0, "http/tcp is in the assignment table");
    let port = memory.read_u16(servent + 16).unwrap();
    assert_eq!(port, 80u16.to_be(), "the port leaves in network byte order");
    let name_ptr = memory.read_u64(servent).unwrap();
    let mut name = [0u8; 5];
    memory.read_bytes(name_ptr, &mut name).unwrap();
    assert_eq!(&name[..], b"http\0");
    // A null protocol accepts the first matching entry.
    let servent = call(&mut memory, MODULE, "getservbyname", &[0x1100, 0]);
    assert_ne!(servent, 0);
    // An unknown service is the honest miss: null, not an invention.
    memory.write_bytes(0x1100, b"nosuchsvc\0").unwrap();
    assert_eq!(call(&mut memory, MODULE, "getservbyname", &[0x1100, 0]), 0);
}
