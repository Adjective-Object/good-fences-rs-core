use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use bumpalo_herd::{Herd, Member};
use thread_local::ThreadLocal;

#[cfg(windows)]
type NativeUnit = u16;
#[cfg(not(windows))]
type NativeUnit = u8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativePath<'a>(&'a [NativeUnit]);

impl NativePath<'_> {
    pub(super) fn to_path_buf(self) -> PathBuf {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            std::ffi::OsString::from_wide(self.0).into()
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            std::ffi::OsStr::from_bytes(self.0).into()
        }
        #[cfg(not(any(unix, windows)))]
        {
            PathBuf::from(std::str::from_utf8(self.0).expect("unsupported native path encoding"))
        }
    }

    pub(super) fn matches(self, path: &Path) -> bool {
        #[cfg(windows)]
        let exact = {
            use std::os::windows::ffi::OsStrExt;
            self.0.iter().copied().eq(path.as_os_str().encode_wide())
        };
        #[cfg(not(windows))]
        let exact = self.0 == path.as_os_str().as_encoded_bytes();
        exact || self.to_path_buf() == path
    }
}

/// Members and their RAII state die before the caller's lexical herd. Only
/// copied bytes/scalars/references enter the arena; no resource is forgotten.
pub(super) struct Scratch<'a> {
    herd: &'a Herd,
    members: ThreadLocal<Member<'a>>,
    #[cfg(test)]
    acquisitions: AtomicUsize,
}

impl<'a> Scratch<'a> {
    pub(super) fn new(herd: &'a Herd) -> Self {
        Self {
            herd,
            members: ThreadLocal::new(),
            #[cfg(test)]
            acquisitions: AtomicUsize::new(0),
        }
    }

    fn member(&self) -> &Member<'a> {
        self.members.get_or(|| {
            #[cfg(test)]
            self.acquisitions.fetch_add(1, Ordering::Relaxed);
            self.herd.get()
        })
    }

    pub(super) fn alloc<T: Copy>(&self, value: T) -> &'a T {
        self.member().alloc(value)
    }

    pub(super) fn text(&self, value: &str) -> &'a str {
        self.member().alloc_str(value)
    }

    pub(super) fn path(&self, path: &Path) -> NativePath<'a> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let mut units = path.as_os_str().encode_wide();
            let len = units.clone().count();
            NativePath(
                self.member()
                    .alloc_slice_fill_with(len, |_| units.next().expect("native path length")),
            )
        }
        #[cfg(not(windows))]
        {
            NativePath(
                self.member()
                    .alloc_slice_copy(path.as_os_str().as_encoded_bytes()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_are_acquired_once_per_worker_and_native_payloads_survive_members() {
        let herd = Herd::new();
        let scratch = Scratch::new(&herd);
        let barrier = std::sync::Barrier::new(4);
        let paths = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let (scratch, barrier) = (&scratch, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        let first = scratch.path(Path::new("native"));
                        for _ in 0..100 {
                            scratch.alloc([1_u32, 2, 3]);
                            scratch.text("payload");
                        }
                        first
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(scratch.acquisitions.load(Ordering::Relaxed), 4);
        drop(scratch);
        assert!(paths
            .into_iter()
            .all(|path| path.matches(Path::new("native"))));
    }

    #[test]
    fn native_path_payloads_preserve_non_unicode_units() {
        #[cfg(windows)]
        let path = {
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_wide(&[
                b'x' as u16,
                0xd800,
                b'y' as u16,
            ]))
        };
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(vec![b'x', 0xff, b'y']))
        };
        let herd = Herd::new();
        let scratch = Scratch::new(&herd);
        let copied = scratch.path(&path);
        assert_eq!(copied.to_path_buf(), path);
        assert!(copied.matches(&path));
    }
}
