use super::*;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

mod review;
mod terminal_tail;

#[derive(Default)]
struct FaultState {
    data: Cursor<Vec<u8>>,
    short_write: Option<usize>,
    fail_after: Option<u64>,
    fail_sync: bool,
    fail_truncate: bool,
    syncs: usize,
    largest_read: usize,
    truncations: Vec<u64>,
}
#[derive(Clone, Default)]
struct FaultIo(Arc<Mutex<FaultState>>);
impl FaultIo {
    fn snapshot_bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().data.get_ref().clone()
    }
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let value = Self::default();
        value.0.lock().unwrap().data = Cursor::new(bytes);
        value
    }
}
impl Read for FaultIo {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap();
        state.largest_read = state.largest_read.max(bytes.len());
        state.data.read(bytes)
    }
}
impl Write for FaultIo {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap();
        let position = state.data.position();
        if state.fail_after.is_some_and(|limit| position >= limit) {
            return Err(io::Error::other("fixture frame write failure"));
        }
        let mut limit = bytes.len();
        if let Some(short) = state.short_write {
            limit = limit.min(short);
        }
        if let Some(end) = state.fail_after {
            limit = limit.min(usize::try_from(end - position).unwrap());
        }
        state.data.write(&bytes[..limit])
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for FaultIo {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.lock().unwrap().data.seek(position)
    }
}
impl OutboxIo for FaultIo {
    fn sync_all(&mut self) -> io::Result<()> {
        let mut state = self.0.lock().unwrap();
        state.syncs += 1;
        if state.fail_sync {
            Err(io::Error::other("fixture frame sync failure"))
        } else {
            Ok(())
        }
    }
    fn byte_len(&self) -> io::Result<u64> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .data
            .get_ref()
            .len()
            .try_into()
            .unwrap())
    }
    fn set_len(&mut self, length: u64) -> io::Result<()> {
        let mut state = self.0.lock().unwrap();
        if state.fail_truncate {
            return Err(io::Error::other("fixture frame truncate failure"));
        }
        state.truncations.push(length);
        state.data.get_mut().resize(length.try_into().unwrap(), 0);
        Ok(())
    }
}

fn open(fixture: &WriterFixture, io: FaultIo) -> Outbox<FaultIo> {
    Outbox::from_io(
        &fixture.fixture.conn,
        &fixture.fixture.run,
        fixture.lock.clone(),
        io,
        deadline(),
    )
    .unwrap()
}

#[test]
fn runner_batch2_outbox_frames_pin_big_endian_length_checksum_and_original_identity() {
    let fixture = WriterFixture::new();
    let io = FaultIo::default();
    let mut outbox = open(&fixture, io.clone());
    let value = transition(&fixture, 1);
    let syncs = io.0.lock().unwrap().syncs;
    outbox.append_synced(&value, deadline()).unwrap();
    assert_eq!(io.snapshot_bytes(), frame(&value));
    assert!(io.0.lock().unwrap().syncs > syncs);
    let scan = outbox.scan(deadline()).unwrap();
    assert_eq!(scan.frames.len(), 1);
    assert_eq!(scan.frames[0].transition, value);
    assert_eq!(scan.frames[0].start, 0);
    assert_eq!(scan.frames[0].end, io.snapshot_bytes().len() as u64);
    assert!(scan.incomplete_tail.is_none());
    assert_eq!(scan.maximum_event_seq, Some(1));
}

#[test]
fn runner_batch2_outbox_matching_duplicates_reconcile_but_conflicting_sequence_bytes_fail() {
    let fixture = WriterFixture::new();
    let original = transition(&fixture, 1);
    let mut duplicate = frame(&original);
    duplicate.extend(frame(&original));
    let io = FaultIo::from_bytes(duplicate);
    let mut outbox = open(&fixture, io);
    assert!(outbox.scan(deadline()).is_ok());
    let mut changed = original.clone();
    changed.observed_at = "2026-10-26T16:41:00.000000+00:00".into();
    let mut bytes = frame(&original);
    bytes.extend(frame(&changed));
    let io = FaultIo::from_bytes(bytes.clone());
    let result = Outbox::from_io(
        &fixture.fixture.conn,
        &fixture.fixture.run,
        fixture.lock.clone(),
        io.clone(),
        deadline(),
    )
    .and_then(|mut outbox| outbox.scan(deadline()));
    assert!(result.is_err());
    assert_eq!(io.snapshot_bytes(), bytes);
}

#[test]
fn runner_batch2_outbox_total_capacity_includes_frame_overhead_and_rejects_before_append() {
    let mut fixture = WriterFixture::new();
    fixture
        .fixture
        .conn
        .execute(
            "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
            [fixture.fixture.run.run_id],
        )
        .unwrap();
    for _ in 1..16 {
        storage::reserve_event_sequences(&mut fixture.fixture.conn, &fixture.fixture.run.owner())
            .unwrap();
    }
    fixture.fixture.run =
        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    let io = FaultIo::default();
    let mut outbox = open(&fixture, io.clone());
    let policy=storage::evidence::EvidencePolicy::redacted(&serde_json::json!({"schema_version":1,"patterns":["absent-fixture-canary"],"sensitive_rules":[]}),&["runner-lifecycle"]).unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    let mut used = 0;
    for sequence in 1..=4096 {
        assert!(
            Instant::now() < until,
            "bounded outbox-capacity fixture exceeded its budget"
        );
        let mut value = transition(&fixture, sequence);
        let mut raw = value.evidence_json.clone();
        raw["sensitive"] = serde_json::json!(false);
        raw["excerpt"] = serde_json::json!("x".repeat(6000));
        value.evidence_json = policy.apply(&raw).unwrap();
        value.kind = RuntimeEventKind::PermissionDeniedObserved;
        value.cause_json = None;
        let size = frame(&value).len();
        if used + size > 16 * 1024 * 1024 {
            let before = io.snapshot_bytes();
            assert!(outbox.append_synced(&value, until).is_err());
            assert_eq!(io.snapshot_bytes(), before);
            assert!(used <= 16 * 1024 * 1024);
            return;
        }
        outbox.append_synced(&value, until).unwrap();
        used += size;
        assert_eq!(io.byte_len().unwrap(), used as u64);
    }
    panic!("fixture must reach the fixed outbox capacity");
}

#[test]
fn runner_batch2_outbox_short_writes_preserve_the_complete_frame() {
    let fixture = WriterFixture::new();
    let io = FaultIo::default();
    io.0.lock().unwrap().short_write = Some(1);
    let mut outbox = open(&fixture, io.clone());
    let value = transition(&fixture, 1);
    outbox.append_synced(&value, deadline()).unwrap();
    assert_eq!(independent_frames(&io.snapshot_bytes()), vec![value]);
}

#[test]
fn runner_batch2_outbox_reopen_recovers_complete_original_frames_in_order() {
    let fixture = WriterFixture::new();
    let values = [
        transition(&fixture, 1),
        transition(&fixture, 2),
        transition(&fixture, 3),
    ];
    let bytes = values.iter().flat_map(frame).collect();
    let io = FaultIo::from_bytes(bytes);
    let mut outbox = open(&fixture, io.clone());
    let scan = outbox.scan(deadline()).unwrap();
    assert_eq!(
        scan.frames
            .iter()
            .map(|entry| entry.transition.clone())
            .collect::<Vec<_>>(),
        values
    );
    assert_eq!(scan.maximum_event_seq, Some(3));
    assert!(scan.incomplete_tail.is_none());
    assert_eq!(independent_frames(&io.snapshot_bytes()), values);
}

#[test]
fn runner_batch2_outbox_each_incomplete_final_component_retains_the_valid_prefix() {
    let fixture = WriterFixture::new();
    let first = frame(&transition(&fixture, 1));
    let second = frame(&transition(&fixture, 2));
    for cut in [1, 3, 4, 20, 35, 36, second.len() - 1] {
        let mut bytes = first.clone();
        bytes.extend_from_slice(&second[..cut]);
        let io = FaultIo::from_bytes(bytes.clone());
        let mut outbox = open(&fixture, io.clone());
        let scan = outbox.scan(deadline()).unwrap();
        assert_eq!(scan.frames.len(), 1, "cut={cut}");
        assert_eq!(scan.frames[0].transition, transition(&fixture, 1));
        assert!(scan.incomplete_tail.is_some(), "cut={cut}");
        assert_eq!(io.snapshot_bytes(), bytes);
    }
}

#[test]
fn runner_batch2_outbox_append_diagnoses_and_truncates_only_an_incomplete_tail() {
    let fixture = WriterFixture::new();
    let first = frame(&transition(&fixture, 1));
    let mut initial = first.clone();
    initial.extend_from_slice(&[0, 0]);
    let io = FaultIo::from_bytes(initial);
    let mut outbox = open(&fixture, io.clone());
    assert!(outbox.scan(deadline()).unwrap().incomplete_tail.is_some());
    outbox
        .append_synced(&transition(&fixture, 2), deadline())
        .unwrap();
    assert_eq!(
        independent_frames(&io.snapshot_bytes()),
        vec![transition(&fixture, 1), transition(&fixture, 2)]
    );
    assert!(
        io.0.lock()
            .unwrap()
            .truncations
            .contains(&(first.len() as u64))
    );
}

#[test]
fn runner_batch2_outbox_completed_corruption_preserves_all_original_bytes() {
    let fixture = WriterFixture::new();
    let valid = frame(&transition(&fixture, 1));
    for corrupt in ["checksum", "json", "run", "sequence"] {
        let mut value = transition(&fixture, 1);
        if corrupt == "run" {
            value.run_id += 1;
        }
        if corrupt == "sequence" {
            value.event_seq = 0;
        }
        let mut bytes = if matches!(corrupt, "run" | "sequence") {
            frame(&value)
        } else {
            valid.clone()
        };
        if corrupt == "checksum" {
            bytes[4] ^= 1;
        }
        if corrupt == "json" {
            bytes[36] = b'!';
            let digest = Sha256::digest(&bytes[36..]);
            bytes[4..36].copy_from_slice(&digest);
        }
        let io = FaultIo::from_bytes(bytes.clone());
        let result = Outbox::from_io(
            &fixture.fixture.conn,
            &fixture.fixture.run,
            fixture.lock.clone(),
            io.clone(),
            deadline(),
        )
        .and_then(|mut outbox| outbox.scan(deadline()));
        assert!(result.is_err(), "{corrupt}");
        assert_eq!(io.snapshot_bytes(), bytes);
    }
}

#[test]
fn runner_batch2_outbox_oversized_declared_length_fails_before_large_reads() {
    let fixture = WriterFixture::new();
    let io = FaultIo::from_bytes(u32::MAX.to_be_bytes().to_vec());
    let result = Outbox::from_io(
        &fixture.fixture.conn,
        &fixture.fixture.run,
        fixture.lock.clone(),
        io.clone(),
        deadline(),
    )
    .and_then(|mut outbox| outbox.scan(deadline()));
    assert!(result.is_err());
    assert!(io.0.lock().unwrap().largest_read <= 36);
    assert_eq!(io.snapshot_bytes(), u32::MAX.to_be_bytes());
}

#[test]
fn runner_batch2_outbox_sync_failure_leaves_a_complete_recoverable_pending_frame() {
    let fixture = WriterFixture::new();
    let io = FaultIo::default();
    let mut outbox = open(&fixture, io.clone());
    io.0.lock().unwrap().fail_sync = true;
    assert!(
        outbox
            .append_synced(&transition(&fixture, 1), deadline())
            .is_err()
    );
    assert_eq!(
        independent_frames(&io.snapshot_bytes()),
        vec![transition(&fixture, 1)]
    );
    io.0.lock().unwrap().fail_sync = false;
    assert_eq!(outbox.scan(deadline()).unwrap().frames.len(), 1);
    assert_eq!(
        fixture
            .fixture
            .conn
            .query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}

#[test]
fn runner_batch2_outbox_partial_write_failure_is_an_explicit_tail_without_a_commit() {
    let fixture = WriterFixture::new();
    let io = FaultIo::default();
    let mut outbox = open(&fixture, io.clone());
    io.0.lock().unwrap().fail_after = Some(40);
    assert!(
        outbox
            .append_synced(&transition(&fixture, 1), deadline())
            .is_err()
    );
    let scan = outbox.scan(deadline()).unwrap();
    assert!(scan.frames.is_empty());
    assert!(scan.incomplete_tail.is_some());
    assert_eq!(io.snapshot_bytes().len(), 40);
}

#[test]
fn runner_batch2_outbox_compaction_retains_committed_prefix_until_every_frame_is_durable() {
    let mut fixture = WriterFixture::new();
    let values = [transition(&fixture, 1), transition(&fixture, 2)];
    let io = FaultIo::from_bytes(values.iter().flat_map(frame).collect());
    let mut outbox = open(&fixture, io.clone());
    let original = io.snapshot_bytes();
    storage::persist_transition(
        &mut fixture.fixture.conn,
        &fixture.fixture.run.owner(),
        &values[0],
        PersistenceMode::Live,
    )
    .unwrap();
    outbox
        .compact_committed(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            deadline(),
        )
        .unwrap();
    assert_eq!(io.snapshot_bytes(), original);
    storage::persist_transition(
        &mut fixture.fixture.conn,
        &fixture.fixture.run.owner(),
        &values[1],
        PersistenceMode::Live,
    )
    .unwrap();
    outbox
        .compact_committed(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            deadline(),
        )
        .unwrap();
    assert!(io.snapshot_bytes().is_empty());
    assert!(io.0.lock().unwrap().syncs > 0);
}

#[test]
fn runner_batch2_outbox_compaction_checks_event_message_association_before_removal() {
    let mut fixture = WriterFixture::new();
    let value = transition(&fixture, 1);
    storage::persist_transition(
        &mut fixture.fixture.conn,
        &fixture.fixture.run.owner(),
        &value,
        PersistenceMode::Live,
    )
    .unwrap();
    let io = FaultIo::from_bytes(frame(&value));
    let mut outbox = open(&fixture, io.clone());
    fixture
        .fixture
        .conn
        .execute("UPDATE messages SET text='corrupt durable association'", [])
        .unwrap();
    let before = io.snapshot_bytes();
    assert!(
        outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline()
            )
            .is_err()
    );
    assert_eq!(io.snapshot_bytes(), before);
}

#[test]
fn runner_batch2_outbox_truncate_failure_preserves_committed_originals_for_retry() {
    let mut fixture = WriterFixture::new();
    let value = transition(&fixture, 1);
    storage::persist_transition(
        &mut fixture.fixture.conn,
        &fixture.fixture.run.owner(),
        &value,
        PersistenceMode::Live,
    )
    .unwrap();
    let io = FaultIo::from_bytes(frame(&value));
    let mut outbox = open(&fixture, io.clone());
    io.0.lock().unwrap().fail_truncate = true;
    let before = io.snapshot_bytes();
    assert!(
        outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline()
            )
            .is_err()
    );
    assert_eq!(io.snapshot_bytes(), before);
    io.0.lock().unwrap().fail_truncate = false;
    outbox
        .compact_committed(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            deadline(),
        )
        .unwrap();
    assert!(io.snapshot_bytes().is_empty());
}

#[test]
fn runner_batch2_outbox_expired_deadline_and_stale_epoch_have_zero_content_effect() {
    let fixture = WriterFixture::new();
    let io = FaultIo::default();
    let mut outbox = open(&fixture, io.clone());
    assert!(
        outbox
            .append_synced(
                &transition(&fixture, 1),
                Instant::now() - Duration::from_millis(1)
            )
            .is_err()
    );
    assert!(io.snapshot_bytes().is_empty());
    fixture
        .fixture
        .conn
        .execute(
            "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
            [fixture.fixture.run.run_id],
        )
        .unwrap();
    assert!(
        outbox
            .append_synced(&transition(&fixture, 1), deadline())
            .is_err()
    );
    assert!(io.snapshot_bytes().is_empty());
}

#[test]
fn runner_batch2_outbox_holds_the_original_lock_for_its_entire_content_lifetime() {
    let WriterFixture {
        fixture,
        artifacts: _artifacts,
        lock,
    } = WriterFixture::new();
    let outbox = Outbox::open_owned(&fixture.conn, &fixture.run, lock.clone(), deadline()).unwrap();
    drop(lock);
    assert!(
        RunLock::acquire(&fixture.conn, &fixture.run.owner())
            .unwrap()
            .is_none()
    );
    drop(outbox);
    assert!(
        RunLock::acquire(&fixture.conn, &fixture.run.owner())
            .unwrap()
            .is_some()
    );
}
