CREATE TABLE member_runtime_events_v6 (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    event_seq INTEGER NOT NULL CHECK(event_seq > 0),
    interaction_id INTEGER REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured','interaction_closed','permission_denied_observed','automatic_refusal_observed','automatic_review_unavailable','runner_degraded','runner_lost','runner_exited')),
    observed_at TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'),
    notification_state TEXT NOT NULL CHECK(notification_state IN ('inbox','archived')),
    message_id INTEGER UNIQUE REFERENCES messages(message_id) ON DELETE RESTRICT,
    archive_reason TEXT CHECK(archive_reason IS NULL OR archive_reason IN ('member_inactive','fleet_deleted','director_inactive')),
    UNIQUE(run_id, event_seq),
    CHECK((notification_state = 'inbox' AND message_id IS NOT NULL AND archive_reason IS NULL) OR
          (notification_state = 'archived' AND message_id IS NULL AND archive_reason IS NOT NULL))
);

INSERT INTO member_runtime_events_v6 SELECT * FROM member_runtime_events;
INSERT INTO sqlite_sequence(name,seq)
SELECT 'member_runtime_events_v6',seq FROM sqlite_sequence
WHERE name='member_runtime_events' AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name='member_runtime_events_v6');
UPDATE sqlite_sequence SET seq=MAX(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_runtime_events'),0))
WHERE name='member_runtime_events_v6';
DELETE FROM sqlite_sequence WHERE name='member_runtime_events_v6'
AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name='member_runtime_events');

CREATE TABLE member_permission_responses_v6 (
    input_id INTEGER PRIMARY KEY REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    event_id INTEGER NOT NULL REFERENCES member_runtime_events_v6(event_id) ON DELETE RESTRICT,
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

INSERT INTO member_permission_responses_v6 SELECT * FROM member_permission_responses;

CREATE TABLE member_permission_response_claims_v6 (
    interaction_id INTEGER PRIMARY KEY REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    input_id INTEGER NOT NULL UNIQUE REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    FOREIGN KEY(interaction_id, input_id)
        REFERENCES member_permission_responses_v6(interaction_id, input_id) ON DELETE RESTRICT
);

INSERT INTO member_permission_response_claims_v6 SELECT * FROM member_permission_response_claims;

DROP TABLE member_permission_response_claims;
DROP TABLE member_permission_responses;
DROP TABLE member_runtime_events;

ALTER TABLE member_runtime_events_v6 RENAME TO member_runtime_events;
ALTER TABLE member_permission_responses_v6 RENAME TO member_permission_responses;
ALTER TABLE member_permission_response_claims_v6 RENAME TO member_permission_response_claims;

CREATE TEMP TABLE v6_profiled_runs AS
  SELECT run_id, profile_version FROM member_runs WHERE profile_version IS NOT NULL;
UPDATE member_runs SET profile_version = NULL WHERE profile_version IS NOT NULL;
ALTER TABLE member_runs ADD COLUMN profile_match TEXT
  CHECK((profile_version IS NULL AND profile_match IS NULL)
     OR (profile_version IS NOT NULL AND profile_match IS NOT NULL
         AND profile_match IN ('exact','fallback')));
UPDATE member_runs SET
  profile_version = (SELECT p.profile_version FROM v6_profiled_runs p WHERE p.run_id = member_runs.run_id),
  profile_match = 'exact'
  WHERE run_id IN (SELECT run_id FROM v6_profiled_runs);
DROP TABLE v6_profiled_runs;
