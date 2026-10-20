//! Explicit execution limits; scheduling never changes artifact identity.
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(test)]
thread_local! {
    pub(crate) static JOB_OBSERVER: std::cell::RefCell<Option<std::sync::Arc<dyn Fn(usize, usize) + Send + Sync>>> =
        const { std::cell::RefCell::new(None) };
}

/// Shared execution configuration for build stages. Currently only output-owner
/// emission is parallel; module preparation and all planning remain serial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionOptions {
    workers: NonZeroUsize,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self::new(NonZeroUsize::MIN)
    }
}

impl ExecutionOptions {
    /// At most this many workers run at once, further limited by the job count.
    /// One runs inline, without spawning threads.
    pub const fn new(workers: NonZeroUsize) -> Self {
        Self { workers }
    }

    pub const fn workers(self) -> NonZeroUsize {
        self.workers
    }

    pub(crate) fn map<T: Sync, R: Send>(
        self,
        jobs: &[T],
        emit: impl Fn(&T) -> R + Sync,
    ) -> eyre::Result<Vec<R>> {
        let workers = self.workers.get().min(jobs.len());
        if workers <= 1 {
            return Ok(jobs.iter().map(emit).collect());
        }
        let next = AtomicUsize::new(0);
        #[cfg(test)]
        let observer = JOB_OBSERVER.with(|observer| observer.borrow().clone());
        let mut results = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            let mut error = None;
            for _ in 0..workers {
                let (next, emit) = (&next, &emit);
                #[cfg(test)]
                let observer = &observer;
                let spawned = std::thread::Builder::new().spawn_scoped(scope, move || {
                    let before = crate::rewrite::parse_counts();
                    let mut results = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(index) else { break };
                        #[cfg(test)]
                        if let Some(observer) = observer {
                            observer(jobs.len(), index);
                        }
                        results.push((index, emit(job)));
                    }
                    (results, crate::rewrite::parse_counts().difference(before))
                });
                match spawned {
                    Ok(handle) => handles.push(handle),
                    Err(spawn_error) => {
                        error = Some(eyre::Report::from(spawn_error));
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
                    Err(_) => {
                        error.get_or_insert_with(|| eyre::eyre!("output emission worker panicked"));
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
        let result = ExecutionOptions::new(NonZeroUsize::new(3).unwrap())
            .map(&[0, 1, 2], |&job| {
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
        assert!(result.unwrap_err().to_string().contains("worker panicked"));
        assert_eq!(crate::rewrite::parse_counts().difference(before).source, 1);
    }
}
