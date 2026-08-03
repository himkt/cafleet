-- Replace the per-member monitor schedule with the fleet-level wake cadence:
-- the tick is unconditional and periodic, so the only durable schedule state
-- is one timestamp per fleet.

DROP TABLE monitor_config;

ALTER TABLE monitor_runtime ADD COLUMN last_wake_at TEXT;
