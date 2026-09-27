import json, numpy as np, matplotlib
matplotlib.use("Agg"); import matplotlib.pyplot as plt
d = json.load(open("toy/toy.json"))
c = np.array(d["c"]); s = np.array(d["s"]).reshape(-1, 2); t = np.array(d["t"]).reshape(-1, 2)
e = np.array(d["e"]).reshape(-1, 2)
nbr = {i: [] for i in range(len(c))}
for a, b in e: nbr[a].append(b); nbr[b].append(a)
cols = ["#7F77DD","#1D9E75","#D85A30","#D4537E","#378ADD","#639922","#BA7517","#E24B4A",
        "#534AB7","#0F6E56","#993C1D","#993556","#185FA5","#3B6D11","#854F0B","#888780"]
focus = [5, 305, 612]          # three sites from different communities
fig, axes = plt.subplots(1, 2, figsize=(12, 6.2), dpi=130)
for ax, xy, title, pur in [(axes[0], s, "Spectral", d["ps"]), (axes[1], t, "Graph t-SNE", d["pt"])]:
    ax.scatter(xy[:, 0], xy[:, 1], s=9, c=[cols[k] for k in c], alpha=0.85, linewidths=0)
    for f in focus:
        for j in nbr[f]:
            ax.plot([xy[f, 0], xy[j, 0]], [xy[f, 1], xy[j, 1]], color="black", lw=0.8, alpha=0.7)
        ax.scatter(*xy[f], s=90, c=cols[c[f]], edgecolors="black", linewidths=1.5, zorder=5)
    ax.set_title(f"{title}\n{int(pur*100)}% of each dot's 10 nearest dots are in its own community", fontsize=11)
    ax.set_xticks([]); ax.set_yticks([]); ax.set_aspect("equal")
fig.suptitle("Toy graph: 800 sites, 16 communities (one color each). Black lines: links of 3 sample sites", fontsize=12)
fig.tight_layout(); fig.savefig("toy/spectral_vs_tsne.png")
