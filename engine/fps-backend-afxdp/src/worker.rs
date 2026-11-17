// SPDX-License-Identifier: GPL-3.0-only
//! Per queue workers: one thread for each [`FrameIo`] queue.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::frame::{FrameIo, QueueStats};

/// The work of one worker. The worker calls [`QueueLoop::turn`] over and over
/// until it is stopped, and the loop receives, processes, and sends frames of
/// its own queue only. A closure works too.
pub trait QueueLoop: Send + 'static {
    /// Runs one pass over the queue and returns whether it moved any frame,
    /// so the worker knows when to idle. An error ends the worker.
    ///
    /// # Errors
    ///
    /// Returns the error that makes the queue unusable.
    fn turn(&mut self, io: &mut dyn FrameIo) -> io::Result<bool>;
}

impl<F> QueueLoop for F
where
    F: FnMut(&mut dyn FrameIo) -> io::Result<bool> + Send + 'static,
{
    fn turn(&mut self, io: &mut dyn FrameIo) -> io::Result<bool> {
        self(io)
    }
}

/// What a worker does when a turn moved no frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idle {
    /// Turn again at once. This keeps a core at 100% and has the lowest
    /// latency, the choice for a rate test.
    Spin,
    /// Give the core to another thread for a moment, then turn again.
    Yield,
    /// Sleep in [`FrameIo::wait_readable`] for at most this long, which a
    /// frame ends at once.
    Wait(Duration),
}

/// Settings of a set of workers.
#[derive(Clone, Debug)]
pub struct WorkerOptions {
    /// CPUs to pin the workers to. Worker `i` runs on `cpus[i % cpus.len()]`,
    /// and an empty list pins nothing.
    pub cpus: Vec<usize>,
    /// What an idle worker does.
    pub idle: Idle,
    /// The prefix of the thread names, which gets the queue number appended.
    pub name: String,
}

impl Default for WorkerOptions {
    /// No pinning, [`Idle::Wait`] of one millisecond, and the name `afxdp`.
    fn default() -> Self {
        Self {
            cpus: Vec::new(),
            idle: Idle::Wait(Duration::from_millis(1)),
            name: "afxdp".to_owned(),
        }
    }
}

/// How one worker ended.
#[derive(Debug)]
pub struct WorkerReport {
    /// The queue that the worker served.
    pub queue_id: u32,
    /// Turns that moved frames.
    pub busy_turns: u64,
    /// Turns that moved nothing.
    pub idle_turns: u64,
    /// The counters of the queue at the end.
    pub stats: QueueStats,
    /// The error that ended the worker early, or `None` when it was stopped.
    pub error: Option<io::Error>,
}

/// A set of running workers, one thread per queue.
///
/// Each worker owns its queue, so queues share no state and need no lock. The
/// workers run until [`Workers::stop`].
#[derive(Debug)]
pub struct Workers {
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<WorkerReport>>,
}

impl Workers {
    /// Starts one worker per queue. `make` builds the loop of a queue from its
    /// queue number, on the calling thread.
    ///
    /// # Errors
    ///
    /// Returns the error of the failed thread spawn. Workers that already
    /// started are stopped first.
    pub fn spawn<Q, L, M>(queues: Vec<Q>, options: &WorkerOptions, mut make: M) -> io::Result<Self>
    where
        Q: FrameIo + 'static,
        L: QueueLoop,
        M: FnMut(u32) -> L,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let mut workers = Self {
            stop: Arc::clone(&stop),
            handles: Vec::with_capacity(queues.len()),
        };
        for (i, queue) in queues.into_iter().enumerate() {
            let queue_id = queue.queue_id();
            let body = make(queue_id);
            let cpu = (!options.cpus.is_empty()).then(|| options.cpus[i % options.cpus.len()]);
            let idle = options.idle;
            let stop = Arc::clone(&stop);
            let handle = std::thread::Builder::new()
                .name(format!("{}-{queue_id}", options.name))
                .spawn(move || run(queue, body, cpu, idle, &stop))?;
            workers.handles.push(handle);
        }
        Ok(workers)
    }

    /// The number of workers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    /// Whether there are no workers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// Whether any worker has ended, which only an error does before
    /// [`Workers::stop`].
    #[must_use]
    pub fn any_finished(&self) -> bool {
        self.handles.iter().any(JoinHandle::is_finished)
    }

    /// Tells every worker to stop, waits for them, and returns one report per
    /// worker in queue order. A worker that has not yet started its first turn
    /// takes that one turn before it stops, so an error of a broken queue is
    /// reported however early the stop comes.
    #[must_use]
    pub fn stop(mut self) -> Vec<WorkerReport> {
        self.stop.store(true, Ordering::Release);
        std::mem::take(&mut self.handles)
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| WorkerReport {
                    queue_id: u32::MAX,
                    busy_turns: 0,
                    idle_turns: 0,
                    stats: QueueStats::default(),
                    error: Some(io::Error::other("the worker thread panicked")),
                })
            })
            .collect()
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for h in std::mem::take(&mut self.handles) {
            let _ = h.join();
        }
    }
}

fn run<Q: FrameIo, L: QueueLoop>(
    mut queue: Q,
    mut body: L,
    cpu: Option<usize>,
    idle: Idle,
    stop: &AtomicBool,
) -> WorkerReport {
    let queue_id = queue.queue_id();
    let mut report = WorkerReport {
        queue_id,
        busy_turns: 0,
        idle_turns: 0,
        stats: QueueStats::default(),
        error: None,
    };
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    if let Some(cpu) = cpu {
        if let Err(e) = crate::sys::pin_current_thread(cpu) {
            report.error = Some(io::Error::new(e.kind(), format!("pin to cpu {cpu}: {e}")));
            return report;
        }
    }
    #[cfg(not(all(target_os = "linux", target_pointer_width = "64")))]
    let _ = cpu;
    // Every loop takes at least one turn, even when the stop request came
    // before this thread was first scheduled. Otherwise a queue that is broken
    // from the start would end as "stopped" with no error, and which workers
    // ran would depend on thread scheduling.
    let mut first = true;
    while std::mem::take(&mut first) || !stop.load(Ordering::Acquire) {
        match body.turn(&mut queue) {
            Ok(true) => report.busy_turns += 1,
            Ok(false) => {
                report.idle_turns += 1;
                match idle {
                    Idle::Spin => std::hint::spin_loop(),
                    Idle::Yield => std::thread::yield_now(),
                    Idle::Wait(d) => {
                        if let Err(e) = queue.wait_readable(d) {
                            report.error = Some(e);
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                report.error = Some(e);
                break;
            }
        }
    }
    report.stats = queue.stats();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{frame_pair, MemoryFrames};
    use std::sync::atomic::AtomicUsize;

    /// An echo loop: every frame that arrives goes back with its first byte
    /// incremented.
    fn echo(io: &mut dyn FrameIo) -> io::Result<bool> {
        let mut batch: Vec<Vec<u8>> = Vec::new();
        let n = io.recv_batch(8, &mut |f| batch.push(f.to_vec()))?;
        for f in &mut batch {
            f[0] = f[0].wrapping_add(1);
        }
        let refs: Vec<&[u8]> = batch.iter().map(Vec::as_slice).collect();
        io.send_batch(&refs)?;
        Ok(n > 0)
    }

    fn client_roundtrip(c: &mut MemoryFrames, first: u8) -> u8 {
        c.send_batch(&[&[first, 1, 2]]).unwrap();
        assert!(c.wait_readable(Duration::from_secs(5)).unwrap());
        let mut got = 0;
        c.recv_batch(1, &mut |f| got = f[0]).unwrap();
        got
    }

    #[test]
    fn one_worker_per_queue_serves_its_own_queue() {
        let (c0, s0) = frame_pair(8, 64);
        let (c1, s1) = frame_pair(8, 64);
        let (mut c0, mut c1) = (c0.with_queue_id(0), c1.with_queue_id(1));
        let made = Arc::new(AtomicUsize::new(0));
        let m = Arc::clone(&made);
        let workers = Workers::spawn(
            vec![s0.with_queue_id(0), s1.with_queue_id(1)],
            &WorkerOptions::default(),
            move |_q| {
                m.fetch_add(1, Ordering::Relaxed);
                echo
            },
        )
        .unwrap();
        assert_eq!(workers.len(), 2);
        assert_eq!(client_roundtrip(&mut c0, 10), 11);
        assert_eq!(client_roundtrip(&mut c1, 20), 21);
        assert!(!workers.any_finished());
        let reports = workers.stop();
        assert_eq!(made.load(Ordering::Relaxed), 2);
        assert_eq!(reports.len(), 2);
        for (r, id) in reports.iter().zip([0, 1]) {
            assert_eq!(r.queue_id, id);
            assert!(r.error.is_none());
            assert_eq!(r.stats.rx_frames, 1);
            assert_eq!(r.stats.tx_frames, 1);
            assert!(r.busy_turns >= 1);
        }
    }

    #[test]
    fn an_error_from_a_loop_ends_only_that_worker() {
        let (c0, s0) = frame_pair(8, 64);
        let (mut c1, s1) = frame_pair(8, 64);
        let _keep = c0;
        let workers = Workers::spawn(
            vec![s0.with_queue_id(0), s1.with_queue_id(1)],
            &WorkerOptions {
                idle: Idle::Yield,
                ..WorkerOptions::default()
            },
            |q| {
                move |io: &mut dyn FrameIo| {
                    if q == 0 {
                        Err(io::Error::other("queue 0 is broken"))
                    } else {
                        echo(io)
                    }
                }
            },
        )
        .unwrap();
        assert_eq!(client_roundtrip(&mut c1, 1), 2);
        let reports = workers.stop();
        assert_eq!(
            reports[0].error.as_ref().unwrap().to_string(),
            "queue 0 is broken"
        );
        assert!(reports[1].error.is_none());
    }

    /// Regression test of #588: the stop request is already there when the
    /// worker starts, which is what a slow thread start looks like.
    #[test]
    fn a_loop_takes_one_turn_even_when_the_stop_came_first() {
        let stop = AtomicBool::new(true);

        let (_c, s) = frame_pair(2, 8);
        let broken = |_: &mut dyn FrameIo| -> io::Result<bool> { Err(io::Error::other("broken")) };
        let r = run(s.with_queue_id(3), broken, None, Idle::Yield, &stop);
        assert_eq!(r.queue_id, 3);
        assert_eq!(r.error.as_ref().unwrap().to_string(), "broken");

        let (_c, s) = frame_pair(2, 8);
        let r = run(s, echo, None, Idle::Spin, &stop);
        assert!(r.error.is_none());
        assert_eq!((r.busy_turns, r.idle_turns), (0, 1));
    }

    #[test]
    fn spinning_workers_stop_promptly_and_count_idle_turns() {
        let (_c, s) = frame_pair(2, 8);
        let workers = Workers::spawn(
            vec![s],
            &WorkerOptions {
                idle: Idle::Spin,
                ..Default::default()
            },
            |_| echo,
        )
        .unwrap();
        // No sleep: every worker takes one turn before it stops, and the queue
        // is empty, so that turn is an idle one.
        let r = workers.stop();
        assert!(r[0].idle_turns > 0);
        assert_eq!(r[0].busy_turns, 0);
    }

    #[test]
    fn a_worker_pins_to_a_cpu_or_reports_why_not() {
        let (_c, s) = frame_pair(2, 8);
        let opts = WorkerOptions {
            cpus: vec![100_000],
            ..Default::default()
        };
        let w = Workers::spawn(vec![s], &opts, |_| echo).unwrap();
        let r = w.stop();
        let e = r[0]
            .error
            .as_ref()
            .expect("a cpu number past CPU_SETSIZE is refused");
        assert!(e.to_string().contains("pin to cpu 100000"), "{e}");
    }

    #[test]
    fn dropping_workers_stops_them() {
        let (_c, s) = frame_pair(2, 8);
        let w = Workers::spawn(vec![s], &WorkerOptions::default(), |_| echo).unwrap();
        drop(w);
    }
}
