-- The monitoring-member kind marker no longer has a reader; strip it so no
-- inert marker survives in existing member cards. `cafleet.kind` was the sole
-- key under `$.cafleet`, so the whole object goes.

UPDATE members
SET member_card_json = json_remove(member_card_json, '$.cafleet')
WHERE json_extract(member_card_json, '$.cafleet.kind') = 'monitoring-member';
