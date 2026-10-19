use super::*;
use crate::build::Entry;
use std::process::Command;
use test_tmpdir::TmpDir;

fn fixture() -> (TmpDir, BuildSession, BuildOptions) {
    let dir = TmpDir::new();
    fs::write(
        dir.root().join("main.mjs"),
        "export const load = () => import('./lazy.mjs'); console.log('main');",
    )
    .unwrap();
    fs::write(dir.root().join("lazy.mjs"), "export const value = 1;").unwrap();
    let session = BuildSession::new(dir.root()).unwrap();
    let options = BuildOptions {
        entries: BTreeMap::from([(
            "main".into(),
            Entry {
                path: "main.mjs".into(),
                output_name: None,
                namespace: true,
            },
        )]),
        ..Default::default()
    };
    (dir, session, options)
}

fn snapshot(directory: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let mut result = BTreeMap::new();
    fn visit(
        root: &Path,
        path: &Path,
        result: &mut BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    (
                        fs::read(&path).unwrap(),
                        fs::metadata(path).unwrap().modified().unwrap(),
                    ),
                );
            }
        }
    }
    visit(directory, directory, &mut result);
    result
}

#[test]
fn named_js_entries_publish_with_owned_module_type_metadata() {
    for (input, source, format) in [
        (
            "entry.mjs",
            "export const value = 1; console.log(value);",
            "module",
        ),
        (
            "entry.cjs",
            "exports.value = 1; console.log(exports.value);",
            "commonjs",
        ),
    ] {
        let (dir, mut session, mut options) = fixture();
        fs::write(dir.root().join(input), source).unwrap();
        options.entries = BTreeMap::from([(
            "app.js".into(),
            Entry {
                path: input.into(),
                output_name: Some("app.js".into()),
                namespace: true,
            },
        )]);
        let (build, published) = session
            .build_and_publish(
                &options,
                &dir.root().join(".bundler-output"),
                &Cancellation::default(),
            )
            .unwrap();
        assert!(published.entries["app.js"].ends_with("app.js"));
        assert_eq!(
            fs::read_to_string(published.directory.join("package.json")).unwrap(),
            format!("{{\"type\":\"{format}\"}}\n"),
        );
        let actual = Command::new("node")
            .arg(&published.entries["app.js"])
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .output()
            .unwrap();
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, b"1\n");
        let mut invalid = build.product.artifacts.clone();
        invalid.files.remove(Path::new("package.json"));
        assert!(validate_artifacts(&invalid)
            .unwrap_err()
            .to_string()
            .contains("module type metadata"));
    }
}

#[test]
fn publication_noop_mtimes_generation_identity_and_bounded_stale_cleanup() {
    let (dir, mut session, options) = fixture();
    let output = dir.root().join(".bundler-output");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("user.txt"), "do not touch").unwrap();
    let cancel = Cancellation::default();
    let (_, first) = session
        .build_and_publish(&options, &output, &cancel)
        .unwrap();
    let before = snapshot(&output);
    let (same, second) = session
        .build_and_publish(&options, &output, &cancel)
        .unwrap();
    assert!(!second.changed);
    assert_eq!(second.files_written, 0);
    assert!(first.files_written > 0);
    assert_eq!(same.counters.emissions, 0);
    assert_eq!(snapshot(&output), before);
    assert_eq!(current_directory(&output).unwrap(), first.directory);
    let old_facades = same.product.artifacts.async_facades.clone();
    assert!(!old_facades.is_empty());
    fs::write(dir.root().join("main.mjs"), "export const value = 2;").unwrap();
    let (build, new) = session
        .build_and_publish(&options, &output, &cancel)
        .unwrap();
    assert!(new.changed);
    assert!(build.product.artifacts.async_facades.is_empty());
    assert_ne!(new.directory, first.directory);
    assert!(
        first.entries["main"].is_file(),
        "old runtime remains usable until explicit quiescent prune"
    );
    let cleaned = prune_stale(&output).unwrap();
    assert!(cleaned.warnings.is_empty(), "{:?}", cleaned.warnings);
    assert_eq!(cleaned.removed, vec![first.directory.clone()]);
    assert!(!first.directory.exists());
    assert!(new.entries["main"].is_file());
    assert_eq!(
        fs::read_to_string(output.join("user.txt")).unwrap(),
        "do not touch"
    );
    assert!(prune_stale(&output).unwrap().removed.is_empty());
    assert!(is_generated_artifact(
        &output.join("modules/x.mjs"),
        &output
    ));
    assert!(is_generated_artifact(
        Path::new("foo.mjs.seg.cbor.stage-ab"),
        &output
    ));
    assert!(is_generated_artifact(
        Path::new("other/.bundler-stage-1/x"),
        &output
    ));
    assert!(!is_generated_artifact(
        &dir.root().join("main.mjs"),
        &output
    ));
}

#[test]
fn cancellation_failure_and_busy_writer_leave_previous_outputs_intact() {
    let (dir, mut session, options) = fixture();
    let output = dir.root().join(".bundler-output");
    let (_, first) = session
        .build_and_publish(&options, &output, &Cancellation::default())
        .unwrap();
    let previous = snapshot(&output);
    fs::write(dir.root().join("main.mjs"), "export const changed = 2;").unwrap();
    let build = session.build(&options).unwrap();
    for phase in [Phase::Staged, Phase::Installed, Phase::BeforeCommit] {
        let cancellation = Cancellation::default();
        let result = publish_inner(&output, &build.product.artifacts, &cancellation, |at| {
            if std::mem::discriminant(&phase) == std::mem::discriminant(&at) {
                cancellation.cancel();
            }

            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(snapshot(&output), previous);
        let failed = publish_inner(
            &output,
            &build.product.artifacts,
            &Cancellation::default(),
            |at| {
                ensure!(
                    std::mem::discriminant(&phase) != std::mem::discriminant(&at),
                    "injected publication failure"
                );
                Ok(())
            },
        );
        assert!(failed.is_err());
        assert_eq!(snapshot(&output), previous);
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(output.join(LOCK))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(publish(&output, &build.product.artifacts, &Cancellation::default()).is_err());
    fs::write(dir.root().join("main.mjs"), "export const broken = ;").unwrap();
    let busy = session
        .build_and_publish(&options, &output, &Cancellation::default())
        .err()
        .unwrap();
    assert!(busy.to_string().contains("another publisher"), "{busy:#}");
    assert_eq!(snapshot(&output), previous);
    lock.unlock().unwrap();
    drop(lock);
    fs::write(dir.root().join("main.mjs"), "export const broken = ;").unwrap();
    assert!(session
        .build_and_publish(&options, &output, &Cancellation::default())
        .is_err());
    assert_eq!(snapshot(&output), previous);
    assert_eq!(current_directory(&output).unwrap(), first.directory);
}

#[test]
fn concurrent_publishers_are_rejected_until_the_first_commits() {
    let (dir, mut session, options) = fixture();
    let output = dir.root().join(".bundler-output");
    let build = session.build(&options).unwrap();
    let artifacts = build.product.artifacts.clone();
    let other_output = output.clone();
    let other_artifacts = artifacts.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let publisher = std::thread::spawn(move || {
        publish_inner(
            &other_output,
            &other_artifacts,
            &Cancellation::default(),
            |phase| {
                if matches!(phase, Phase::Staged) {
                    ready_tx.send(()).unwrap();
                    continue_rx.recv().unwrap();
                }
                Ok(())
            },
        )
        .unwrap()
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(publish(&output, &artifacts, &Cancellation::default()).is_err());
    continue_tx.send(()).unwrap();
    let first = publisher.join().unwrap();
    assert!(first.changed);
    let same = publish(&output, &artifacts, &Cancellation::default()).unwrap();
    assert!(!same.changed);
    assert_eq!(same.directory, first.directory);
}

#[test]
fn ownership_rejects_traversal_collisions_modified_files_and_foreign_metadata() {
    let (dir, mut session, options) = fixture();
    let output = dir.root().join(".bundler-output");
    let (build, first) = session
        .build_and_publish(&options, &output, &Cancellation::default())
        .unwrap();
    let mut unsafe_artifacts = build.product.artifacts.clone();
    unsafe_artifacts
        .files
        .insert("../escape".into(), "bad".into());
    assert!(publish(&output, &unsafe_artifacts, &Cancellation::default()).is_err());
    unsafe_artifacts.files.remove(Path::new("../escape"));
    let missing = unsafe_artifacts.modules.values().next().unwrap().clone();
    let code = unsafe_artifacts.files.remove(&missing).unwrap();
    assert!(publish(&output, &unsafe_artifacts, &Cancellation::default()).is_err());
    unsafe_artifacts.files.insert(missing, code);
    unsafe_artifacts.files.insert(PathBuf::new(), "bad".into());
    assert!(publish(&output, &unsafe_artifacts, &Cancellation::default()).is_err());
    fs::write(first.directory.join("user-extra"), "mine").unwrap();
    fs::write(&first.entries["main"], "user-modified").unwrap();
    assert!(publish(&output, &build.product.artifacts, &Cancellation::default()).is_err());
    fs::write(dir.root().join("main.mjs"), "console.log('new');").unwrap();
    let (_, new) = session
        .build_and_publish(&options, &output, &Cancellation::default())
        .unwrap();
    let cleanup = prune_stale(&output).unwrap();
    assert!(!cleanup.warnings.is_empty());
    assert_eq!(fs::read(&first.entries["main"]).unwrap(), b"user-modified");
    assert_eq!(
        fs::read(first.directory.join("user-extra")).unwrap(),
        b"mine"
    );
    assert!(new.entries["main"].is_file());
    let state = fs::read(output.join(STATE)).unwrap();
    fs::write(output.join(STATE), r#"{"owner":"foreign"}"#).unwrap();
    assert!(publish(&output, &build.product.artifacts, &Cancellation::default()).is_err());
    fs::write(output.join(STATE), state).unwrap();
}

#[test]
fn old_running_entry_keeps_lazy_native_files_until_quiescent_prune() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let (dir, mut session, options) = fixture();
    let output = dir.root().join(".bundler-output");
    let (_, old) = session
        .build_and_publish(&options, &output, &Cancellation::default())
        .unwrap();
    let script = format!(
        "setTimeout(() => process.exit(99), 5000).unref(); const entry = await import({}); console.log('ready'); process.stdin.once('data', async () => {{ console.log('old', (await entry.load()).value); process.stdin.pause(); }});",
        serde_json::to_string(old.entries["main"].to_str().unwrap()).unwrap()
    );
    let mut child = Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .current_dir(dir.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "main\n");
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    fs::write(dir.root().join("lazy.mjs"), "export const value = 2;").unwrap();
    let (_, new) = session
        .build_and_publish(&options, &output, &Cancellation::default())
        .unwrap();
    assert_ne!(new.directory, old.directory);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"continue\n")
        .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "old 1\n");
    assert!(child.wait().unwrap().success());
    let fresh = Command::new("node")
        .args([
            "--input-type=module",
            "-e",
            &format!(
                "const entry = await import({}); console.log('new', (await entry.load()).value);",
                serde_json::to_string(new.entries["main"].to_str().unwrap()).unwrap()
            ),
        ])
        .current_dir(dir.root())
        .output()
        .unwrap();
    assert!(
        fresh.status.success(),
        "{}",
        String::from_utf8_lossy(&fresh.stderr)
    );
    assert_eq!(fresh.stdout, b"main\nnew 2\n");
    let (_, same) = session
        .build_and_publish(&options, &output, &Cancellation::default())
        .unwrap();
    assert!(!same.changed);
    assert_eq!(same.files_written, 0);
    assert_eq!(prune_stale(&output).unwrap().removed, vec![old.directory]);
}

#[cfg(unix)]
#[test]
fn cleanup_never_follows_owned_file_or_directory_symlinks() {
    let (dir, mut session, options) = fixture();
    let output = dir.root().join(".bundler-output");
    let (_, first) = session
        .build_and_publish(&options, &output, &Cancellation::default())
        .unwrap();
    let user = dir.root().join("user.txt");
    fs::write(&user, "mine").unwrap();
    fs::remove_file(&first.entries["main"]).unwrap();
    std::os::unix::fs::symlink(&user, &first.entries["main"]).unwrap();
    fs::write(dir.root().join("main.mjs"), "console.log('new');").unwrap();
    session
        .build_and_publish(&options, &output, &Cancellation::default())
        .unwrap();
    let cleanup = prune_stale(&output).unwrap();
    assert!(!cleanup.warnings.is_empty());
    assert_eq!(fs::read(user).unwrap(), b"mine");
    assert!(fs::symlink_metadata(&first.entries["main"])
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
fn explicit_unlock_releases_lock_even_while_a_fork_style_descriptor_survives() {
    let dir = TmpDir::new();
    let (_, lock) = lock_output(dir.root()).unwrap();
    let inherited = lock.0.as_ref().unwrap().try_clone().unwrap();
    lock.release().unwrap();
    let (_, next) = lock_output(dir.root()).unwrap();
    next.release().unwrap();
    drop(inherited);
    let unwind = std::panic::catch_unwind(|| {
        let (_, _lock) = lock_output(dir.root()).unwrap();
        panic!("abort before commit");
    });
    assert!(unwind.is_err());
    let (_, after_unwind) = lock_output(dir.root()).unwrap();
    after_unwind.release().unwrap();
}
