use super::*;

fn rejected(result: Result<ProgrammaticWrite>) {
    assert!(
        result.is_err(),
        "genuine guarded write is refused before native I/O"
    );
}

fn binding(h: &mut Host, other: Option<&mut Transport>, wrong_bytes: bool, stale: bool) {
    let id = h.arm("prompt:guard-binding");
    h.drain();
    let before = storage::get_input(&h.fixture.conn, id).unwrap();
    let mut guard = h
        .coordinator
        .acquire_write_guard(
            &mut h.fixture.conn,
            &h.fixture.run,
            &mut h.transport,
            &h.controller,
            h.until,
        )
        .unwrap()
        .expect("actual Armed acquisition");
    if stale {
        h.transport
            .tick(
                &mut h.human,
                &mut h.pty,
                &mut h.output,
                &mut h.controller.observer(&h.writer, &RealObservationClock),
            )
            .unwrap();
    }
    let bytes = if wrong_bytes {
        b"unselected bytes".to_vec()
    } else {
        encoded()
    };
    match other {
        Some(other) => rejected(other.write_programmatic(&mut h.pty, &bytes, &mut guard, h.until)),
        None => rejected(
            h.transport
                .write_programmatic(&mut h.pty, &bytes, &mut guard, h.until),
        ),
    }
    rejected(
        h.transport
            .write_programmatic(&mut h.pty, &encoded(), &mut guard, h.until),
    );
    assert!(h.pty.offers.is_empty());
    drop(guard);
    let point = h.coordinator.retained_checkpoint(id).unwrap();
    assert!(matches!(point.stage, SubmissionStage::ZeroBytes));
    assert_eq!(point.written_bytes, 0);
    assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), before);
}

#[test]
fn step5_prompt_genuine_guard_binds_transport_turn_and_selected_bytes_and_consumes_rejection() {
    with_host(|h| with_host(|other| binding(h, Some(&mut other.transport), false, false)));
    with_host(|h| binding(h, None, true, false));
    with_host(|h| binding(h, None, false, true));
}

#[test]
fn step5_prompt_guard_drop_distinguishes_consumed_pre_io_from_unrecorded_possible_io() {
    for possible in [false, true] {
        with_host(|h| {
            let id = h.arm("prompt:guard-drop");
            h.drain();
            let before = storage::get_input(&h.fixture.conn, id).unwrap();
            let hooks = h.hooks.points.lock().unwrap().len();
            let mut guard = h
                .coordinator
                .acquire_write_guard(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    &mut h.transport,
                    &h.controller,
                    h.until,
                )
                .unwrap()
                .expect("real reservation and fresh drain");
            let bound = if possible {
                h.until
            } else {
                Instant::now() + Duration::from_millis(30)
            };
            let effective = guard.begin_write(&h.transport, &encoded(), bound).unwrap();
            assert!(effective <= bound);
            if possible {
                guard.mark_io_possible();
            } else {
                assert!(
                    guard
                        .begin_write(&h.transport, &encoded(), h.until)
                        .is_err(),
                    "successful pre-I/O validation consumes the permit too"
                );
                std::thread::sleep(
                    effective.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                );
            }
            drop(guard);
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            if possible {
                assert!(matches!(point.stage, SubmissionStage::Unknown));
            } else {
                assert!(matches!(point.stage, SubmissionStage::ZeroBytes));
            }
            assert_eq!(point.written_bytes, 0);
            assert_eq!(point.output_seq, h.transport.snapshot().screen_seq);
            assert!(h.pty.offers.is_empty());
            assert_eq!(
                h.hooks.points.lock().unwrap().len(),
                hooks,
                "drop creates no checkpoint hook"
            );
            assert_eq!(
                storage::get_input(&h.fixture.conn, id).unwrap(),
                before,
                "durable InProgress stays unchanged"
            );
        });
    }
}
