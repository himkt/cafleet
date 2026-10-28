use std::io;
use std::ops::{Deref, DerefMut};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

pub(crate) struct OwnedChild {
    child: Child,
    cleanup_deadline: Option<Instant>,
}
impl OwnedChild {
    pub(crate) fn new(child: Child) -> Self {
        Self {
            child,
            cleanup_deadline: None,
        }
    }
    pub(crate) fn poll_until(&mut self, deadline: Instant) -> io::Result<Option<ExitStatus>> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(
                Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    pub(crate) fn terminate_until(&mut self, deadline: Instant) {
        let deadline = self
            .cleanup_deadline
            .map_or(deadline, |original| original.min(deadline));
        self.cleanup_deadline = Some(deadline);
        match self.child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(cause) => {
                eprintln!(
                    "owned child {} cleanup status failed: {cause}",
                    self.child.id()
                );
                return;
            }
        }
        if let Err(cause) = self.child.kill() {
            eprintln!("owned child {} kill failed: {cause}", self.child.id());
        }
        match self.poll_until(deadline) {
            Ok(Some(_)) => {}
            Ok(None) => eprintln!(
                "owned child {} remained unreaped at its cleanup deadline",
                self.child.id()
            ),
            Err(cause) => eprintln!(
                "owned child {} reap polling failed: {cause}",
                self.child.id()
            ),
        }
    }
}
impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.terminate_until(
            self.cleanup_deadline
                .unwrap_or_else(|| Instant::now() + Duration::from_secs(2)),
        );
    }
}

#[test]
fn runner_batch2_owned_child_guard_cleans_up_when_identity_setup_unwinds() {
    let mut identity = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let child = OwnedChild::new(
            std::process::Command::new("/bin/cat")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        identity = crate::runtime::runner::ProcessIdentity::read(i64::from(child.id())).unwrap();
        panic!("owned identity setup fixture unwind");
    }));
    assert!(result.is_err());
    assert!(
        !identity
            .expect("owned child identity before intentional unwind")
            .matches()
            .unwrap()
    );
}
