use std::process::{Command, Output};

use test_tmpdir::TmpDir;

fn command(dir: &TmpDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bundler-cli"))
        .current_dir(dir.root())
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_and_version_exit_successfully_without_building() {
    let dir = TmpDir::new();
    for flag in ["--help", "--version"] {
        let result = command(&dir, &[flag]);
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert!(String::from_utf8(result.stdout)
            .unwrap()
            .contains("bundler-cli"));
    }
    assert!(!dir.root().join(".bundler-out").exists());
}

#[test]
fn import_meta_env_project_configuration_builds_and_runs() {
    let dir = TmpDir::new();
    std::fs::write(
        dir.root().join("main.mjs"),
        "console.log(import.meta.env ? import.meta.env.MODE : void 0);",
    )
    .unwrap();
    std::fs::write(
        dir.root().join("bundler.config.json"),
        r#"{"entries":["main.mjs"],"target":"browser","env":{"MODE":"production"}}"#,
    )
    .unwrap();
    for _ in 0..2 {
        let result = command(&dir, &[]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        let entry = std::path::Path::new(report["directory"].as_str().unwrap())
            .join(report["entries"]["main.mjs"].as_str().unwrap());
        let observed = Command::new("node")
            .arg(entry.file_name().unwrap())
            .current_dir(entry.parent().unwrap())
            .output()
            .unwrap();
        assert!(
            observed.status.success(),
            "{}",
            String::from_utf8_lossy(&observed.stderr)
        );
        assert_eq!(observed.stdout, b"production\n");
    }
    std::fs::write(
        dir.root().join("bundler.config.json"),
        r#"{"entries":["main.mjs"],"env":[]}"#,
    )
    .unwrap();
    let result = command(&dir, &[]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("parsing configuration"));
}

#[test]
fn invalid_arguments_use_clap_diagnostics_without_building() {
    let dir = TmpDir::new();
    for args in [
        vec!["--unknown"],
        vec!["--config"],
        vec!["--config=a", "--config=b"],
        vec!["--config=a", "--fixture-config=b"],
        vec!["root"],
        vec!["--fixture-config=a"],
    ] {
        let result = command(&dir, &args);
        assert_eq!(result.status.code(), Some(2), "{args:?}");
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(stderr.starts_with("error:"), "{args:?}: {stderr}");
    }
    assert!(!dir.root().join(".bundler-out").exists());
}

#[test]
fn binary_publishes_project_and_positional_builds_as_json() {
    let dir = TmpDir::new();
    std::fs::write(dir.root().join("-entry.mjs"), "export const value = 42;").unwrap();
    std::fs::write(
        dir.root().join("bundler.config.json"),
        r#"{"entries":["-entry.mjs"],"workers":2}"#,
    )
    .unwrap();
    for args in [vec![], vec![".", ".bundler-out", "--", "-entry.mjs"]] {
        let result = command(&dir, &args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["backend"], "native");
        let directory = std::path::Path::new(report["directory"].as_str().unwrap());
        let entry = directory.join(report["entries"]["-entry.mjs"].as_str().unwrap());
        assert!(std::fs::read_to_string(entry).unwrap().contains("42"));
        assert_progress(&result.stderr);
    }
}

fn assert_progress(stderr: &[u8]) {
    let stderr = std::str::from_utf8(stderr).unwrap();
    for line in stderr.lines() {
        let mut prefix = line.split_whitespace();
        let elapsed = prefix
            .next()
            .unwrap()
            .strip_suffix('s')
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!(elapsed.is_finite() && elapsed >= 0.0, "{line}");
        assert_eq!(prefix.next(), Some("INFO"), "{line}");
    }
    let mut remaining = stderr;
    for phase in [
        "Loading build configuration",
        "Acquiring publication lock",
        "Preparing effect declarations",
        "Loading resolver and lockfile",
        "Discovering local source candidates",
        "Discovering dependency packages",
        "Traversing local source directory",
        "Discovered local source file",
        "Discovered local source candidates",
        "Preparing modules and resolving dependencies",
        "Modules prepared",
        "Checking build fingerprints",
        "Constructing and validating source graph",
        "Planning retained modules and chunks",
        "Linking and emitting native chunks",
        "Finalizing resources and rendering output",
        "Rechecking build inputs",
        "Publishing output artifacts",
        "Build complete",
    ] {
        let (_, rest) = remaining
            .split_once(phase)
            .unwrap_or_else(|| panic!("missing or out-of-order phase {phase}: {stderr}"));
        remaining = rest;
    }
    for field in ["modules=1", "disk_hits=", "elapsed_ms=", "files_written="] {
        assert!(stderr.contains(field), "{stderr}");
    }
    assert!(stderr.contains("file=\"-entry.mjs\""), "{stderr}");
    assert!(stderr.contains("directory=\".\""), "{stderr}");
    assert_eq!(stderr.matches("Discovered local source file").count(), 1);
    assert_eq!(
        stderr.matches("Traversing local source directory").count(),
        1
    );
    assert!(!stderr.contains('\u{1b}'), "{stderr}");
}

#[test]
fn progress_and_duration_trace_coexist_without_changing_json_or_rebuilding_artifacts() {
    let dir = TmpDir::new();
    std::fs::write(dir.root().join("-entry.mjs"), "export const value = 42;").unwrap();
    std::fs::write(
        dir.root().join("package-lock.json"),
        r#"{"name":"root","lockfileVersion":3,"packages":{
            "":{"name":"root","dependencies":{"a":"1","b":"1"}},
            "node_modules/a":{"version":"1"},
            "node_modules/b":{"version":"1"}
        }}"#,
    )
    .unwrap();
    std::fs::write(dir.root().join("workers.json"), r#"{"workers":4}"#).unwrap();
    let plain = command(
        &dir,
        &[
            ".",
            ".bundler-out",
            "--fixture-config",
            "workers.json",
            "--",
            "-entry.mjs",
        ],
    );
    assert!(plain.status.success());
    let traced = command(
        &dir,
        &[
            ".",
            ".bundler-out",
            "--fixture-config",
            "workers.json",
            "--trace-file",
            ".bundler-trace-progress.json",
            "--",
            "-entry.mjs",
        ],
    );
    assert!(
        traced.status.success(),
        "{}",
        String::from_utf8_lossy(&traced.stderr)
    );
    assert_progress(&traced.stderr);
    let stderr = String::from_utf8_lossy(&traced.stderr);
    for message in [
        "Parsing project lockfile",
        "Fingerprinting lockfile package graph",
        "completed=0 total=3 status=running",
        "completed=3 total=3 status=complete",
        "disk_hits=1",
        "changed=false",
    ] {
        assert!(stderr.contains(message), "{stderr}");
    }
    let plain: serde_json::Value = serde_json::from_slice(&plain.stdout).unwrap();
    let traced: serde_json::Value = serde_json::from_slice(&traced.stdout).unwrap();
    assert_eq!(plain["directory"], traced["directory"]);
    assert_eq!(plain["entries"], traced["entries"]);
    assert_eq!(traced["artifactWrites"], 0);
    let trace = std::fs::read_to_string(dir.root().join(".bundler-trace-progress.json")).unwrap();
    assert!(!trace.contains("-entry.mjs"));
    let events: Vec<serde_json::Value> = serde_json::from_str(&trace).unwrap();
    for name in [
        "build.prepare",
        "build.plan",
        "build.publish",
        "lockfile.parse",
        "lockfile.normalize",
        "lockfile.fingerprint",
        "lockfile.member",
        "lockfile.component",
        "lockfile.validate",
        "lockfile.recheck",
    ] {
        assert!(
            events.iter().any(|event| event["name"] == name),
            "{events:?}"
        );
    }
    let coordinator = &events
        .iter()
        .find(|event| event["name"] == "build")
        .unwrap()["tid"];
    for name in [
        "lockfile.component",
        "discovery.package",
        "discovery.directory",
    ] {
        assert!(
            events
                .iter()
                .any(|event| event["name"] == name && &event["tid"] != coordinator),
            "{name}"
        );
    }
    assert!(events.iter().all(|event| event["ph"] != "i"));
}

#[test]
fn failed_build_reports_progress_without_claiming_completion_or_writing_json() {
    let dir = TmpDir::new();
    std::fs::write(dir.root().join("entry.mjs"), "export const = ;").unwrap();
    let result = command(&dir, &[".", ".bundler-out", "entry.mjs"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let stderr = String::from_utf8(result.stderr).unwrap();
    let progress = stderr
        .find("Preparing modules and resolving dependencies")
        .unwrap();
    assert!(progress < stderr.find("Error:").unwrap(), "{stderr}");
    for absent in [
        "Modules prepared",
        "Publishing output artifacts",
        "Build complete",
    ] {
        assert!(!stderr.contains(absent), "{stderr}");
    }
}
