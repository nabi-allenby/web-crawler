"""Benchmark alternative large-graph layouts. One method per process (clean peak RSS).
Usage: python bench.py TAG METHOD [key=val ...]
Writes results/TAG_METHOD[_suffix].json/.png and data/TAG_xy_METHOD[_suffix].npy
"""
import sys, os, time, json, resource
import numpy as np
import scipy.sparse as sp

TAG, METHOD = sys.argv[1], sys.argv[2]
OPT = dict(kv.split("=", 1) for kv in sys.argv[3:])
SUFFIX = OPT.pop("suffix", "")
NAME = METHOD + (("_" + SUFFIX) if SUFFIX else "")
HERE = os.path.dirname(os.path.abspath(__file__))
OLD = os.path.join(HERE, "..", "layout-spike", "data")
THREADS = int(OPT.get("threads", 8))
res = {"tag": TAG, "method": NAME, "opts": OPT, "threads": THREADS}


def rss_mb():
    return round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 2**20, 1)  # macOS bytes


def edge_len_ratio(xy, a, b, seed=0):  # identical to layout-spike/run.py
    rng = np.random.default_rng(seed)
    idx = rng.choice(a.size, min(a.size, 2_000_000), replace=False)
    el = np.linalg.norm(xy[a[idx]] - xy[b[idx]], axis=1).mean()
    p = rng.integers(0, xy.shape[0], (2_000_000, 2))
    rl = np.linalg.norm(xy[p[:, 0]] - xy[p[:, 1]], axis=1).mean()
    return round(float(el / rl), 4)


def knn_purity(xy, lab, k=10, n=20000, seed=0):
    """fraction of each sampled node's k nearest 2D neighbours sharing its planted community"""
    from scipy.spatial import cKDTree
    rng = np.random.default_rng(seed)
    q = rng.choice(xy.shape[0], n, replace=False)
    _, nb = cKDTree(xy).query(xy[q], k + 1, workers=THREADS)
    return round(float((lab[nb[:, 1:]] == lab[q, None]).mean()), 4)


z = np.load(os.path.join(OLD, f"{TAG}.npz"))
a, b, w, planted = z["a"].astype(np.int64), z["b"].astype(np.int64), z["w"].astype(np.float64), z["planted"]
N = planted.size
if "drop" in OPT:  # simulate "tomorrow": drop a fraction of edges (for stability test)
    keep = np.random.default_rng(7).random(a.size) >= float(OPT["drop"])
    a, b, w = a[keep], b[keep], w[keep]
A = sp.coo_matrix((np.r_[w, w], (np.r_[a, b], np.r_[b, a])), shape=(N, N)).tocsr()
res.update(N=N, E=int(a.size), rss_after_load_mb=rss_mb())
init = None
if "init" in OPT:  # e.g. init=hier (previous spike) or init=tsne (this spike)
    f = os.path.join(OLD, f"{TAG}_xy_{OPT['init']}.npy")
    if not os.path.exists(f):
        f = os.path.join(HERE, "data", f"{TAG}_xy_{OPT['init']}.npy")
    init = np.load(f).astype(np.float64)

t0 = time.time()
if METHOD == "pmds":
    import networkit as nk
    nk.setNumberOfThreads(THREADS)
    G = nk.GraphFromCoo((w, (a, b)), n=N, weighted=False, directed=False)  # hop distance
    alg = nk.viz.PivotMDS(G, 2, int(OPT.get("pivots", 50)))
    alg.run()
    xy = np.asarray(alg.getCoordinates(), dtype=np.float64)

elif METHOD == "maxent":
    import networkit as nk
    nk.setNumberOfThreads(THREADS)
    G = nk.GraphFromCoo((w, (a, b)), n=N, weighted=False, directed=False)
    coords = [] if init is None else [tuple(p) for p in init]
    alg = nk.viz.MaxentStress(G, 2, int(OPT.get("k", 1)), coords, 1e-3,
                              nk.viz.LinearSolverType.CONJUGATE_GRADIENT_DIAGONAL_PRECONDITIONER, True)
    alg.run()
    xy = np.asarray(alg.getCoordinates(), dtype=np.float64)

elif METHOD == "sgd2":
    import s_gd2
    xy = s_gd2.layout_sparse(a.astype(np.int32), b.astype(np.int32), int(OPT.get("pivots", 30)),
                             t_max=int(OPT.get("iters", 15)), init=init)

elif METHOD == "fa2":
    from fa2_modified import ForceAtlas2
    fa = ForceAtlas2(outboundAttractionDistribution=True, barnesHutOptimize=True, barnesHutTheta=1.2,
                     scalingRatio=2.0, gravity=1.0, verbose=False)
    pos = None if init is None else init
    xy = np.asarray(fa.forceatlas2(A, pos=pos, iterations=int(OPT.get("iters", 50))))

elif METHOD == "spectral":
    from scipy.sparse.linalg import eigsh
    d = np.asarray(A.sum(1)).ravel()
    Dm = sp.diags(1 / np.sqrt(d))
    L = Dm @ A @ Dm
    vals, vecs = eigsh(L, k=3, which="LA", tol=1e-4)
    xy = (Dm @ vecs[:, :2])  # random-walk eigenvectors 2,3
    xy = np.asarray(xy)

elif METHOD in ("tsne", "umap"):
    # neighbour embedding driven directly by the graph (no feature vectors): P = sym. row-normalised adjacency
    P = sp.diags(1 / np.asarray(A.sum(1)).ravel()) @ A
    P = (P + P.T)
    P = P / P.sum()
    if init is None:  # cheap init: PivotMDS-free, random-walk-smoothed random projection -> 2D via PCA
        rng = np.random.default_rng(int(OPT.get("seed", 0)))
        R = rng.standard_normal((N, 16))
        T = sp.diags(1 / np.asarray(A.sum(1)).ravel()) @ A
        for _ in range(6):
            R = T @ R
        R -= R.mean(0)
        U, S, Vt = np.linalg.svd(R[rng.choice(N, min(N, 50000), replace=False)], full_matrices=False)
        init = R @ Vt[:2].T
    init = init - init.mean(0)
    if "keepscale" not in OPT:  # fresh run: openTSNE convention, tiny init (std 1e-4)
        init = init / init[:, 0].std() * 1e-4
    res["setup_s"] = round(time.time() - t0, 2)
    if METHOD == "tsne":
        from openTSNE import TSNEEmbedding
        from openTSNE.affinity import PrecomputedAffinities
        aff = PrecomputedAffinities(P.tocsr(), normalize=False)
        emb = TSNEEmbedding(init.astype(np.float64), aff,
                            n_jobs=THREADS, negative_gradient_method="fft", random_state=0, verbose=False)
        ee_iter = int(OPT.get("ee", 250)) if "init" not in OPT else int(OPT.get("ee", 0))
        lr = N / 12
        if ee_iter:
            emb = emb.optimize(ee_iter, exaggeration=12, momentum=0.5, learning_rate=lr)
        emb = emb.optimize(int(OPT.get("iters", 500)), exaggeration=float(OPT.get("exag", 1)), momentum=0.8,
                           learning_rate=lr)
        xy = np.asarray(emb)
    else:
        import umap.umap_ as uu
        from umap.umap_ import find_ab_params
        g = P.tocoo()
        g.data = g.data / g.data.max()  # membership strengths in (0,1]
        g.data = np.maximum(g.data, 1e-3)  # keep every edge sampled at least occasionally
        a_, b_ = find_ab_params(1.0, float(OPT.get("min_dist", 0.1)))
        emb, _ = uu.simplicial_set_embedding(
            data=np.zeros((N, 1), np.float32), graph=g, n_components=2, initial_alpha=1.0, a=a_, b=b_,
            gamma=1.0, negative_sample_rate=int(OPT.get("neg", 5)), n_epochs=int(OPT.get("epochs", 200)),
            init=(init / np.abs(init).max() * 10).astype(np.float32), random_state=np.random.RandomState(0),
            metric="euclidean", metric_kwds={}, densmap=False, densmap_kwds={}, output_dens=False,
            parallel=True, verbose=False)
        xy = np.asarray(emb)

elif METHOD == "fastrp_umap":
    # classic "embedding atlas": node embedding (FastRP, Chen et al. 2019) -> kNN -> UMAP
    import umap
    rng = np.random.default_rng(0)
    dim = int(OPT.get("dim", 64))
    d = np.asarray(A.sum(1)).ravel()
    T = sp.diags(1 / d) @ A
    R = sp.random(N, dim, density=1 / 3, random_state=0, data_rvs=lambda k: rng.choice([-1.0, 1.0], k)).toarray()
    R = R * (d[:, None] ** -0.5)  # degree normalisation (beta=-0.5) damps hubs
    E = np.zeros((N, dim)); cur = R
    for wgt in (0.0, 1.0, 1.0, 1.0):  # powers 1..4 weighted
        cur = T @ cur
        E += wgt * cur
    E /= np.linalg.norm(E, axis=1, keepdims=True) + 1e-12
    res["embed_s"] = round(time.time() - t0, 2)
    xy = umap.UMAP(n_neighbors=15, min_dist=0.1, metric="cosine", n_epochs=int(OPT.get("epochs", 200)),
                   low_memory=True, n_jobs=THREADS, init="spectral" if N <= 200000 else "random",
                   random_state=None).fit_transform(E.astype(np.float32))

elif METHOD == "hier2":
    # recursive hierarchical: previous spike's top level (community centres + radii), but inside each
    # community the members get a graph-tSNE layout of the induced subgraph, scaled into the community disc.
    from openTSNE import TSNEEmbedding
    from openTSNE.affinity import PrecomputedAffinities
    memb = np.load(os.path.join(OLD, f"{TAG}_memb_leiden2.npy"))
    base = np.load(os.path.join(OLD, f"{TAG}_xy_hier.npy")).astype(np.float64)
    C = memb.max() + 1
    sizes = np.bincount(memb, minlength=C)
    xy = base.copy()
    minsz = int(OPT.get("minsz", 2000))
    order = np.argsort(memb, kind="stable"); starts = np.r_[0, np.cumsum(sizes)[:-1]]
    for c in np.argsort(-sizes):
        if sizes[c] < minsz:
            break
        nodes = order[starts[c]:starts[c] + sizes[c]]
        sub = A[nodes][:, nodes]
        dg = np.asarray(sub.sum(1)).ravel(); dg[dg == 0] = 1
        P = sp.diags(1 / dg) @ sub; P = P + P.T; P = (P / P.sum()).tocsr()
        loc0 = base[nodes] - base[nodes].mean(0)
        emb = TSNEEmbedding(loc0 / loc0[:, 0].std() * 1e-4, PrecomputedAffinities(P, normalize=False),
                            n_jobs=THREADS, negative_gradient_method="fft", random_state=0, verbose=False)
        lr = max(200, nodes.size / 12)
        emb = emb.optimize(125, exaggeration=12, momentum=0.5, learning_rate=lr)
        emb = emb.optimize(250, exaggeration=2, momentum=0.8, learning_rate=lr)
        loc = np.asarray(emb); loc -= np.median(loc, 0)
        r_new = np.percentile(np.linalg.norm(loc, axis=1), 98) + 1e-9
        r_old = np.percentile(np.linalg.norm(loc0, axis=1), 98)
        xy[nodes] = base[nodes].mean(0) + loc / r_new * r_old
    res["refined_communities"] = int((sizes >= minsz).sum())
elif METHOD == "fm3":  # OGDF FMMMLayout (FM^3) via ogdf-python/cppyy; needs DYLD_FALLBACK_LIBRARY_PATH=/opt/homebrew/lib
    import cppyy
    from ogdf_python import ogdf, cppinclude
    cppinclude("ogdf/energybased/FMMMLayout.h")
    cppyy.cppdef(r'''
    #include <vector>
    void run_fmmm(int n, int m, const int* a, const int* b, double* out, int q) {
      ogdf::Graph G; std::vector<ogdf::node> v(n);
      for (int i = 0; i < n; i++) v[i] = G.newNode();
      for (int j = 0; j < m; j++) G.newEdge(v[a[j]], v[b[j]]);
      ogdf::GraphAttributes GA(G, ogdf::GraphAttributes::nodeGraphics | ogdf::GraphAttributes::edgeGraphics);
      ogdf::FMMMLayout f; f.useHighLevelOptions(true); f.unitEdgeLength(15.0); f.newInitialPlacement(true);
      f.qualityVersusSpeed(q == 0 ? ogdf::FMMMOptions::QualityVsSpeed::NiceAndIncredibleSpeed
                          : ogdf::FMMMOptions::QualityVsSpeed::BeautifulAndFast);
      f.call(GA);
      for (int i = 0; i < n; i++) { out[2*i] = GA.x(v[i]); out[2*i+1] = GA.y(v[i]); }
    }''')
    out = np.zeros(2 * N)
    ai, bi = a.astype(np.int32), b.astype(np.int32)
    cppyy.gbl.run_fmmm(N, int(a.size), ai, bi, out, int(OPT.get("q", 0)))
    xy = out.reshape(N, 2)

elif METHOD == "eval":  # score an existing layout from the previous spike (no timing)
    xy = np.load(os.path.join(OLD, f"{TAG}_xy_{OPT['src']}.npy"))
else:
    raise SystemExit(f"unknown method {METHOD}")

res["layout_s"] = round(time.time() - t0, 2)
res["peak_rss_layout_mb"] = rss_mb()
xy = np.asarray(xy, dtype=np.float64)
res["nan"] = int(np.isnan(xy).sum())
xy = np.nan_to_num(xy)
np.save(os.path.join(HERE, "data", f"{TAG}_xy_{NAME}.npy"), xy.astype(np.float32))
res["edge_len_ratio"] = edge_len_ratio(xy, a, b)
res["knn10_purity"] = knn_purity(xy, planted)
if init is not None and "init" in OPT:
    # stability vs the seed layout: median displacement / median distance-from-centre (both normalised)
    def nrm(x):
        x = x - np.median(x, 0); return x / np.median(np.linalg.norm(x, axis=1))
    res["disp_vs_init_median"] = round(float(np.median(np.linalg.norm(nrm(xy) - nrm(init), axis=1))), 4)

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
rng = np.random.default_rng(0)
pal = rng.random((planted.max() + 1, 3)) * 0.8 + 0.1
lo, hi = np.percentile(xy, [0.5, 99.5], axis=0)
fig, ax = plt.subplots(figsize=(10, 10), dpi=150)
ax.scatter(xy[:, 0], xy[:, 1], c=pal[planted], s=0.05 if N > 3e5 else 0.2, linewidths=0, rasterized=True)
pad = (hi - lo) * 0.05
ax.set_xlim(lo[0] - pad[0], hi[0] + pad[0]); ax.set_ylim(lo[1] - pad[1], hi[1] + pad[1])
ax.set_aspect("equal"); ax.set_axis_off()
ax.set_title(f"{TAG} {NAME}: {res['layout_s']}s, ELR {res['edge_len_ratio']}, kNN purity {res['knn10_purity']}")
out = os.path.join(HERE, "results", f"{TAG}_{NAME}.png")
fig.savefig(out, bbox_inches="tight"); res["png"] = out
print(json.dumps(res))
json.dump(res, open(os.path.join(HERE, "results", f"{TAG}_{NAME}.json"), "w"))
