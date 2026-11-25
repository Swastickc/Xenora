// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded manual work and completion ports for synthetic runtime services.
//! The caller supplies monotonic time, allowing deterministic dispatch in tests.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

/// Identifier issued by a task queue and never reused within that queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskId(u64);

impl TaskId {
    pub(crate) fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub(crate) fn raw(self) -> u64 {
        self.0
    }
}

/// Why the queue could not accept a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    Full,
    IdExhausted,
}

/// Result delivered once through the completion port.
#[derive(Debug, PartialEq, Eq)]
pub enum TaskResult<T, E> {
    Finished(Result<T, E>),
    Cancelled,
    TimedOut,
    WorkerPanicked,
}

type Work<T, E> = Box<dyn FnOnce() -> Result<T, E> + Send>;
type Callback<T, E> = Box<dyn FnOnce(TaskId, TaskResult<T, E>) + Send>;

/// A cancellation request that a running operation can observe at safe points.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn request(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }
}

struct Pending<T, E> {
    id: TaskId,
    deadline: Option<Duration>,
    work: Work<T, E>,
    callback: Callback<T, E>,
    cancellation: Option<CancellationToken>,
}

struct Completion<T, E> {
    id: TaskId,
    result: TaskResult<T, E>,
    callback: Callback<T, E>,
}

struct Worker<T, E> {
    id: TaskId,
    callback: Callback<T, E>,
    thread: JoinHandle<Result<T, E>>,
    cancellation: Option<CancellationToken>,
    deadline: Option<Duration>,
    timed_out: bool,
}

/// A manual task queue whose capacity covers pending work and undelivered completions.
pub struct TaskQueue<T, E> {
    capacity: usize,
    last_id: u64,
    pending: VecDeque<Pending<T, E>>,
    completions: VecDeque<Completion<T, E>>,
    worker: Option<Worker<T, E>>,
}

impl<T, E> TaskQueue<T, E> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            last_id: 0,
            pending: VecDeque::new(),
            completions: VecDeque::new(),
            worker: None,
        }
    }

    /// Enqueue work and a completion callback. The deadline uses the same clock as dispatch.
    pub fn submit(
        &mut self,
        deadline: Option<Duration>,
        work: impl FnOnce() -> Result<T, E> + Send + 'static,
        callback: impl FnOnce(TaskId, TaskResult<T, E>) + Send + 'static,
    ) -> Result<TaskId, SubmitError> {
        self.enqueue(deadline, Box::new(work), Box::new(callback), None)
    }

    /// Enqueue work that can observe a request received while it is running.
    pub fn submit_cancellable(
        &mut self,
        deadline: Option<Duration>,
        work: impl FnOnce(CancellationToken) -> Result<T, E> + Send + 'static,
        callback: impl FnOnce(TaskId, TaskResult<T, E>) + Send + 'static,
    ) -> Result<TaskId, SubmitError> {
        let token = CancellationToken::default();
        self.enqueue(
            deadline,
            Box::new({
                let token = token.clone();
                move || work(token)
            }),
            Box::new(callback),
            Some(token),
        )
    }

    fn enqueue(
        &mut self,
        deadline: Option<Duration>,
        work: Work<T, E>,
        callback: Callback<T, E>,
        cancellation: Option<CancellationToken>,
    ) -> Result<TaskId, SubmitError> {
        if self.pending.len() + self.completions.len() + usize::from(self.worker.is_some())
            >= self.capacity
        {
            return Err(SubmitError::Full);
        }
        self.last_id = self
            .last_id
            .checked_add(1)
            .ok_or(SubmitError::IdExhausted)?;
        let id = TaskId(self.last_id);
        self.pending.push_back(Pending {
            id,
            deadline,
            work,
            callback,
            cancellation,
        });
        Ok(id)
    }

    /// Cancel queued work or request cooperative cancellation of an active worker.
    pub fn cancel(&mut self, id: TaskId) -> bool {
        self.collect_worker();
        if let Some(worker) = &self.worker
            && worker.id == id
        {
            return worker
                .cancellation
                .as_ref()
                .is_some_and(CancellationToken::request);
        }
        let Some(index) = self.pending.iter().position(|task| task.id == id) else {
            return false;
        };
        let task = self
            .pending
            .remove(index)
            .expect("position in pending queue");
        self.completions.push_back(Completion {
            id,
            result: TaskResult::Cancelled,
            callback: task.callback,
        });
        true
    }

    /// Run one queued operation. An expired operation completes without running.
    pub fn dispatch_work(&mut self, now: Duration) -> bool {
        self.advance_time(now);
        if self.worker.is_some() {
            return false;
        }
        let Some(task) = self.pending.pop_front() else {
            return false;
        };
        let result = if task.deadline.is_some_and(|deadline| now >= deadline) {
            TaskResult::TimedOut
        } else {
            TaskResult::Finished((task.work)())
        };
        self.completions.push_back(Completion {
            id: task.id,
            result,
            callback: task.callback,
        });
        true
    }

    /// Invoke one callback on the caller's thread.
    pub fn dispatch_completion(&mut self) -> bool {
        self.collect_worker();
        let Some(completion) = self.completions.pop_front() else {
            return false;
        };
        (completion.callback)(completion.id, completion.result);
        true
    }

    /// Record expiry of the active worker and request cooperative exit when available.
    /// A completion already available when the clock is sampled keeps its result.
    pub fn advance_time(&mut self, now: Duration) {
        self.collect_worker();
        if let Some(worker) = &mut self.worker
            && worker.deadline.is_some_and(|deadline| now >= deadline)
            && worker
                .cancellation
                .as_ref()
                .is_none_or(CancellationToken::request)
        {
            worker.timed_out = true;
        }
    }

    fn collect_worker(&mut self) {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread.is_finished())
        {
            let worker = self.worker.take().expect("finished worker");
            let result = match worker.thread.join() {
                Err(_) => TaskResult::WorkerPanicked,
                Ok(_) if worker.timed_out => TaskResult::TimedOut,
                Ok(_)
                    if worker
                        .cancellation
                        .as_ref()
                        .is_some_and(CancellationToken::is_cancelled) =>
                {
                    TaskResult::Cancelled
                }
                Ok(result) => TaskResult::Finished(result),
            };
            self.completions.push_back(Completion {
                id: worker.id,
                result,
                callback: worker.callback,
            });
        }
    }
}

impl<T: Send + 'static, E: Send + 'static> TaskQueue<T, E> {
    /// Start one background operation. At most one worker runs per queue.
    pub fn dispatch_background(&mut self, now: Duration) -> std::io::Result<bool> {
        self.advance_time(now);
        if self.worker.is_some() {
            return Ok(false);
        }
        let Some(task) = self.pending.front() else {
            return Ok(false);
        };
        if task.deadline.is_some_and(|deadline| now >= deadline) {
            return Ok(self.dispatch_work(now));
        }
        let (send, receive) = std::sync::mpsc::channel::<Work<T, E>>();
        // Start the thread before removing the task so spawn failure leaves it queued.
        let thread = std::thread::Builder::new()
            .name("xenora-task".into())
            .spawn(move || receive.recv().expect("worker receives one operation")())?;
        let task = self.pending.pop_front().expect("pending task");
        assert!(send.send(task.work).is_ok());
        self.worker = Some(Worker {
            id: task.id,
            callback: task.callback,
            thread,
            cancellation: task.cancellation,
            deadline: task.deadline,
            timed_out: false,
        });
        Ok(true)
    }
}

impl<T, E> Drop for TaskQueue<T, E> {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            if let Some(token) = worker.cancellation {
                token.request();
            }
            let _ = worker.thread.join();
        }
    }
}

#[cfg(test)]
#[test]
fn running_task_observes_cancellation_and_deadlines_before_completion() {
    use std::sync::mpsc::channel;
    use std::time::Instant;

    let timeout = Duration::from_millis(5);
    for expire in [false, true] {
        let mut queue = TaskQueue::<u32, u32>::new(1);
        let (running, started) = channel();
        let (completed, results) = channel();
        let id = queue
            .submit_cancellable(
                Some(timeout),
                move |token| {
                    running.send(()).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !token.is_cancelled() {
                        assert!(
                            Instant::now() < deadline,
                            "work did not observe cancellation"
                        );
                        std::thread::yield_now();
                    }
                    Ok(17)
                },
                move |id, result| completed.send((id, result)).unwrap(),
            )
            .unwrap();
        assert!(queue.dispatch_background(Duration::ZERO).unwrap());
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        if expire {
            queue.advance_time(timeout - Duration::from_millis(1));
            assert!(!queue.dispatch_completion());
            queue.advance_time(timeout);
        } else {
            assert!(queue.cancel(id));
        }
        assert!(!queue.cancel(id));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !queue.dispatch_completion() {
            assert!(Instant::now() < deadline, "worker did not complete");
            std::thread::yield_now();
        }
        assert_eq!(
            results.try_recv().unwrap(),
            (
                id,
                if expire {
                    TaskResult::TimedOut
                } else {
                    TaskResult::Cancelled
                }
            )
        );
        assert!(!queue.dispatch_completion());
    }
}

#[cfg(test)]
#[test]
fn background_worker_preserves_capacity_and_manual_completion() {
    use std::sync::mpsc::channel;
    use std::time::Instant;

    let mut queue = TaskQueue::<u32, u32>::new(1);
    let (release, gate) = channel();
    let (completed, results) = channel();
    let id = queue
        .submit(
            None,
            move || {
                gate.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(17)
            },
            move |id, result| completed.send((id, result)).unwrap(),
        )
        .unwrap();
    assert!(queue.dispatch_background(Duration::ZERO).unwrap());
    assert!(!queue.dispatch_background(Duration::ZERO).unwrap());
    assert!(!queue.cancel(id));
    assert!(!queue.dispatch_completion());
    assert_eq!(
        queue.submit(None, || Ok(0), |_, _| {}),
        Err(SubmitError::Full)
    );
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !queue.dispatch_completion() {
        assert!(Instant::now() < deadline, "worker did not complete");
        std::thread::yield_now();
    }
    assert_eq!(
        results.try_recv().unwrap(),
        (id, TaskResult::Finished(Ok(17)))
    );
    assert!(!queue.dispatch_completion());
}

#[cfg(test)]
#[test]
fn sampled_deadline_applies_to_a_plain_worker_until_it_exits() {
    use std::sync::mpsc::channel;
    use std::time::Instant;

    let mut queue = TaskQueue::<u32, u32>::new(1);
    let (release, gate) = channel();
    let (completed, results) = channel();
    let deadline = Duration::from_millis(5);
    let id = queue
        .submit(
            Some(deadline),
            move || {
                gate.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(17)
            },
            move |id, result| completed.send((id, result)).unwrap(),
        )
        .unwrap();
    assert!(queue.dispatch_background(Duration::ZERO).unwrap());
    queue.advance_time(deadline);
    assert!(!queue.dispatch_completion());
    assert_eq!(
        queue.submit(None, || Ok(0), |_, _| {}),
        Err(SubmitError::Full)
    );

    release.send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    while !queue.dispatch_completion() {
        assert!(Instant::now() < until, "worker did not complete");
        std::thread::yield_now();
    }
    assert_eq!(results.try_recv().unwrap(), (id, TaskResult::TimedOut));
    assert!(!queue.dispatch_completion());
    assert!(queue.submit(None, || Ok(0), |_, _| {}).is_ok());
}

#[cfg(test)]
#[test]
fn manual_ports_deliver_errors_cancellation_and_deadlines_once() {
    use std::sync::mpsc::channel;

    let mut queue = TaskQueue::<u32, u32>::new(3);
    let (sender, receiver) = channel();
    let callback = || {
        let sender = sender.clone();
        move |id, result| sender.send((id, result)).unwrap()
    };
    let first = queue.submit(None, || Err(7), callback()).unwrap();
    let cancelled = queue
        .submit(None, || panic!("cancelled"), callback())
        .unwrap();
    let deadline = Duration::from_millis(10);
    let expired = queue
        .submit(Some(deadline), || panic!("expired"), callback())
        .unwrap();
    assert_eq!(
        queue.submit(None, || Ok(1), callback()),
        Err(SubmitError::Full)
    );
    assert!(queue.cancel(cancelled));
    assert!(!queue.cancel(cancelled));
    assert!(queue.dispatch_work(Duration::ZERO));
    assert!(queue.dispatch_work(deadline));
    assert!(!queue.dispatch_work(deadline));
    assert_eq!(
        queue.submit(None, || Ok(1), callback()),
        Err(SubmitError::Full)
    );
    assert!(receiver.try_recv().is_err());
    for expected in [
        (cancelled, TaskResult::Cancelled),
        (first, TaskResult::Finished(Err(7))),
        (expired, TaskResult::TimedOut),
    ] {
        assert!(queue.dispatch_completion());
        assert_eq!(receiver.try_recv().unwrap(), expected);
    }
    assert!(!queue.dispatch_completion());
    let next = queue.submit(None, || Ok(1), callback()).unwrap();
    assert_ne!(next, first);
    assert!(queue.dispatch_work(deadline));
    assert!(queue.dispatch_completion());
    assert_eq!(
        receiver.try_recv().unwrap(),
        (next, TaskResult::Finished(Ok(1)))
    );
}
