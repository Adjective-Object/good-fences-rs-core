use super::*;
use std::sync::{Arc, Mutex};
use tracing_subscriber::prelude::*;

#[test]
fn esm_require_context_policy_waivers_are_exact_and_readmitted_on_every_cache_path() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export const load = name => require('../parts/' + name);",
    );
    write(dir.root(), "parts/a.js", "exports.default = 1;");
    let mut config = options();
    config.policy_allowlist =
        vec![PolicyAllowance::new("main.mjs", [PolicyName::EsmRequireContext]).unwrap()];
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    assert_eq!(cold.warnings.len(), 1);
    assert!(cold.warnings[0].contains("esm-require-context"));
    let memory = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&cold.product, &memory.product));
    assert_eq!(memory.warnings, cold.warnings);
    let mut disk_session = BuildSession::new(dir.root()).unwrap();
    let disk = disk_session.build(&config).unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.warnings, cold.warnings);
    config.policy_allowlist.clear();
    for session in [&mut session, &mut disk_session] {
        let error = session.build(&config).err().unwrap();
        let failures = error.downcast_ref::<PreparationErrors>().unwrap();
        assert!(failures.errors().values().flatten().any(|error| {
            error
                .downcast_ref::<ast_segmenter::policy::PolicyErrors>()
                .is_some_and(|cause| {
                    cause.violations()[0].policy() == PolicyName::EsmRequireContext
                })
        }));
    }
    config.policy_allowlist =
        vec![PolicyAllowance::new("other.mjs", [PolicyName::EsmRequireContext]).unwrap()];
    assert!(BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .is_err());
}

#[derive(Clone, Default)]
struct Diagnostics(Arc<Mutex<Vec<String>>>);

#[test]
fn guarded_computed_module_probe_preserves_browser_fallback_and_cache_parity() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "export function load() { return require('./probe.cjs'); }",
    );
    write(
        dir.root(),
        "probe.cjs",
        r#"
        trace.push('factory');
        if (globalThis.clearModule) module = null;
        var enqueueTaskImpl;
        try {
            var requireString = ('require' + Math.random()).slice(0, 7);
            var nodeRequire = module && module[requireString];
            enqueueTaskImpl = nodeRequire.call(module, 'timers').setImmediate;
        } catch (error) {
            trace.push('fallback');
            var channel = new MessageChannel();
            channel.port1.onmessage = function() { trace.push('delivered'); };
            enqueueTaskImpl = function() { channel.port2.postMessage(null); };
        }
        exports.enqueue = function() { enqueueTaskImpl(); };
        "#,
    );
    let mut config = options();
    config.resolver.target_env = TargetEnv::Browser;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    let memory = session.build(&config).unwrap();
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(cold.counters.source_parses, 2);
    assert_eq!(memory.counters.source_parses, 0);
    assert_eq!(memory.counters.memory_hits, 2);
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.counters.disk_hits, 2);
    assert!(cold.warnings.is_empty());
    assert!(memory.warnings.is_empty());
    assert!(disk.warnings.is_empty());
    assert_eq!(cold.product.artifacts.backend, crate::link::Backend::Native);
    assert_eq!(cold.product.artifacts, disk.product.artifacts);
    let probe = cold
        .product
        .graph
        .file_id(&dir.root().join("probe.cjs"))
        .unwrap();
    assert!(cold
        .product
        .graph
        .file_facts(probe)
        .unwrap()
        .dependencies()
        .is_empty());
    for mode in ["absent", "throw", "callable", "null"] {
        let entry = serde_json::to_string(&format!(
            "./{}",
            cold.product.artifacts.entries["main"].display()
        ))
        .unwrap();
        let check = format!(
            r#"
            import assert from 'node:assert/strict';
            globalThis.trace = [];
            globalThis.clearModule = '{mode}' === 'null';
            globalThis.MessageChannel = class {{
                constructor() {{
                    trace.push('channel');
                    this.port1 = {{}};
                    this.port2 = {{postMessage: () => this.port1.onmessage()}};
                }}
            }};
            if ('{mode}' !== 'absent') Object.defineProperty(Object.prototype, 'require', {{
                configurable: true,
                get() {{
                    assert.equal(Object.hasOwn(this, 'exports'), true);
                    trace.push('get');
                    if ('{mode}' === 'throw') throw Error('unavailable');
                    return function(request) {{
                        assert.equal(Object.hasOwn(this, 'exports'), true);
                        assert.equal(request, 'timers');
                        trace.push('call');
                        return {{setImmediate() {{ trace.push('immediate'); }}}};
                    }};
                }}
            }});
            try {{
                const ns = await import({entry});
                assert.deepEqual(trace, []);
                const value = ns.load();
                assert.equal(ns.load(), value);
                value.enqueue();
                assert.deepEqual(trace, '{mode}' === 'callable'
                    ? ['factory', 'get', 'call', 'immediate']
                    : '{mode}' === 'throw'
                        ? ['factory', 'get', 'fallback', 'channel', 'delivered']
                        : ['factory', 'fallback', 'channel', 'delivered']);
            }} finally {{
                delete Object.prototype.require;
            }}
            console.log('probe passed');
            "#
        );
        for build in [&cold, &memory, &disk] {
            assert_eq!(
                observe_named(&build.product.artifacts, &check),
                b"probe passed\n"
            );
        }
    }
}

#[test]
fn guarded_computed_module_probe_does_not_cache_adjacent_module_waivers() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.cjs",
        "const key = 'require'; const r = module && module[key]; r.call(module, 'timers'); module[other];",
    );
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.cjs".into();
    config.policy_allowlist =
        vec![PolicyAllowance::new("main.cjs", [PolicyName::ModuleAccess]).unwrap()];
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    let memory = session.build(&config).unwrap();
    let mut disk_session = BuildSession::new(dir.root()).unwrap();
    let disk = disk_session.build(&config).unwrap();
    assert_eq!(cold.warnings.len(), 1);
    assert_eq!(memory.warnings, cold.warnings);
    assert_eq!(disk.warnings, cold.warnings);
    assert_eq!(memory.counters.memory_hits, 1);
    assert_eq!(disk.counters.disk_hits, 1);
    config.policy_allowlist.clear();
    for session in [&mut session, &mut disk_session] {
        let error = session.build(&config).err().unwrap();
        let failures = error.downcast_ref::<PreparationErrors>().unwrap();
        let cause = failures.errors().values().next().unwrap()[0]
            .downcast_ref::<ast_segmenter::policy::PolicyErrors>()
            .unwrap();
        assert_eq!(cause.violations().len(), 1);
        assert_eq!(cause.violations()[0].policy(), PolicyName::ModuleAccess);
        assert!(format!("{error:#}").contains("module[other]"));
    }
    assert!(BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .is_err());
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Diagnostics {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != "bundler::diagnostic" {
            return;
        }
        struct Message(String);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, _: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{value:?}"));
            }
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0.lock().unwrap().push(message.0);
    }
}

#[test]
fn policy_waivers_reapply_on_cold_disk_memory_and_noop_builds_and_can_be_revoked() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.cjs",
        "module.exports = eval('({answer: 42})');",
    );
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.cjs".into();
    config.policy_allowlist =
        vec![PolicyAllowance::new("main.cjs", [PolicyName::UsesEval]).unwrap()];
    let capture = Diagnostics::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(capture.clone()));
    tracing::dispatcher::with_default(&dispatch, || {
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        assert_eq!(cold.warnings.len(), 1);
        assert!(cold.warnings[0].contains("bundled output may be incorrect"));
        assert!(cold.warnings[0].contains("policy uses-eval"));
        assert!(cold
            .product
            .artifacts
            .files
            .values()
            .any(|text| text.contains("eval('({answer: 42})')")));
        let memory = session.build(&config).unwrap();
        assert_eq!(memory.counters.source_parses, 0);
        assert_eq!(memory.counters.memory_hits, 1);
        assert_eq!(memory.warnings, cold.warnings);
        assert!(Rc::ptr_eq(&memory.product, &cold.product));
        let mut disk_session = BuildSession::new(dir.root()).unwrap();
        let disk = disk_session.build(&config).unwrap();
        assert_eq!(disk.counters.disk_hits, 1);
        assert_eq!(disk.counters.source_parses, 0);
        assert_eq!(disk.warnings, cold.warnings);
        assert_eq!(capture.0.lock().unwrap().len(), 3);
        config.policy_allowlist.clear();
        for session in [&mut session, &mut disk_session] {
            let error = session.build(&config).err().unwrap();
            let failures = error.downcast_ref::<PreparationErrors>().unwrap();
            let cause = failures.errors().values().next().unwrap()[0]
                .downcast_ref::<ast_segmenter::policy::PolicyErrors>()
                .unwrap();
            assert_eq!(cause.violations()[0].policy(), PolicyName::UsesEval);
            assert!(format!("{error:#}").contains("eval('({answer: 42})')"));
            assert!(session.last.is_some());
        }
        assert!(BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .is_err());
    });
}

#[test]
fn policy_waivers_match_only_the_exact_source_and_policy() {
    let dir = TmpDir::new();
    write(dir.root(), "main.cjs", "eval('one'); module.require('x');");
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.cjs".into();
    config.cache = CacheMode::Disabled;
    config.policy_allowlist = vec![PolicyAllowance::new(
        "other.cjs",
        [PolicyName::UsesEval, PolicyName::ModuleAccess],
    )
    .unwrap()];
    let mut session = BuildSession::new(dir.root()).unwrap();
    let denied = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        denied.contains("policy uses-eval") && denied.contains("policy module-access"),
        "{denied}"
    );
    config.policy_allowlist =
        vec![PolicyAllowance::new("main.cjs", [PolicyName::UsesEval]).unwrap()];
    let capture = Diagnostics::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(capture.clone()));
    tracing::dispatcher::with_default(&dispatch, || {
        let denied = format!("{:#}", session.build(&config).err().unwrap());
        assert!(!denied.contains("policy uses-eval"), "{denied}");
        assert!(denied.contains("policy module-access"), "{denied}");
        assert!(capture
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|message| message.contains("POLICY WAIVER") && message.contains("eval('one')")));
    });
}

#[test]
fn resolver_aliases_do_not_expand_policy_allowlist_paths() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import 'alias.cjs';");
    write(dir.root(), "actual.cjs", "module.exports = eval('42');");
    let mut config = options();
    config
        .resolver
        .alias
        .insert("alias.cjs".into(), "./actual.cjs".into());
    config.policy_allowlist =
        vec![PolicyAllowance::new("alias.cjs", [PolicyName::UsesEval]).unwrap()];
    let mut session = BuildSession::new(dir.root()).unwrap();
    let denied = format!("{:#}", session.build(&config).err().unwrap());
    assert!(denied.contains("actual.cjs: policy uses-eval"), "{denied}");
    config.policy_allowlist =
        vec![PolicyAllowance::new("actual.cjs", [PolicyName::UsesEval]).unwrap()];
    let allowed = session.build(&config).unwrap();
    assert_eq!(allowed.warnings.len(), 1);
    assert!(allowed.warnings[0].contains("actual.cjs: policy uses-eval"));
    let outside = dir.root().parent().unwrap().join("actual.cjs");
    let admission = policy::Admission::check(
        dir.root(),
        &outside,
        "eval('42')",
        [ast_segmenter::policy::PolicyViolation::new(
            PolicyName::UsesEval,
            oxc_span::Span::new(0, 4),
        )],
        &config.policy_allowlist,
    );
    assert!(admission.error.is_some());
    assert!(admission.warnings.is_empty());
}

#[test]
fn demanded_policy_errors_log_before_extraction_finishes_and_drain_reachable_work() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './absent.mjs'; import './bad.mjs'; import './valid.mjs'; eval('main');",
    );
    write(dir.root(), "bad.mjs", "import './deeper.mjs'; eval('bad');");
    write(dir.root(), "deeper.mjs", "eval('deeper');");
    write(
        dir.root(),
        "valid.mjs",
        "import './data.json'; export const valid = 42;",
    );
    write(dir.root(), "data.json", "{invalid}");
    write(dir.root(), "other.json", "{invalid too}");
    write(dir.root(), "last.mjs", "export const last = 1;");
    write(dir.root(), "syntax.mjs", "export const = ;");
    let mut config = options();
    config.entries.insert(
        "other".into(),
        Entry {
            path: "other.json".into(),
            output_name: None,
            namespace: true,
        },
    );
    config.entries.insert(
        "last".into(),
        Entry {
            path: "last.mjs".into(),
            output_name: None,
            namespace: true,
        },
    );
    for path in ["missing-entry.mjs", "syntax.mjs"] {
        config.entries.insert(
            path.into(),
            Entry {
                path: path.into(),
                output_name: None,
                namespace: true,
            },
        );
    }
    config.cache = CacheMode::Disabled;
    config.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(3).unwrap());
    let capture = Diagnostics::default();
    let messages = capture.clone();
    let _reset = PreparationObserver;
    // This runs on the worker after the early syntax event, but before recipes/Ecco.
    prepare::AFTER_DEPENDENCIES.with(|slot| {
        *slot.borrow_mut() = Some(Arc::new(move |path| {
            if path.file_name().unwrap() == "main.mjs" {
                let started = Instant::now();
                loop {
                    if messages
                        .0
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|message| message.contains("eval('main')"))
                    {
                        break;
                    }
                    assert!(
                        started.elapsed() < Duration::from_secs(10),
                        "policy failure was held until extraction completed"
                    );
                    std::thread::yield_now();
                }
            }
        }));
    });
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(capture.clone()));
    tracing::dispatcher::with_default(&dispatch, || {
        let mut session = BuildSession::new(dir.root()).unwrap();
        let error = session.build(&config).err().unwrap();
        let failures = error.downcast_ref::<PreparationErrors>().unwrap();
        assert_eq!(failures.errors().len(), 7, "{error:#}");
        let summary = format!("{error:#}");
        assert!(summary.contains("absent.mjs"), "{summary}");
        for source in [
            "main.mjs",
            "bad.mjs",
            "deeper.mjs",
            "data.json",
            "other.json",
            "missing-entry.mjs",
            "syntax.mjs",
        ] {
            assert!(summary.contains(source), "{summary}");
        }
        for valid in ["valid.mjs", "last.mjs"] {
            assert!(
                session.sources.contains_key(&dir.root().join(valid)),
                "{valid} was not prepared"
            );
        }
        assert!(session.last.is_none());
        assert!(!dir.root().join(".bundler-current.json").exists());
        assert_eq!(
            capture
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|message| message.contains("policy uses-eval"))
                .count(),
            3
        );
    });
}

#[test]
fn unused_speculative_policy_errors_and_waivers_stay_private() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    write(dir.root(), "main.mjs", "export const value = 1;");
    write(dir.root(), "unused.mjs", "eval('unused');");
    let (done, wait) = std::sync::mpsc::channel();
    let wait = Mutex::new(wait);
    let observer = observe_preparation(move |path| {
        if path.file_name().unwrap() == "unused.mjs" {
            done.send(()).unwrap();
        }
        if path.file_name().unwrap() == "main.mjs" {
            wait.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
    });
    let mut config = options();
    config.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(3).unwrap());
    config.policy_allowlist =
        vec![PolicyAllowance::new("unused.mjs", [PolicyName::UsesEval]).unwrap()];
    let capture = Diagnostics::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(capture.clone()));
    tracing::dispatcher::with_default(&dispatch, || {
        let built = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert!(built.warnings.is_empty());
        assert!(capture.0.lock().unwrap().is_empty());
    });
    drop(observer);
}

#[test]
fn promoted_speculative_policy_evidence_is_reported_before_the_owner_finishes() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    write(dir.root(), "main.mjs", "import './dep.mjs';");
    write(dir.root(), "dep.mjs", "eval('promoted');");
    let (started, wait) = std::sync::mpsc::channel();
    let wait = Mutex::new(wait);
    let capture = Diagnostics::default();
    let messages = capture.clone();
    let observer = observe_preparation(move |path| {
        if path.file_name().unwrap() == "main.mjs" {
            wait.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
        if path.file_name().unwrap() == "dep.mjs" {
            started.send(()).unwrap();
            let started = Instant::now();
            loop {
                if messages
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|message| message.contains("POLICY WAIVER"))
                {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "available promoted waiver was held until final completion"
                );
                std::thread::yield_now();
            }
        }
    });
    let mut config = options();
    config.cache = CacheMode::Disabled;
    config.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(3).unwrap());
    config.policy_allowlist =
        vec![PolicyAllowance::new("dep.mjs", [PolicyName::UsesEval]).unwrap()];
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(capture.clone()));
    tracing::dispatcher::with_default(&dispatch, || {
        let built = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(built.warnings.len(), 1);
        assert_eq!(capture.0.lock().unwrap().len(), 1);
    });
    drop(observer);
}
