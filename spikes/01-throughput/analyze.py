#!/usr/bin/env python3
"""Summarize an explore-spike run: python3 analyze.py out A"""
import json, re, sys, statistics as st
from collections import Counter

out, run = sys.argv[1], sys.argv[2]
S = json.load(open(f"{out}/{run}_summary.json"))
V = [json.loads(l) for l in open(f"{out}/{run}_visits.jsonl")]
dur = S["duration_s"]


def pct(xs, p):
    xs = sorted(xs)
    if not xs:
        return float("nan")
    k = (len(xs) - 1) * p / 100
    f = int(k)
    c = min(f + 1, len(xs) - 1)
    return xs[f] + (xs[c] - xs[f]) * (k - f)


def med(xs):
    return pct(xs, 50) if xs else float("nan")


def mean(xs):
    return sum(xs) / len(xs) if xs else float("nan")


comp = [v for v in V if v["outcome"] is None]
fail = [v for v in V if v["outcome"] is not None]
pages_ok = sum(v["pages_ok"] for v in V)
reqs = sum(v["requests"] for v in V)
print(f"== Run {run}: concurrency={S['concurrency']} duration={dur:.0f}s resumed={S['resumed']}")
print(f"attempted={S['attempted']} finished={S['finished']} completed={S['completed']} "
      f"aborted_in_flight={S['in_flight_aborted']} failed={len(fail)}")
print(f"sites/s (completed)={len(comp)/dur:.3f}  visits finished/s={len(V)/dur:.3f}  "
      f"pages/s={pages_ok/dur:.3f}  http requests/s={reqs/dur:.3f}")

# steady state: exclude first 60s ramp
ss = [v for v in comp if v["t_end"] >= 60]
print(f"steady-state (t>=60s) sites/s={len(ss)/(dur-60):.3f}")

d = [v["dur_ms"] / 1000 for v in comp]
df = [v["dur_ms"] / 1000 for v in fail]
print(f"visit duration completed: median={med(d):.1f}s p95={pct(d,95):.1f}s mean={mean(d):.1f}s | "
      f"failed: median={med(df):.1f}s p95={pct(df,95):.1f}s")
ppv = Counter(v["pages_ok"] for v in comp)
print("pages-per-visit (completed):", dict(sorted(ppv.items())), f"mean={mean([v['pages_ok'] for v in comp]):.2f}")
print("homepage-only due to crawl-delay>10s:", sum(1 for v in V if v["homepage_only_crawl_delay"]),
      " crawl-delay present:", sum(1 for v in V if v.get("crawl_delay") is not None))

pev = [e for v in V for e in v["events"] if e["kind"] == "page" and e["status"] and 200 <= e["status"] < 300 and e["decoded"] > 0]
wire = [e["wire"] for e in pev]
dec = [e["decoded"] for e in pev]
print(f"per HTML page: wire median={med(wire)/1024:.1f}KB mean={mean(wire)/1024:.1f}KB | "
      f"decoded median={med(dec)/1024:.1f}KB mean={mean(dec)/1024:.1f}KB  (n={len(pev)})")
gz_share = mean([1 if e["wire"] < e["decoded"] else 0 for e in pev])
print(f"share of pages served compressed: {gz_share:.2f}")
vw = [v["wire_bytes"] for v in comp]
vd = [v["decoded_bytes"] for v in comp]
print(f"per completed visit: wire median={med(vw)/1024:.1f}KB mean={mean(vw)/1024:.1f}KB | "
      f"decoded median={med(vd)/1024:.1f}KB mean={mean(vd)/1024:.1f}KB")
all_wire = sum(v["wire_bytes"] for v in V)
all_dec = sum(v["decoded_bytes"] for v in V)
print(f"total body bytes (all visits): wire={all_wire/1e6:.1f}MB decoded={all_dec/1e6:.1f}MB "
      f"-> wire per completed visit incl. failures={all_wire/max(1,len(comp))/1024:.1f}KB")
print(f"requests per finished visit: {reqs/len(V):.2f} (robots {mean([v['robots_requests'] for v in V]):.2f})")

ext = [v["ext_domains"] for v in comp]
print(f"external registered domains per completed visit: median={med(ext):.0f} mean={mean(ext):.1f} "
      f"p95={pct(ext,95):.0f}  weight-sum mean={mean([v['ext_weight_sum'] for v in comp]):.1f}")
disc = S["seen_end"] - S["seen_at_start"]
print(f"new domains discovered={disc}  discovery ratio={disc/max(1,len(comp)):.2f} per completed visit")
print("discovery trend (per 60s bucket by visit end): bucket completed new ratio")
for b in range(0, int(dur) + 1, 60):
    bv = [v for v in V if b <= v["t_end"] < b + 60]
    c = sum(1 for v in bv if v["outcome"] is None)
    n = sum(v["new_domains"] for v in bv)
    if bv:
        print(f"   {b:4d}-{b+60:<4d} {c:5d} {n:6d} {n/max(1,c):6.2f}")

print("visit outcome breakdown (finished visits):")
oc = Counter(v["outcome"] or "completed" for v in V)
for k, n in oc.most_common():
    print(f"   {k:24s} {n:6d} {100*n/len(V):5.1f}%")
print("internal-page failures (completed visits, pages 2..5):")
ic = Counter(p["fail"] for v in comp for p in v["pages"][1:] if not p["ok"])
for k, n in ic.most_common():
    print(f"   {k:24s} {n:6d}")
print("request-level failures / statuses (all requests):")
rc = Counter()
for v in V:
    for e in v["events"]:
        if e["fail"]:
            rc[e["kind"] + ":" + e["fail"]] += 1
        else:
            s = e["status"]
            rc[e["kind"] + ":" + ("2xx" if s < 300 else "3xx" if s < 400 else str(s) if s in (429, 503) else "4xx" if s < 500 else "5xx")] += 1
for k, n in sorted(rc.items()):
    print(f"   {k:24s} {n:6d}")
print("body-cap (2MB) hits:", sum(1 for v in V for e in v["events"] if e["capped"]))
print("visits stopped on 429/503:", sum(1 for v in V if v["stopped_throttled"]),
      " visits failed with 429/503:", sum(1 for v in V if v["outcome"] in ("http429", "http503")))
print("scheme http fallback used:", sum(1 for v in V if v.get("scheme") == "http"))

pc = [p["parse_cpu_us"] for v in V for p in v["pages"] if p["ok"]]
cc = [p["parse_cpu_us"] + p["classify_cpu_us"] for v in V for p in v["pages"] if p["ok"]]
print(f"parse+extract CPU/page: median={med(pc)/1000:.2f}ms p95={pct(pc,95)/1000:.2f}ms mean={mean(pc)/1000:.2f}ms; "
      f"incl. normalize/classify: median={med(cc)/1000:.2f}ms p95={pct(cc,95)/1000:.2f}ms mean={mean(cc)/1000:.2f}ms")
print(f"total parse CPU = {sum(cc)/1e6:.1f}s over {dur:.0f}s wall ({100*sum(cc)/1e6/dur:.1f}% of one core)")

try:
    t = open(f"{out}/{run}_stderr.txt").read()
    m = re.search(r"(\d+)\s+maximum resident set size", t)
    pf = re.search(r"(\d+)\s+peak memory footprint", t)
    ut = re.search(r"([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys", t)
    if m:
        print(f"peak RSS (/usr/bin/time -l) = {int(m.group(1))/1048576:.1f}MB"
              + (f", peak footprint={int(pf.group(1))/1048576:.1f}MB" if pf else ""))
    if ut:
        print(f"process CPU: user={ut.group(2)}s sys={ut.group(3)}s over real={ut.group(1)}s")
except FileNotFoundError:
    pass
print(f"end state: frontier={S['frontier_end']} seen={S['seen_end']} avg domain len={S['avg_domain_len']:.1f}B")
print(f"mem/entry: seen HashSet<String> {S['seen_hashset_bytes_per_entry_rounded16']:.1f}B, "
      f"frontier VecDeque<String> {S['frontier_vecdeque_bytes_per_entry_rounded16']:.1f}B, "
      f"HashSet<u64> {S['seen_u64hash_bytes_per_entry']:.1f}B  (heap live at end={S['heap_live_bytes_end']/1048576:.1f}MB)")
