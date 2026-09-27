\timing on
DROP INDEX domains_status_disc;
CREATE INDEX domains_frontier ON domains (discovered_at, id) WHERE status = 0;
CREATE INDEX domains_frontier_rk ON domains ((id >> 13), rk) WHERE status = 0;
VACUUM ANALYZE domains;
