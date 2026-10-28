//! Coarse numeric checkpoints; no source identities or allocation interception.
use std::time::Instant;

pub(crate) struct Stage {
    phase: &'static str,
    started: Instant,
    completed: bool,
    _span: tracing::span::EnteredSpan,
}

impl Stage {
    pub(crate) fn new(phase: &'static str) -> Self {
        let span = tracing::info_span!("planning.stage", phase).entered();
        tracing::info!(
            target: "bundler::progress",
            phase,
            checkpoint = 0,
            "Planning stage begin"
        );
        Self {
            phase,
            started: Instant::now(),
            completed: false,
            _span: span,
        }
    }

    pub(crate) fn finish(mut self) {
        self.completed = true;
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        tracing::info!(
            target: "bundler::progress",
            phase = self.phase,
            checkpoint = if self.completed { 1 } else { 2 },
            elapsed_ms = self.started.elapsed().as_millis() as u64,
            "Planning stage end"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use source_graph::{GraphNode, SegmentKey, SourceGraph};
    use tracing_subscriber::prelude::*;

    use crate::{
        chunks::{partition, RootId, RootSeed},
        policy::EffectPolicy,
    };

    #[derive(Default)]
    struct Checkpoint {
        phase: Option<String>,
        checkpoint: Option<u64>,
    }

    impl tracing::field::Visit for Checkpoint {
        fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "phase" {
                self.phase = Some(value.to_owned());
            }
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            if field.name() == "checkpoint" {
                self.checkpoint = Some(value);
            }
        }

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            if field.name() == "checkpoint" {
                self.checkpoint = u64::try_from(value).ok();
            }
        }
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<(String, u64)>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if event.metadata().target() == "bundler::progress" {
                let mut checkpoint = Checkpoint::default();
                event.record(&mut checkpoint);
                if let (Some(phase), Some(checkpoint)) = (checkpoint.phase, checkpoint.checkpoint) {
                    self.0.lock().unwrap().push((phase, checkpoint));
                }
            }
        }
    }

    #[test]
    fn checkpoints_balance_on_real_planning_success_and_error_without_changing_results() {
        let graph =
            SourceGraph::from_owned([crate::tests::input("main.mjs", "export const value = 7;")]);
        let mut roots = [RootSeed {
            id: RootId::Entry("main".into()),
            evaluation_roots: vec![0],
            binding_roots: vec![GraphNode::Module(0)],
        }];
        let policy = EffectPolicy::default();
        let expected = partition(&graph, &roots, &policy).unwrap();
        let capture = Capture::default();
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(capture.clone()));
        tracing::dispatcher::with_default(&dispatch, || {
            assert_eq!(partition(&graph, &roots, &policy).unwrap(), expected);
            roots[0].binding_roots = vec![GraphNode::Segment(SegmentKey::new(0, 100))];
            let error = partition(&graph, &roots, &policy).unwrap_err();
            assert!(format!("{error:#}").contains("invalid binding segment root"));
        });
        let events = capture.0.lock().unwrap();
        let mut active = Vec::new();
        let mut interrupted = Vec::new();
        for (phase, checkpoint) in events.iter() {
            match checkpoint {
                0 => active.push(phase),
                1 | 2 => {
                    assert_eq!(active.pop(), Some(phase), "{events:?}");
                    if *checkpoint == 2 {
                        interrupted.push(phase.as_str());
                    }
                }
                _ => panic!("unexpected checkpoint {checkpoint}"),
            }
        }
        assert!(active.is_empty(), "{events:?}");
        assert_eq!(interrupted, ["retention", "partition"]);
        eprintln!("planning_checkpoint_events={events:?}");
    }
}
