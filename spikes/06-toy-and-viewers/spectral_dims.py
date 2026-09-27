import json, time, numpy as np, scipy.sparse as sp
from scipy.sparse.linalg import eigsh
from sklearn.neighbors import NearestNeighbors

def spectral(A, dims):
    d = np.asarray(A.sum(1)).ravel(); Dm = sp.diags(1 / np.sqrt(d))
    vals, vecs = eigsh(Dm @ A @ Dm, k=dims + 1, which="LA", tol=1e-4)
    order = np.argsort(-vals)
    return np.asarray(Dm @ vecs[:, order[1:dims + 1]])

def scores(xy, lab, A, n=20000, seed=0):
    rng = np.random.default_rng(seed); q = rng.choice(len(lab), min(n, len(lab)), replace=False)
    nn = NearestNeighbors(n_neighbors=31).fit(xy)
    _, idx = nn.kneighbors(xy[q]); idx = idx[:, 1:]
    purity = float((lab[idx[:, :10]] == lab[q][:, None]).mean())
    A = A.tocsr(); rec = []
    for r, i in enumerate(q):
        nb = A.indices[A.indptr[i]:A.indptr[i + 1]]; k = min(len(nb), 30)
        if k: rec.append(len(set(idx[r, :k]) & set(nb)) / k)
    return round(purity, 3), round(float(np.mean(rec)), 3)

if __name__ == "__main__":
    out = {}
    # toy graph (rebuild adjacency from toy.json)
    t = json.load(open("toy/toy.json")); lab = np.array(t["c"]); e = np.array(t["e"]).reshape(-1, 2); N = len(lab)
    A = sp.coo_matrix((np.ones(len(e)), (e[:, 0], e[:, 1])), shape=(N, N)); A = (A + A.T).tocsr()
    for dims in (2, 3, 16):
        out[f"toy_{dims}d"] = scores(spectral(A, dims), lab, A, n=N)
    np.save("toy/toy_spec3d.npy", spectral(A, 3))
    # 1M synthetic graph
    z = np.load("layout-spike/data/g1m.npz"); a, b, w, planted = z["a"], z["b"], z["w"], z["planted"]; N = planted.size
    A = sp.coo_matrix((w.astype(float), (a, b)), shape=(N, N)); A = (A + A.T).tocsr()
    for dims in (2, 3, 16):
        t0 = time.time(); xy = spectral(A, dims)
        out[f"g1m_{dims}d"] = scores(xy, planted, A) + (round(time.time() - t0, 1),)
        if dims == 3: np.save("toy/g1m_spec3d.npy", xy.astype(np.float32))
    json.dump(out, open("toy/spectral_dims.json", "w"), indent=1); print(out)
