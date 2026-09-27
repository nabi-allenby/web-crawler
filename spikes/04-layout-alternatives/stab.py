"""Day-to-day stability: displacement of day-2 layout vs day-1 layout, after optimal Procrustes alignment
(rotation/reflection/scale/translation), in units of the layout's median radius. Also cluster-centroid shift.
Usage: python stab.py TAG day1 day2 [day2 ...]"""
import sys, os, json, numpy as np
from scipy.linalg import orthogonal_procrustes
TAG = sys.argv[1]; H = os.path.dirname(os.path.abspath(__file__))
pl = np.load(os.path.join(H, "..", "layout-spike", "data", f"{TAG}.npz"))["planted"]
def load(n):
    x = np.load(os.path.join(H, "data", f"{TAG}_xy_{n}.npy")).astype(np.float64)
    x -= np.median(x, 0); return x / np.median(np.linalg.norm(x, axis=1))
X = load(sys.argv[2])
cnt = np.bincount(pl)
def cent(x): return np.column_stack([np.bincount(pl, x[:, 0]) / cnt, np.bincount(pl, x[:, 1]) / cnt])
for n in sys.argv[3:]:
    Y = load(n)
    R, s = orthogonal_procrustes(Y, X); Ya = Y @ R * (np.trace(X.T @ (Y @ R)) / np.trace((Y @ R).T @ (Y @ R)))
    d = np.linalg.norm(Ya - X, axis=1); dc = np.linalg.norm(cent(Ya) - cent(X), axis=1)
    print(json.dumps(dict(day1=sys.argv[2], day2=n, node_disp_median=round(float(np.median(d)), 4),
          node_disp_p90=round(float(np.percentile(d, 90)), 4),
          cluster_centroid_disp_median=round(float(np.median(dc)), 4),
          cluster_centroid_disp_sizeweighted=round(float((dc * cnt).sum() / cnt.sum()), 4))))
