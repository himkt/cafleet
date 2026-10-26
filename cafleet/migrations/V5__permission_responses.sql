CREATE TABLE member_input_operations_v5 (
    input_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    operation_key TEXT NOT NULL CHECK(length(trim(operation_key)) > 0),
    kind TEXT NOT NULL CHECK(kind IN ('preview','prompt','exec','permission_response')),
    payload_digest TEXT NOT NULL CHECK(length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
    state TEXT NOT NULL CHECK(state IN ('ready','in_progress','accepted','uncertain','settled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    message_id INTEGER REFERENCES messages(message_id) ON DELETE RESTRICT,
    disposition TEXT CHECK(disposition IS NULL OR disposition IN ('submitted','not_submitted','abandoned_uncertain')),
    evidence_ref TEXT,
    UNIQUE(run_id, operation_key),
    CHECK(kind != 'preview' OR message_id IS NOT NULL),
    CHECK((state = 'settled' AND disposition IS NOT NULL) OR (state != 'settled' AND disposition IS NULL))
);

INSERT INTO member_input_operations_v5 SELECT * FROM member_input_operations;
INSERT INTO sqlite_sequence(name,seq)
SELECT 'member_input_operations_v5',seq FROM sqlite_sequence
WHERE name='member_input_operations' AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name='member_input_operations_v5');
UPDATE sqlite_sequence SET seq=MAX(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_input_operations'),0))
WHERE name='member_input_operations_v5';
DELETE FROM sqlite_sequence WHERE name='member_input_operations_v5'
AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name='member_input_operations');
DROP TABLE member_input_operations;
ALTER TABLE member_input_operations_v5 RENAME TO member_input_operations;

CREATE TABLE member_permission_responses (
    input_id INTEGER PRIMARY KEY REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    event_id INTEGER NOT NULL REFERENCES member_runtime_events(event_id) ON DELETE RESTRICT,
    interaction_id INTEGER NOT NULL REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    expected_owner_epoch INTEGER NOT NULL CHECK(expected_owner_epoch > 0),
    request_token TEXT NOT NULL CHECK(length(request_token) = 64 AND request_token NOT GLOB '*[^0-9a-f]*'),
    decision TEXT NOT NULL CHECK(decision IN ('allow','deny')),
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    director_mode TEXT NOT NULL CHECK(director_mode IN ('auto','manual','unknown')),
    decision_source TEXT NOT NULL CHECK(decision_source IN ('director-auto','user')),
    mode_evidence_ref TEXT NOT NULL CHECK(length(CAST(mode_evidence_ref AS BLOB)) BETWEEN 1 AND 2048),
    decision_evidence_ref TEXT NOT NULL CHECK(length(CAST(decision_evidence_ref AS BLOB)) BETWEEN 1 AND 2048),
    answer_profile_version TEXT NOT NULL CHECK(length(trim(answer_profile_version)) > 0),
    answer_rule_id TEXT NOT NULL CHECK(length(trim(answer_rule_id)) > 0),
    decided_at TEXT NOT NULL CHECK(length(trim(decided_at)) > 0),
    UNIQUE(interaction_id, input_id),
    CHECK(decision_source != 'director-auto' OR director_mode = 'auto')
);

CREATE TABLE member_permission_response_claims (
    interaction_id INTEGER PRIMARY KEY REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    input_id INTEGER NOT NULL UNIQUE REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    FOREIGN KEY(interaction_id, input_id)
        REFERENCES member_permission_responses(interaction_id, input_id) ON DELETE RESTRICT
);
