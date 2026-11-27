// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use xenora_loader::startup::run_synthetic_startup_scheduled;
use xenora_loader::{ExitReason, ImportRegistry, MemoryError};

use super::{
    Direction, Finished, GUEST_LAYOUT, Guest, MAX_EVENTS, Report, WIDTH, Workload, lock,
    require_guest_exit,
};

pub(super) const GUEST: &[u8] = include_bytes!("../../../../../../fixtures/pe/threaded-guest.exe");
const MODULE: &str = "xenora_threaded_app.dll";
const JOURNAL: &str = "C:\\threaded-scene.log";
const WORKER_STATUS: u32 = 0x4C0D_E12A;
pub(super) const EXIT_STATUS: u32 = 0x73D0_CA11;
const QUANTUM: usize = 64;

pub(super) fn column_after(column: usize, direction: Direction) -> usize {
    (column
        + match direction {
            Direction::Left => WIDTH - 1,
            Direction::Right => 1,
        })
        % WIDTH
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct WorkerJob {
    id: u32,
    address: u64,
}

#[derive(Default)]
struct Scene {
    column: usize,
    pending_column: Option<usize>,
    worker: Option<WorkerJob>,
    presented: usize,
    stopping: bool,
    finished: bool,
}

fn install_protocol(
    imports: &mut ImportRegistry,
    commands: Receiver<Direction>,
    report: Sender<Report>,
    guest: &Guest,
) -> Arc<Mutex<Scene>> {
    let scene = Arc::new(Mutex::new(Scene::default()));
    let requests = Arc::clone(&scene);
    let commands = Mutex::new(commands);
    imports.register(MODULE, "XenoraNextSceneCommand", move |_, _, _| {
        {
            let state = lock(&requests)?;
            if state.pending_column.is_some() || state.stopping {
                return Err(ExitReason::ServiceAbiViolation);
            }
        }
        let event = lock(&commands)?.recv();
        let mut state = lock(&requests)?;
        match event {
            Ok(direction) if state.presented < MAX_EVENTS => {
                state.pending_column = Some(column_after(state.column, direction));
                Ok(match direction {
                    Direction::Left => 2,
                    Direction::Right => 1,
                })
            }
            Ok(_) => Err(ExitReason::ServiceAbiViolation),
            Err(_) => {
                state.stopping = true;
                Ok(0)
            }
        }
    });

    let frames = Arc::clone(&scene);
    let frame_services = Arc::clone(&guest.services);
    let graphics = Arc::clone(&guest.graphics);
    imports.register(MODULE, "XenoraReportSceneFrame", move |args, _, memory| {
        let mut state = lock(&frames)?;
        let column = state
            .pending_column
            .ok_or(ExitReason::ServiceAbiViolation)?;
        let next = state.presented + 1;
        let worker = WorkerJob {
            id: args.rdx as u32,
            address: args.r9,
        };
        if args.rcx as u32 as usize != next
            || args.r8 as u32 as usize != column
            || worker.id == 0
            || worker.id == frame_services.thread_id
            || worker.address == 0
            || state.worker.is_some_and(|previous| previous != worker)
            || memory.read_u8(worker.address).is_err()
            || frame_services.open_handle_count() != Workload::Threaded.retained_handles()
        {
            return Err(ExitReason::ServiceAbiViolation);
        }
        let frame = graphics.capture(next, column)?;
        report
            .send(Report::Frame(frame))
            .map_err(|_| ExitReason::ServiceAbiViolation)?;
        state.worker = Some(worker);
        state.presented = next;
        state.column = column;
        state.pending_column = None;
        Ok(1)
    });

    let cleanup = Arc::clone(&scene);
    let services = Arc::clone(&guest.services);
    imports.register(
        MODULE,
        "XenoraReportSceneCleanup",
        move |args, _, memory| {
            let mut state = lock(&cleanup)?;
            let worker = WorkerJob {
                id: args.rcx as u32,
                address: args.r8,
            };
            if !state.stopping
                || state.pending_column.is_some()
                || state.finished
                || worker.id == 0
                || worker.id == services.thread_id
                || worker.address == 0
                || state.worker.is_some_and(|previous| previous != worker)
                || args.rdx as u32 != WORKER_STATUS
                || args.r9 as u32 as usize != state.presented
                || memory.read_u8(worker.address)
                    != Err(MemoryError::UnmappedAddress(worker.address))
                || services.open_handle_count() != 0
            {
                return Err(ExitReason::ServiceAbiViolation);
            }
            state.finished = true;
            Ok(1)
        },
    );
    scene
}

pub(super) fn execute(
    guest: &Guest,
    mut imports: ImportRegistry,
    steps: usize,
    commands: Receiver<Direction>,
    report: Sender<Report>,
) -> Result<Finished, String> {
    let scene = install_protocol(&mut imports, commands, report, guest);
    let result = run_synthetic_startup_scheduled(
        Workload::Threaded.guest(),
        GUEST_LAYOUT,
        imports,
        Arc::clone(&guest.services),
        steps,
        QUANTUM,
        |_| {},
    )
    .map_err(|error| format!("guest startup: {error}"))?;
    require_guest_exit(result.execution.exit_reason, EXIT_STATUS, steps)?;
    let state = scene.lock().map_err(|_| "guest scene state is poisoned")?;
    if !state.finished {
        return Err("guest exited without completing its threaded scene protocol".to_owned());
    }
    guest.finish(state.presented, EXIT_STATUS, JOURNAL)
}
