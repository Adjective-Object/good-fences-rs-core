use super::*;
use std::io::{self, Write};

#[test]
fn snapshots_stream_and_clones_outlive_external_mutation_and_replacement() {
    let directory = tempfile::tempdir_in(".").unwrap();
    let path = directory.path().join("mutable");
    let mut original = File::create(&path).unwrap();
    original.write_all(b"original bytes").unwrap();
    let bytes = SharedBytes::snapshot_in(directory.path(), File::open(&path).unwrap()).unwrap();
    assert!(bytes.is_mapped());
    let clone = bytes.clone();
    assert_eq!(bytes.as_ptr(), clone.as_ptr());
    original.set_len(0).unwrap();
    original.write_all(b"changed").unwrap();
    drop(original);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"replacement").unwrap();
    drop(bytes);
    std::thread::spawn(move || assert_eq!(&*clone, b"original bytes"))
        .join()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    directory.close().unwrap();
}

#[test]
fn empty_snapshots_remain_disk_backed_and_cleanup_after_last_clone() {
    let directory = tempfile::tempdir_in(".").unwrap();
    let bytes = SharedBytes::snapshot_in(directory.path(), io::empty()).unwrap();
    assert!(bytes.is_mapped());
    assert!(bytes.is_empty());
    let clone = bytes.clone();
    drop(bytes);
    assert!(clone.is_empty());
    drop(clone);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    directory.close().unwrap();
}

#[test]
fn failed_snapshot_preserves_io_cause_and_cleans_partial_file() {
    struct FailingReader(bool);
    impl Read for FailingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0 {
                return Err(io::Error::other("snapshot input failed"));
            }
            self.0 = true;
            buffer[0] = 42;
            Ok(1)
        }
    }
    let directory = tempfile::tempdir_in(".").unwrap();
    let error = SharedBytes::snapshot_in(directory.path(), FailingReader(false)).unwrap_err();
    assert!(format!("{error:#}").contains("snapshot input failed"));
    assert!(format!("{error:#}").contains(&directory.path().display().to_string()));
    assert!(error.downcast_ref::<io::Error>().is_some());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    directory.close().unwrap();
}
