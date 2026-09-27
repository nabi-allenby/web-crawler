"""Build a Parquet table of the 1M-site synthetic graph's t-SNE layout for Embedding Atlas.

Row i is site i, so the neighbor ids (zero-based row indices) line up with Embedding Atlas.
"""
import numpy as np
import pyarrow as pa
import pyarrow.parquet as pq

SPIKE = "<scratch>"
TOP_K = 10

z = np.load(f"{SPIKE}/layout-spike/data/g1m.npz")
a, b, w, planted = z["a"], z["b"], z["w"], z["planted"]
n = planted.size
xy = np.load(f"{SPIKE}/layout-alt-spike/data/g1m_xy_tsne.npy")
leiden = np.load(f"{SPIKE}/layout-spike/data/g1m_memb_leiden2.npy")

# Undirected edge list -> both directions, sorted by source then by descending weight.
src = np.concatenate([a, b]).astype(np.int64)
dst = np.concatenate([b, a]).astype(np.int32)
wt = np.concatenate([w, w]).astype(np.float32)
order = np.lexsort((-wt, src))
src, dst, wt = src[order], dst[order], wt[order]

degree = np.bincount(src, minlength=n).astype(np.int32)
wdegree = np.bincount(src, weights=wt, minlength=n).astype(np.float32)

# Keep each site's TOP_K strongest links as its "neighbors".
starts = np.concatenate([[0], np.cumsum(degree)[:-1]])
rank = np.arange(src.size) - starts[src]
keep = rank < TOP_K
nb_ids, nb_w = dst[keep], wt[keep]
counts = np.minimum(degree, TOP_K)
offsets = np.concatenate([[0], np.cumsum(counts)]).astype(np.int32)
# Embedding Atlas expects distances; stronger links = closer.
nb_dist = (1.0 / nb_w).astype(np.float32)

neighbors = pa.StructArray.from_arrays(
    [pa.ListArray.from_arrays(pa.array(offsets), pa.array(nb_ids)),
     pa.ListArray.from_arrays(pa.array(offsets), pa.array(nb_dist))],
    names=["ids", "distances"],
)

ids = np.arange(n)
table = pa.table({
    "site": pa.array([f"site-{i}" for i in ids]),
    "x": pa.array(xy[:, 0]),
    "y": pa.array(xy[:, 1]),
    "community": pa.array([f"c{c}" for c in planted]),
    "leiden_cluster": pa.array([f"L{c}" for c in leiden]),
    "links": pa.array(degree),
    "link_weight": pa.array(wdegree),
    "neighbors": neighbors,
})
pq.write_table(table, f"{SPIKE}/mock-map/g1m_tsne.parquet", compression="zstd")
print(table.schema)
print(f"rows={table.num_rows} max_links={degree.max()} median_links={int(np.median(degree))}")
