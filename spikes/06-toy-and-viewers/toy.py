import json, numpy as np, scipy.sparse as sp
from scipy.sparse.linalg import eigsh
from openTSNE import TSNEEmbedding
from openTSNE.affinity import PrecomputedAffinities
rng = np.random.default_rng(7)
K, S = 16, 50; N = K * S
comm = np.repeat(np.arange(K), S)
src, dst = [], []
for i in range(N):
    for _ in range(4):                      # ~8 links per site after symmetrizing
        if rng.random() < 0.85:
            j = rng.integers(0, S) + comm[i] * S
        else:
            j = rng.integers(0, N)
        if j != i: src.append(i); dst.append(j)
A = sp.coo_matrix((np.ones(len(src)), (src, dst)), shape=(N, N)).tocsr()
A = ((A + A.T) > 0).astype(float)
d = np.asarray(A.sum(1)).ravel()
# spectral: random-walk eigenvectors 2 and 3 (same as the spike)
Dm = sp.diags(1 / np.sqrt(d)); vals, vecs = eigsh(Dm @ A @ Dm, k=3, which="LA")
order = np.argsort(-vals); spec = np.asarray(Dm @ vecs[:, order[1:3]])
# graph t-SNE (same affinities as the spike)
P = sp.diags(1 / d) @ A; P = P + P.T; P = P / P.sum()
init = rng.standard_normal((N, 2)) * 1e-4
emb = TSNEEmbedding(init, PrecomputedAffinities(P.tocsr(), normalize=False), random_state=0, verbose=False)
emb = emb.optimize(250, exaggeration=12, momentum=0.5, learning_rate=N / 12)
emb = emb.optimize(500, momentum=0.8, learning_rate=N / 12)
tsne = np.asarray(emb)
def norm(xy):
    xy = xy - xy.mean(0); return xy / np.abs(xy).max()
def purity(xy, k=10):
    dd = ((xy[:, None, :] - xy[None, :, :]) ** 2).sum(-1); np.fill_diagonal(dd, np.inf)
    nn = np.argsort(dd, 1)[:, :k]; return float((comm[nn] == comm[:, None]).mean())
spec, tsne = norm(spec), norm(tsne)
Au = sp.triu(A, 1).tocoo()
out = {"c": comm.tolist(),
       "s": np.round(spec, 4).ravel().tolist(), "t": np.round(tsne, 4).ravel().tolist(),
       "e": np.stack([Au.row, Au.col], 1).ravel().tolist(),
       "ps": round(purity(spec), 2), "pt": round(purity(tsne), 2)}
json.dump(out, open("toy/toy.json", "w"), separators=(",", ":"))
print(N, "nodes", Au.nnz, "edges", "purity spectral", out["ps"], "tsne", out["pt"])
