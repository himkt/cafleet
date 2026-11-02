-- PRAGMA foreign_keys cannot change inside refinery's migration transaction, and
-- renaming member_runs would rewrite the child tables' REFERENCES clauses. The
-- rows are therefore parked in scratch tables, member_runs is dropped and
-- recreated under its own name, and the rows are restored. With deferred
-- foreign keys, the restored parent rows settle the child references the drop
-- counted as violations before the transaction commits.
PRAGMA defer_foreign_keys = ON;

CREATE TABLE member_runs_v1 AS SELECT * FROM member_runs;
CREATE TABLE member_runs_v1_sequence AS
    SELECT seq FROM sqlite_sequence WHERE name = 'member_runs';

DROP TABLE member_runs;

CREATE TABLE member_runs (
    run_id INTEGER PRIMARY KEY AUTOINCREMENT,
    member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    fleet_id INTEGER NOT NULL REFERENCES fleets(fleet_id) ON DELETE RESTRICT,
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    backend TEXT NOT NULL CHECK(backend IN ('claude','codex','opencode','hermes')),
    database_identity_json TEXT NOT NULL CHECK(json_valid(database_identity_json) AND json_type(database_identity_json) = 'object'),
    launch_nonce TEXT NOT NULL CHECK(length(launch_nonce) = 64 AND launch_nonce NOT GLOB '*[^0-9a-f]*'),
    runtime_path TEXT NOT NULL CHECK(length(trim(runtime_path)) > 0),
    requested_mode TEXT,
    resolved_mode TEXT NOT NULL CHECK(length(trim(resolved_mode)) > 0),
    mode_source TEXT NOT NULL CHECK(mode_source IN ('default','explicit')),
    permission_argv_json TEXT NOT NULL CHECK(json_valid(permission_argv_json) AND json_type(permission_argv_json) = 'array'),
    event_evidence TEXT NOT NULL CHECK(event_evidence IN ('metadata','redacted')),
    coverage TEXT NOT NULL CHECK(coverage IN ('tested','unknown')),
    status TEXT NOT NULL CHECK(status IN ('starting','prepared','running','degraded','stopping','recovering','exited','failed','lost')),
    created_at TEXT NOT NULL,
    native_version TEXT,
    effective_policy_json TEXT CHECK(effective_policy_json IS NULL OR (json_valid(effective_policy_json) AND json_type(effective_policy_json) = 'object')),
    owner_epoch INTEGER NOT NULL CHECK(owner_epoch > 0),
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('creator','runner','reconciler')),
    owner_pid INTEGER NOT NULL CHECK(owner_pid > 0),
    owner_start_identity TEXT NOT NULL CHECK(length(trim(owner_start_identity)) > 0),
    reachability TEXT NOT NULL CHECK(reachability IN ('reachable','suspect')),
    cleanup_state TEXT NOT NULL CHECK(cleanup_state IN ('complete','incomplete')),
    event_seq_reserved_through INTEGER NOT NULL CHECK(event_seq_reserved_through >= 0),
    observation_gap_count INTEGER NOT NULL CHECK(observation_gap_count >= 0),
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
    exit_signal INTEGER,
    cleanup_diagnostics_json TEXT CHECK(cleanup_diagnostics_json IS NULL OR (json_valid(cleanup_diagnostics_json) AND json_type(cleanup_diagnostics_json) = 'array')),
    recovery_origin_status TEXT CHECK(recovery_origin_status IS NULL OR recovery_origin_status IN ('starting','prepared','running','degraded','stopping','recovering','exited','failed','lost')),
    native_session_json TEXT CHECK(native_session_json IS NULL OR (json_valid(native_session_json) AND json_type(native_session_json) = 'object')),
    CHECK((mode_source = 'default' AND requested_mode IS NULL) OR (mode_source = 'explicit' AND requested_mode IS NOT NULL))
);

INSERT INTO member_runs (
    run_id, member_id, fleet_id, director_member_id, backend, database_identity_json,
    launch_nonce, runtime_path, requested_mode, resolved_mode, mode_source,
    permission_argv_json, event_evidence, coverage, status, created_at, native_version,
    effective_policy_json, owner_epoch, owner_kind, owner_pid, owner_start_identity,
    reachability, cleanup_state, event_seq_reserved_through, observation_gap_count,
    runtime_dir_identity_json, run_lock_identity_json, runner_pid, runner_start_identity,
    child_pid, child_start_identity, started_at, last_health_at, ended_at, exit_code,
    exit_signal, cleanup_diagnostics_json, recovery_origin_status
)
SELECT
    run_id, member_id, fleet_id, director_member_id, backend, database_identity_json,
    launch_nonce, runtime_path, requested_mode, resolved_mode, mode_source,
    permission_argv_json, event_evidence, coverage, status, created_at, native_version,
    effective_policy_json, owner_epoch, owner_kind, owner_pid, owner_start_identity,
    reachability, cleanup_state, event_seq_reserved_through, observation_gap_count,
    runtime_dir_identity_json, run_lock_identity_json, runner_pid, runner_start_identity,
    child_pid, child_start_identity, started_at, last_health_at, ended_at, exit_code,
    exit_signal, cleanup_diagnostics_json, recovery_origin_status
FROM member_runs_v1;

-- AUTOINCREMENT never reuses an id the V1 table issued, including deleted ones.
DELETE FROM sqlite_sequence WHERE name = 'member_runs';
INSERT INTO sqlite_sequence (name, seq)
    SELECT 'member_runs', seq FROM member_runs_v1_sequence;

DROP TABLE member_runs_v1;
DROP TABLE member_runs_v1_sequence;

CREATE UNIQUE INDEX idx_member_runs_active ON member_runs(member_id) WHERE status NOT IN ('exited','failed','lost');
