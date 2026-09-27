"""One phase per process so peak RSS is clean.
Usage: python run.py TAG PHASE [opts]
  PHASE: leiden2 | leidenall | drl | hier | hierdrl | pos | plot
Writes results/TAG_PHASE.json and data/TAG_*.npy
"""
import sys, time, json, resource, gzip, os
import numpy as np
import igraph as ig

TAG, PHASE = sys.argv[1], sys.argv[2]
D = os.path.dirname(os.path.abspath(__file__))
data = lambda f: os.path.join(D, "data", f"{TAG}_{f}")
res = {"tag": TAG, "phase": PHASE}


def rss_mb():
    return round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 2**20, 1)  # macOS: bytes


def load_graph():
    t = time.time()
    z = np.load(os.path.join(D, "data", f"{TAG}.npz"))
    a, b, w, planted = z["a"], z["b"], z["w"], z["planted"]
    N = planted.size
    g = ig.Graph(n=N, edges=np.column_stack([a, b]), directed=False)
    g.es["weight"] = w.astype(np.float64)
    res.update(N=N, E=g.ecount(), load_s=round(time.time() - t, 2), rss_after_load_mb=rss_mb())
    return g, planted, a, b, w


def edge_len_ratio(xy, a, b, seed=0):
    """mean edge length / mean random-pair distance (lower = neighbours closer)."""
    rng = np.random.default_rng(seed)
    idx = rng.choice(a.size, min(a.size, 2_000_000), replace=False)
    el = np.linalg.norm(xy[a[idx]] - xy[b[idx]], axis=1).mean()
    p = rng.integers(0, xy.shape[0], (2_000_000, 2))
    rl = np.linalg.norm(xy[p[:, 0]] - xy[p[:, 1]], axis=1).mean()
    return round(float(el / rl), 4)


def vogel(n):
    """sunflower spiral, unit density; index 0 at centre"""
    k = np.arange(n) + 0.5
    r = np.sqrt(k)
    th = k * np.pi * (3 - np.sqrt(5))
    return np.column_stack([r * np.cos(th), r * np.sin(th)])


if PHASE in ("leiden2", "leidenall"):
    g, planted, *_ = load_graph()
    niter = 2 if PHASE == "leiden2" else -1
    t = time.time()
    cl = g.community_leiden(objective_function="modularity", weights="weight", n_iterations=niter)
    res["leiden_s"] = round(time.time() - t, 2)
    memb = np.asarray(cl.membership, dtype=np.int32)
    sizes = np.bincount(memb)
    res.update(n_iter=niter, communities=int(sizes.size), comms_ge_100=int((sizes >= 100).sum()),
               largest=int(sizes.max()),
               modularity=round(g.modularity(cl.membership, weights="weight"), 4),
               nmi_vs_planted=round(ig.compare_communities(planted.tolist(), cl.membership, method="nmi"), 4),
               planted_modularity=round(g.modularity(planted.tolist(), weights="weight"), 4))
    np.save(data(f"memb_{PHASE}.npy"), memb)

elif PHASE in ("drl", "drlfast"):
    g, planted, a, b, w = load_graph()
    t = time.time()
    opts = None if PHASE == "drl" else dict(liquid_iterations=50, expansion_iterations=50, cooldown_iterations=50,
                                             crunch_iterations=25, simmer_iterations=25)
    lay = g.layout_drl(weights="weight", options=opts)
    res["drl_s"] = round(time.time() - t, 2)
    xy = np.asarray(lay.coords, dtype=np.float32)
    np.save(data(f"xy_{PHASE}.npy"), xy)
    res["edge_len_ratio"] = edge_len_ratio(xy, a, b)

elif PHASE in ("hier", "hierdrl"):
    # hier: community graph layout + Vogel spiral by degree inside each community
    # hierdrl: same, but DrL per community (FR for tiny ones)
    z = np.load(os.path.join(D, "data", f"{TAG}.npz"))
    a, b, w = z["a"], z["b"], z["w"]
    N = z["planted"].size
    memb = np.load(data("memb_leiden2.npy"))
    t0 = time.time()
    C = int(memb.max()) + 1
    sizes = np.bincount(memb, minlength=C)
    ca, cb = memb[a], memb[b]
    m = ca != cb
    lo, hi = np.minimum(ca[m], cb[m]).astype(np.int64), np.maximum(ca[m], cb[m]).astype(np.int64)
    key, inv = np.unique(lo * C + hi, return_inverse=True)
    cw = np.bincount(inv, weights=w[m])
    cg = ig.Graph(n=C, edges=np.column_stack([key // C, key % C]), directed=False)
    t1 = time.time()
    # community-level layout
    cw_norm = cw / np.sqrt(sizes[key // C] * sizes[key % C])  # normalise so big comms don't dominate
    if C <= 3000:
        cl = cg.layout_fruchterman_reingold(weights=list(cw_norm), niter=1000, grid="nogrid" if C < 1000 else "grid")
    else:
        cl = cg.layout_drl(weights=list(cw_norm))
    cxy = np.asarray(cl.coords, dtype=np.float64)
    t2 = time.time()
    # size-aware placement: radius ~ sqrt(size), then push overlapping circles apart
    rad = np.sqrt(sizes).astype(np.float64)
    # initial scale so average neighbour spacing ~ typical diameter
    span = np.ptp(cxy, axis=0).max()
    cxy = cxy / span * np.sqrt(sizes.sum()) * 2.0
    big = np.argsort(-sizes)[:2000]
    for _ in range(60):
        P = cxy[big]; R = rad[big]
        d = P[:, None, :] - P[None, :, :]
        dist = np.sqrt((d ** 2).sum(-1)) + 1e-9
        ov = (R[:, None] + R[None, :] + 1.0) - dist
        np.fill_diagonal(ov, 0)
        ov = np.maximum(ov, 0)
        if ov.max() < 0.5:
            break
        push = (d / dist[..., None] * ov[..., None] * 0.5).sum(1)
        cxy[big] += push * 0.5
    t3 = time.time()
    # local layout per community
    deg = np.bincount(np.concatenate([a, b]), weights=np.concatenate([w, w]), minlength=N)
    order = np.lexsort((-deg, memb))  # by community, then degree desc
    starts = np.concatenate([[0], np.cumsum(sizes)[:-1]])
    xy = np.empty((N, 2), dtype=np.float64)
    if PHASE == "hier":
        rank = np.empty(N, dtype=np.int64)
        rank[order] = np.arange(N) - np.repeat(starts, sizes)
        sp = vogel(int(sizes.max()))
        # jitter to avoid a perfect lattice look
        xy = cxy[memb] + sp[rank] * 0.9
    else:
        g = ig.Graph(n=N, edges=np.column_stack([a, b]), directed=False)
        g.es["weight"] = w.astype(np.float64)
        for c in range(C):
            nodes = order[starts[c]:starts[c] + sizes[c]]
            if sizes[c] < 3:
                loc = vogel(sizes[c])
            else:
                sg = g.induced_subgraph(nodes.tolist())
                if sizes[c] < 500:
                    loc = np.asarray(sg.layout_fruchterman_reingold(weights="weight").coords)
                else:
                    loc = np.asarray(sg.layout_drl(weights="weight").coords)
                loc = loc - np.median(loc, 0)
                r = np.percentile(np.linalg.norm(loc, axis=1), 95) + 1e-9
                loc = loc / r * rad[c]
            xy[nodes] = cxy[c] + loc
    t4 = time.time()
    xy = xy.astype(np.float32)
    np.save(data(f"xy_{PHASE}.npy"), xy)
    res.update(N=N, communities=C, contract_s=round(t1 - t0, 2), comm_layout_s=round(t2 - t1, 2),
               overlap_s=round(t3 - t2, 2), local_s=round(t4 - t3, 2), hier_total_s=round(t4 - t0, 2),
               edge_len_ratio=edge_len_ratio(xy, a, b))

elif PHASE == "pos":
    src = sys.argv[3]  # which layout
    xy = np.load(data(f"xy_{src}.npy"))
    memb = np.load(data("memb_leiden2.npy")).astype(np.uint32)
    N = xy.shape[0]
    rng = np.random.default_rng(1)
    tld = rng.choice(16, N, p=np.r_[0.45, 0.1, 0.08, 0.06, np.full(12, 0.31 / 12)]).astype(np.uint8)
    lang = rng.choice(12, N, p=np.r_[0.55, 0.1, 0.07, np.full(9, 0.28 / 9)]).astype(np.uint8)
    day = rng.integers(0, 400, N).astype(np.uint16)
    rec = np.dtype([("x", "<f4"), ("y", "<f4"), ("c", "<u4"), ("tld", "u1"), ("lang", "u1"), ("day", "<u2")])
    arr = np.empty(N, rec)
    arr["x"], arr["y"], arr["c"], arr["tld"], arr["lang"], arr["day"] = xy[:, 0], xy[:, 1], memb, tld, lang, day
    raw_aos = arr.tobytes()
    soa = b"".join(arr[f].tobytes() for f in rec.names)  # columnar layout, same size
    t = time.time(); gz_aos = gzip.compress(raw_aos, 6); tg = time.time() - t
    gz_soa = gzip.compress(soa, 6)
    with open(data(f"positions_{src}.bin.gz"), "wb") as f:
        f.write(gz_soa)
    res.update(N=N, layout=src, bytes_per_node=rec.itemsize, raw_mb=round(len(raw_aos) / 1e6, 2),
               gzip_interleaved_mb=round(len(gz_aos) / 1e6, 2), gzip_columnar_mb=round(len(gz_soa) / 1e6, 2),
               gzip_s=round(tg, 2))

elif PHASE == "plot":
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    src = sys.argv[3]
    xy = np.load(data(f"xy_{src}.npy"))
    memb = np.load(data("memb_leiden2.npy"))
    rng = np.random.default_rng(0)
    pal = rng.random((memb.max() + 1, 3)) * 0.8 + 0.1
    lo, hi = np.percentile(xy, [0.5, 99.5], axis=0)
    fig, ax = plt.subplots(figsize=(12, 12), dpi=150)
    ax.scatter(xy[:, 0], xy[:, 1], c=pal[memb], s=0.05 if xy.shape[0] > 3e5 else 0.2, linewidths=0, rasterized=True)
    pad = (hi - lo) * 0.05
    ax.set_xlim(lo[0] - pad[0], hi[0] + pad[0]); ax.set_ylim(lo[1] - pad[1], hi[1] + pad[1])
    ax.set_aspect("equal"); ax.set_axis_off()
    ax.set_title(f"{TAG} {src} layout, coloured by Leiden cluster (0.5-99.5 pct window)")
    out = os.path.join(D, "results", f"{TAG}_{src}.png")
    fig.savefig(out, bbox_inches="tight"); res["png"] = out

res["peak_rss_mb"] = rss_mb()
print(json.dumps(res))
with open(os.path.join(D, "results", f"{TAG}_{PHASE}{'_' + sys.argv[3] if len(sys.argv) > 3 else ''}.json"), "w") as f:
    json.dump(res, f)
