use super::*;
use crate::coding_agent::CodingAgent;

fn request(
    host: &Host,
    key: String,
    kind: storage::InputKind,
    message_id: Option<i64>,
) -> InputControlRequest {
    let mut request = host.request(&key);
    let input = SubmitInput {
        operation_key: key,
        input_kind: kind,
        message_id,
        literal: LITERAL.into(),
    };
    request.operation = if kind == storage::InputKind::Exec {
        InputControlOperation::SubmitExec(input)
    } else {
        InputControlOperation::SubmitPrompt(input)
    };
    request
}

fn complete(host: &mut Host, request: InputControlRequest) -> SubmissionResult {
    assert!(
        host.coordinator
            .handle(
                &mut host.fixture.conn,
                &host.fixture.run,
                request,
                &host.controller,
                host.until,
            )
            .unwrap()
            .is_none()
    );
    let reply = submitted(
        (0..3)
            .find_map(|_| {
                host.drain();
                host.advance()
            })
            .expect("operation completes through three genuine drained turns"),
    );
    assert_eq!(reply.status, SubmissionStatus::Accepted);
    assert_eq!(reply.stage, SubmissionStage::Submitted);
    reply
}

fn refuse(host: &mut Host, request: InputControlRequest) {
    let result = host.coordinator.handle(
        &mut host.fixture.conn,
        &host.fixture.run,
        request,
        &host.controller,
        host.until,
    );
    let result = match result {
        Ok(None) => (0..3)
            .find_map(|_| {
                host.drain();
                match host.coordinator.advance(
                    &mut host.fixture.conn,
                    &host.fixture.run,
                    &mut host.transport,
                    &mut host.pty,
                    &host.controller,
                    host.until,
                ) {
                    Ok(None) => None,
                    result => Some(result),
                }
            })
            .expect("ineligible operation resolves within bounded drained turns"),
        result => result,
    };
    match result {
        Err(_) => {}
        Ok(Some(reply)) => assert!(matches!(
            submitted(reply).status,
            SubmissionStatus::Held | SubmissionStatus::Rejected
        )),
        Ok(None) => panic!("ineligible operation returns a refusal"),
    }
    assert!(host.pty.offers.is_empty());
    assert_eq!(
        host.fixture
            .conn
            .query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}

#[test]
fn step5_operations_preview_binds_the_exact_run_message_key_and_durable_recipient() {
    with_host(|host| {
        let message = crate::broker::send_message(
            &mut host.fixture.conn,
            host.fixture.run.director_member_id,
            host.fixture.run.member_id,
            LITERAL,
        )
        .unwrap();
        let key = format!("preview:{}:{}", host.fixture.run.run_id, message.message_id);
        let input = request(
            host,
            key.clone(),
            storage::InputKind::Preview,
            Some(message.message_id),
        );
        let reply = complete(host, input);
        let row = storage::get_input(&host.fixture.conn, reply.input_id.unwrap()).unwrap();
        assert_eq!(row.kind, storage::InputKind::Preview);
        assert_eq!(row.operation_key, key);
        assert_eq!(row.message_id, Some(message.message_id));
        assert_eq!(row.state, storage::InputState::Accepted);
        assert_eq!(host.pty.offers, vec![encoded()]);
        assert_eq!(
            crate::broker::get_message(&host.fixture.conn, message.message_id).unwrap(),
            message
        );
        let notified: Option<String> = host
            .fixture
            .conn
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message.message_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(notified.is_none());
    });

    for wrong_recipient in [false, true] {
        with_host(|host| {
            let run = &host.fixture.run;
            let (sender, recipient) = if wrong_recipient {
                (run.member_id, run.director_member_id)
            } else {
                (run.director_member_id, run.member_id)
            };
            let message =
                crate::broker::send_message(&mut host.fixture.conn, sender, recipient, LITERAL)
                    .unwrap();
            let key_run = run.run_id + i64::from(!wrong_recipient);
            let input = request(
                host,
                format!("preview:{key_run}:{}", message.message_id),
                storage::InputKind::Preview,
                Some(message.message_id),
            );
            refuse(host, input);
            assert_eq!(
                crate::broker::get_message(&host.fixture.conn, message.message_id).unwrap(),
                message
            );
        });
    }
}

#[test]
fn step5_operations_exec_selects_its_native_shell_plan_and_requires_its_own_coverage() {
    let mut definition = catalog_definition();
    let mut fixture = definition["fixtures"][0].clone();
    fixture["id"] = serde_json::json!("synthetic-exec-plan");
    definition["fixtures"].as_array_mut().unwrap().push(fixture);
    definition["profiles"][0]["capabilities"]["composer"]["submission"] = serde_json::json!({
        "prompt":null,
        "exec":{"prefix_hex":"2120","submit_hex":"6b6c","fixture_ids":["synthetic-exec-plan"]}
    });
    let exec_catalog = ProfileCatalog::from_definitions(&definition).unwrap();
    let coverage = exec_catalog.capabilities(CodingAgent::Codex, Some(VERSION));
    assert_eq!(coverage.composer, storage::Coverage::Tested);
    assert_eq!(coverage.prompt_submission, storage::Coverage::Unknown);
    assert_eq!(coverage.native_exec_submission, storage::Coverage::Tested);
    let mut expected = b"! ".to_vec();
    expected.extend_from_slice(LITERAL.as_bytes());
    expected.extend_from_slice(b"kl");
    with_catalog_host(exec_catalog, expected.clone(), |host| {
        let key = format!("exec:{}:{}", host.fixture.run.run_id, "b".repeat(64));
        let input = request(host, key.clone(), storage::InputKind::Exec, None);
        let reply = complete(host, input);
        let row = storage::get_input(&host.fixture.conn, reply.input_id.unwrap()).unwrap();
        assert_eq!(row.kind, storage::InputKind::Exec);
        assert_eq!(row.operation_key, key);
        assert_eq!(row.message_id, None);
        assert_eq!(row.state, storage::InputState::Accepted);
        assert_eq!(host.pty.offers, vec![expected]);
    });

    let prompt_catalog = catalog();
    let coverage = prompt_catalog.capabilities(CodingAgent::Codex, Some(VERSION));
    assert_eq!(coverage.prompt_submission, storage::Coverage::Tested);
    assert_eq!(coverage.native_exec_submission, storage::Coverage::Unknown);
    with_catalog_host(prompt_catalog, encoded(), |host| {
        let input = request(
            host,
            format!("exec:{}:{}", host.fixture.run.run_id, "c".repeat(64)),
            storage::InputKind::Exec,
            None,
        );
        refuse(host, input);
    });
}
