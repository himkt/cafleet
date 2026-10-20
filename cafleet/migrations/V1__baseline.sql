-- The full head schema (SPEC.md §8).
-- members is created first: members.fleet_id forward-references the
-- still-uncreated fleets table, which SQLite tolerates because FK
-- enforcement is per-connection (PRAGMA foreign_keys=ON), not DDL-time.

CREATE TABLE members (
    member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    fleet_id INTEGER NOT NULL REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_at TEXT NOT NULL,
    deregistered_at TEXT,
    member_card_json TEXT NOT NULL
);

CREATE INDEX idx_members_fleet_status ON members (fleet_id, status);

CREATE UNIQUE INDEX idx_members_one_active_monitor_per_fleet
ON members(fleet_id)
WHERE status = 'active'
  AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor';

CREATE TABLE fleets (
    fleet_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    created_at TEXT NOT NULL,
    deleted_at TEXT,
    director_member_id INTEGER REFERENCES members (member_id) ON DELETE RESTRICT
);

CREATE TABLE asset_installs (
    coding_agent TEXT NOT NULL,
    path TEXT NOT NULL,
    cafleet_version TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    PRIMARY KEY (coding_agent, path)
);

CREATE TABLE member_placements (
    member_id INTEGER NOT NULL PRIMARY KEY REFERENCES members (member_id) ON DELETE CASCADE,
    mux_session TEXT NOT NULL,
    mux_window_id TEXT NOT NULL,
    mux_pane_id TEXT,
    backend TEXT NOT NULL DEFAULT 'tmux',
    coding_agent TEXT NOT NULL,
    created_at TEXT NOT NULL,
    keystroke_at TEXT,
    forced_at TEXT,
    silence_notice_at TEXT
);

CREATE TABLE monitor_runtime (
    fleet_id INTEGER NOT NULL PRIMARY KEY REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    pid INTEGER,
    started_at TEXT,
    last_tick_at TEXT,
    tick_seconds INTEGER NOT NULL DEFAULT 5,
    last_wake_at TEXT,
    wake_interval_seconds INTEGER,
    wake_requested_at TEXT
);

CREATE TABLE messages (
    message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    owner_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    from_member_id INTEGER NOT NULL,
    to_member_id INTEGER,
    type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    status_state TEXT NOT NULL,
    status_timestamp TEXT NOT NULL,
    origin_message_id INTEGER,
    text TEXT NOT NULL,
    notified_at TEXT
);

CREATE INDEX idx_messages_owner_member_status_ts ON messages (owner_member_id, status_timestamp);
CREATE INDEX idx_messages_from_member_status_ts ON messages (from_member_id, status_timestamp);

CREATE TABLE member_execs (
    exec_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    command TEXT NOT NULL,
    created_at TEXT NOT NULL,
    dispatched_at TEXT,
    started_at TEXT,
    pid INTEGER,
    finished_at TEXT,
    exit_code INTEGER,
    resumed_at TEXT
);
