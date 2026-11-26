// SPDX-License-Identifier: GPL-3.0-or-later

//! The handle-lifetime foundation of the publicly documented WinHTTP API.
//!
//! These functions do not send a request on Windows: `WinHttpConnect` names
//! a server and `WinHttpOpenRequest` builds a request object. Xenora implements
//! that object model directly. Transport, TLS, responses, cancellation and
//! asynchronous callbacks are separate unsupported slices.

use super::{ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, ServicesState};
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::GuestMemory;
use std::collections::BTreeMap;
use std::sync::Arc;

const MODULE: &str = "winhttp.dll";
const MAX_HANDLES: usize = 256;
const MAX_WIDE_UNITS: usize = 512;
const MAX_ACCEPT_TYPES: usize = 32;
const ERROR_OUTOFMEMORY: u32 = 14;
const ERROR_WINHTTP_INVALID_URL: u32 = 12005;
const ERROR_WINHTTP_INCORRECT_HANDLE_TYPE: u32 = 12018;

const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
const WINHTTP_ACCESS_TYPE_NO_PROXY: u32 = 1;
const WINHTTP_FLAG_ASYNC: u32 = 0x1000_0000;
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
const DOCUMENTED_REQUEST_FLAGS: u32 = 0x0000_01cc | WINHTTP_FLAG_SECURE;

#[derive(Clone, Debug, PartialEq, Eq)]
enum HandleKind {
    Session {
        agent: Option<String>,
        access_type: u32,
    },
    Connection {
        session: u64,
        server: String,
        port: u32,
    },
    Request {
        connection: u64,
        server: String,
        port: u32,
        verb: String,
        object_name: String,
        version: String,
        referrer: Option<String>,
        accept_types: Vec<String>,
        secure: bool,
    },
}

/// The WinHTTP handle table. Handles are opaque guest values; the table owns
/// immutable snapshots of every string, so later guest-memory writes cannot
/// mutate an existing object.
#[derive(Default)]
pub(super) struct WinHttpHandles {
    next: u64,
    handles: BTreeMap<u64, HandleKind>,
}

impl WinHttpHandles {
    pub(super) fn open_count(&self) -> usize {
        self.handles.len()
    }

    pub(super) fn close_all(&mut self) {
        self.handles.clear();
    }

    fn insert(&mut self, kind: HandleKind) -> Result<u64, u32> {
        if self.handles.len() >= MAX_HANDLES {
            return Err(ERROR_OUTOFMEMORY);
        }
        self.next = self
            .next
            .checked_add(1)
            .filter(|next| *next < (1 << 32))
            .ok_or(ERROR_OUTOFMEMORY)?;
        let handle = 0x5748_0000_0000_0000 | self.next;
        self.handles.insert(handle, kind);
        Ok(handle)
    }

    fn kind(&self, handle: u64) -> Option<&HandleKind> {
        self.handles.get(&handle)
    }

    fn close(&mut self, handle: u64) -> bool {
        self.handles.remove(&handle).is_some()
    }
}

fn read_wide_string(memory: &GuestMemory, address: u64, limit: usize) -> Result<String, u32> {
    if address == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut units = Vec::new();
    for index in 0..limit {
        let unit = memory
            .read_u16(address + 2 * index as u64)
            .map_err(|_| ERROR_INVALID_PARAMETER)?;
        if unit == 0 {
            return Ok(String::from_utf16_lossy(&units));
        }
        units.push(unit);
    }
    Err(ERROR_INVALID_PARAMETER)
}

fn read_optional_wide_string(memory: &GuestMemory, address: u64) -> Result<Option<String>, u32> {
    if address == 0 {
        Ok(None)
    } else {
        read_wide_string(memory, address, MAX_WIDE_UNITS).map(Some)
    }
}

fn valid_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte) && byte != b' ' && byte != b'\t')
}

fn valid_server_name(text: &str) -> bool {
    !text.is_empty()
        && text.is_ascii()
        && text.bytes().all(|byte| byte > 0x20 && byte != 0x7f)
        && !text
            .bytes()
            .any(|byte| matches!(byte, b'/' | b'\\' | b'?' | b'#'))
}

fn close_result(state: &ServicesState, teb: u64, closed: bool) -> u64 {
    if closed {
        1
    } else {
        state.fail(teb, ERROR_INVALID_HANDLE, 0).unwrap()
    }
}

/// Register the implemented handle-lifecycle subset. Every function remains
/// partial because documented proxy, asynchronous and transport-related flag
/// combinations are intentionally refused rather than silently accepted.
pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let open = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WinHttpOpen",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let agent = match read_optional_wide_string(memory, args.rcx) {
                Ok(value) => value,
                Err(error) => return open.fail(cpu.gs_base, error, 0),
            };
            let access_type = args.rdx as u32;
            match access_type {
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY | WINHTTP_ACCESS_TYPE_NO_PROXY => {}
                3 | 4 => return open.fail(cpu.gs_base, ERROR_NOT_SUPPORTED, 0),
                _ => return open.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0),
            }
            // Proxy strings are inputs to WINHTTP_ACCESS_TYPE_NAMED_PROXY.
            // That access type is refused above; direct and default modes do
            // not consume them, so they are deliberately not dereferenced.
            let flags = args.get(4, memory).map_err(memory_fault)? as u32;
            if flags == WINHTTP_FLAG_ASYNC {
                return open.fail(cpu.gs_base, ERROR_NOT_SUPPORTED, 0);
            }
            if flags != 0 {
                return open.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
            }
            let mut handles = open.winhttp.lock().expect("WinHTTP handle table");
            match handles.insert(HandleKind::Session { agent, access_type }) {
                Ok(handle) => Ok(handle),
                Err(error) => open.fail(cpu.gs_base, error, 0),
            }
        },
    );

    let connect = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WinHttpConnect",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            if args.r9 != 0 {
                return connect.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
            }
            let server = match read_wide_string(memory, args.rdx, MAX_WIDE_UNITS) {
                Ok(server) if valid_server_name(&server) => server,
                Ok(_) | Err(_) => {
                    return connect.fail(cpu.gs_base, ERROR_WINHTTP_INVALID_URL, 0);
                }
            };
            // INTERNET_PORT is a WORD. As in the Win64 ABI, only the low
            // sixteen bits of the register carry the argument.
            let port = u32::from(args.r8 as u16);
            let mut handles = connect.winhttp.lock().expect("WinHTTP handle table");
            match handles.kind(args.rcx) {
                Some(HandleKind::Session { .. }) => {}
                Some(_) => {
                    return connect.fail(cpu.gs_base, ERROR_WINHTTP_INCORRECT_HANDLE_TYPE, 0);
                }
                None => return connect.fail(cpu.gs_base, ERROR_INVALID_HANDLE, 0),
            }
            match handles.insert(HandleKind::Connection {
                session: args.rcx,
                server,
                port,
            }) {
                Ok(handle) => Ok(handle),
                Err(error) => connect.fail(cpu.gs_base, error, 0),
            }
        },
    );

    let request = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WinHttpOpenRequest",
        ServiceStatus::Partial,
        move |args, cpu, memory| {
            let verb = match read_optional_wide_string(memory, args.rdx) {
                Ok(Some(verb)) if valid_token(&verb) => verb,
                Ok(Some(_)) | Err(_) => {
                    return request.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
                }
                Ok(None) => "GET".to_owned(),
            };
            let object_name = match read_optional_wide_string(memory, args.r8) {
                Ok(Some(object)) => object,
                // Microsoft's examples pass NULL successfully but do not
                // specify an expansion. Retain the absent target honestly;
                // a transport slice must define wire behavior separately.
                Ok(None) => String::new(),
                Err(_) => return request.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0),
            };
            let version = match read_optional_wide_string(memory, args.r9) {
                Ok(Some(version)) if valid_token(&version) => version,
                Ok(Some(_)) | Err(_) => {
                    return request.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
                }
                Ok(None) => "HTTP/1.1".to_owned(),
            };
            let referrer_ptr = args.get(4, memory).map_err(memory_fault)?;
            let referrer = match read_optional_wide_string(memory, referrer_ptr) {
                Ok(value) => value,
                Err(error) => return request.fail(cpu.gs_base, error, 0),
            };
            let accept_ptr = args.get(5, memory).map_err(memory_fault)?;
            let mut accept_types = Vec::new();
            if accept_ptr != 0 {
                let mut terminated = false;
                for index in 0..MAX_ACCEPT_TYPES {
                    let pointer = memory
                        .read_u64(accept_ptr + 8 * index as u64)
                        .map_err(|_| ERROR_INVALID_PARAMETER);
                    let pointer = match pointer {
                        Ok(pointer) => pointer,
                        Err(error) => return request.fail(cpu.gs_base, error, 0),
                    };
                    if pointer == 0 {
                        terminated = true;
                        break;
                    }
                    let Ok(text) = read_wide_string(memory, pointer, MAX_WIDE_UNITS) else {
                        return request.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
                    };
                    accept_types.push(text);
                }
                if !terminated {
                    return request.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
                }
            }
            let flags = args.get(6, memory).map_err(memory_fault)? as u32;
            if flags & !DOCUMENTED_REQUEST_FLAGS != 0 {
                return request.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
            }
            if flags & !WINHTTP_FLAG_SECURE != 0 {
                return request.fail(cpu.gs_base, ERROR_NOT_SUPPORTED, 0);
            }
            let mut handles = request.winhttp.lock().expect("WinHTTP handle table");
            let (server, port) = match handles.kind(args.rcx) {
                Some(HandleKind::Connection { server, port, .. }) => (server.clone(), *port),
                Some(_) => {
                    return request.fail(cpu.gs_base, ERROR_WINHTTP_INCORRECT_HANDLE_TYPE, 0);
                }
                None => return request.fail(cpu.gs_base, ERROR_INVALID_HANDLE, 0),
            };
            match handles.insert(HandleKind::Request {
                connection: args.rcx,
                server,
                port,
                verb,
                object_name,
                version,
                referrer,
                accept_types,
                secure: flags & WINHTTP_FLAG_SECURE != 0,
            }) {
                Ok(handle) => Ok(handle),
                Err(error) => request.fail(cpu.gs_base, error, 0),
            }
        },
    );

    let close = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "WinHttpCloseHandle",
        ServiceStatus::Partial,
        move |args, cpu, _| {
            let mut handles = close.winhttp.lock().expect("WinHTTP handle table");
            Ok(close_result(&close, cpu.gs_base, handles.close(args.rcx)))
        },
    );
}

// The service handlers return `Result<_, ExitReason>`; keeping the conversion
// in one place avoids exposing memory faults as successful handle values.
fn memory_fault(error: crate::memory::MemoryError) -> crate::cpu::ExitReason {
    crate::cpu::ExitReason::MemoryFault(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::{CpuState, Register};
    use crate::import_table::ImportName;
    use crate::imports::Win64Args;
    use crate::memory::MemoryPermissions;
    use crate::services::MockClock;

    fn write_wide(memory: &mut GuestMemory, address: u64, text: &str) {
        for (index, unit) in text.encode_utf16().chain([0]).enumerate() {
            memory.write_u16(address + 2 * index as u64, unit).unwrap();
        }
    }

    struct Fixture {
        registry: ImportRegistry,
        state: Arc<ServicesState>,
        memory: GuestMemory,
    }

    impl Fixture {
        fn new() -> Self {
            let mut registry = ImportRegistry::new();
            let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
            register(&mut registry, &state);
            let mut memory = GuestMemory::new();
            memory
                .map_region(0x1000, 0x8000, MemoryPermissions::READ_WRITE)
                .unwrap();
            write_wide(&mut memory, 0x2000, "XenoraTest/1.0");
            write_wide(&mut memory, 0x2100, "example.test");
            write_wide(&mut memory, 0x2200, "GET");
            write_wide(&mut memory, 0x2300, "/fixture");
            write_wide(&mut memory, 0x2400, "HTTP/1.1");
            Self {
                registry,
                state,
                memory,
            }
        }

        fn call(&mut self, symbol: &str, args: &[u64]) -> u64 {
            let mut cpu = CpuState::default();
            for (register, value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
                .into_iter()
                .zip(args.iter().copied())
            {
                cpu.set(register, value);
            }
            cpu.set(Register::Rsp, 0x7000);
            for (index, value) in args.iter().copied().enumerate().skip(4) {
                self.memory
                    .write_u64(0x7000 + 0x28 + (index as u64 - 4) * 8, value)
                    .unwrap();
            }
            let descriptor = self
                .registry
                .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
                .unwrap();
            (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, &mut self.memory).unwrap()
        }

        fn open_session(&mut self) -> u64 {
            self.call("WinHttpOpen", &[0x2000, 1, 0, 0, 0])
        }

        fn open_connection(&mut self) -> (u64, u64) {
            let session = self.open_session();
            let connection = self.call("WinHttpConnect", &[session, 0x2100, 80, 0]);
            (session, connection)
        }
    }

    #[test]
    fn session_connection_request_and_close_have_owner_lifetimes() {
        let mut fixture = Fixture::new();
        let (session, connection) = fixture.open_connection();
        assert_ne!(session, 0);
        assert_ne!(connection, 0);
        assert_ne!(session, connection);
        let request = fixture.call(
            "WinHttpOpenRequest",
            &[connection, 0x2200, 0x2300, 0x2400, 0, 0, 0],
        );
        assert_ne!(request, 0);
        assert_eq!(fixture.state.open_handle_count(), 3);
        assert_eq!(fixture.call("WinHttpCloseHandle", &[request]), 1);
        assert_eq!(fixture.call("WinHttpCloseHandle", &[request]), 0);
        assert_eq!(fixture.state.last_error(0), ERROR_INVALID_HANDLE);
        assert_eq!(fixture.call("WinHttpCloseHandle", &[connection]), 1);
        assert_eq!(fixture.call("WinHttpCloseHandle", &[session]), 1);
        assert_eq!(fixture.state.open_handle_count(), 0);
    }

    #[test]
    fn defaults_and_secure_flag_are_retained_without_transport() {
        let mut fixture = Fixture::new();
        let (_session, connection) = fixture.open_connection();
        let request = fixture.call(
            "WinHttpOpenRequest",
            &[connection, 0, 0, 0, 0, 0, WINHTTP_FLAG_SECURE as u64],
        );
        assert_ne!(request, 0);
        let handles = fixture.state.winhttp.lock().unwrap();
        assert!(matches!(
            handles.kind(request),
            Some(HandleKind::Request {
                verb,
                object_name,
                version,
                secure: true,
                ..
            }) if verb == "GET" && object_name.is_empty() && version == "HTTP/1.1"
        ));
    }

    #[test]
    fn invalid_parents_flags_strings_and_reserved_values_fail_honestly() {
        let mut fixture = Fixture::new();
        assert_eq!(fixture.call("WinHttpConnect", &[1234, 0x2100, 80, 0]), 0);
        assert_eq!(fixture.state.last_error(0), ERROR_INVALID_HANDLE);
        let wrong_parent = fixture.open_session();
        assert_eq!(
            fixture.call("WinHttpOpenRequest", &[wrong_parent, 0, 0, 0, 0, 0, 0]),
            0
        );
        assert_eq!(
            fixture.state.last_error(0),
            ERROR_WINHTTP_INCORRECT_HANDLE_TYPE
        );
        assert_eq!(fixture.call("WinHttpCloseHandle", &[wrong_parent]), 1);
        assert_eq!(fixture.call("WinHttpOpen", &[0x2000, 4, 0, 0, 0]), 0);
        assert_eq!(fixture.state.last_error(0), ERROR_NOT_SUPPORTED);
        assert_eq!(
            fixture.call("WinHttpOpen", &[0x2000, 1, 0, 0, WINHTTP_FLAG_ASYNC as u64]),
            0
        );
        assert_eq!(fixture.state.last_error(0), ERROR_NOT_SUPPORTED);
        let (_session, connection) = fixture.open_connection();
        assert_eq!(
            fixture.call(
                "WinHttpOpenRequest",
                &[connection, 0x2200, 0x2300, 0x2400, 0, 0, 4]
            ),
            0
        );
        assert_eq!(fixture.state.last_error(0), ERROR_NOT_SUPPORTED);
        fixture.memory.write_u16(0x2100, 0).unwrap();
        assert_eq!(fixture.call("WinHttpConnect", &[1234, 0x2100, 80, 0]), 0);
        assert_eq!(fixture.state.last_error(0), ERROR_WINHTTP_INVALID_URL);
    }

    #[test]
    fn process_exit_closes_the_winhttp_handle_table() {
        let mut fixture = Fixture::new();
        let (_session, connection) = fixture.open_connection();
        assert_ne!(connection, 0);
        assert_eq!(fixture.state.open_handle_count(), 2);
        assert_eq!(fixture.state.exit_process(7), 7);
        assert_eq!(fixture.state.open_handle_count(), 0);
    }

    #[test]
    fn internet_port_uses_only_the_low_sixteen_bits() {
        let mut fixture = Fixture::new();
        let session = fixture.open_session();
        let connection = fixture.call("WinHttpConnect", &[session, 0x2100, 0x1_0000_0050, 0]);
        assert_ne!(connection, 0);
        let handles = fixture.state.winhttp.lock().unwrap();
        assert!(matches!(
            handles.kind(connection),
            Some(HandleKind::Connection { port: 80, .. })
        ));
    }

    #[test]
    fn accept_types_are_copied_out_of_guest_memory() {
        let mut fixture = Fixture::new();
        write_wide(&mut fixture.memory, 0x2500, "text/plain");
        fixture.memory.write_u64(0x2600, 0x2500).unwrap();
        fixture.memory.write_u64(0x2608, 0).unwrap();
        let (_session, connection) = fixture.open_connection();
        let request = fixture.call("WinHttpOpenRequest", &[connection, 0, 0, 0, 0, 0x2600, 0]);
        assert_ne!(request, 0);
        write_wide(&mut fixture.memory, 0x2500, "text/html");
        let handles = fixture.state.winhttp.lock().unwrap();
        assert!(matches!(
            handles.kind(request),
            Some(HandleKind::Request { accept_types, .. }) if accept_types == &vec!["text/plain".to_owned()]
        ));
    }

    #[test]
    fn closing_a_parent_does_not_mutate_an_existing_request_snapshot() {
        let mut fixture = Fixture::new();
        let (session, connection) = fixture.open_connection();
        let request = fixture.call("WinHttpOpenRequest", &[connection, 0, 0, 0, 0, 0, 0]);
        assert_ne!(request, 0);
        assert_eq!(fixture.call("WinHttpCloseHandle", &[connection]), 1);
        assert_eq!(fixture.call("WinHttpCloseHandle", &[session]), 1);
        let handles = fixture.state.winhttp.lock().unwrap();
        assert!(matches!(
            handles.kind(request),
            Some(HandleKind::Request {
                server,
                port: 80,
                ..
            }) if server == "example.test"
        ));
    }

    #[test]
    fn concurrent_creation_is_allowed_on_the_same_parent_handles() {
        use std::sync::{Arc, Barrier};

        let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
        let mut fixture = Fixture {
            registry: ImportRegistry::new(),
            state: Arc::clone(&state),
            memory: GuestMemory::new(),
        };
        register(&mut fixture.registry, &fixture.state);
        fixture
            .memory
            .map_region(0x1000, 0x8000, MemoryPermissions::READ_WRITE)
            .unwrap();
        write_wide(&mut fixture.memory, 0x2000, "agent");
        write_wide(&mut fixture.memory, 0x2100, "example.test");
        let session = fixture.open_session();
        let connection = fixture.call("WinHttpConnect", &[session, 0x2100, 80, 0]);
        assert_ne!(session, 0);
        assert_ne!(connection, 0);

        let barrier = Arc::new(Barrier::new(8));
        let mut threads = Vec::new();
        for use_connection in [false, true] {
            for _ in 0..4 {
                let state = Arc::clone(&state);
                let barrier = Arc::clone(&barrier);
                threads.push(std::thread::spawn(move || {
                    let mut registry = ImportRegistry::new();
                    register(&mut registry, &state);
                    let mut memory = GuestMemory::new();
                    memory
                        .map_region(0x1000, 0x8000, MemoryPermissions::READ_WRITE)
                        .unwrap();
                    write_wide(&mut memory, 0x2100, "example.test");
                    barrier.wait();
                    let parent = if use_connection { connection } else { session };
                    let symbol = if use_connection {
                        "WinHttpOpenRequest"
                    } else {
                        "WinHttpConnect"
                    };
                    let arguments = if use_connection {
                        [parent, 0, 0, 0, 0, 0, 0]
                    } else {
                        [parent, 0x2100, 80, 0, 0, 0, 0]
                    };
                    let mut cpu = CpuState::default();
                    for (register, value) in
                        [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
                            .into_iter()
                            .zip(arguments.iter().copied())
                    {
                        cpu.set(register, value);
                    }
                    cpu.set(Register::Rsp, 0x7000);
                    for (index, value) in arguments.iter().copied().enumerate().skip(4) {
                        memory
                            .write_u64(0x7000 + 0x28 + (index as u64 - 4) * 8, value)
                            .unwrap();
                    }
                    let descriptor = registry
                        .resolve(MODULE, &ImportName::Name(symbol.to_owned()))
                        .unwrap();
                    let handle =
                        (descriptor.handler)(&Win64Args::new(&cpu), &mut cpu, &mut memory).unwrap();
                    assert_ne!(handle, 0);
                }));
            }
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(state.open_handle_count(), 10);
    }

    #[test]
    fn concurrent_services_share_one_thread_safe_handle_table() {
        use std::sync::{Arc, Barrier};

        let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
        let barrier = Arc::new(Barrier::new(4));
        let mut threads = Vec::new();
        for _ in 0..4 {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                let mut fixture = Fixture {
                    registry: ImportRegistry::new(),
                    state,
                    memory: GuestMemory::new(),
                };
                register(&mut fixture.registry, &fixture.state);
                fixture
                    .memory
                    .map_region(0x1000, 0x8000, MemoryPermissions::READ_WRITE)
                    .unwrap();
                write_wide(&mut fixture.memory, 0x2000, "agent");
                write_wide(&mut fixture.memory, 0x2100, "example.test");
                barrier.wait();
                let session = fixture.open_session();
                let connection = fixture.call("WinHttpConnect", &[session, 0x2100, 80, 0]);
                assert_ne!(session, 0);
                assert_ne!(connection, 0);
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(state.open_handle_count(), 8);
        let handles = state.winhttp.lock().unwrap();
        assert_eq!(handles.open_count(), 8);
    }
}
