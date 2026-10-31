ALTER TABLE member_input_operations ADD COLUMN uncertain_cause TEXT
  CHECK(uncertain_cause IS NULL
     OR (uncertain_cause IN ('manual_input','manual_accounting_failed','interaction_candidate',
                             'interaction_open','observer_unavailable','short_write','write_failed',
                             'runner_stopped')
         AND state IN ('uncertain','settled')));
