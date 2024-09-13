use anyhow::{anyhow, Context, Error, Result};
use dashmap::DashMap;
use parking_lot::{MappedRwLockReadGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::{
    backtrace::Backtrace,
    hash::Hash,
    ops::Deref,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};
use tracing::debug;

/// Represents some state which is represented by the existence of a path on the filesystem
///
/// This is used to cache the result of a directory probe for tsconfig.json/package.json files,
/// and node_modules directories without redundant trips to disk for each resolve.
pub trait ContextData<TArgs: Copy = ()>: Sized {
    /// Creates a new ContextData from a filepath
    fn read_context_data(args: TArgs, path: &Path) -> Result<Option<Self>, Error>;
}

impl ContextData for () {
    fn read_context_data(_: (), path: &Path) -> Result<Option<Self>, Error> {
        // check if the path exists
        match path.try_exists() {
            Ok(true) => Ok(Some(())),
            Ok(false) => Ok(None),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }

                Err(anyhow!(
                    "Failed check path {:?} for existence: {:?}",
                    path,
                    e
                ))
            }
        }
    }
}

#[derive(Debug)]
pub struct WithCache<TVal: ContextData<TArgs>, TCached: Default, TArgs: Copy = ()> {
    _phantom_targs: std::marker::PhantomData<TArgs>,
    inner: TVal,
    cached: RwLock<TCached>,
}

pub type WCReadGuard<'a, TCached> = RwLockReadGuard<'a, TCached>;
pub type WCMappedReadGuard<'a, TMapped> = MappedRwLockReadGuard<'a, TMapped>;
pub type WCWriteGuard<'a, TCached> = RwLockWriteGuard<'a, TCached>;

impl<TVal, TCached, TArgs: Copy> ContextData<TArgs> for WithCache<TVal, TCached, TArgs>
where
    TVal: ContextData<TArgs>,
    TCached: Default,
{
    fn read_context_data(args: TArgs, path: &Path) -> Result<Option<Self>, Error> {
        let inner = TVal::read_context_data(args, path)?;
        Ok(inner.map(|inner| WithCache {
            _phantom_targs: Default::default(),
            inner,
            cached: RwLock::new(TCached::default()),
        }))
    }
}

impl<TVal, TCached, TArgs: Copy> WithCache<TVal, TCached, TArgs>
where
    TVal: ContextData<TArgs>,
    TCached: Default,
{
    pub fn inner(&self) -> &TVal {
        &self.inner
    }

    pub fn get_cached(&self) -> WCReadGuard<'_, TCached> {
        self.cached.read()
    }

    pub fn get_cached_mut(&self) -> WCWriteGuard<'_, TCached> {
        self.cached.write()
    }
}

fn unsafe_map_unwrap_locked_option<T>(
    readonly_lock: RwLockReadGuard<Option<T>>,
) -> MappedRwLockReadGuard<'_, T> {
    RwLockReadGuard::<Option<T>>::map(readonly_lock, |x: &Option<T>| -> &T { x.as_ref().unwrap() })
}

impl<TVal, TDerived> WithCache<TVal, Option<TDerived>>
where
    TVal: ContextData,
{
    pub fn get_cached_or_init<TFn>(&self, f: TFn) -> WCMappedReadGuard<'_, TDerived>
    where
        TFn: Fn(&TVal) -> TDerived,
    {
        // most of the time, we expect this to already be initialized, so try to read it first
        {
            let read_lock = self.cached.read();
            if read_lock.is_some() {
                return unsafe_map_unwrap_locked_option(read_lock);
            }
        }

        // failed reading, try to grab a write lock
        let mut write_lock = self.cached.write();

        // check that nobody else filled the cache while we were waiting before
        // we call the initializer function
        if write_lock.as_ref().is_none() {
            *write_lock = Some(f(&self.inner));
        }

        // downgrade and map the
        let readonly_lock = RwLockWriteGuard::downgrade(write_lock);

        // downgrade the write lock to a read lock, map the result, and return it
        unsafe_map_unwrap_locked_option(readonly_lock)
    }

    pub fn try_get_cached_or_init<'a, TFn>(&'a self, f: TFn) -> Result<WCMappedReadGuard<TDerived>>
    where
        TFn: Fn(&'a TVal) -> Result<TDerived>,
    {
        // most of the time, we expect this to already be initialized, so try to read it first
        {
            let read_lock = self.cached.read();
            if read_lock.is_some() {
                return Ok(unsafe_map_unwrap_locked_option(read_lock));
            }
        }

        // failed reading, try to grab a write lock
        let mut write_lock = self.cached.write();

        // check that nobody else filled the cache while we were waiting before
        // we call the initializer function
        if write_lock.as_ref().is_none() {
            let val = f(&self.inner)?;
            *write_lock = Some(val);
        }

        // downgrade and map the
        let readonly_lock = RwLockWriteGuard::downgrade(write_lock);

        // downgrade the write lock to a read lock, map the result, and return it
        Ok(unsafe_map_unwrap_locked_option(readonly_lock))
    }
}

#[derive(Debug)]
pub struct FileContextCache<
    T: ContextData<TArgs>,
    const CONTEXT_FNAME: &'static str,
    TArgs: Copy = (),
> {
    /// Map of directories to their contained context files, if any
    ///
    /// A directory is considered to have no context file if the entry is None
    /// A directory is considered to have a context file if the entry is Some(T)
    ///
    /// If there is no entry, the directory has not been probed yet
    cache: DashMap<PathBuf, Option<T>>,
    args: TArgs,
}

struct TimerCanceller {
    cancel_sender: mpsc::Sender<()>,
}

impl TimerCanceller {
    pub fn cancel(&self) {
        if let Err(e) = self.cancel_sender.send(()) {
            println!("failed to signal release of DeadlockDebugDroppable! {e}")
        }
    }
}

fn run_with_timer(timeout: Duration, msg: String) -> TimerCanceller {
    let allocation_trace = Backtrace::capture();
    let (cancel_sender, cancel_receiver) = mpsc::channel();

    thread::spawn(move || {
        thread::sleep(timeout);
        match cancel_receiver.try_recv() {
            Ok(_) => {} // noop, the object has already been dropped
            Err(_) => println!(
                // no message sent before timeout. Warn about deadlock
                "{msg} timed out after {} ms \
                Allocation site:\n{allocation_trace}",
                timeout.as_millis()
            ),
        }
    });

    return TimerCanceller{cancel_sender};
}

struct DeadlockDebugDroppable {
    deadlock_warning: TimerCanceller
}

impl DeadlockDebugDroppable {
    fn new(timeout: Duration, name: String) -> Self {
        Self {
            deadlock_warning: run_with_timer(timeout, format!("release {name}",)),
        }
    }
}

impl Drop for DeadlockDebugDroppable {
    fn drop(&mut self) {
        self.deadlock_warning.cancel()
    }
}

/* Wrapper reference type that logs when created and logs when released.
 * If it fails to release, maybe there is a deadlock?
 */
pub struct DeadlockDebugRef<TRef: Deref, const REFERENCE_TIMEOUT_MS: u64 = 5000> {
    inner_ref: TRef,
    drop_tracker: DeadlockDebugDroppable,
}

impl<TRef: Deref, const REFERENCE_TIMEOUT_MS: u64> DeadlockDebugRef<TRef, REFERENCE_TIMEOUT_MS> {
    fn wrap(to_wrap: TRef, name: String) -> Self {
        Self {
            inner_ref: to_wrap,
            drop_tracker: DeadlockDebugDroppable::new(
                Duration::from_millis(REFERENCE_TIMEOUT_MS),
                name,
            ),
        }
    }

    fn create<TInitFn: FnOnce() -> TRef>(name: String, f: TInitFn) -> Self {
        // Run a potentially deadlocking function, with a timer in parallel to log if it
        // over-ran the timer.
        let deadlock_warning = run_with_timer(REFERENCE_TIMEOUT_MS, format!("lock {name}"))
        let reference = f();
        deadlock_warning.cancel();
        Self::wrap(reference, name)
    }
}

impl<T, TRef> Deref for DeadlockDebugRef<TRef>
where
    TRef: Deref<Target = T>,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        return self.inner_ref.deref();
    }
}

pub type CtxRef<'a, T> = DeadlockDebugRef<dashmap::mapref::one::Ref<'a, PathBuf, T>>;
pub type CtxOptRef<'a, T> =
    DeadlockDebugRef<dashmap::mapref::one::MappedRef<'a, PathBuf, Option<T>, T>>;

impl<T: ContextData<()>, const CONTEXT_FNAME: &'static str> FileContextCache<T, CONTEXT_FNAME, ()> {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            args: (),
        }
    }
}

pub trait MappingRef<T>
where
    Self: Sized + Deref<Target = T>,
{
    type MappedRef<T2>: Deref<Target = T2>;

    fn map<F, T2>(self, f: F) -> Self::MappedRef<T2>
    where
        F: FnOnce(&Self::Target) -> &T2;

    fn try_map<F, T2>(self, f: F) -> Result<Self::MappedRef<T2>, Self>
    where
        F: FnOnce(&Self::Target) -> Option<&T2>;
}

impl<'a, K: Eq + Hash, V> MappingRef<V> for dashmap::mapref::one::Ref<'a, K, V> {
    type MappedRef<T> = dashmap::mapref::one::MappedRef<'a, K, V, T>;

    fn map<F, T2>(self, f: F) -> Self::MappedRef<T2>
    where
        F: FnOnce(&<Self as Deref>::Target) -> &T2,
    {
        let me: dashmap::mapref::one::Ref<'a, K, V> = self;
        me.map(f)
    }

    fn try_map<F, T2>(self, f: F) -> Result<Self::MappedRef<T2>, Self>
    where
        F: FnOnce(&Self::Target) -> Option<&T2>,
    {
        let me: dashmap::mapref::one::Ref<'a, K, V> = self;
        me.try_map(f)
    }
}

impl<'a, K: Eq + Hash, V> DeadlockDebugRef<dashmap::mapref::one::Ref<'a, K, V>> {
    pub fn key(&self) -> &K {
        self.inner_ref.key()
    }

    pub fn value(&self) -> &V {
        self.inner_ref.value()
    }

    pub fn pair(&self) -> (&K, &V) {
        self.inner_ref.pair()
    }
}

impl<V, TInnerRef: MappingRef<V>> MappingRef<V> for DeadlockDebugRef<TInnerRef> {
    type MappedRef<T> = DeadlockDebugRef<<TInnerRef as MappingRef<V>>::MappedRef<T>>;

    fn map<F, T2>(self, f: F) -> Self::MappedRef<T2>
    where
        F: FnOnce(&<Self as Deref>::Target) -> &T2,
    {
        let inner_mapped = self.inner_ref.map(f);
        DeadlockDebugRef::<<TInnerRef as MappingRef<V>>::MappedRef<T2>> {
            inner_ref: inner_mapped,
            drop_tracker: self.drop_tracker,
        }
    }

    fn try_map<F, T2>(self, f: F) -> Result<Self::MappedRef<T2>, Self>
    where
        F: FnOnce(&Self::Target) -> Option<&T2>,
    {
        match self.inner_ref.try_map(f) {
            Ok(inner_mapped) => Ok(
                DeadlockDebugRef::<<TInnerRef as MappingRef<V>>::MappedRef<T2>> {
                    inner_ref: inner_mapped,
                    drop_tracker: self.drop_tracker,
                },
            ),
            Err(inner_ref) => Err(DeadlockDebugRef::<TInnerRef> {
                inner_ref,
                drop_tracker: self.drop_tracker,
            }),
        }
    }
}

impl<'a, K: Eq + Hash, V, T2> MappingRef<T2> for dashmap::mapref::one::MappedRef<'a, K, V, T2> {
    type MappedRef<T3> = dashmap::mapref::one::MappedRef<'a, K, V, T3>;

    fn map<F, T3>(self, f: F) -> Self::MappedRef<T3>
    where
        F: FnOnce(&<Self as Deref>::Target) -> &T3,
    {
        let me: dashmap::mapref::one::MappedRef<'a, K, V, T2> = self;
        me.map(f)
    }

    fn try_map<F, T3>(self, f: F) -> Result<Self::MappedRef<T3>, Self>
    where
        F: FnOnce(&Self::Target) -> Option<&T3>,
    {
        let me: dashmap::mapref::one::MappedRef<'a, K, V, T2> = self;
        me.try_map(f)
    }
}

impl<'a, K: Eq + Hash, V, T2> DeadlockDebugRef<dashmap::mapref::one::MappedRef<'a, K, V, T2>> {
    pub fn key(&self) -> &K {
        self.inner_ref.key()
    }

    pub fn value(&self) -> &T2 {
        self.inner_ref.value()
    }

    pub fn pair(&self) -> (&K, &T2) {
        self.inner_ref.pair()
    }
}

impl<T: ContextData<()>, const CONTEXT_FNAME: &'static str> Default
    for FileContextCache<T, CONTEXT_FNAME, ()>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ContextData<TArgs>, TArgs: Copy, const CONTEXT_FNAME: &'static str>
    FileContextCache<T, CONTEXT_FNAME, TArgs>
{
    const MAX_PROBE_DEPTH: i64 = 1000;

    /// Checks the given path for a context file (e.g. a package.json file or a tsconfig.json file)
    ///
    /// This function will probe all parent directories between the given path and the root directory
    /// for a context file. If a context file is found, it will be cached and returned.
    ///
    /// If no context file is found, that is also cached and returned.
    pub fn probe_path<'a, 'base_path>(
        &'a self,
        root_dir: &Path,
        base: &'base_path Path,
    ) -> Result<Option<(&'base_path Path, CtxOptRef<'a, T>)>, Error> {
        if !root_dir.is_absolute() {
            return Err(anyhow!(
                "probe_path must be called with an absolute root_dir (got {})",
                root_dir.display()
            ));
        }
        let is_abs = base.is_absolute();

        let mut head: Option<&'base_path Path> = base.parent();
        for _ in 0..Self::MAX_PROBE_DEPTH {
            let head_path = match head {
                None => return Ok(None),
                Some(p) => p,
            };

            let probe_result = self.check_dir(head_path)?;
            if let Ok(res) = probe_result.try_map(|x| x.as_ref()) {
                return Ok(Some((head_path, res)));
            }

            // if we would walk above <root>/tsconfig.json,
            // we should stop the traversal
            if is_abs && head_path == root_dir {
                // walked to root dir, and we don't want to
                // escape the root dir
                return Ok(None);
            }

            head = head_path.parent()
        }

        Err(anyhow!(
            "Max probe depth reached while searching for a tsconfig file among parent directories"
        ))
    }

    pub fn probe_path_iter<'context_cache, 'root_dir, 'base_path>(
        &'context_cache self,
        root_dir: &'root_dir Path,
        base: &'base_path Path,
    ) -> ProbePathIterator<'context_cache, 'root_dir, 'base_path, T, CONTEXT_FNAME, TArgs> {
        ProbePathIterator::new(self, root_dir, base)
    }

    // checks an individual directory for a tsconfig.json file
    //
    // First, checks the cache. If no entry is found, checks the real
    // filesystem for a tsconfig.json file and caches the result.
    pub fn check_dir<'a>(&'a self, base: &Path) -> Result<CtxRef<'a, Option<T>>, Error> {
        println!("{} check_dir! {}", CONTEXT_FNAME, base.display());
        let entry = self.cache.entry(base.to_owned());
        let res = entry.or_try_insert_with(|| {
            println!(
                "{} check_dir! - populate - {}",
                CONTEXT_FNAME,
                base.display()
            );
            let x = self
                .check_dir_os_fs(base)
                .with_context(|| format!("Failed while checking {:?} for {}", base, CONTEXT_FNAME));
            println!(
                "{} check_dir! - populated! - {}",
                CONTEXT_FNAME,
                base.display()
            );
            x
        })?;

        return Ok(DeadlockDebugRef::wrap(
            res.downgrade(),
            format!("{CONTEXT_FNAME}:{}", base.display()),
        ));
    }

    fn check_dir_os_fs(&self, base: &Path) -> Result<Option<T>, Error> {
        // probe the real FS for a tsconfig.json file
        let context_file_path = base.to_owned().join(CONTEXT_FNAME);
        let result = T::read_context_data(self.args, &context_file_path)
            .with_context(|| "in read_context_data");
        debug!(
            "Checking {}/{}: {}",
            base.to_string_lossy(),
            CONTEXT_FNAME,
            match &result {
                Err(ref e) => format!("error: {:?}", e),
                Ok(None) => "not found".to_string(),
                Ok(Some(_)) => "found".to_string(),
            }
        );

        result
    }

    /// Clears the cache for all paths under a subdirectory, recursively.
    pub fn mark_dirty_root(&self, path: &Path) {
        self.cache.retain(|key, _| !key.starts_with(path));
    }
}

// Represents an iterator that steps up all the discovered context files in a directory
pub struct ProbePathIterator<
    'context_cache,
    'root_dir,
    'base_path,
    TContext,
    const CONTEXT_FNAME: &'static str,
    TArgs: Copy,
> where
    TContext: ContextData<TArgs>,
{
    i: i64,
    cache: &'context_cache FileContextCache<TContext, CONTEXT_FNAME, TArgs>,
    root_dir: &'root_dir Path,
    head: Option<&'base_path Path>,
}
impl<
        'context_cache,
        'root_dir,
        'base_path,
        TContext,
        TArgs: Copy,
        const CONTEXT_FNAME: &'static str,
    > ProbePathIterator<'context_cache, 'root_dir, 'base_path, TContext, CONTEXT_FNAME, TArgs>
where
    TContext: ContextData<TArgs>,
{
    const MAX_PROBE_DEPTH: i64 = 1000;

    fn new(
        iter: &'context_cache FileContextCache<TContext, CONTEXT_FNAME, TArgs>,
        root_dir: &'root_dir Path,
        base: &'base_path Path,
    ) -> Self {
        Self {
            i: 0,
            cache: iter,
            root_dir,
            head: Some(base),
        }
    }
}

impl<
        'context_cache,
        'root_dir,
        'base_path,
        TContext,
        TArgs: Copy,
        const CONTEXT_FNAME: &'static str,
    > Iterator
    for ProbePathIterator<'context_cache, 'root_dir, 'base_path, TContext, CONTEXT_FNAME, TArgs>
where
    TContext: ContextData<TArgs>,
    Self: 'context_cache,
{
    type Item = Result<(&'base_path Path, CtxOptRef<'context_cache, TContext>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.i < Self::MAX_PROBE_DEPTH {
            let head_path: &'base_path Path = match self.head {
                None => {
                    debug!("path probe walked up to fs root");
                    return None;
                }
                Some(p) => p,
            };
            self.i += 1;
            self.head = head_path.parent();

            let check_result = self.cache.check_dir(head_path);

            debug!(
                "Probe path: {}/{} -> {}",
                head_path.to_string_lossy(),
                CONTEXT_FNAME,
                match check_result {
                    Ok(ref x) => match x.as_ref() {
                        Some(_) => "found".to_owned(),
                        None => "not found".to_owned(),
                    },
                    Err(ref e) => format!("error: {:?}", e),
                }
            );

            let probe_result = match check_result {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };

            if let Ok(res) = probe_result.try_map(|x| x.as_ref()) {
                let r: CtxOptRef<'context_cache, TContext> = res;
                return Some(Ok((head_path, r)));
            }

            if head_path == self.root_dir {
                // make sure we end iteration forever
                self.head = None;
                debug!(
                    "Probe for {} walked to root dir {}",
                    CONTEXT_FNAME,
                    self.root_dir.to_string_lossy()
                );
                return None;
            }
        }

        // we hit the max probe depth, this is an issue!
        Some(Err(anyhow!(
            "Max probe depth reached while searching for {} in parent directories",
            CONTEXT_FNAME
        )))
    }
}
