use std::{
    io::{IsTerminal, Write},
    sync::Mutex,
    time::{Duration, Instant},
};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use tracing::{field::Visit, Event, Subscriber};
use tracing_subscriber::fmt::{format::Writer, time::FormatTime};
use tracing_subscriber::{layer::Context, Layer};

pub(crate) struct LockfileProgress<W> {
    state: Mutex<State<W>>,
}

struct State<W> {
    bar: ProgressBar,
    interactive: bool,
    active: bool,
    last_log: Option<Instant>,
    output: W,
}

#[derive(Default)]
struct Counts {
    completed: Option<u64>,
    total: Option<u64>,
    finished: bool,
    success: bool,
}

impl Visit for Counts {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "completed" => self.completed = Some(value),
            "total" => self.total = Some(value),
            _ => {}
        }
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        match field.name() {
            "finished" => self.finished = value,
            "success" => self.success = value,
            _ => {}
        }
    }

    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl LockfileProgress<std::io::Stderr> {
    pub(crate) fn stderr() -> Self {
        let output = std::io::stderr();
        let interactive = output.is_terminal();
        Self::new(interactive, ProgressDrawTarget::stderr_with_hz(10), output)
    }
}

impl<W> LockfileProgress<W> {
    fn new(interactive: bool, target: ProgressDrawTarget, output: W) -> Self {
        let bar = ProgressBar::with_draw_target(None, target).with_style(
            ProgressStyle::with_template(
                "Lockfile fingerprints [{bar:24}] {pos}/{len} {elapsed_precise}",
            )
            .expect("valid lockfile progress template")
            .progress_chars("=>-"),
        );
        Self {
            state: Mutex::new(State {
                bar,
                interactive,
                active: false,
                last_log: None,
                output,
            }),
        }
    }
}

impl<S: Subscriber, W: Write + Send + 'static> Layer<S> for LockfileProgress<W> {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut counts = Counts::default();
        event.record(&mut counts);
        let (Some(completed), Some(total)) = (counts.completed, counts.total) else {
            return;
        };
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let first = !state.active;
        if first {
            state.bar.reset();
            state.last_log = None;
            state.active = true;
        }
        state.bar.set_length(total);
        state.bar.set_position(completed);
        if state.interactive {
            if first {
                state.bar.force_draw();
            }
        } else if first
            || counts.finished
            || state
                .last_log
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
        {
            let status = match (counts.finished, counts.success) {
                (false, _) => "running",
                (true, true) => "complete",
                (true, false) => "failed",
            };
            let mut timestamp = String::new();
            crate::trace::uptime()
                .format_time(&mut Writer::new(&mut timestamp))
                .expect("formatting uptime into a string");
            let _ = writeln!(
                state.output,
                "{timestamp}  INFO Fingerprinting lockfile package graph completed={completed} total={total} status={status}"
            );
            state.last_log = Some(Instant::now());
        }
        if counts.finished {
            state.bar.finish_and_clear();
            state.active = false;
        }
    }
}

impl<W> Drop for State<W> {
    fn drop(&mut self) {
        if self.active {
            self.bar.finish_and_clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tracing_subscriber::prelude::*;

    #[derive(Clone, Debug, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);

    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone, Debug, Default)]
    struct Screen {
        output: Output,
        clears: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl indicatif::TermLike for Screen {
        fn width(&self) -> u16 {
            100
        }
        fn move_cursor_up(&self, _: usize) -> std::io::Result<()> {
            Ok(())
        }
        fn move_cursor_down(&self, _: usize) -> std::io::Result<()> {
            Ok(())
        }
        fn move_cursor_right(&self, _: usize) -> std::io::Result<()> {
            Ok(())
        }
        fn move_cursor_left(&self, _: usize) -> std::io::Result<()> {
            Ok(())
        }
        fn write_line(&self, text: &str) -> std::io::Result<()> {
            writeln!(self.output.clone(), "{text}")
        }
        fn write_str(&self, text: &str) -> std::io::Result<()> {
            self.output.clone().write_all(text.as_bytes())
        }
        fn clear_line(&self) -> std::io::Result<()> {
            self.clears
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn flush(&self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn update(completed: u64, total: u64, finished: bool, success: bool) {
        tracing::info!(
            target: "bundler::progress::lockfile",
            completed, total, finished, success,
            "Fingerprinting lockfile package graph"
        );
    }

    #[test]
    fn redirected_progress_is_throttled_and_reports_partial_failure() {
        let output = Output::default();
        let reporter = LockfileProgress::new(false, ProgressDrawTarget::hidden(), output.clone());
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(reporter));
        tracing::dispatcher::with_default(&dispatch, || {
            update(0, 100, false, true);
            for completed in 1..100 {
                update(completed, 100, false, true);
            }
            update(100, 100, true, true);
            update(0, 10, false, true);
            update(3, 10, true, false);
        });
        let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        for (line, expected) in lines.iter().zip([
            "completed=0 total=100 status=running",
            "completed=100 total=100 status=complete",
            "completed=0 total=10 status=running",
            "completed=3 total=10 status=failed",
        ]) {
            assert!(line.contains(expected), "{text}");
            let mut prefix = line.split_whitespace();
            assert!(
                prefix
                    .next()
                    .unwrap()
                    .strip_suffix('s')
                    .unwrap()
                    .parse::<f64>()
                    .unwrap()
                    .is_finite(),
                "{line}"
            );
            assert_eq!(prefix.next(), Some("INFO"), "{line}");
        }
        assert!(!text.contains('\r') && !text.contains('\u{1b}'), "{text}");
    }

    #[test]
    fn terminal_bar_tracks_counts_and_clears_on_success_failure_and_early_drop() {
        use std::sync::atomic::Ordering;
        for finished in [Some(true), Some(false), None] {
            let screen = Screen::default();
            let output = Output::default();
            let reporter = LockfileProgress::new(
                true,
                ProgressDrawTarget::term_like(Box::new(screen.clone())),
                output.clone(),
            );
            let bar = reporter.state.lock().unwrap().bar.clone();
            let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(reporter));
            let before = tracing::dispatcher::with_default(&dispatch, || {
                update(0, 8, false, true);
                update(4, 8, false, true);
                bar.force_draw();
                let text = String::from_utf8(screen.output.0.lock().unwrap().clone()).unwrap();
                assert!(text.contains("Lockfile fingerprints ["), "{text}");
                assert!(text.contains("4/8"), "{text}");
                let before = screen.clears.load(Ordering::SeqCst);
                if let Some(success) = finished {
                    update(if success { 8 } else { 4 }, 8, true, success);
                }
                before
            });
            drop(dispatch);
            assert!(bar.is_finished());
            assert!(screen.clears.load(Ordering::SeqCst) > before);
            assert!(output.0.lock().unwrap().is_empty());
        }
    }
}
