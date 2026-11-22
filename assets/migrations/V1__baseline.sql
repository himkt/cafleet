PRAGMA application_id = 1128351289;

CREATE TABLE members (
    member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    fleet_id INTEGER NOT NULL REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_at TEXT NOT NULL
);
CREATE INDEX idx_members_fleet_status ON members (fleet_id, status);

CREATE TABLE fleets (
    fleet_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    deleted_at TEXT,
    director_member_id INTEGER REFERENCES members (member_id) ON DELETE RESTRICT,
    last_window_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(last_window_ordinal >= 0)
);

CREATE TABLE member_placements (
    member_id INTEGER NOT NULL PRIMARY KEY REFERENCES members (member_id) ON DELETE CASCADE,
    mux_session TEXT NOT NULL,
    mux_window_id TEXT,
    mux_window_name TEXT,
    window_ordinal INTEGER NOT NULL CHECK(window_ordinal >= 0),
    column_side TEXT NOT NULL CHECK(column_side IN ('left','right')),
    mux_pane_id TEXT,
    backend TEXT NOT NULL,
    mux_server TEXT NOT NULL,
    coding_agent TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK((mux_pane_id IS NULL) = (mux_window_id IS NULL)
          AND (mux_window_id IS NULL) = (mux_window_name IS NULL))
);

CREATE TABLE messages (
    message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    from_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    to_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL,
    status_state TEXT NOT NULL,
    status_timestamp TEXT NOT NULL,
    text TEXT NOT NULL,
    notified_at TEXT
);
CREATE INDEX idx_messages_to_member_status_ts ON messages (to_member_id, status_timestamp);
CREATE INDEX idx_messages_from_member_created_at ON messages (from_member_id, created_at);

CREATE TABLE runtime_database_identity (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    instance_id TEXT NOT NULL CHECK(length(instance_id) = 32 AND instance_id NOT GLOB '*[^0-9a-f]*')
);
INSERT INTO runtime_database_identity VALUES(1, lower(hex(randomblob(16))));

CREATE TABLE cafleet_database (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    cafleet_version TEXT NOT NULL CHECK(length(trim(cafleet_version)) > 0)
);

CREATE TABLE member_runs (
    run_id INTEGER PRIMARY KEY AUTOINCREMENT,
    member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    fleet_id INTEGER NOT NULL REFERENCES fleets(fleet_id) ON DELETE RESTRICT,
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    backend TEXT NOT NULL CHECK(backend IN ('claude','codex','opencode','hermes','pi')),
    database_identity_json TEXT NOT NULL CHECK(json_valid(database_identity_json) AND json_type(database_identity_json) = 'object'),
    launch_nonce TEXT NOT NULL CHECK(length(launch_nonce) = 64 AND launch_nonce NOT GLOB '*[^0-9a-f]*'),
    runtime_path TEXT NOT NULL CHECK(length(trim(runtime_path)) > 0),
    requested_mode TEXT,
    resolved_mode TEXT NOT NULL CHECK(length(trim(resolved_mode)) > 0),
    mode_source TEXT NOT NULL CHECK(mode_source IN ('default','explicit')),
    permission_argv_json TEXT NOT NULL CHECK(json_valid(permission_argv_json) AND json_type(permission_argv_json) = 'array'),
    coverage TEXT NOT NULL CHECK(coverage IN ('tested','unknown')),
    status TEXT NOT NULL CHECK(status IN ('starting','prepared','running','degraded','recovering','exited','failed','lost')),
    native_version TEXT,
    owner_epoch INTEGER NOT NULL CHECK(owner_epoch > 0),
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('creator','runner','reconciler')),
    owner_pid INTEGER NOT NULL CHECK(owner_pid > 0),
    owner_start_identity TEXT NOT NULL CHECK(length(trim(owner_start_identity)) > 0),
    cleanup_state TEXT NOT NULL CHECK(cleanup_state IN ('complete','incomplete')),
    runtime_dir_identity_json TEXT CHECK(runtime_dir_identity_json IS NULL OR (json_valid(runtime_dir_identity_json) AND json_type(runtime_dir_identity_json) = 'object')),
    run_lock_identity_json TEXT CHECK(run_lock_identity_json IS NULL OR (json_valid(run_lock_identity_json) AND json_type(run_lock_identity_json) = 'object')),
    runner_pid INTEGER CHECK(runner_pid IS NULL OR runner_pid > 0),
    runner_start_identity TEXT,
    child_pid INTEGER CHECK(child_pid IS NULL OR child_pid > 0),
    child_start_identity TEXT,
    started_at TEXT,
    last_health_at TEXT,
    ended_at TEXT,
    exit_code INTEGER,
    cleanup_diagnostics_json TEXT CHECK(cleanup_diagnostics_json IS NULL OR (json_valid(cleanup_diagnostics_json) AND json_type(cleanup_diagnostics_json) = 'array')),
    recovery_origin_status TEXT CHECK(recovery_origin_status IS NULL OR recovery_origin_status IN ('starting','prepared','running','degraded','recovering','exited','failed','lost')),
    native_session_json TEXT CHECK(native_session_json IS NULL OR (json_valid(native_session_json) AND json_type(native_session_json) = 'object')),
    CHECK((mode_source = 'default' AND requested_mode IS NULL) OR (mode_source = 'explicit' AND requested_mode IS NOT NULL))
);
CREATE UNIQUE INDEX idx_member_runs_active ON member_runs(member_id) WHERE status NOT IN ('exited','failed','lost');

CREATE TABLE member_interactions (
    interaction_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    episode_seq INTEGER NOT NULL CHECK(episode_seq > 0),
    classification TEXT NOT NULL CHECK(classification IN ('permission_request','question','unknown_interaction')),
    evidence_digest TEXT NOT NULL,
    opened_at TEXT NOT NULL,
    closed_at TEXT,
    close_reason TEXT CHECK(close_reason IS NULL OR close_reason IN ('not_visible','superseded','process_exited','monitor_lost')),
    evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json) AND json_type(evidence_json) = 'object'),
    UNIQUE(run_id, episode_seq),
    CHECK((closed_at IS NULL) = (close_reason IS NULL))
);
CREATE UNIQUE INDEX idx_member_interactions_open ON member_interactions(run_id) WHERE closed_at IS NULL;

CREATE TABLE member_runtime_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    event_seq INTEGER NOT NULL CHECK(event_seq > 0),
    interaction_id INTEGER REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_closed','input_delivery_result','input_delivery_stalled','runner_degraded','runner_lost','runner_exited')),
    observed_at TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'),
    notification_state TEXT NOT NULL CHECK(notification_state IN ('inbox','archived')),
    message_id INTEGER UNIQUE REFERENCES messages(message_id) ON DELETE RESTRICT,
    archive_reason TEXT CHECK(archive_reason IS NULL OR archive_reason IN ('member_inactive','fleet_deleted','director_inactive','head_not_ready','duplicate_delivery')),
    UNIQUE(run_id, event_seq),
    CHECK((notification_state = 'inbox' AND message_id IS NOT NULL AND archive_reason IS NULL) OR
          (notification_state = 'archived' AND message_id IS NULL AND archive_reason IS NOT NULL))
);

CREATE TABLE member_input_operations (
    input_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    operation_key TEXT NOT NULL CHECK(length(trim(operation_key)) > 0),
    kind TEXT NOT NULL CHECK(kind IN ('prompt','exec')),
    payload_digest TEXT NOT NULL CHECK(length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
    state TEXT NOT NULL CHECK(state IN ('ready','in_progress','accepted','uncertain','settled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    disposition TEXT CHECK(disposition IS NULL OR disposition IN ('submitted','not_submitted','abandoned_uncertain')),
    evidence_ref TEXT,
    delivery_method TEXT CHECK(delivery_method IS NULL OR delivery_method IN ('profile','fallback')),
    delivery_reason TEXT CHECK(delivery_reason IS NULL OR length(trim(delivery_reason)) > 0),
    deferred_at TEXT,
    defer_reason TEXT CHECK((deferred_at IS NULL AND defer_reason IS NULL)
        OR (deferred_at IS NOT NULL AND defer_reason IS NOT NULL
            AND kind IN ('prompt','exec'))),
    stop_cause TEXT CHECK(stop_cause IS NULL
        OR (stop_cause IN ('interaction_open','interaction_candidate','submit_unconfirmed',
                          'deadline_expired','short_write','write_failed','runner_stopped')
            AND state IN ('uncertain','settled'))),
    UNIQUE(run_id, operation_key),
    CHECK((state = 'settled' AND disposition IS NOT NULL) OR (state != 'settled' AND disposition IS NULL))
);

CREATE TABLE member_runner_diagnostics (
    diagnostic_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    source TEXT NOT NULL CHECK(source IN ('event_writer','observation','deferred_input','queue_housekeeping','preview_sweep','input_result','permission_answer','shutdown')),
    message TEXT NOT NULL CHECK(length(trim(message)) > 0 AND length(CAST(message AS BLOB)) <= 512),
    first_at TEXT NOT NULL,
    last_at TEXT NOT NULL,
    count INTEGER NOT NULL CHECK(count > 0),
    UNIQUE(run_id, source, message)
);
CREATE INDEX idx_member_runner_diagnostics_recent ON member_runner_diagnostics(run_id, last_at);

CREATE TABLE refinery_schema_history (
    version INT4 PRIMARY KEY,
    name VARCHAR(255),
    applied_on VARCHAR(255),
    checksum VARCHAR(255)
);
