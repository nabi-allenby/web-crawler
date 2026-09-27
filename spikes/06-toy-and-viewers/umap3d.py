import json, time, numpy as np, scipy.sparse as sp
import umap.umap_ as uu
from umap.umap_ import find_ab_params
import sys; sys.path.insert(0, "toy")
from spectral_dims import scores  # same purity/recall as before (module guarded below)

z = np.load("layout-spike/data/g1m.npz"); a, b, w, planted = z["a"], z["b"], z["w"], z["planted"]; N = planted.size
A = sp.coo_matrix((w.astype(float), (a, b)), shape=(N, N)); A = (A + A.T).tocsr()
d = np.asarray(A.sum(1)).ravel()
P = sp.diags(1 / d) @ A; P = P + P.T; P = P / P.sum()          # same affinities as the spike
g = P.tocoo(); g.data = np.maximum(g.data / g.data.max(), 1e-3)
init = np.load("toy/g1m_spec3d.npy").astype(np.float64); init -= init.mean(0); init = init / np.abs(init).max() * 10
a_, b_ = find_ab_params(1.0, 0.1)
t0 = time.time()
emb, _ = uu.simplicial_set_embedding(
    data=np.zeros((N, 1), np.float32), graph=g, n_components=3, initial_alpha=1.0, a=a_, b=b_, gamma=1.0,
    negative_sample_rate=5, n_epochs=200, init=init.astype(np.float32), random_state=np.random.RandomState(0),
    metric="euclidean", metric_kwds={}, densmap=False, densmap_kwds={}, output_dens=False, parallel=True, verbose=False)
secs = round(time.time() - t0, 1)
xyz = np.asarray(emb, dtype=np.float32)
pur, rec = scores(xyz, planted, A)
print({"umap3d_seconds": secs, "purity": pur, "recall": rec})
# viewer files: positions (float32 x3, centered, scaled to +-100), community (uint16), links (uint32)
xyz -= np.median(xyz, 0); xyz = xyz / np.percentile(np.abs(xyz), 99.5) * 100
xyz.astype("<f4").tofile("mock-map/3d/positions.f32")
planted.astype("<u2").tofile("mock-map/3d/community.u16")
np.bincount(np.concatenate([a, b]), minlength=N).astype("<u4").tofile("mock-map/3d/links.u32")
json.dump({"n": int(N), "seconds": secs, "purity": pur, "recall": rec}, open("mock-map/3d/meta.json", "w"))
