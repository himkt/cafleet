use super::*;
use std::fmt;

const RECORD_LIMIT: usize = 64;
const IDENTITY_LIMIT: usize = 16;
const START_LIMIT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnershipRefreshPhase {
    Spawn,
    TryWait,
    Signal(nix::sys::signal::Signal),
    Survivors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnershipRefreshOutcome {
    NoNative,
    AlreadyClosed,
    NoMembers,
    FreshEmpty,
    NoActive,
    Anchored,
    Unanchored,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnershipGroupRecheck {
    Empty,
    Nonempty { members: usize },
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnershipMatchOutcome {
    Matches,
    DifferentOrAbsent,
    Error,
}

#[derive(Clone)]
pub(crate) struct OwnershipObservedIdentity {
    pub pid: i64,
    pub start_identity: String,
    pub original_start_bytes: usize,
    pub start_truncated: bool,
}

impl fmt::Debug for OwnershipObservedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnershipObservedIdentity")
            .field("pid", &self.pid)
            .field("start_identity", &self.start_identity)
            .field("original_start_bytes", &self.original_start_bytes)
            .field("start_truncated", &self.start_truncated)
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct OwnershipIdentitySnapshot {
    pub total: usize,
    pub entries: Vec<OwnershipObservedIdentity>,
    pub truncated: bool,
}

impl fmt::Debug for OwnershipIdentitySnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnershipIdentitySnapshot")
            .field("total", &self.total)
            .field("entries", &self.entries)
            .field("truncated", &self.truncated)
            .finish()
    }
}

impl OwnershipIdentitySnapshot {
    fn capture<'a>(identities: impl ExactSizeIterator<Item = &'a ProcessIdentity>) -> Self {
        let total = identities.len();
        let entries = identities
            .take(IDENTITY_LIMIT)
            .map(|identity| {
                let original_start_bytes = identity.start_identity.len();
                let mut end = original_start_bytes.min(START_LIMIT);
                while !identity.start_identity.is_char_boundary(end) {
                    end -= 1;
                }
                OwnershipObservedIdentity {
                    pid: identity.pid,
                    start_identity: identity.start_identity[..end].into(),
                    original_start_bytes,
                    start_truncated: end < original_start_bytes,
                }
            })
            .collect();
        Self {
            total,
            entries,
            truncated: total > IDENTITY_LIMIT,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct OwnershipObservedMatch {
    pub pid: i64,
    pub outcome: OwnershipMatchOutcome,
}

impl fmt::Debug for OwnershipObservedMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnershipObservedMatch")
            .field("pid", &self.pid)
            .field("outcome", &self.outcome)
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct OwnershipMatchSnapshot {
    pub attempted: usize,
    pub entries: Vec<OwnershipObservedMatch>,
    pub truncated: bool,
}

impl fmt::Debug for OwnershipMatchSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnershipMatchSnapshot")
            .field("attempted", &self.attempted)
            .field("entries", &self.entries)
            .field("truncated", &self.truncated)
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct OwnershipRefreshRecord {
    pub sequence: u64,
    pub phase: OwnershipRefreshPhase,
    pub child_present: bool,
    pub native_exit: Option<ChildExit>,
    pub group_closed_before: bool,
    pub sampled_member_count: Option<usize>,
    pub active: Option<OwnershipIdentitySnapshot>,
    pub pinned: OwnershipIdentitySnapshot,
    pub matches: OwnershipMatchSnapshot,
    pub seeded_anchor: bool,
    pub anchored: Option<bool>,
    pub group_recheck: Option<OwnershipGroupRecheck>,
    pub outcome: OwnershipRefreshOutcome,
}

impl fmt::Debug for OwnershipRefreshRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnershipRefreshRecord")
            .field("sequence", &self.sequence)
            .field("phase", &self.phase)
            .field("child_present", &self.child_present)
            .field("native_exit", &self.native_exit)
            .field("group_closed_before", &self.group_closed_before)
            .field("sampled_member_count", &self.sampled_member_count)
            .field("active", &self.active)
            .field("pinned", &self.pinned)
            .field("matches", &self.matches)
            .field("seeded_anchor", &self.seeded_anchor)
            .field("anchored", &self.anchored)
            .field("group_recheck", &self.group_recheck)
            .field("outcome", &self.outcome)
            .finish()
    }
}

pub(crate) struct OwnershipDiagnostics {
    pub records: Vec<OwnershipRefreshRecord>,
    pub records_discarded: u64,
    pub first_unanchored: Option<Box<OwnershipRefreshRecord>>,
}

impl fmt::Debug for OwnershipDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnershipDiagnostics")
            .field("records", &self.records)
            .field("records_discarded", &self.records_discarded)
            .field("first_unanchored", &self.first_unanchored)
            .finish()
    }
}

pub(super) struct OwnershipDiagnosticBuffer {
    output: OwnershipDiagnostics,
    next_sequence: u64,
}

pub(super) struct PendingOwnershipRefresh {
    sequence: u64,
    phase: OwnershipRefreshPhase,
    child_present: bool,
    native_exit: Option<ChildExit>,
    group_closed_before: bool,
    pub sampled_member_count: Option<usize>,
    active: Option<OwnershipIdentitySnapshot>,
    pinned: Option<OwnershipIdentitySnapshot>,
    matches: [Option<OwnershipObservedMatch>; IDENTITY_LIMIT],
    matches_attempted: usize,
    seeded_anchor: bool,
    anchored: Option<bool>,
    pub group_recheck: Option<OwnershipGroupRecheck>,
    pub outcome: OwnershipRefreshOutcome,
}

impl PendingOwnershipRefresh {
    pub(super) fn observe_match(&mut self, pid: i64, result: &Result<bool>) {
        if self.matches_attempted < IDENTITY_LIMIT {
            self.matches[self.matches_attempted] = Some(OwnershipObservedMatch {
                pid,
                outcome: match result {
                    Ok(true) => OwnershipMatchOutcome::Matches,
                    Ok(false) => OwnershipMatchOutcome::DifferentOrAbsent,
                    Err(_) => OwnershipMatchOutcome::Error,
                },
            });
        }
        self.matches_attempted = self
            .matches_attempted
            .checked_add(1)
            .expect("ownership diagnostic match count fits usize");
    }

    pub(super) fn observe_decision(
        &mut self,
        active: &[ProcessIdentity],
        pinned: &BTreeMap<i64, ProcessIdentity>,
        anchored: Option<bool>,
    ) {
        self.active = Some(OwnershipIdentitySnapshot::capture(active.iter()));
        self.pinned = Some(OwnershipIdentitySnapshot::capture(pinned.values()));
        self.anchored = anchored;
    }

    fn finish(
        self,
        pinned: &BTreeMap<i64, ProcessIdentity>,
        failed: bool,
    ) -> OwnershipRefreshRecord {
        let outcome = if failed && self.outcome != OwnershipRefreshOutcome::Unanchored {
            OwnershipRefreshOutcome::Error
        } else {
            self.outcome
        };
        OwnershipRefreshRecord {
            sequence: self.sequence,
            phase: self.phase,
            child_present: self.child_present,
            native_exit: self.native_exit,
            group_closed_before: self.group_closed_before,
            sampled_member_count: self.sampled_member_count,
            active: self.active,
            pinned: match self.pinned {
                Some(snapshot) => snapshot,
                None => OwnershipIdentitySnapshot::capture(pinned.values()),
            },
            matches: OwnershipMatchSnapshot {
                attempted: self.matches_attempted,
                entries: self.matches.into_iter().flatten().collect(),
                truncated: self.matches_attempted > IDENTITY_LIMIT,
            },
            seeded_anchor: self.seeded_anchor,
            anchored: self.anchored,
            group_recheck: self.group_recheck,
            outcome,
        }
    }
}

impl OwnershipDiagnosticBuffer {
    fn new() -> Self {
        Self {
            output: OwnershipDiagnostics {
                records: Vec::with_capacity(RECORD_LIMIT),
                records_discarded: 0,
                first_unanchored: None,
            },
            next_sequence: 1,
        }
    }

    fn begin(
        &mut self,
        phase: OwnershipRefreshPhase,
        session: &PtySession,
    ) -> PendingOwnershipRefresh {
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .expect("ownership diagnostic sequence fits u64");
        PendingOwnershipRefresh {
            sequence,
            phase,
            child_present: session.child.is_some(),
            native_exit: session.exit,
            group_closed_before: session.group_closed,
            sampled_member_count: None,
            active: None,
            pinned: None,
            matches: [None; IDENTITY_LIMIT],
            matches_attempted: 0,
            seeded_anchor: session.child.is_some() && session.exit.is_none(),
            anchored: None,
            group_recheck: None,
            outcome: OwnershipRefreshOutcome::Error,
        }
    }

    fn retain(&mut self, record: OwnershipRefreshRecord) {
        if record.anchored == Some(false) && self.output.first_unanchored.is_none() {
            self.output.first_unanchored = Some(Box::new(record.clone()));
        }
        if self.output.records.len() == RECORD_LIMIT {
            self.output.records.remove(0);
            self.output.records_discarded = self
                .output
                .records_discarded
                .checked_add(1)
                .expect("ownership diagnostic discarded count fits u64");
        }
        self.output.records.push(record);
    }
}

impl PtySession {
    pub(crate) fn enable_ownership_diagnostics(&mut self) -> Result<()> {
        ensure(
            self.native.is_none() && self.child.is_none() && self.ownership_diagnostics.is_none(),
            "ownership diagnostics require an unobserved prepared session",
        )?;
        self.ownership_diagnostics = Some(OwnershipDiagnosticBuffer::new());
        Ok(())
    }

    pub(crate) fn take_ownership_diagnostics(&mut self) -> Result<OwnershipDiagnostics> {
        Ok(self
            .ownership_diagnostics
            .take()
            .ok_or_else(|| error("ownership diagnostics were not enabled"))?
            .output)
    }

    pub(super) fn refresh_owned_traced(&mut self, phase: OwnershipRefreshPhase) -> Result<()> {
        if self.ownership_diagnostics.is_none() {
            return self.refresh_owned();
        }
        self.refresh_owned_traced_with(phase, &mut NativeProcessObservations)
    }

    pub(super) fn refresh_owned_traced_with(
        &mut self,
        phase: OwnershipRefreshPhase,
        observed: &mut impl ProcessObservations,
    ) -> Result<()> {
        let Some(mut buffer) = self.ownership_diagnostics.take() else {
            return self.refresh_owned_with(observed);
        };
        assert!(self.pending_ownership_refresh.is_none());
        self.pending_ownership_refresh = Some(buffer.begin(phase, self));
        let result = self.refresh_owned_with(observed);
        let pending = self
            .pending_ownership_refresh
            .take()
            .expect("ownership refresh retains its diagnostic record");
        buffer.retain(pending.finish(&self.owned, result.is_err()));
        self.ownership_diagnostics = Some(buffer);
        result
    }

    pub(super) fn note_ownership_outcome(&mut self, outcome: OwnershipRefreshOutcome) {
        if let Some(record) = self.pending_ownership_refresh.as_mut() {
            record.outcome = outcome;
        }
    }
}
