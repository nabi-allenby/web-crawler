\pset pager off
BEGIN;
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
WITH win AS MATERIALIZED (
  SELECT id FROM domains WHERE status = 0 ORDER BY discovered_at, id LIMIT 10000
), pick AS (SELECT id FROM win ORDER BY random() LIMIT 32
), locked AS (
  SELECT d.id FROM domains d WHERE d.id IN (SELECT id FROM pick) AND d.status = 0
  FOR UPDATE OF d SKIP LOCKED)
UPDATE domains d SET status = 1, claimed_at = now() FROM locked WHERE d.id = locked.id RETURNING d.id, d.name;
ROLLBACK;
