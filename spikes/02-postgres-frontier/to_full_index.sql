\timing on
DROP INDEX domains_frontier; DROP INDEX domains_frontier_rk;
CREATE INDEX domains_status_disc ON domains (status, discovered_at);
VACUUM ANALYZE domains;
SELECT indexrelname, pg_size_pretty(pg_relation_size(indexrelid)) FROM pg_stat_user_indexes WHERE relname='domains';
