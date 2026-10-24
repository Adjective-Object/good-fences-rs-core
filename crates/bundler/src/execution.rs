//! Explicit execution limits; scheduling never changes artifact identity.
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        use std::sync::{mpsc, Mutex};
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
        enum Event<R> {
            Completed(R, bool),
            Exited,
        }
        // An exit notification also runs during unwinding: a lost job cannot
        // leave the coordinator waiting while the other workers wait for work.
        struct Exit<R>(mpsc::SyncSender<Event<R>>);
        impl<R> Drop for Exit<R> {
            fn drop(&mut self) {
                let _ = self.0.send(Event::Exited);
            }
        }
        let workers = self.workers.get();
        let (jobs_tx, jobs_rx) = mpsc::sync_channel(workers);
        let jobs_rx = Mutex::new(jobs_rx);
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        std::thread::scope(|scope| {
            let jobs_tx = jobs_tx;
            // Drop this receiver before joining on every path, so even exit
            // notifications during multiple panics cannot block worker cleanup.
            let (results_tx, results_rx) = mpsc::sync_channel(workers);
            let mut handles = Vec::new();
            let mut error = None;
            for worker_id in 0..workers {
                let (jobs_rx, work) = (&jobs_rx, &work);
                let results_tx = results_tx.clone();
                let (dispatch, parent) = (dispatch.clone(), parent.clone());
                match std::thread::Builder::new().spawn_scoped(scope, move || {
                    let _dispatch = tracing::dispatcher::set_default(&dispatch);
                    let _exit = Exit(results_tx.clone());
                    let before = crate::rewrite::parse_counts();
                    loop {
                        let job = jobs_rx.lock().expect("job queue poisoned").recv();
                        let Ok(job) = job else { break };
                        let (job, eager) = job;
                        let result = stage.span(&parent, worker_id).in_scope(|| work(job));
                        if results_tx.send(Event::Completed(result, eager)).is_err() {
                            break;
                        }
                    }
                    crate::rewrite::parse_counts().difference(before)
                }) {
                    Ok(handle) => handles.push(handle),
                    Err(spawn_error) => {
                        error = Some(
                            eyre::Report::from(spawn_error)
                                .wrap_err(format!("spawning {} worker", stage.name())),
                        );
                        break;
                    }
                }
            }
            drop(results_tx);
            let mut outstanding = 0;
            let mut precise = 0;
            let mut waiting = false;
            while error.is_none() {
                while outstanding < workers {
                    let next = pending.pop_front().map(|job| (job, false)).or_else(|| {
                        (precise > 0 || waiting)
                            .then(|| eager.pop_front().map(|job| (job, true)))
                            .flatten()
                    });
                    let Some((job, is_eager)) = next else {
                        break;
                    };
                    if jobs_tx.send((job, is_eager)).is_err() {
                        error = Some(eyre::eyre!("{} workers disconnected", stage.name()));
                        break;
                    }
                    outstanding += 1;
                    precise += usize::from(!is_eager);
                }
                if error.is_some() || outstanding == 0 {
                    break;
                }
                match results_rx.recv() {
                    Ok(Event::Completed(result, is_eager)) => {
                        outstanding -= 1;
                        precise -= usize::from(!is_eager);
                        match integrate(result) {
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
                        }
                    }
                    Ok(Event::Exited) | Err(_) => {
                        error = Some(eyre::eyre!("{} worker exited prematurely", stage.name()));
                    }
                }
            }
            drop(jobs_tx);
            drop(results_rx);
            for handle in handles {
                match handle.join() {
                    Ok(counts) => crate::rewrite::record_worker_parses(counts),
                    Err(payload) => {
                        let panic = worker_panic(payload)
                            .wrap_err(format!("{} worker panicked", stage.name()));
                        error = Some(match error.take() {
                            Some(error) => error.wrap_err(format!("{panic:#}")),
                            None => panic,
                        });
                    }
                }
            }
            match error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }

    pub(crate) fn resolve<R: Send>(
        self,
        work: impl FnOnce() -> eyre::Result<R> + Send,
    ) -> eyre::Result<R> {
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let parent = tracing::Span::current();
        std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .spawn_scoped(scope, move || {
                    let _dispatch = tracing::dispatcher::set_default(&dispatch);
                    let _span = tracing::info_span!(parent: &parent, "resolve.worker").entered();
                    let before = crate::rewrite::parse_counts();
                    let result = work();
                    (result, crate::rewrite::parse_counts().difference(before))
                })
                .map_err(|error| {
                    eyre::Report::from(error).wrap_err("spawning resolution worker")
                })?;
            match worker.join() {
                Ok((result, counts)) => {
                    crate::rewrite::record_worker_parses(counts);
                    result
                }
                Err(payload) => Err(worker_panic(payload).wrap_err("resolution worker panicked")),
            }
        })
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
        let mut results = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            let mut error = None;
            for worker_id in 0..workers {
                let (next, emit) = (&next, &emit);
                let (dispatch, parent) = (dispatch.clone(), parent.clone());
                #[cfg(test)]
                let observer = &observer;
                let spawned = std::thread::Builder::new().spawn_scoped(scope, move || {
                    let _dispatch = tracing::dispatcher::set_default(&dispatch);
                    let before = crate::rewrite::parse_counts();
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
                    (results, crate::rewrite::parse_counts().difference(before))
                });
                match spawned {
                    Ok(handle) => handles.push(handle),
                    Err(spawn_error) => {
                        error = Some(
                            eyre::Report::from(spawn_error)
                                .wrap_err("spawning output emission worker"),
                        );
                        break;
                    }
                }
            }
            let mut results = Vec::new();
            for handle in handles {
                match handle.join() {
                    Ok((mut output, counts)) => {
                        crate::rewrite::record_worker_parses(counts);
                        results.append(&mut output);
                    }
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
        })?;
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, output)| output).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

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
                        .replenish(0..workers, |_| work(), |_| vec![])
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
