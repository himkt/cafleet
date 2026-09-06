CREATE UNIQUE INDEX idx_members_one_active_monitor_per_fleet
ON members(fleet_id)
WHERE status = 'active'
  AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor';
