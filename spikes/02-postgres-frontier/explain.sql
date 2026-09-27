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
BEGIN;
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
WITH l AS (SELECT id FROM domains WHERE status = 0 ORDER BY (id >> 13), rk LIMIT 32 FOR UPDATE SKIP LOCKED)
UPDATE domains d SET status = 1, claimed_at = now() FROM l WHERE d.id = l.id RETURNING d.id, d.name;
ROLLBACK;
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF) SELECT id FROM domains WHERE status = 0 ORDER BY discovered_at, id LIMIT 10000;
BEGIN;
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
UPDATE domains SET status = 1, claimed_at = now()
WHERE id = ANY(ARRAY(SELECT id FROM domains WHERE status=0 ORDER BY discovered_at, id LIMIT 32 OFFSET 500)) AND status = 0 RETURNING id, name;
ROLLBACK;
