"""3D layouts of the synthetic web graphs. One method per process (clean peak RSS).
Usage: python bench3d.py TAG METHOD [key=val ...]   METHOD in tsne3d | globe | pacmap3d
Writes results/TAG_NAME.json and data/TAG_NAME.npy (xyz float32, node-id order)."""
import sys, os, time, json, resource
import numpy as np, scipy.sparse as sp
HERE = os.path.dirname(os.path.abspath(__file__)); ROOT = os.path.join(HERE, "..")
sys.path.insert(0, os.path.join(ROOT, "toy"))
from spectral_dims import scores, spectral

TAG, METHOD = sys.argv[1], sys.argv[2]
OPT = dict(kv.split("=", 1) for kv in sys.argv[3:])
NAME = METHOD + (("_" + OPT.pop("suffix")) if "suffix" in OPT else "")
THREADS = int(OPT.get("threads", 8))
rss_mb = lambda: round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 2**20, 1)  # macOS: bytes
res = {"tag": TAG, "method": NAME, "opts": OPT, "threads": THREADS}

z = np.load(os.path.join(ROOT, "layout-spike", "data", f"{TAG}.npz"))
a, b, w, planted = z["a"], z["b"], z["w"], z["planted"]; N = planted.size
A = sp.coo_matrix((w.astype(float), (a, b)), shape=(N, N)); A = (A + A.T).tocsr()
d = np.asarray(A.sum(1)).ravel()
P = sp.diags(1 / d) @ A; P = P + P.T; P = (P / P.sum()).tocsr()  # identical to layout-alt-spike/bench.py
f = os.path.join(ROOT, "toy", "g1m_spec3d.npy") if TAG == "g1m" else os.path.join(HERE, "data", f"{TAG}_spec3d.npy")
if not os.path.exists(f):
    t0 = time.time(); np.save(f, spectral(A, 3).astype(np.float32)); res["spectral_s"] = round(time.time() - t0, 1)
init = np.load(f).astype(np.float64); init -= init.mean(0)
res.update(N=N, E=int(a.size), rss_after_load_mb=rss_mb())

t0 = time.time(); log = []
if METHOD == "tsne3d":
    from openTSNE import TSNEEmbedding
    from openTSNE.affinity import PrecomputedAffinities
    init = init / init[:, 0].std() * 1e-4
    emb = TSNEEmbedding(init, PrecomputedAffinities(P, normalize=False), n_jobs=THREADS,
                        negative_gradient_method="bh", theta=float(OPT.get("theta", 0.5)), random_state=0, verbose=False)
    T0 = time.time()
    def cb(i, err, e):
        log.append((i, round(time.time() - T0, 1), round(float(err), 4))); print("iter", i, log[-1], flush=True)
    cbk = dict(callbacks=cb, callbacks_every_iters=int(OPT.get("every", 50)))
    lr, ee, it = N / 12, int(OPT.get("ee", 250)), int(OPT.get("iters", 500))
    emb = emb.optimize(ee, exaggeration=12, momentum=0.5, learning_rate=lr, **cbk); log.append(("ee_done", round(time.time() - t0, 1)))
    emb = emb.optimize(it, exaggeration=float(OPT.get("exag", 1)), momentum=0.8, learning_rate=lr, **cbk)
    xyz = np.asarray(emb)
elif METHOD == "tsne3d_sk":  # sklearn's BH kernel (OpenMP-parallel, 3D-capable) driven with our own P
    from sklearn.manifold._t_sne import _kl_divergence_bh, _gradient_descent
    init = init / init[:, 0].std() * 1e-4
    lr, ee, it = N / 48, int(OPT.get("ee", 250)), int(OPT.get("iters", 500))  # sklearn grad = 4x openTSNE grad
    kw = dict(degrees_of_freedom=float(OPT.get("dof", 1)), n_samples=N, n_components=3,
              angle=float(OPT.get("theta", 0.5)), num_threads=THREADS, verbose=False)
    Pe = P.copy(); Pe.data *= 12.0
    T0 = time.time()
    def obj(p, P, **k):
        global nev
        nev += 1
        if nev % 50 == 0: log.append((nev, round(time.time() - T0, 1))); print("iter", log[-1], flush=True)
        return _kl_divergence_bh(p, P, **k)
    nev = 0
    ga = dict(n_iter_check=10**9, n_iter_without_progress=10**9, min_grad_norm=0.0, learning_rate=lr)
    p, _, i = _gradient_descent(obj, init.ravel().astype(np.float32), 0, ee, momentum=0.5, args=[Pe], kwargs=dict(kw), **ga)
    p, _, i = _gradient_descent(obj, p, ee, ee + it, momentum=0.8, args=[P], kwargs=dict(kw), **ga)
    xyz = p.reshape(N, 3)
elif METHOD == "sgtsne3d":  # SG-t-SNE-Pi (pysgtsnepi port): FFT-interpolated repulsion in 3D, our P as-is
    import pysgtsnepi.embedding as se
    from scipy.sparse import csc_matrix
    init = init / init[:, 0].std() * 1e-4
    rep = se.compute_repulsive_forces; T0 = time.time(); cnt = [0]
    def rep_logged(Y, h):
        cnt[0] += 1
        if cnt[0] % 50 == 0:
            ext = float(np.ptp(Y, 0).max()); log.append((cnt[0], round(time.time() - T0, 1), round(ext, 1))); print("iter", log[-1], flush=True)
        return rep(Y, h)
    se.compute_repulsive_forces = rep_logged
    ee, it = int(OPT.get("ee", 250)), int(OPT.get("iters", 500))
    xyz = se.sgtsne_embedding(csc_matrix(P), d=3, max_iter=ee + it, early_exag=ee, alpha=12.0,
                              eta=float(OPT.get("eta", N / 12)), h=float(OPT.get("h", 1.0)), Y0=init, random_state=0)
elif METHOD == "globe":
    import umap.umap_ as uu, umap.distances as dist
    from umap.umap_ import find_ab_params
    g = P.tocoo(); g.data = np.maximum(g.data / g.data.max(), 1e-3)
    u = init / np.linalg.norm(init, axis=1, keepdims=True)           # spectral 3D direction -> sphere
    ang = np.c_[np.arccos(np.clip(u[:, 2], -1, 1)), np.arctan2(u[:, 1], u[:, 0])]  # (colatitude, longitude)
    s = float(OPT.get("scale", 1.0))  # kernel scale relative to unit sphere (smaller = roomier globe)
    a_, b_ = find_ab_params(s, 0.1 * s)
    emb, _ = uu.simplicial_set_embedding(
        data=np.zeros((N, 1), np.float32), graph=g, n_components=2, initial_alpha=float(OPT.get("alpha", 1.0)) * s,
        a=a_, b=b_, gamma=1.0, negative_sample_rate=5, n_epochs=int(OPT.get("epochs", 200)),
        init=ang.astype(np.float32), random_state=np.random.RandomState(0), metric="euclidean", metric_kwds={},
        densmap=False, densmap_kwds={}, output_dens=False, output_metric=dist.haversine_grad, output_metric_kwds={},
        euclidean_output=False, parallel=True, verbose=False)
    th, ph = emb[:, 0].astype(np.float64), emb[:, 1].astype(np.float64)   # umap docs' sphere convention
    xyz = np.c_[np.sin(th) * np.cos(ph), np.sin(th) * np.sin(ph), np.cos(th)]
elif METHOD == "sphere":  # "globe": UMAP-style edge-sampling SGD with every point constrained to a sphere of radius R
    import numba
    from umap.umap_ import find_ab_params, make_epochs_per_sample
    R = float(OPT.get("R", 10.0)); E_ = int(OPT.get("epochs", 200)); neg = 5
    g = P.tocoo(); g.data = np.maximum(g.data / g.data.max(), 1e-3)
    eps = make_epochs_per_sample(g.data, E_).astype(np.float64)
    a_, b_ = find_ab_params(1.0, 0.1)
    Y = init / np.linalg.norm(init, axis=1, keepdims=True) * R
    @numba.njit(parallel=True, fastmath=True)
    def run(Y, h, t, eps, E_, a_, b_, R, neg, seed):
        n = Y.shape[0]; nxt = eps.copy(); nxt_neg = eps / neg
        for ep in range(E_):
            alpha = 1.0 - ep / E_
            for i in numba.prange(h.size):
                if nxt[i] > ep + 1: continue
                j, k = h[i], t[i]
                d2 = 0.0
                for c in range(3): d2 += (Y[j, c] - Y[k, c]) ** 2
                coef = (-2.0 * a_ * b_ * d2 ** (b_ - 1.0)) / (a_ * d2 ** b_ + 1.0) if d2 > 0 else 0.0
                for c in range(3):
                    gr = min(4.0, max(-4.0, coef * (Y[j, c] - Y[k, c])))
                    Y[j, c] += gr * alpha; Y[k, c] -= gr * alpha
                nn = int((ep + 1 - nxt_neg[i]) / (eps[i] / neg))
                for q in range(nn):
                    kk = (seed * 1103515245 + i * 12345 + ep * 7919 + q * 104729) % n
                    d2 = 0.0
                    for c in range(3): d2 += (Y[j, c] - Y[kk, c]) ** 2
                    if d2 <= 0: continue
                    coef = 2.0 * b_ / ((0.001 + d2) * (a_ * d2 ** b_ + 1.0))
                    for c in range(3):
                        Y[j, c] += min(4.0, max(-4.0, coef * (Y[j, c] - Y[kk, c]))) * alpha
                nxt_neg[i] += nn * eps[i] / neg; nxt[i] += eps[i]
                r = 0.0
                for c in range(3): r += Y[j, c] ** 2
                r = R / np.sqrt(r)
                for c in range(3): Y[j, c] *= r      # project back onto the sphere
                r = 0.0
                for c in range(3): r += Y[k, c] ** 2
                r = R / np.sqrt(r)
                for c in range(3): Y[k, c] *= r
        return Y
    xyz = run(Y, g.row.astype(np.int64), g.col.astype(np.int64), eps, E_, a_, b_, R, neg, 1)
elif METHOD == "umap3d":  # same call as toy/umap3d.py
    import umap.umap_ as uu
    from umap.umap_ import find_ab_params
    g = P.tocoo(); g.data = np.maximum(g.data / g.data.max(), 1e-3)
    a_, b_ = find_ab_params(1.0, 0.1)
    xyz, _ = uu.simplicial_set_embedding(data=np.zeros((N, 1), np.float32), graph=g, n_components=3, initial_alpha=1.0,
        a=a_, b=b_, gamma=1.0, negative_sample_rate=5, n_epochs=200, init=(init / np.abs(init).max() * 10).astype(np.float32),
        random_state=np.random.RandomState(0), metric="euclidean", metric_kwds={}, densmap=False, densmap_kwds={},
        output_dens=False, parallel=True, verbose=False)
elif METHOD == "eval":  # score an existing layout file (2D or 3D)
    xyz = np.load(OPT["src"]); log.append("no timing: " + OPT["src"])
elif METHOD == "pacmap3d":
    import pacmap
    X = spectral(A, int(OPT.get("dims", 16)))  # PaCMAP needs features: use 16-D spectral coordinates
    xyz = pacmap.PaCMAP(n_components=3, n_neighbors=10, random_state=0).fit_transform(X.astype(np.float32), init="pca")
else:
    raise SystemExit("unknown " + METHOD)
res["layout_s"] = round(time.time() - t0, 1); res["log"] = log; res["peak_rss_mb"] = rss_mb()
xyz = np.nan_to_num(np.asarray(xyz, dtype=np.float32)); np.save(os.path.join(HERE, "data", f"{TAG}_{NAME}.npy"), xyz)
res["purity"], res["recall"] = scores(xyz, planted, A)
print(json.dumps(res)); json.dump(res, open(os.path.join(HERE, "results", f"{TAG}_{NAME}.json"), "w"))
