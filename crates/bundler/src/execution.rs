//! Explicit execution limits; scheduling never changes artifact identity.
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

thread_local! {
    static BUILD_POOL: std::cell::RefCell<Option<Arc<rayon::ThreadPool>>> =
        const { std::cell::RefCell::new(None) };
    static FORWARDED: std::cell::Cell<crate::rewrite::ParseCounts> =
        const { std::cell::Cell::new(crate::rewrite::ParseCounts::ZERO) };
    static BUILD_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(usize::MAX) };
}

pub(crate) fn owned_pool() -> Option<Arc<rayon::ThreadPool>> {
    BUILD_POOL.with(|pool| pool.borrow().clone())
}

/// Enter CPU work only, never move the non-Send build coordinator.
pub(crate) fn cpu<R: Send>(work: impl FnOnce() -> R + Send) -> R {
    let limit = BUILD_LIMIT.with(std::cell::Cell::get);
    let limited = || ecco::with_worker_limit(NonZeroUsize::new(limit).unwrap(), work);
    if rayon::current_thread_index().is_some() {
        return limited();
    }
    match owned_pool() {
        Some(pool) => {
            let dispatch = tracing::dispatcher::get_default(Clone::clone);
            let parent = tracing::Span::current();
            pool.install(|| {
                tracing::dispatcher::with_default(&dispatch, || parent.in_scope(limited))
            })
        }
        None => limited(),
    }
}

fn cooperate() {
    if rayon::current_thread_index().is_some() {
        rayon::yield_now();
    }
    std::thread::yield_now();
}

fn receive<T>(receiver: &std::sync::mpsc::Receiver<T>) -> Result<T, std::sync::mpsc::RecvError> {
    if rayon::current_thread_index().is_none() {
        return receiver.recv();
    }
    loop {
        match receiver.try_recv() {
            Ok(value) => return Ok(value),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(std::sync::mpsc::RecvError)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => cooperate(),
        }
    }
}

fn run_job<R>(
    origin: std::thread::ThreadId,
    workers: NonZeroUsize,
    work: impl FnOnce() -> R,
) -> (std::thread::Result<R>, crate::rewrite::ParseCounts) {
    let before = crate::rewrite::parse_counts();
    let forwarded = FORWARDED.with(std::cell::Cell::get);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ecco::with_worker_limit(workers, work)
    }));
    // A nested Rayon wait may run another module on this same thread.
    let counts = if std::thread::current().id() == origin {
        crate::rewrite::ParseCounts::ZERO
    } else {
        let counts = crate::rewrite::parse_counts()
            .difference(before)
            .difference(FORWARDED.with(std::cell::Cell::get).difference(forwarded));
        FORWARDED.with(|total| total.set(total.get().plus(counts)));
        counts
    };
    (result, counts)
}

fn worker_panic(payload: Box<dyn std::any::Any + Send>) -> eyre::Report {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload");
    eyre::eyre!("{message}")
}

#[cfg(test)]
thread_local! {
    pub(crate) static JOB_OBSERVER: std::cell::RefCell<Option<std::sync::Arc<dyn Fn(usize, usize) + Send + Sync>>> =
        const { std::cell::RefCell::new(None) };
}

/// Shared execution limits for fingerprinting, preparation and emission stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionOptions {
    workers: NonZeroUsize,
}

#[derive(Clone, Copy)]
pub(crate) enum WorkStage {
    Discovery,
    Preparation,
    Lockfile,
}

pub(crate) enum PreparationEvent<R, E> {
    Early(E),
    Complete(R),
}

impl WorkStage {
    fn name(self) -> &'static str {
        match self {
            Self::Discovery => "package discovery",
            Self::Preparation => "module preparation",
            Self::Lockfile => "lockfile fingerprint",
        }
    }

    fn span(self, parent: &tracing::Span, worker_id: usize) -> tracing::Span {
        match self {
            Self::Discovery => tracing::info_span!(parent: parent, "discovery.package", worker_id),
            Self::Preparation => tracing::info_span!(parent: parent, "prepare.job", worker_id),
            Self::Lockfile => tracing::info_span!(parent: parent, "lockfile.member", worker_id),
        }
    }
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self::new(NonZeroUsize::MIN)
    }
}

impl ExecutionOptions {
    /// Two messages may belong to one preparation slot. Only final completion
    /// releases it; reserve CPU capacity for priority resolution continuations.
    pub(crate) fn prepare_events<J: Send, R: Send, E: Send>(
        self,
        jobs: impl IntoIterator<Item = J>,
        work: impl Fn(J, &dyn Fn(E)) -> R + Sync,
        mut integrate: impl FnMut(PreparationEvent<R, E>) -> eyre::Result<(Vec<J>, bool)>,
        priority: impl Fn(&J) -> usize,
        preparation: impl Fn(&J) -> bool,
    ) -> eyre::Result<()> {
        use std::collections::VecDeque;
        use std::sync::mpsc;
        enum Message<R, E> {
            Early(E),
            Complete(
                std::thread::Result<R>,
                crate::rewrite::ParseCounts,
                usize,
                bool,
                bool,
            ),
        }
        let mut queues: [VecDeque<J>; 4] = std::array::from_fn(|_| VecDeque::new());
        for job in jobs {
            queues[priority(&job)].push_back(job);
        }
        if queues[0].is_empty() && queues[1].is_empty() {
            return Ok(());
        }
        let workers = self.workers.get();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        let origin = std::thread::current().id();
        self.cpu_scope(|scope| {
            let capacity = owned_pool()
                .map_or_else(rayon::current_num_threads, |pool| {
                    pool.current_num_threads()
                })
                .min(workers);
            let preparation_limit = capacity.saturating_sub(1).max(1);
            // A sole caller-pool worker may execute a whole job inside yield_now.
            // Both its early event and final result must fit before it returns.
            let (send, receive_events) = mpsc::sync_channel(workers * 2);
            let mut available: Vec<_> = (0..workers).rev().collect();
            let (mut outstanding, mut preparing, mut precise) = (0, 0, 0);
            let mut waiting = false;
            let mut error: Option<eyre::Report> = None;
            loop {
                if error.is_none() {
                    while outstanding < workers {
                        let rank = (0..4).find(|&rank| {
                            (rank < 2 || precise > 0 || waiting)
                                && queues[rank].front().is_some_and(|job| {
                                    !preparation(job) || preparing < preparation_limit
                                })
                        });
                        let Some(rank) = rank else { break };
                        let job = queues[rank].pop_front().expect("nonempty priority queue");
                        let is_preparation = preparation(&job);
                        let is_precise = rank < 2;
                        let worker_id = available.pop().expect("bounded worker slot");
                        let send = send.clone();
                        let work = &work;
                        let (dispatch, parent) = (dispatch.clone(), parent.clone());
                        scope.spawn(move |_| {
                            let reported = std::cell::Cell::new(false);
                            let limit = if is_preparation {
                                NonZeroUsize::MIN
                            } else {
                                self.workers
                            };
                            let (result, counts) = run_job(origin, limit, || {
                                tracing::dispatcher::with_default(&dispatch, || {
                                    WorkStage::Preparation
                                        .span(&parent, worker_id)
                                        .in_scope(|| {
                                            work(job, &|event| {
                                                assert!(
                                                    !reported.replace(true),
                                                    "duplicate early dependency event"
                                                );
                                                let _ = send.send(Message::Early(event));
                                            })
                                        })
                                })
                            });
                            let _ = send.send(Message::Complete(
                                result,
                                counts,
                                worker_id,
                                is_preparation,
                                is_precise,
                            ));
                        });
                        outstanding += 1;
                        preparing += usize::from(is_preparation);
                        precise += usize::from(is_precise);
                    }
                }
                if outstanding == 0 {
                    break;
                }
                let event = match receive(&receive_events).expect("scoped job completion") {
                    Message::Early(event) => Some(PreparationEvent::Early(event)),
                    Message::Complete(result, counts, worker_id, is_preparation, is_precise) => {
                        crate::rewrite::record_worker_parses(counts);
                        outstanding -= 1;
                        preparing -= usize::from(is_preparation);
                        precise -= usize::from(is_precise);
                        available.push(worker_id);
                        match result {
                            Ok(result) => Some(PreparationEvent::Complete(result)),
                            Err(payload) => {
                                let panic = worker_panic(payload)
                                    .wrap_err("module preparation worker panicked");
                                error = Some(match error.take() {
                                    Some(error) => error.wrap_err(format!("{panic:#}")),
                                    None => panic,
                                });
                                None
                            }
                        }
                    }
                };
                if error.is_none() {
                    if let Some(event) = event {
                        match integrate(event) {
                            Ok((jobs, required)) => {
                                waiting = required;
                                for job in jobs {
                                    queues[priority(&job)].push_back(job);
                                }
                            }
                            Err(failure) => error = Some(failure),
                        }
                    }
                }
            }
            error.map_or(Ok(()), Err)
        })?
    }

    pub(crate) fn scoped_pool<R>(self, work: impl FnOnce() -> R) -> eyre::Result<R> {
        struct ResetLimit(usize);
        impl Drop for ResetLimit {
            fn drop(&mut self) {
                BUILD_LIMIT.with(|limit| limit.set(self.0));
            }
        }
        let previous = BUILD_LIMIT.with(|limit| limit.replace(limit.get().min(self.workers.get())));
        let _limit = ResetLimit(previous);
        if rayon::current_thread_index().is_some() || owned_pool().is_some() {
            return Ok(work());
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.workers.get())
            .thread_name(|index| format!("bundler-cpu-{index}"))
            .build()
            .map_err(|error| eyre::Report::from(error).wrap_err("starting build CPU workers"))?;
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                BUILD_POOL.with(|pool| *pool.borrow_mut() = None);
            }
        }
        BUILD_POOL.with(|slot| *slot.borrow_mut() = Some(Arc::new(pool)));
        let _reset = Reset;
        Ok(work())
    }

    fn cpu_scope<'scope, R>(
        self,
        work: impl FnOnce(&rayon::Scope<'scope>) -> R,
    ) -> eyre::Result<R> {
        self.scoped_pool(|| {
            if rayon::current_thread_index().is_some() {
                rayon::in_place_scope(work)
            } else {
                owned_pool().expect("build CPU pool").in_place_scope(work)
            }
        })
    }
    /// At most this many workers run at once, further limited by the job count.
    /// Fingerprinting and emission may run inline with one worker.
    pub const fn new(workers: NonZeroUsize) -> Self {
        Self { workers }
    }

    pub const fn workers(self) -> NonZeroUsize {
        self.workers
    }

    /// Only paths/jobs wait in the queue. At most `workers` jobs (including
    /// completed results) are outstanding; each completion immediately replenishes
    /// the frontier. Integration runs exclusively on the calling thread.
    #[cfg(test)]
    pub(crate) fn replenish<J: Send, R: Send>(
        self,
        jobs: impl IntoIterator<Item = J>,
        work: impl Fn(J) -> R + Sync,
        mut integrate: impl FnMut(R) -> Vec<J>,
    ) -> eyre::Result<()> {
        self.replenish_stage(WorkStage::Preparation, jobs, work, |result| {
            Ok(integrate(result))
        })
    }

    pub(crate) fn replenish_stage<J: Send, R: Send>(
        self,
        stage: WorkStage,
        jobs: impl IntoIterator<Item = J>,
        work: impl Fn(J) -> R + Sync,
        mut integrate: impl FnMut(R) -> eyre::Result<Vec<J>>,
    ) -> eyre::Result<()> {
        self.replenish_prioritized(
            stage,
            jobs,
            work,
            |result| integrate(result).map(|jobs| (jobs, false)),
            |_| false,
        )
    }

    pub(crate) fn replenish_prioritized<J: Send, R: Send>(
        self,
        stage: WorkStage,
        jobs: impl IntoIterator<Item = J>,
        work: impl Fn(J) -> R + Sync,
        mut integrate: impl FnMut(R) -> eyre::Result<(Vec<J>, bool)>,
        speculative: impl Fn(&J) -> bool,
    ) -> eyre::Result<()> {
        use std::sync::mpsc;
        let (mut eager, mut pending): (std::collections::VecDeque<_>, _) =
            jobs.into_iter().partition(&speculative);
        if self.workers.get() == 1 && matches!(stage, WorkStage::Lockfile) {
            while let Some(job) = pending.pop_front() {
                let result = stage
                    .span(&tracing::Span::current(), 0)
                    .in_scope(|| work(job));
                for job in integrate(result)?.0 {
                    if !speculative(&job) {
                        pending.push_back(job);
                    }
                }
            }
            return Ok(());
        }
        if pending.is_empty() {
            return Ok(());
        }
        let workers = self.workers.get();
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        let origin = std::thread::current().id();
        self.cpu_scope(|cpu_scope| {
            let (results_tx, results_rx) = mpsc::sync_channel(workers);
            let mut available: Vec<_> = (0..workers).rev().collect();
            let mut outstanding = 0;
            let mut precise = 0;
            let mut waiting = false;
            let mut error = None;
            while error.is_none() {
                while outstanding < workers {
                    let next = pending.pop_front().map(|job| (job, false)).or_else(|| {
                        (precise > 0 || waiting)
                            .then(|| eager.pop_front().map(|job| (job, true)))
                            .flatten()
                    });
                    let Some((job, is_eager)) = next else { break };
                    let worker_id = available.pop().expect("bounded worker slot");
                    let work = &work;
                    let results = results_tx.clone();
                    let (dispatch, parent) = (dispatch.clone(), parent.clone());
                    cpu_scope.spawn(move |_| {
                        let (result, counts) = run_job(origin, self.workers, || {
                            tracing::dispatcher::with_default(&dispatch, || {
                                stage.span(&parent, worker_id).in_scope(|| work(job))
                            })
                        });
                        let _ = results.send((result, counts, is_eager, worker_id));
                    });
                    outstanding += 1;
                    precise += usize::from(!is_eager);
                }
                if outstanding == 0 {
                    break;
                }
                let (result, counts, is_eager, worker_id) =
                    receive(&results_rx).expect("scoped jobs always report completion");
                crate::rewrite::record_worker_parses(counts);
                outstanding -= 1;
                precise -= usize::from(!is_eager);
                available.push(worker_id);
                match result {
                    Ok(result) => match integrate(result) {
                        Ok((jobs, required)) => {
                            waiting = required;
                            for job in jobs {
                                if speculative(&job) {
                                    eager.push_front(job);
                                } else {
                                    pending.push_back(job);
                                }
                            }
                        }
                        Err(integration_error) => error = Some(integration_error),
                    },
                    Err(payload) => {
                        error = Some(
                            worker_panic(payload)
                                .wrap_err(format!("{} worker panicked", stage.name())),
                        );
                    }
                }
            }
            // Drain submitted work on both success and failure before leaving
            // the scope, preserving counts and panic diagnostics.
            for _ in 0..outstanding {
                let (result, counts, _, _) =
                    receive(&results_rx).expect("scoped jobs always report completion");
                crate::rewrite::record_worker_parses(counts);
                if let Err(payload) = result {
                    let panic =
                        worker_panic(payload).wrap_err(format!("{} worker panicked", stage.name()));
                    error = Some(match error.take() {
                        Some(error) => error.wrap_err(format!("{panic:#}")),
                        None => panic,
                    });
                }
            }
            match error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })?
    }

    pub(crate) fn resolve<R: Send>(
        self,
        work: impl FnOnce() -> eyre::Result<R> + Send,
    ) -> eyre::Result<R> {
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        let origin = std::thread::current().id();
        self.cpu_scope(|scope| {
            let (send, done) = std::sync::mpsc::sync_channel(1);
            scope.spawn(move |_| {
                let result = run_job(origin, self.workers, || {
                    tracing::dispatcher::with_default(&dispatch, || {
                        tracing::info_span!(parent: &parent, "resolve.worker").in_scope(work)
                    })
                });
                let _ = send.send(result);
            });
            let (result, counts) = receive(&done).expect("scoped job always reports completion");
            crate::rewrite::record_worker_parses(counts);
            result
                .map_err(|payload| worker_panic(payload).wrap_err("resolution worker panicked"))?
        })?
    }

    pub(crate) fn map<T: Sync, R: Send>(
        self,
        jobs: &[T],
        emit: impl Fn(&T) -> R + Sync,
    ) -> eyre::Result<Vec<R>> {
        let workers = self.workers.get().min(jobs.len());
        if workers <= 1 {
            return Ok(jobs
                .iter()
                .enumerate()
                .map(|(owner_id, job)| {
                    tracing::info_span!("emit.owner", worker_id = 0, owner_id)
                        .in_scope(|| emit(job))
                })
                .collect());
        }
        let next = AtomicUsize::new(0);
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        #[cfg(test)]
        let observer = JOB_OBSERVER.with(|observer| observer.borrow().clone());
        let origin = std::thread::current().id();
        let mut results = self.cpu_scope(|cpu_scope| {
            let (send, done) = std::sync::mpsc::sync_channel(workers);
            for worker_id in 0..workers {
                let (next, emit) = (&next, &emit);
                let (dispatch, parent) = (dispatch.clone(), parent.clone());
                let send = send.clone();
                #[cfg(test)]
                let observer = &observer;
                cpu_scope.spawn(move |_| {
                    let result = run_job(origin, self.workers, || {
                        let _dispatch = tracing::dispatcher::set_default(&dispatch);
                        let mut results = Vec::new();
                        loop {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some(job) = jobs.get(index) else { break };
                            #[cfg(test)]
                            if let Some(observer) = observer {
                                observer(jobs.len(), index);
                            }
                            let result = tracing::info_span!(
                                parent: &parent, "emit.owner", worker_id, owner_id = index
                            )
                            .in_scope(|| emit(job));
                            results.push((index, result));
                        }
                        results
                    });
                    let _ = send.send(result);
                });
            }
            let mut results = Vec::new();
            let mut error = None;
            for _ in 0..workers {
                let (result, counts) =
                    receive(&done).expect("scoped jobs always report completion");
                crate::rewrite::record_worker_parses(counts);
                match result {
                    Ok(mut output) => results.append(&mut output),
                    Err(payload) => {
                        error.get_or_insert_with(|| {
                            worker_panic(payload).wrap_err("output emission worker panicked")
                        });
                    }
                }
            }
            if let Some(error) = error {
                return Err(error);
            }
            Ok::<_, eyre::Report>(results)
        })??;
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, output)| output).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn nested_module_expressions_reuse_the_build_or_caller_pool() {
        use std::collections::HashSet;
        for (pool_workers, workers) in [(1, 1), (4, 4), (4, 1), (8, 4)] {
            let execution = ExecutionOptions::new(NonZeroUsize::new(workers).unwrap());
            let source = format!(
                "export function check(a) {{ return ({}); }}",
                vec!["a.x + 1"; 256].join(",")
            );
            let expected = ecco::parse_source(
                &source,
                std::path::Path::new("wide.mjs"),
                ecco::LocalFacts::default(),
            )
            .unwrap();
            let expected = serde_json::to_vec(&expected).unwrap();
            let exercise = |pool: &rayon::ThreadPool| {
                let members: HashSet<_> = pool
                    .broadcast(|_| std::thread::current().id())
                    .into_iter()
                    .collect();
                let mut completed = 0;
                let before = crate::rewrite::parse_counts();
                execution
                    .replenish(
                        0..8,
                        |_| {
                            assert!(members.contains(&std::thread::current().id()));
                            let parsed = ecco::parse_source(
                                &source,
                                std::path::Path::new("wide.mjs"),
                                ecco::LocalFacts::default(),
                            )
                            .unwrap();
                            assert_eq!(serde_json::to_vec(&parsed).unwrap(), expected);
                            ast_segmenter::parse_module(
                                &logger::StdioLogger::new(),
                                &source,
                                std::path::Path::new("wide.mjs"),
                                ast_segmenter::ModuleFormat::Esm,
                            )
                            .unwrap();
                            rayon::scope(|scope| {
                                for _ in 0..8 {
                                    scope.spawn(|_| {
                                        assert!(members.contains(&std::thread::current().id()));
                                    });
                                }
                            });
                        },
                        |_| {
                            completed += 1;
                            vec![]
                        },
                    )
                    .unwrap();
                assert_eq!(completed, 8);
                let counts = crate::rewrite::parse_counts().difference(before);
                assert_eq!(counts.source, 8);
                assert_eq!(counts.declarations, 8);
            };
            execution
                .scoped_pool(|| {
                    let pool = owned_pool().unwrap();
                    exercise(&pool);
                    execution
                        .scoped_pool(|| {
                            assert!(Arc::ptr_eq(&pool, &owned_pool().unwrap()));
                        })
                        .unwrap();
                })
                .unwrap();
            assert!(owned_pool().is_none());
            let caller = rayon::ThreadPoolBuilder::new()
                .num_threads(pool_workers)
                .build()
                .unwrap();
            caller.install(|| {
                exercise(&caller);
                assert!(owned_pool().is_none());
            });
        }
    }

    #[test]
    fn precise_frontier_overtakes_speculation_and_does_not_drain_the_eager_tail() {
        let execution = ExecutionOptions::new(NonZeroUsize::new(2).unwrap());
        let rendezvous = Barrier::new(2);
        let (release, wait) = std::sync::mpsc::channel();
        let wait = std::sync::Mutex::new(wait);
        let completed = std::sync::Mutex::new(Vec::new());
        let jobs = (0..100).map(|id| (true, id)).chain([(false, 0)]);
        execution
            .replenish_prioritized(
                WorkStage::Preparation,
                jobs,
                |(eager, id)| {
                    match (eager, id) {
                        (false, 0) => {
                            rendezvous.wait();
                        }
                        (true, 0) => {
                            rendezvous.wait();
                            wait.lock()
                                .unwrap()
                                .recv_timeout(std::time::Duration::from_secs(10))
                                .expect("precise child must overtake pending speculative jobs");
                        }
                        (false, 1) => {}
                        _ => panic!("unneeded speculation must not delay the precise frontier"),
                    }
                    (eager, id)
                },
                |job| {
                    completed.lock().unwrap().push(job);
                    let jobs = match job {
                        (false, 0) => vec![(false, 1)],
                        (false, 1) => {
                            release.send(()).unwrap();
                            vec![]
                        }
                        _ => vec![],
                    };
                    Ok((jobs, false))
                },
                |job| job.0,
            )
            .unwrap();
        let mut completed = completed.into_inner().unwrap();
        completed.sort();
        assert_eq!(completed, [(false, 0), (false, 1), (true, 0)]);
    }

    #[test]
    fn tracing_propagates_local_dispatch_and_parent_to_all_worker_stages() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

        #[derive(Clone, Default)]
        struct Observe(Arc<Mutex<Vec<(&'static str, Vec<&'static str>, std::thread::ThreadId)>>>);
        impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
            fn on_enter(&self, id: &tracing::span::Id, context: Context<'_, S>) {
                let span = context.span(id).unwrap();
                self.0.lock().unwrap().push((
                    span.name(),
                    span.scope().from_root().map(|span| span.name()).collect(),
                    std::thread::current().id(),
                ));
            }
        }
        let observed = Observe::default();
        let dispatch =
            tracing::Dispatch::new(tracing_subscriber::registry().with(observed.clone()));
        let other = Observe::default();
        let other_dispatch =
            tracing::Dispatch::new(tracing_subscriber::registry().with(other.clone()));
        let exercise = |observed: &Observe, dispatch: &tracing::Dispatch| {
            for workers in [1, 2, 4] {
                observed.0.lock().unwrap().clear();
                let barrier = Barrier::new(workers);
                tracing::dispatcher::with_default(dispatch, || {
                    let _caller = tracing::info_span!("caller").entered();
                    let execution = ExecutionOptions::new(NonZeroUsize::new(workers).unwrap());
                    let work = || {
                        let _span = tracing::info_span!("work").entered();
                        barrier.wait();
                    };
                    execution
                        .prepare_events(
                            0..workers,
                            |_, _: &dyn Fn(())| work(),
                            |_| Ok((vec![], false)),
                            |_| 0,
                            |_| false,
                        )
                        .unwrap();
                    execution
                        .replenish_stage(
                            WorkStage::Lockfile,
                            0..workers,
                            |_| work(),
                            |_| Ok(vec![]),
                        )
                        .unwrap();
                    execution
                        .map(&(0..workers).collect::<Vec<_>>(), |_| work())
                        .unwrap();
                });
                let records = observed.0.lock().unwrap();
                for stage in ["prepare.job", "lockfile.member", "emit.owner"] {
                    let jobs: Vec<_> = records
                        .iter()
                        .filter(|(name, _, _)| *name == stage)
                        .collect();
                    assert_eq!(jobs.len(), workers, "{stage}: {records:?}");
                    assert!(jobs
                        .iter()
                        .all(|(_, parents, _)| parents == &["caller", stage]));
                    let threads: std::collections::HashSet<_> =
                        jobs.iter().map(|(_, _, thread)| *thread).collect();
                    assert_eq!(threads.len(), workers);
                    if workers > 1 {
                        assert!(!threads.contains(&std::thread::current().id()));
                    }
                    assert_eq!(
                        records
                            .iter()
                            .filter(|(name, parents, _)| {
                                *name == "work" && parents == &["caller", stage, "work"]
                            })
                            .count(),
                        workers,
                        "{stage}: {records:?}"
                    );
                }
            }
        };
        std::thread::scope(|scope| {
            let sibling = scope.spawn(|| exercise(&other, &other_dispatch));
            exercise(&observed, &dispatch);
            sibling.join().unwrap();
        });
    }

    #[test]
    fn frontier_replenishes_before_a_slow_sibling_finishes() {
        use std::sync::{mpsc, Mutex};
        let (release, wait) = mpsc::channel();
        let wait = Mutex::new(wait);
        let barrier = Barrier::new(2);
        let mut completed = Vec::new();
        ExecutionOptions::new(NonZeroUsize::new(2).unwrap())
            .replenish(
                [0, 1],
                |job| {
                    if job < 2 {
                        barrier.wait();
                    }
                    if job == 0 {
                        wait.lock()
                            .unwrap()
                            .recv_timeout(std::time::Duration::from_secs(30))
                            .expect(
                                "new dependency must start while its ancestor's sibling is blocked",
                            );
                    } else if job == 2 {
                        release.send(()).unwrap();
                    }
                    job
                },
                |job| {
                    completed.push(job);
                    if job == 1 {
                        vec![2]
                    } else {
                        vec![]
                    }
                },
            )
            .unwrap();
        completed.sort();
        assert_eq!(completed, [0, 1, 2]);
    }

    #[test]
    fn early_frontier_bounds_slots_and_drains_events_on_errors_and_panics() {
        #[derive(Clone, Copy)]
        enum Job {
            Prepare(usize),
            Resolve,
        }
        for workers in [1, 2, 4] {
            let exercise = || {
                let started = AtomicUsize::new(0);
                let integrated = AtomicUsize::new(0);
                let mut resolutions = 0;
                ExecutionOptions::new(NonZeroUsize::new(workers).unwrap())
                    .prepare_events(
                        (0..40).map(Job::Prepare),
                        |job, report| {
                            if let Job::Prepare(id) = job {
                                let starts = started.fetch_add(1, Ordering::SeqCst) + 1;
                                let capacity = workers.min(rayon::current_num_threads());
                                assert!(
                                    starts
                                        <= integrated.load(Ordering::SeqCst)
                                            + capacity.saturating_sub(1).max(1)
                                );
                                report(id);
                            }
                            job
                        },
                        |event| {
                            let jobs = match event {
                                PreparationEvent::Early(_) => vec![Job::Resolve],
                                PreparationEvent::Complete(Job::Prepare(_)) => {
                                    integrated.fetch_add(1, Ordering::SeqCst);
                                    vec![]
                                }
                                PreparationEvent::Complete(Job::Resolve) => {
                                    resolutions += 1;
                                    vec![]
                                }
                            };
                            Ok((jobs, false))
                        },
                        |job| usize::from(matches!(job, Job::Prepare(_))),
                        |job| matches!(job, Job::Prepare(_)),
                    )
                    .unwrap();
                assert_eq!(integrated.load(Ordering::SeqCst), 40);
                assert_eq!(resolutions, 40);
                for panic in [false, true] {
                    let completed = AtomicUsize::new(0);
                    let error = ExecutionOptions::new(NonZeroUsize::new(workers).unwrap())
                        .prepare_events(
                            0..40,
                            |_, report| {
                                report(0);
                                completed.fetch_add(1, Ordering::SeqCst);
                                if panic {
                                    panic!("early worker panic");
                                }
                            },
                            |_| Err(eyre::eyre!("early integration error")),
                            |_| 1,
                            |_| true,
                        )
                        .unwrap_err();
                    assert!(completed.load(Ordering::SeqCst) > 0);
                    let error = format!("{error:#}");
                    assert!(error.contains("early integration error"), "{error}");
                    if panic {
                        assert!(error.contains("early worker panic"), "{error}");
                    }
                }
            };
            exercise();
            rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap()
                .install(exercise);
            rayon::ThreadPoolBuilder::new()
                .num_threads(2)
                .build()
                .unwrap()
                .install(exercise);
        }
    }

    #[test]
    fn frontier_joins_multiple_panics_and_does_not_wait_for_lost_jobs() {
        let barrier = Barrier::new(3);
        let result = ExecutionOptions::new(NonZeroUsize::new(3).unwrap()).replenish(
            [0, 1, 2],
            |job| {
                barrier.wait();
                if job < 2 {
                    panic!("expected preparation failure");
                }
                job
            },
            |_| vec![],
        );
        let error = format!("{:#}", result.unwrap_err());
        assert!(
            error.contains("module preparation worker panicked"),
            "{error}"
        );
        assert!(error.contains("expected preparation failure"), "{error}");
        // More surviving idle workers than panicked jobs must also terminate.
        let result = ExecutionOptions::new(NonZeroUsize::new(4).unwrap()).replenish(
            [0],
            |_| std::panic::panic_any(String::from("single preparation failure")),
            |(): ()| vec![],
        );
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("single preparation failure"), "{error}");
    }

    #[test]
    fn fingerprint_frontier_joins_workers_on_integration_error_and_panic() {
        use std::sync::atomic::AtomicBool;
        let joined = AtomicBool::new(false);
        let barrier = Barrier::new(2);
        let error = ExecutionOptions::new(NonZeroUsize::new(2).unwrap())
            .replenish_stage(
                WorkStage::Lockfile,
                [0, 1],
                |job| {
                    barrier.wait();
                    if job == 1 {
                        joined.store(true, Ordering::SeqCst);
                    }
                    job
                },
                |_| Err(eyre::eyre!("expected fingerprint integration failure")),
            )
            .unwrap_err();
        assert!(joined.load(Ordering::SeqCst));
        assert!(
            format!("{error:#}").contains("expected fingerprint integration failure"),
            "{error:#}"
        );
        let error = ExecutionOptions::new(NonZeroUsize::new(2).unwrap())
            .replenish_stage(
                WorkStage::Lockfile,
                [0],
                |_| panic!("expected fingerprint worker failure"),
                |(): ()| Ok(vec![]),
            )
            .unwrap_err();
        let error = format!("{error:#}");
        assert!(
            error.contains("lockfile fingerprint worker panicked"),
            "{error}"
        );
        assert!(
            error.contains("expected fingerprint worker failure"),
            "{error}"
        );
    }

    #[test]
    fn frontier_aggregates_real_worker_parse_and_fingerprint_counts() {
        let before = crate::rewrite::parse_counts();
        ExecutionOptions::new(NonZeroUsize::new(2).unwrap())
            .replenish(
                [0, 1],
                |_| {
                    ast_segmenter::parse_module(
                        &logger::StdioLogger::new(),
                        "export const value = 1;",
                        std::path::Path::new("worker.mjs"),
                        ast_segmenter::ModuleFormat::Esm,
                    )
                    .unwrap();
                },
                |_| vec![],
            )
            .unwrap();
        let counts = crate::rewrite::parse_counts().difference(before);
        assert_eq!(counts.source, 2);
        assert_eq!(counts.declarations, 2);
    }

    #[test]
    fn frontier_bounds_started_and_completed_jobs_together() {
        for workers in [1, 2, 4] {
            let started = AtomicUsize::new(0);
            let mut integrated = 0;
            ExecutionOptions::new(NonZeroUsize::new(workers).unwrap())
                .replenish(
                    0..100,
                    |job| {
                        started.fetch_add(1, Ordering::SeqCst);
                        job
                    },
                    |_| {
                        assert!(started.load(Ordering::SeqCst) <= integrated + workers);
                        integrated += 1;
                        vec![]
                    },
                )
                .unwrap();
            assert_eq!(integrated, 100);
        }
    }

    #[test]
    fn bounded_workers_rendezvous_and_return_input_order() {
        for workers in [2, 4] {
            let barrier = Barrier::new(workers);
            let jobs: Vec<_> = (0..workers * 3).collect();
            let output = ExecutionOptions::new(NonZeroUsize::new(workers).unwrap())
                .map(&jobs, |&job| {
                    // Each wave needs every worker; later jobs reuse the same bound.
                    barrier.wait();
                    (job, std::thread::current().id())
                })
                .unwrap();
            assert_eq!(output.iter().map(|(job, _)| *job).collect::<Vec<_>>(), jobs);
            let threads: std::collections::HashSet<_> =
                output.iter().map(|(_, thread)| *thread).collect();
            assert_eq!(threads.len(), workers);
        }
    }

    #[test]
    fn worker_parser_counts_are_returned_even_for_failed_jobs() {
        let before = crate::rewrite::parse_counts();
        let result = ExecutionOptions::new(NonZeroUsize::new(2).unwrap())
            .map(&[0, 1], |_| {
                ast_segmenter::parse_module(
                    &logger::StdioLogger::new(),
                    "export const value = 1;",
                    std::path::Path::new("worker.mjs"),
                    ast_segmenter::ModuleFormat::Esm,
                )
                .unwrap();
                crate::rewrite::validate(
                    std::path::Path::new("worker.mjs"),
                    "export const value = 1;",
                )
                .unwrap();
                Err::<(), _>("expected job failure")
            })
            .unwrap();
        assert!(result.iter().all(Result::is_err));
        let counts = crate::rewrite::parse_counts().difference(before);
        assert_eq!(counts.source, 2);
        let validations = if cfg!(debug_assertions) { 2 } else { 0 };
        assert_eq!(counts.output, validations);
        assert_eq!(counts.validation, validations);
    }

    #[test]
    fn multiple_panics_are_joined_and_successful_worker_counts_are_preserved() {
        let before = crate::rewrite::parse_counts();
        let barrier = Barrier::new(3);
        let result = ExecutionOptions::new(NonZeroUsize::new(3).unwrap()).map(&[0, 1, 2], |&job| {
            barrier.wait();
            if job < 2 {
                panic!("expected worker panic");
            }
            ast_segmenter::parse_module(
                &logger::StdioLogger::new(),
                "export const value = 1;",
                std::path::Path::new("worker.mjs"),
                ast_segmenter::ModuleFormat::Esm,
            )
            .unwrap();
        });
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("output emission worker panicked"), "{error}");
        assert!(error.contains("expected worker panic"), "{error}");
        assert_eq!(crate::rewrite::parse_counts().difference(before).source, 1);
    }
}
