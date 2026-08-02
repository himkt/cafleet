-- Drop the root Director from the monitor watched set: ordinary pane-bound
-- members are the sole enrollment class (SPEC.md §6.2). Soft-deleted fleets
-- already had their monitor_config rows cascaded away by delete_fleet.

DELETE FROM monitor_config
WHERE member_id IN (
    SELECT director_member_id FROM fleets WHERE director_member_id IS NOT NULL
);
