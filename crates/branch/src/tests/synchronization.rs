//! Per-thread, one-shot coordination at the actual branch-lock boundary.
//! No sleeps, global hooks shared between tests, or production API are needed.

use std::{
    cell::RefCell,
    sync::{LockResult, Mutex, MutexGuard, TryLockError, mpsc},
    time::Duration,
};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Hooks {
    contended: Option<Box<dyn FnOnce()>>,
    acquired: Option<Box<dyn FnOnce()>>,
}

thread_local! {
    static HOOKS: RefCell<Hooks> = RefCell::new(Hooks::default());
}

fn with_hooks<T>(hooks: Hooks, operation: impl FnOnce() -> T) -> T {
    // Restore the previous hooks even if the operation panics.
    struct Restore(Hooks);
    impl Drop for Restore {
        fn drop(&mut self) {
            HOOKS.with(|slot| slot.replace(std::mem::take(&mut self.0)));
        }
    }
    let _restore = Restore(HOOKS.with(|slot| slot.replace(hooks)));
    operation()
}

pub(crate) fn signal_contention<T>(signal: mpsc::Sender<()>, operation: impl FnOnce() -> T) -> T {
    with_hooks(
        Hooks {
            contended: Some(Box::new(move || signal.send(()).unwrap())),
            ..Hooks::default()
        },
        operation,
    )
}

pub(crate) fn pause_after_lock<T>(
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    operation: impl FnOnce() -> T,
) -> T {
    with_hooks(
        Hooks {
            acquired: Some(Box::new(move || {
                entered.send(()).unwrap();
                release.recv_timeout(TIMEOUT).unwrap();
            })),
            ..Hooks::default()
        },
        operation,
    )
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> LockResult<MutexGuard<'_, T>> {
    let guard = match mutex.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::Poisoned(error)) => Err(error),
        Err(TryLockError::WouldBlock) => {
            // The entry has been cloned from the registry and is currently
            // locked. This is stronger than signalling just before the API call.
            let hook = HOOKS.with(|slot| slot.borrow_mut().contended.take());
            if let Some(hook) = hook {
                hook();
            }
            mutex.lock()
        }
    }?;
    let hook = HOOKS.with(|slot| slot.borrow_mut().acquired.take());
    if let Some(hook) = hook {
        hook();
    }
    Ok(guard)
}
