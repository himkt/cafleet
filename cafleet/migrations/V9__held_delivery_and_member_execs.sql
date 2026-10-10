ALTER TABLE messages ADD COLUMN notified_at TEXT;
UPDATE messages SET notified_at = created_at;

ALTER TABLE member_placements ADD COLUMN keystroke_at TEXT;
ALTER TABLE member_placements ADD COLUMN forced_at TEXT;
ALTER TABLE member_placements ADD COLUMN silence_notice_at TEXT;

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
