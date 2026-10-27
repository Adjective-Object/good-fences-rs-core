use crate::{trace::open_trace, Cli};
use clap::Parser;
use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
};
use test_tmpdir::TmpDir;

// Scoped subscribers share tracing's process-wide callsite interest cache.
static CLI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn project_policy_waivers_keep_stdout_json_and_diagnostics_out_of_duration_traces() {
    let dir = TmpDir::new();
    let root = dir.root();
    std::fs::write(root.join("entry.cjs"), "module.exports = eval('42');").unwrap();
    let config = |entries: &[&str], trace: &str| {
        serde_json::json!({
            "entries": entries,
            "traceFile": trace,
            "policyAllowlist": [{"source": "entry.cjs", "policies": ["uses-eval"]}]
        })
        .to_string()
    };
    std::fs::write(
        root.join("bundler.config.json"),
        config(&["entry.cjs"], ".bundler-trace-policy-success.json"),
    )
    .unwrap();
    let mut stdout = Vec::new();
    run_from(root, [], &mut stdout).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(report["warnings"].as_array().unwrap().len(), 1);
    assert!(report["warnings"][0]
        .as_str()
        .unwrap()
        .contains("bundled output may be incorrect"));
    std::fs::write(root.join("other.cjs"), "eval('unallowed');").unwrap();
    std::fs::write(
        root.join("bundler.config.json"),
        config(
            &["entry.cjs", "other.cjs"],
            ".bundler-trace-policy-error.json",
        ),
    )
    .unwrap();
    stdout.clear();
    let error = run_from(root, [], &mut stdout).unwrap_err();
    assert!(stdout.is_empty());
    assert!(format!("{error:#}").contains("other.cjs: policy uses-eval"));
    for trace in [
        ".bundler-trace-policy-success.json",
        ".bundler-trace-policy-error.json",
    ] {
        let path = root.join(trace);
        assert!(!complete_trace(&path).is_empty());
        let bytes = std::fs::read_to_string(path).unwrap();
        for excluded in [
            "POLICY WAIVER",
            "uses-eval",
            "unallowed",
            "eval('42')",
            "bundler::diagnostic",
        ] {
            assert!(!bytes.contains(excluded), "{excluded} leaked into {trace}");
        }
    }
}

#[test]
fn failure_context_preserves_config_causes_and_module_stages() {
    let dir = TmpDir::new();
    let root = dir.root();
    let config = root.join("bundler.config.json");
    let error = run_from(root, [], &mut Vec::new()).unwrap_err();
    assert!(format!("{error:#}").contains(&config.display().to_string()));
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );

    std::fs::write(&config, "{ invalid JSON").unwrap();
    let error = run_from(root, [], &mut Vec::new()).unwrap_err();
    assert!(format!("{error:#}").contains(&config.display().to_string()));
    assert!(error.downcast_ref::<serde_json::Error>().is_some());

    std::fs::write(
        &config,
        r#"{"entries":["entry.mjs"],"cache":"disabled","workers":2}"#,
    )
    .unwrap();
    for (source, expected) in [
        ("export const = ;", "parser diagnostics"),
        ("import './absent.mjs';", "resolving"),
    ] {
        std::fs::write(root.join("entry.mjs"), source).unwrap();
        let error = run_from(root, [], &mut Vec::new()).unwrap_err();
        let chain = format!("{error:#}");
        for context in ["building project", "entry.mjs", expected] {
            assert!(chain.contains(context), "{chain}");
        }
    }
}

#[test]
fn cli_embeds_as_a_subcommand_and_runs_with_an_explicit_working_directory() {
    #[derive(clap::Parser)]
    struct Host {
        #[command(subcommand)]
        command: Commands,
    }

    #[derive(clap::Subcommand)]
    enum Commands {
        Bundle(Cli),
    }

    let dir = TmpDir::new();
    std::fs::write(dir.root().join("entry.mjs"), "export const value = 42;").unwrap();
    std::fs::write(
        dir.root().join("bundler.config.json"),
        r#"{"entries":["entry.mjs"]}"#,
    )
    .unwrap();
    let host = Host::try_parse_from(["host", "bundle"]).unwrap();
    let Commands::Bundle(cli) = host.command;
    let mut stdout = Vec::new();
    let _cli = CLI_TEST_LOCK.lock().unwrap();
    cli.run_from(dir.root(), &mut stdout).unwrap();
    let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(result["backend"], "native");
    assert!(result["entries"]["entry.mjs"].is_string());
}

fn run(args: impl IntoIterator<Item = OsString>, stdout: &mut impl Write) -> eyre::Result<()> {
    run_from(&std::env::current_dir()?, args, stdout)
}

fn run_from(
    cwd: &Path,
    args: impl IntoIterator<Item = OsString>,
    stdout: &mut impl Write,
) -> eyre::Result<()> {
    let _cli = CLI_TEST_LOCK.lock().unwrap();
    Cli::try_parse_from(std::iter::once(OsString::from("bundler-cli")).chain(args))?
        .run_from(cwd, stdout)
}

fn args(root: &Path, extra: &[&str]) -> Vec<OsString> {
    [
        root.as_os_str().to_owned(),
        root.join(".bundler-out").into_os_string(),
    ]
    .into_iter()
    .chain(extra.iter().map(OsString::from))
    .collect()
}

#[test]
fn project_config_resolves_paths_from_its_location_and_supports_discovery() {
    let dir = TmpDir::new();
    let cwd = dir.root();
    let project = cwd.join("project");
    let root = project.join("sources");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("entry.mjs"), "export const value = 42;").unwrap();
    std::fs::write(
        project.join("bundler.config.json"),
        r#"{
            "root": "sources",
            "outdir": ".bundler-custom-output",
            "entries": ["entry.mjs"],
            "target": "browser",
            "workers": 2,
            "cache": "disabled",
            "output": {"source_maps": true}
        }"#,
    )
    .unwrap();
    std::fs::write(cwd.join("bundler.config.json"), "invalid").unwrap();
    let relative_config = Path::new("project").join("bundler.config.json");
    let commands = [
        vec![OsString::from(format!(
            "--config={}",
            relative_config.display()
        ))],
        vec![
            OsString::from("--config"),
            project.join("bundler.config.json").into_os_string(),
        ],
        Vec::new(),
    ];
    let mut previous = None;
    for (index, command) in commands.into_iter().enumerate() {
        let mut stdout = Vec::new();
        run_from(
            if index == 2 { &project } else { cwd },
            command,
            &mut stdout,
        )
        .unwrap();
        let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(result["backend"], "native");
        assert_eq!(result["sourceParses"], 1);
        assert_eq!(result["entries"].as_object().unwrap().len(), 1);
        let directory = PathBuf::from(result["directory"].as_str().unwrap());
        assert!(directory.starts_with(root.canonicalize().unwrap().join(".bundler-custom-output")));
        let entry = directory.join(result["entries"]["entry.mjs"].as_str().unwrap());
        assert!(entry.is_file());
        assert!(std::fs::read_to_string(&entry)
            .unwrap()
            .contains("sourceMappingURL="));
        if let Some(previous) = previous {
            assert_eq!(result["directory"], previous);
            assert_eq!(result["artifactWrites"], 0);
        }
        previous = Some(result["directory"].clone());
    }
}

#[test]
fn resolve_extensions_selects_compound_suffixes_in_order_for_project_and_fixture_builds() {
    let dir = TmpDir::new();
    let root = dir.root();
    std::fs::write(
        root.join("entry.mjs"),
        "export { value } from './component'; export { value as indexed } from './parts';",
    )
    .unwrap();
    std::fs::write(
        root.join("component.js"),
        "export const value = 'PLAIN_EXTENSION_SELECTED';",
    )
    .unwrap();
    std::fs::write(
        root.join("component.web.js"),
        "export const value = 'WEB_EXTENSION_SELECTED';",
    )
    .unwrap();
    std::fs::create_dir(root.join("parts")).unwrap();
    for (filename, value) in [
        ("index.js", "PLAIN_EXTENSION_SELECTED"),
        ("index.web.js", "WEB_EXTENSION_SELECTED"),
    ] {
        std::fs::write(
            root.join("parts").join(filename),
            format!("export const value = '{value}';"),
        )
        .unwrap();
    }
    let config_path = root.join("bundler.config.json");
    let fixture_path = root.join("settings.json");
    for (extensions, expected, excluded) in [
        (None, "PLAIN_EXTENSION_SELECTED", "WEB_EXTENSION_SELECTED"),
        (
            Some(serde_json::json!([".web.js", ".js"])),
            "WEB_EXTENSION_SELECTED",
            "PLAIN_EXTENSION_SELECTED",
        ),
        (
            Some(serde_json::json!(["js", "web.js"])),
            "PLAIN_EXTENSION_SELECTED",
            "WEB_EXTENSION_SELECTED",
        ),
    ] {
        let mut settings = serde_json::json!({"target": "browser"});
        if let Some(extensions) = extensions {
            settings["resolutionExtensions"] = extensions;
        }
        std::fs::write(&fixture_path, serde_json::to_vec(&settings).unwrap()).unwrap();
        settings["entries"] = serde_json::json!(["entry.mjs"]);
        std::fs::write(&config_path, serde_json::to_vec(&settings).unwrap()).unwrap();
        for command in [
            Vec::new(),
            args(root, &["--fixture-config", "settings.json", "entry.mjs"]),
        ] {
            let mut stdout = Vec::new();
            run_from(root, command, &mut stdout).unwrap();
            let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
            let entry = PathBuf::from(result["directory"].as_str().unwrap())
                .join(result["entries"]["entry.mjs"].as_str().unwrap());
            let emitted = std::fs::read_to_string(entry).unwrap();
            assert!(emitted.contains(expected), "{emitted}");
            assert!(!emitted.contains(excluded), "{emitted}");
        }
    }
}

#[test]
fn resolve_extensions_can_disable_probing_without_disabling_explicit_imports() {
    let dir = TmpDir::new();
    let root = dir.root();
    std::fs::write(
        root.join("bundler.config.json"),
        r#"{"entries":["entry.mjs"],"resolutionExtensions":[]}"#,
    )
    .unwrap();
    std::fs::write(root.join("component.js"), "export const value = 1;").unwrap();
    std::fs::write(
        root.join("entry.mjs"),
        "export { value } from './component';",
    )
    .unwrap();
    let mut stdout = Vec::new();
    let error = run_from(root, [], &mut stdout).unwrap_err();
    assert!(format!("{error:#}").contains("resolving"), "{error:#}");
    assert!(stdout.is_empty());

    std::fs::write(
        root.join("entry.mjs"),
        "export { value } from './component.js';",
    )
    .unwrap();
    run_from(root, [], &mut stdout).unwrap();
    let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(result["backend"], "native");
}

#[test]
fn resolve_extensions_rejects_invalid_suffixes_before_building() {
    let dir = TmpDir::new();
    let root = dir.root();
    for suffix in [
        "", ".", "..js", "web..js", "js.", " ", " js", "j s", "js\n", "../js", "web/js", "web\\js",
        "C:js", "*.js", "j?s", "j\"s", "j<s", "j>s", "j|s", "js\0",
    ] {
        let config = serde_json::json!({
            "entries": ["entry.mjs"],
            "resolutionExtensions": [suffix]
        });
        std::fs::write(
            root.join("bundler.config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let mut stdout = Vec::new();
        let error = run_from(root, [], &mut stdout).unwrap_err();
        assert!(
            format!("{error:#}").contains("resolutionExtensions"),
            "{suffix:?}: {error:#}"
        );
        assert!(stdout.is_empty());
    }
    assert!(!root.join(".bundler-out").exists());
}

#[test]
fn project_defaults_and_legacy_calls_do_not_depend_on_another_config() {
    let dir = TmpDir::new();
    let root = dir.root();
    std::fs::write(root.join("entry.mjs"), "export const value = 1;").unwrap();
    let config = root.join("bundler.config.json");
    std::fs::write(&config, r#"{"entries":["entry.mjs"]}"#).unwrap();
    let child = root.join("child");
    std::fs::create_dir(&child).unwrap();
    assert!(run_from(&child, [], &mut Vec::new()).is_err());
    let mut stdout = Vec::new();
    run_from(root, [], &mut stdout).unwrap();
    let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert!(Path::new(result["directory"].as_str().unwrap())
        .starts_with(root.canonicalize().unwrap().join(".bundler-out")));

    std::fs::write(config, "invalid").unwrap();
    let mut stdout = Vec::new();
    run_from(root, args(root, &["entry.mjs"]), &mut stdout).unwrap();
    let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(result["backend"], "native");
    assert!(run_from(root, [], &mut Vec::new()).is_err());
}

#[test]
fn project_trace_paths_and_cli_overrides_preserve_trace_safety() {
    let dir = TmpDir::new();
    let root = dir.root();
    std::fs::write(root.join("entry.mjs"), "export const value = 1;").unwrap();
    std::fs::write(
        root.join("bundler.config.json"),
        r#"{"entries":["entry.mjs"],"traceFile":".bundler-trace-config.json"}"#,
    )
    .unwrap();
    run_from(root, [], &mut Vec::new()).unwrap();
    let configured = root.join(".bundler-trace-config.json");
    complete_trace(&configured);
    let original = std::fs::read(&configured).unwrap();
    run_from(
        root,
        [OsString::from("--trace-file=.bundler-trace-override.json")],
        &mut Vec::new(),
    )
    .unwrap();
    complete_trace(&root.join(".bundler-trace-override.json"));
    assert_eq!(std::fs::read(configured).unwrap(), original);

    std::fs::write(root.join("entry.mjs"), "export const = ;").unwrap();
    assert!(run_from(
        root,
        [
            OsString::from("--trace-file"),
            OsString::from(".bundler-trace-failure.json")
        ],
        &mut Vec::new(),
    )
    .is_err());
    complete_trace(&root.join(".bundler-trace-failure.json"));
}

#[test]
fn project_config_errors_never_fall_back_to_discovery_or_build() {
    let dir = TmpDir::new();
    let root = dir.root();
    let config = root.join("bundler.config.json");
    std::fs::write(root.join("entry.mjs"), "export const value = 1;").unwrap();
    std::fs::write(&config, r#"{"entries":["entry.mjs"]}"#).unwrap();
    for args in [
        vec!["--config"],
        vec!["--config="],
        vec!["--config", "--trace-file", "trace.json"],
        vec!["--config", "missing.json"],
        vec!["--config=missing.json"],
        vec!["--config=a", "--config", "b"],
        vec!["--config=a", "--fixture-config=b"],
        vec!["--config=bundler.config.json", "root", "output"],
        vec!["--unknown"],
    ] {
        let mut stdout = Vec::new();
        assert!(
            run_from(root, args.iter().map(OsString::from), &mut stdout).is_err(),
            "{args:?}"
        );
        assert!(stdout.is_empty());
    }
    for content in [
        "invalid".to_owned(),
        "{}".to_owned(),
        r#"{"entries":[]}"#.to_owned(),
        r#"{"entries":["entry.mjs"],"typo":true}"#.to_owned(),
        r#"{"entries":["entry.mjs"],"output":{"typo":true}}"#.to_owned(),
        " ".repeat(16 * 1024 + 1),
    ] {
        std::fs::write(&config, content).unwrap();
        assert!(run_from(root, [], &mut Vec::new()).is_err());
    }
    assert!(!root.join(".bundler-out").exists());
}

fn complete_trace(path: &Path) -> Vec<serde_json::Value> {
    let bytes = std::fs::read(path).unwrap();
    let events: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    let mut stacks = std::collections::BTreeMap::<u64, Vec<String>>::new();
    for event in &events {
        let thread = event["tid"].as_u64().unwrap();
        match event["ph"].as_str().unwrap() {
            "B" => stacks
                .entry(thread)
                .or_default()
                .push(event["name"].as_str().unwrap().to_owned()),
            "E" => {
                stacks
                    .get_mut(&thread)
                    .unwrap()
                    .pop()
                    .expect("unmatched span exit");
            }
            _ => {}
        }
    }
    assert!(stacks.values().all(Vec::is_empty));
    assert!(
        events
            .iter()
            .any(|event| event["name"] == "build" && event["ph"] == "B"),
        "{}: missing build span",
        path.display()
    );
    events
}

#[test]
fn tracing_preserves_json_and_artifacts_and_flushes_on_returned_error() {
    let dir = TmpDir::new();
    let root = dir.root();
    std::fs::write(
        root.join("entry.mjs"),
        "import './other.mjs'; export const value = 'PRIVATE_SOURCE_LITERAL';",
    )
    .unwrap();
    std::fs::write(root.join("other.mjs"), "export const other = 2;").unwrap();
    std::fs::write(root.join("workers.json"), r#"{"workers":4}"#).unwrap();
    let trace_args = |name: &str| {
        let mut command = args(root, &["--trace-file"]);
        command.push(root.join(name).into_os_string());
        command.push("--fixture-config".into());
        command.push(root.join("workers.json").into_os_string());
        command.push("entry.mjs".into());
        command.push("other.mjs".into());
        command
    };
    let mut cold = Vec::new();
    run(trace_args(".bundler-trace-cold.json"), &mut cold).unwrap();
    let cold: serde_json::Value = serde_json::from_slice(&cold).unwrap();
    assert_eq!(cold["sourceParses"], 2);
    assert_eq!(cold["declarationFingerprints"], 1);
    let cold_events = complete_trace(&root.join(".bundler-trace-cold.json"));
    let cold_coordinator = &cold_events
        .iter()
        .find(|event| event["name"] == "build")
        .unwrap()["tid"];
    for name in ["discovery.package", "discovery.directory"] {
        assert!(
            cold_events
                .iter()
                .any(|event| event["name"] == name && &event["tid"] != cold_coordinator),
            "{name}: {cold_events:?}"
        );
    }
    for name in [
        "build.declarations",
        "inventory.admit",
        "inventory.recheck",
        "build.resolver_policy",
        "resolve.lockfile",
        "resolve.recheck",
        "resolve.counts",
        "prepare.package_format",
        "prepare.read_bytes",
        "cache.extract",
        "cache.validate",
        "cache.encode",
        "cache.publish",
        "build.resolve",
        "resolve.dependency",
        "build.plan",
        "emit.planning",
        "emit.owner",
        "emit.finalize",
        "build.finalize",
        "build.render",
        "build.recheck",
        "build.publish",
    ] {
        assert!(
            cold_events.iter().any(|event| event["name"] == name),
            "{name}"
        );
    }
    for name in [
        "build.inventory",
        "build.resolution_fingerprint",
        "inventory.read_directory",
        "inventory.config_probes",
    ] {
        assert!(
            !cold_events.iter().any(|event| event["name"] == name),
            "{name}"
        );
    }
    let mut plain = Vec::new();
    run(args(root, &["entry.mjs", "other.mjs"]), &mut plain).unwrap();
    let plain: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    let mut traced = Vec::new();
    run(trace_args(".bundler-trace-success.json"), &mut traced).unwrap();
    let traced: serde_json::Value = serde_json::from_slice(&traced).unwrap();
    assert_eq!(plain, traced);
    assert_eq!(cold["directory"], traced["directory"]);
    assert_eq!(traced["diskHits"], 2);
    assert_eq!(cold["resolutions"], 1);
    assert_eq!(cold["resolutionCacheHits"], 0);
    assert_eq!(cold["resolutionCacheMisses"], 1);
    assert_eq!(traced["resolutions"], 0);
    assert_eq!(traced["resolutionCacheHits"], 1);
    assert_eq!(traced["resolutionCacheMisses"], 0);
    assert_eq!(traced["inventoryDirectoryReads"], 0);
    assert_eq!(traced["sourceParses"], 0);
    assert_eq!(traced["sourceReparses"], 0);
    assert_eq!(traced["fragmentReparses"], 0);
    assert_eq!(traced["finalValidations"], cold["finalValidations"]);
    assert_eq!(traced["outputParses"], traced["finalValidations"]);
    assert_eq!(traced["artifactWrites"], 0);
    let trace_path = root.join(".bundler-trace-success.json");
    let events = complete_trace(&trace_path);
    for name in ["cache.read", "cache.decode", "cache.validate"] {
        assert!(events.iter().any(|event| event["name"] == name), "{name}");
    }
    let coordinator = &events
        .iter()
        .find(|event| event["name"] == "build")
        .unwrap()["tid"];
    for name in ["prepare.job", "emit.owner"] {
        let jobs: Vec<_> = events
            .iter()
            .filter(|event| event["name"] == name)
            .collect();
        assert!(!jobs.is_empty());
        assert!(
            jobs.iter().any(|event| &event["tid"] != coordinator),
            "{name}: {jobs:?}; coordinator: {coordinator}"
        );
    }
    assert!(events.iter().any(|event| event["name"] == "emit.owner"
        && event["args"]["owner_id"]
            .as_str()
            .is_some_and(|id| id.parse::<usize>().is_ok())));
    assert!(events.iter().any(|event| event["name"] == "build.publish"));
    for path in [root.join(".bundler-trace-cold.json"), trace_path] {
        let text = std::fs::read_to_string(path).unwrap();
        for private in [
            "PRIVATE_SOURCE_LITERAL",
            "entry.mjs",
            "other.mjs",
            "workers.json",
        ] {
            assert!(!text.contains(private));
        }
    }
    assert!(!String::from_utf8(serde_json::to_vec(&traced).unwrap())
        .unwrap()
        .contains("PRIVATE_SOURCE_LITERAL"));

    // A parse failure occurs after the trace and source worker have started.
    std::fs::write(root.join("entry.mjs"), "export const = ;").unwrap();
    let error_path = root.join(".bundler-trace-error.json");
    let mut command = args(root, &["--trace-file"]);
    command.push(error_path.clone().into_os_string());
    command.push("--fixture-config".into());
    command.push(root.join("workers.json").into_os_string());
    command.push("entry.mjs".into());
    let mut stdout = Vec::new();
    assert!(run(command, &mut stdout).is_err());
    assert!(stdout.is_empty());
    let events = complete_trace(&error_path);
    assert!(events.iter().any(|event| event["name"] == "resolve.counts"));

    // Failure while writing the result still closes the completed build trace.
    struct BrokenOutput;
    impl Write for BrokenOutput {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    std::fs::write(root.join("entry.mjs"), "export const value = 1;").unwrap();
    assert!(run(
        trace_args(".bundler-trace-stdout-error.json"),
        &mut BrokenOutput
    )
    .is_err());
    complete_trace(&root.join(".bundler-trace-stdout-error.json"));

    std::fs::write(root.join("workers.json"), "invalid config").unwrap();
    assert!(run(
        trace_args(".bundler-trace-config-error.json"),
        &mut Vec::new()
    )
    .is_err());
    let events: Vec<serde_json::Value> = serde_json::from_slice(
        &std::fs::read(root.join(".bundler-trace-config-error.json")).unwrap(),
    )
    .unwrap();
    assert!(events.is_empty());
}

#[test]
fn trace_destinations_never_overwrite_inputs_outputs_or_inventory() {
    let dir = TmpDir::new();
    let root = dir.root();
    let output = root.join(".bundler-out");
    std::fs::create_dir(&output).unwrap();
    let input = root.join("entry.mjs");
    std::fs::write(&input, "original").unwrap();
    for path in [&input, &root.join("trace.json"), &output.join("trace.json")] {
        assert!(open_trace(root, &output, path).is_err());
    }
    let reserved = root.join(".bundler-trace-existing.json");
    std::fs::write(&reserved, "original").unwrap();
    assert!(open_trace(root, &output, &reserved).is_err());
    assert_eq!(std::fs::read_to_string(reserved).unwrap(), "original");
    assert_eq!(std::fs::read_to_string(input).unwrap(), "original");
    assert!(!output.join("trace.json").exists());
    assert!(!root.join("trace.json").exists());
    assert!(open_trace(root, &output, &root.join(".bundler-trace-file:stream")).is_err());
    let outer = root.join("project");
    std::fs::create_dir(&outer).unwrap();
    drop(open_trace(&outer, &output, &root.join("outside.json")).unwrap());
}

#[test]
fn trace_arguments_reject_missing_duplicate_and_unknown_options() {
    let dir = TmpDir::new();
    for extra in [
        vec!["--trace-file"],
        vec!["--trace-file", "a", "--trace-file", "b"],
        vec!["--fixture-config", "a", "--fixture-config", "b"],
        vec!["--unknown"],
    ] {
        assert!(run(args(dir.root(), &extra), &mut Vec::new()).is_err());
    }
}
