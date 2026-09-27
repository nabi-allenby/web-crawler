\timing on
\pset pager off
UPDATE domains SET status = 1, claimed_at = now() - interval '10 minutes'
 WHERE id IN (SELECT id FROM domains WHERE status = 0 ORDER BY (id >> 13), rk LIMIT 5000);
CHECKPOINT;
SELECT status, count(*) FROM domains GROUP BY status ORDER BY 1;
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF) UPDATE domains SET status = 0, claimed_at = NULL
 WHERE status = 1 AND claimed_at < now() - interval '5 minutes';
BEGIN; UPDATE domains SET status = 1, claimed_at = now() - interval '10 minutes'
 WHERE id IN (SELECT id FROM domains WHERE status = 0 ORDER BY (id >> 13), rk LIMIT 5000); COMMIT;
CREATE INDEX domains_visiting ON domains (claimed_at) WHERE status = 1;
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF) UPDATE domains SET status = 0, claimed_at = NULL
 WHERE status = 1 AND claimed_at < now() - interval '5 minutes';
SELECT pg_size_pretty(pg_relation_size('domains_visiting')) visiting_idx;
SELECT count(*) AS visiting FROM domains WHERE status = 1;
