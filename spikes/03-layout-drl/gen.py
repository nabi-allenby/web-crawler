"""Synthetic web site graph generator.

Directed edges A->B with:
  - out-degree ~ lognormal (median 8, mean ~13, p95 ~40)
  - targets chosen by per-node fitness (Pareto) -> heavy-tailed in-degree (hubs)
  - planted communities (Pareto-ish sizes), P_INTRA of edges stay inside the community
  - weight 1..5 skewed toward 1
Output: undirected, deduplicated, weight-summed (capped at 5) edge list in .npz.
Usage: python gen.py N K out.npz
"""
import sys, time, json, resource
import numpy as np

N = int(sys.argv[1]); K = int(sys.argv[2]); out = sys.argv[3]
P_INTRA = 0.8
SEED = 42
rng = np.random.default_rng(SEED)
t0 = time.time()

# community sizes: Pareto, normalized to N, min size 20
raw = rng.pareto(1.2, K) + 1.0
sizes = np.maximum(20, np.floor(raw / raw.sum() * N)).astype(np.int64)
while sizes.sum() > N:
    sizes[np.argmax(sizes)] -= sizes.sum() - N
sizes[np.argmax(sizes)] += N - sizes.sum()
comm = np.repeat(np.arange(K), sizes)            # node i -> community (nodes sorted by comm)
starts = np.concatenate([[0], np.cumsum(sizes)[:-1]])

# out-degree: lognormal median 8, sigma ~0.985 -> mean ~13, p95 ~40
outdeg = np.maximum(1, np.round(rng.lognormal(np.log(8), 0.985, N))).astype(np.int64)
outdeg = np.minimum(outdeg, 2000)
# fitness -> in-degree heavy tail
fit = rng.pareto(1.5, N) + 1.0

M = int(outdeg.sum())
src = np.repeat(np.arange(N), outdeg)
intra = rng.random(M) < P_INTRA
dst = np.empty(M, dtype=np.int64)

# intra-community: inverse-CDF sampling within community's cumulative fitness block
cf = np.cumsum(fit)
cf_before = np.concatenate([[0.0], cf])  # cf_before[i] = sum fit[:i]
c = comm[src[intra]]
lo = cf_before[starts[c]]
hi = cf_before[starts[c] + sizes[c]]
u = lo + rng.random(c.size) * (hi - lo)
dst[intra] = np.minimum(np.searchsorted(cf, u, side="right"), starts[c] + sizes[c] - 1)
# inter: global preferential choice
ninter = int((~intra).sum())
u = rng.random(ninter) * cf[-1]
dst[~intra] = np.minimum(np.searchsorted(cf, u, side="right"), N - 1)

w = rng.choice(np.arange(1, 6), size=M, p=[0.6, 0.2, 0.1, 0.06, 0.04]).astype(np.int64)
directed_stats = dict(
    directed_edges_raw=M,
    outdeg_median=float(np.median(outdeg)), outdeg_mean=float(outdeg.mean()),
    outdeg_p95=float(np.percentile(outdeg, 95)),
)
# drop self loops, make undirected, dedup, sum weights (cap 5)
keep = src != dst
a = np.minimum(src[keep], dst[keep]); b = np.maximum(src[keep], dst[keep]); w = w[keep]
key = a * N + b
del src, dst, a, b, intra
ukey, inv = np.unique(key, return_inverse=True)
wsum = np.minimum(np.bincount(inv, weights=w), 5).astype(np.float32)
ea = (ukey // N).astype(np.int32); eb = (ukey % N).astype(np.int32)
# shuffle node ids so ids are not sorted by community
perm = rng.permutation(N).astype(np.int32)
ea = perm[ea]; eb = perm[eb]
planted = np.empty(N, dtype=np.int32); planted[perm] = comm

deg = np.bincount(np.concatenate([ea, eb]), minlength=N)
gen_s = time.time() - t0
np.savez(out, a=ea, b=eb, w=wsum, planted=planted)
stats = dict(N=N, K=K, undirected_edges=int(ukey.size), gen_s=round(gen_s, 2),
             undeg_max=int(deg.max()), undeg_p99=float(np.percentile(deg, 99)),
             isolated=int((deg == 0).sum()),
             comm_size_max=int(sizes.max()), comm_size_min=int(sizes.min()),
             peak_rss_mb=round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 2**20, 1),
             **directed_stats)
print(json.dumps(stats))
