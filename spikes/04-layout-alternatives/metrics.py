"""Neighbour recall (Böhm et al. 2025 metric): for sampled nodes, share of graph neighbours found among the
k nearest 2D neighbours, k = min(degree, 30). Also within-community version (only intra-community neighbours,
k-NN restricted to same community) = does the layout show structure INSIDE clusters.
Usage: python metrics.py TAG path/to/xy.npy [...]"""
import sys, os, json, numpy as np, scipy.sparse as sp
from scipy.spatial import cKDTree
TAG = sys.argv[1]
HERE = os.path.dirname(os.path.abspath(__file__)); OLD = os.path.join(HERE, "..", "layout-spike", "data")
z = np.load(os.path.join(OLD, f"{TAG}.npz")); a, b, pl = z["a"], z["b"], z["planted"]; N = pl.size
A = sp.coo_matrix((np.ones(2 * a.size, np.int8), (np.r_[a, b], np.r_[b, a])), shape=(N, N)).tocsr()
rng = np.random.default_rng(0); q = rng.choice(N, 5000, replace=False)
# baseline: random placement inside the right community -> expected recall = k / community size
for f in sys.argv[2:]:
    xy = np.load(f).astype(np.float64); tree = cKDTree(xy)
    _, nb = tree.query(xy[q], 31, workers=8)
    rec, rec_in = [], []
    for j, i in enumerate(q):
        nbrs = A.indices[A.indptr[i]:A.indptr[i + 1]]
        k = min(nbrs.size, 30)
        if k == 0: continue
        rec.append(np.isin(nb[j, 1:k + 1], nbrs).mean())
    # intra-community recall: among same-community nodes only
    for i in q[:1000]:
        nbrs = A.indices[A.indptr[i]:A.indptr[i + 1]]; nbrs = nbrs[pl[nbrs] == pl[i]]
        k = min(nbrs.size, 30)
        if k == 0: continue
        same = np.flatnonzero(pl == pl[i]); same = same[same != i]
        d = np.linalg.norm(xy[same] - xy[i], axis=1)
        top = same[np.argpartition(d, k)[:k]] if same.size > k else same
        rnd = k / same.size
        rec_in.append((np.isin(top, nbrs).mean(), rnd))
    ri = np.array(rec_in)
    out = dict(file=os.path.basename(f), nbr_recall=round(float(np.mean(rec)), 4),
               intra_recall=round(float(ri[:, 0].mean()), 4), intra_recall_random=round(float(ri[:, 1].mean()), 4))
    print(json.dumps(out)); sys.stdout.flush()
