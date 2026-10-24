use nix::sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask};

use crate::error::CafleetError;

/// Blocks SIGINT, SIGTERM and SIGHUP for the calling thread until dropped;
/// a signal received meanwhile takes its default action when the previous mask is restored.
pub(crate) struct DeferTermination {
    previous: SigSet,
}

impl DeferTermination {
    pub(crate) fn begin() -> Result<Self, CafleetError> {
        let mut deferred = SigSet::empty();
        for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
            deferred.add(signal);
        }
        let mut previous = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&deferred), Some(&mut previous)).map_err(
            |error| CafleetError::App(format!("cannot defer termination signals: {error}")),
        )?;
        Ok(Self { previous })
    }
}

impl Drop for DeferTermination {
    fn drop(&mut self) {
        pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&self.previous), None)
            .expect("restoring a mask previously returned by pthread_sigmask succeeds");
    }
}

#[cfg(test)]
mod tests {
    use super::DeferTermination;
    use nix::sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask};

    const DEFERRED: [Signal; 3] = [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP];

    fn blocked() -> SigSet {
        SigSet::thread_get_mask().unwrap()
    }

    fn defers_all(mask: &SigSet) -> bool {
        DEFERRED.iter().all(|signal| mask.contains(*signal))
    }

    fn defers_none(mask: &SigSet) -> bool {
        DEFERRED.iter().all(|signal| !mask.contains(*signal))
    }

    fn on_fresh_thread(test: impl FnOnce() + Send + 'static) {
        std::thread::spawn(move || {
            pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::empty()), None).unwrap();
            test();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_span_blocks_sigint_sigterm_and_sighup_until_dropped() {
        on_fresh_thread(|| {
            let span = DeferTermination::begin().unwrap();
            assert!(defers_all(&blocked()));
            drop(span);
            assert!(defers_none(&blocked()));
        });
    }

    #[test]
    fn nested_spans_restore_the_outer_mask() {
        on_fresh_thread(|| {
            let outer = DeferTermination::begin().unwrap();
            let inner = DeferTermination::begin().unwrap();
            drop(inner);
            assert!(
                defers_all(&blocked()),
                "dropping the inner span keeps the outer span's mask"
            );
            drop(outer);
            assert!(defers_none(&blocked()));
        });
    }

    #[test]
    fn a_span_restores_signals_blocked_before_it_began() {
        on_fresh_thread(|| {
            let mut previous = SigSet::empty();
            previous.add(Signal::SIGUSR1);
            previous.add(Signal::SIGTERM);
            pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&previous), None).unwrap();
            drop(DeferTermination::begin().unwrap());
            let mask = blocked();
            assert!(mask.contains(Signal::SIGUSR1));
            assert!(
                mask.contains(Signal::SIGTERM),
                "a signal the caller had blocked stays blocked"
            );
            assert!(!mask.contains(Signal::SIGINT));
            assert!(!mask.contains(Signal::SIGHUP));
        });
    }

    #[test]
    fn a_helper_thread_started_inside_a_span_inherits_the_blocked_set() {
        on_fresh_thread(|| {
            let _span = DeferTermination::begin().unwrap();
            let inherited = std::thread::spawn(blocked).join().unwrap();
            assert!(defers_all(&inherited));
        });
    }
}
