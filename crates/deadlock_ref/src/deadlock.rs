use std::{backtrace::Backtrace, hash::Hash, ops::Deref, sync::mpsc, thread, time::Duration};

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

    TimerCanceller { cancel_sender }
}

struct DeadlockDebugDroppable {
    deadlock_warning: TimerCanceller,
}

impl DeadlockDebugDroppable {
    pub fn new(timeout: Duration, name: String) -> Self {
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
    pub fn wrap(to_wrap: TRef, name: String) -> Self {
        Self {
            inner_ref: to_wrap,
            drop_tracker: DeadlockDebugDroppable::new(
                Duration::from_millis(REFERENCE_TIMEOUT_MS),
                name,
            ),
        }
    }

    pub fn create<TInitFn: FnOnce() -> TRef>(name: String, f: TInitFn) -> Self {
        // Run a potentially deadlocking function, with a timer in parallel to log if it
        // over-ran the timer.
        let reference = f();
        let deadlock_warning = run_with_timer(
            Duration::from_millis(REFERENCE_TIMEOUT_MS),
            format!("lock {name}"),
        );
        let to_ret = Self::wrap(reference, name);
        deadlock_warning.cancel();
        to_ret
    }

    pub fn try_create<Err, TInitFn: FnOnce() -> Result<TRef, Err>>(
        name: String,
        f: TInitFn,
    ) -> Result<Self, Err> {
        // Run a potentially deadlocking function, with a timer in parallel to log if it
        // over-ran the timer.
        let deadlock_warning = run_with_timer(
            Duration::from_millis(REFERENCE_TIMEOUT_MS),
            format!("lock {name}"),
        );
        let to_ret = Self::wrap(f()?, name);
        deadlock_warning.cancel();
        Ok(to_ret)
    }
}

impl<T, TRef> Deref for DeadlockDebugRef<TRef>
where
    TRef: Deref<Target = T>,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner_ref.deref()
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

impl<K: Eq + Hash, V> DeadlockDebugRef<dashmap::mapref::one::Ref<'_, K, V>> {
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

#[cfg(feature = "dashmap")]
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

#[cfg(feature = "dashmap")]
impl<K: Eq + Hash, V, T2> DeadlockDebugRef<dashmap::mapref::one::MappedRef<'_, K, V, T2>> {
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
