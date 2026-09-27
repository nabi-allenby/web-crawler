"""Frontier-claim spike driver. Usage: bench.py <db> <command> [...]"""
import hashlib, json, random, sys, threading, time, uuid
import psycopg

DSN = "host=localhost port=55432 user=postgres password=spike dbname={}"

SQL_A = """
WITH win AS MATERIALIZED (
  SELECT id FROM domains WHERE status = 0
  ORDER BY discovered_at, id LIMIT 10000
), pick AS (
  SELECT id FROM win ORDER BY random() LIMIT %(k)s
), locked AS (
  SELECT d.id FROM domains d
  WHERE d.id IN (SELECT id FROM pick) AND d.status = 0
  FOR UPDATE OF d SKIP LOCKED
)
UPDATE domains d SET status = 1, claimed_at = now()
FROM locked WHERE d.id = locked.id
RETURNING d.id, d.name
"""

SQL_C_WINDOW = """SELECT id FROM domains WHERE status = 0
ORDER BY discovered_at, id LIMIT 10000"""
SQL_C_CLAIM = """UPDATE domains SET status = 1, claimed_at = now()
WHERE id = ANY(%(ids)s) AND status = 0 RETURNING id, name"""

SQL_D = """
WITH l AS (
  SELECT id FROM domains WHERE status = 0
  ORDER BY (id >> 13), rk LIMIT %(k)s
  FOR UPDATE SKIP LOCKED
)
UPDATE domains d SET status = 1, claimed_at = now()
FROM l WHERE d.id = l.id
RETURNING d.id, d.name
"""

SQL_VISITED = """UPDATE domains SET status = 2, visited_at = now(), http_status = 200,
 title = 'Some page title for a visited site', lang = 'en', ip = '10.1.2.3',
 server = 'nginx', content_length = 12345 WHERE id = ANY(%(ids)s)"""

SQL_INSERT = """INSERT INTO domains (name) SELECT unnest(%(names)s::text[])
ON CONFLICT (name) DO NOTHING"""
SQL_BG_UPDATE = """UPDATE domains SET status = 3, visited_at = now(), http_status = 500,
 error = 'timeout' WHERE id = %(id)s"""


def conn(db):
    return psycopg.connect(DSN.format(db), autocommit=True)


def pct(xs, p):
    if not xs:
        return None
    s = sorted(xs)
    return s[min(len(s) - 1, int(round(p / 100 * (len(s) - 1))))]


def summary(lat_ms):
    return {"n": len(lat_ms), "p50": round(pct(lat_ms, 50), 2), "p95": round(pct(lat_ms, 95), 2),
            "p99": round(pct(lat_ms, 99), 2), "max": round(max(lat_ms), 2)}


class Claimer:
    def __init__(self, c, approach, k):
        self.c, self.approach, self.k = c, approach, k
        self.window = []
        self.refreshes = 0

    def claim(self):
        if self.approach == "A":
            return self.c.execute(SQL_A, {"k": self.k}).fetchall()
        if self.approach == "D":
            return self.c.execute(SQL_D, {"k": self.k}).fetchall()
        # C: cached window, sample locally, claim by PK
        if len(self.window) < self.k:
            self.window = [r[0] for r in self.c.execute(SQL_C_WINDOW).fetchall()]
            random.shuffle(self.window)
            self.refreshes += 1
        ids, self.window = self.window[:self.k], self.window[self.k:]
        return self.c.execute(SQL_C_CLAIM, {"ids": ids}).fetchall()


def max_id(db):
    with conn(db) as c:
        return c.execute("select max(id) from domains").fetchone()[0]


def name_for(i):
    return hashlib.md5(str(i).encode()).hexdigest()[:8] + f"-{i}.example"


def bg_load(db, rate, stop, stats, seed):
    """rate visits/s; each visit = INSERT 13 names (~90% new) + 1 status UPDATE."""
    rnd = random.Random(seed)
    mid = max_id(db)
    c = conn(db)
    t_next = time.perf_counter()
    while not stop.is_set():
        names = [name_for(rnd.randint(1, mid)) if rnd.random() < 0.1 else
                 "n" + uuid.uuid4().hex[:14] + ".example" for _ in range(13)]
        t0 = time.perf_counter()
        c.execute(SQL_INSERT, {"names": names})
        t1 = time.perf_counter()
        c.execute(SQL_BG_UPDATE, {"id": rnd.randint(1, mid)})
        stats["ins"].append((t1 - t0) * 1000)
        t_next += 1.0 / rate
        d = t_next - time.perf_counter()
        if d > 0:
            stop.wait(d)
    c.close()


def start_bg(db, visits_per_s):
    stop = threading.Event()
    stats = {"ins": []}
    threads = []
    if visits_per_s > 0:
        n = max(1, int(visits_per_s // 10))
        for i in range(n):
            t = threading.Thread(target=bg_load, args=(db, visits_per_s / n, stop, stats, i), daemon=True)
            t.start()
            threads.append(t)
    return stop, threads, stats


def run_claims(db, approach, k, n_claims, max_s, mark_visited=True):
    c = conn(db)
    cl = Claimer(c, approach, k)
    lat, got = [], 0
    for _ in range(5):  # warm-up
        rows = cl.claim()
        if mark_visited and rows:
            c.execute(SQL_VISITED, {"ids": [r[0] for r in rows]})
    t_end = time.perf_counter() + max_s
    while len(lat) < n_claims and time.perf_counter() < t_end:
        t0 = time.perf_counter()
        rows = cl.claim()
        lat.append((time.perf_counter() - t0) * 1000)
        got += len(rows)
        if mark_visited and rows:
            c.execute(SQL_VISITED, {"ids": [r[0] for r in rows]})
    c.close()
    s = summary(lat)
    s["yield"] = round(got / (len(lat) * k), 3)
    if approach == "C":
        s["window_refreshes"] = cl.refreshes
    return s


def cmd_matrix(db, approaches, ks, loads, n_claims, out):
    res = []
    for load in loads:
        stop, threads, stats = start_bg(db, load)
        time.sleep(2 if load else 0)
        for a in approaches:
            for k in ks:
                s = run_claims(db, a, k, n_claims, 30)
                s.update(db=db, approach=a, k=k, load_visits_per_s=load)
                print(json.dumps(s), flush=True)
                res.append(s)
        stop.set()
        for t in threads:
            t.join()
        if stats["ins"]:
            print(json.dumps({"bg_load": load, "bg_insert13": summary(stats["ins"])}), flush=True)
    with open(out, "a") as f:
        for r in res:
            f.write(json.dumps(r) + "\n")


def cmd_concurrent(db, approach, k, secs):
    """2 claimers, no mark-visited, check for duplicates."""
    claimed = [[], []]
    lat = [[], []]
    stop = time.perf_counter() + secs

    def worker(i):
        c = conn(db)
        cl = Claimer(c, approach, k)
        while time.perf_counter() < stop:
            t0 = time.perf_counter()
            rows = cl.claim()
            lat[i].append((time.perf_counter() - t0) * 1000)
            claimed[i].extend(r[0] for r in rows)
        c.close()

    ts = [threading.Thread(target=worker, args=(i,)) for i in range(2)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    allids = claimed[0] + claimed[1]
    with conn(db) as c:
        n_visiting = c.execute("select count(*) from domains where id = any(%s) and status = 1",
                               (allids,)).fetchone()[0]
    r = {"approach": approach, "k": k, "claims": [len(lat[0]), len(lat[1])],
         "rows_claimed": len(allids), "unique": len(set(allids)),
         "duplicates": len(allids) - len(set(allids)),
         "overlap_between_claimers": len(set(claimed[0]) & set(claimed[1])),
         "db_rows_visiting": n_visiting,
         "yield": round(len(allids) / ((len(lat[0]) + len(lat[1])) * k), 3),
         "lat_c1": summary(lat[0]), "lat_c2": summary(lat[1])}
    print(json.dumps(r), flush=True)


def cmd_churn(db, approach, k, total_rows, bg):
    """claim->visited cycles; report latency per chunk and table stats."""
    stop, threads, _ = start_bg(db, bg)
    c = conn(db)
    mon = conn(db)
    cl = Claimer(c, approach, k)
    done, lat, t_start = 0, [], time.perf_counter()
    next_report = 0

    def stats():
        return mon.execute("""select n_live_tup, n_dead_tup, n_tup_upd, n_tup_hot_upd, autovacuum_count,
              pg_size_pretty(pg_relation_size('domains')),
              pg_size_pretty(pg_relation_size('domains_frontier')),
              pg_size_pretty(pg_relation_size('domains_frontier_rk'))
              from pg_stat_user_tables where relname='domains'""").fetchone()

    print("start", stats(), flush=True)
    while done < total_rows:
        t0 = time.perf_counter()
        rows = cl.claim()
        lat.append((time.perf_counter() - t0) * 1000)
        if rows:
            c.execute(SQL_VISITED, {"ids": [r[0] for r in rows]})
        done += len(rows)
        if done >= next_report:
            print(json.dumps({"rows_done": done, "elapsed_s": round(time.perf_counter() - t_start, 1),
                              "claim": summary(lat), "stats": list(map(str, stats()))}), flush=True)
            lat = []
            next_report += 100000
    stop.set()
    [t.join() for t in threads]
    print("end", stats(), flush=True)


def cmd_insert(db, conns, secs):
    lat_all = []
    stop = time.perf_counter() + secs

    def worker(seed):
        rnd = random.Random(seed)
        mid = max_id(db)
        c = conn(db)
        lat = []
        while time.perf_counter() < stop:
            names = [name_for(rnd.randint(1, mid)) if rnd.random() < 0.1 else
                     "n" + uuid.uuid4().hex[:14] + ".example" for _ in range(13)]
            t0 = time.perf_counter()
            c.execute(SQL_INSERT, {"names": names})
            lat.append((time.perf_counter() - t0) * 1000)
        lat_all.extend(lat)
        c.close()

    ts = [threading.Thread(target=worker, args=(i + 100,)) for i in range(conns)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    r = {"db": db, "conns": conns, "batches_per_s": round(len(lat_all) / secs, 1),
         "rows_per_s_offered": round(13 * len(lat_all) / secs), "lat": summary(lat_all)}
    print(json.dumps(r), flush=True)


if __name__ == "__main__":
    db, cmd, *a = sys.argv[1:]
    if cmd == "matrix":
        cmd_matrix(db, a[0].split(","), [int(x) for x in a[1].split(",")],
                   [float(x) for x in a[2].split(",")], int(a[3]), a[4])
    elif cmd == "concurrent":
        cmd_concurrent(db, a[0], int(a[1]), float(a[2]))
    elif cmd == "churn":
        cmd_churn(db, a[0], int(a[1]), int(a[2]), float(a[3]))
    elif cmd == "insert":
        cmd_insert(db, int(a[0]), float(a[1]))
