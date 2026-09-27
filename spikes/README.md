# Explorer design spikes

The evidence behind [the explorer design](../docs/explorer-design.md): throwaway code and measured results from the design review of 2026-09-25 to 27. This isn't production code, and none of it is built or tested in CI.

**Running the scripts again:**
- They were run from a scratch folder, shown in the files as `<scratch>`. That folder held `layout-spike/`, `layout-alt-spike/`, `layout-3d-spike/`, `pg-frontier-spike/`, `toy/` and `mock-map/` side by side, plus Python virtualenvs.
- Generated data isn't committed: the synthetic graphs (`*.npz`), the layouts (`*.npy`) and the raw crawl logs. Recreate the synthetic graphs with `03-layout-drl/gen.py`, e.g. `python gen.py 1000000 800 g1m.npz`.

## Spikes

| # | Question | Result | Decided |
|---|---|---|---|
| [01-throughput](01-throughput) | Can one process crawl 1M sites in 90 days politely? | Yes. About 0.07 completed sites/s per parallel visit, scaling linearly. 64 parallel visits used ~19% of a core and 185 MB. Politeness is the limit, not CPU. | One explorer, 8 parallel visits (Q19, Q25) |
| [02-postgres-frontier](02-postgres-frontier) | Can Postgres pick "random among the oldest" at 50M domains? | Yes. Claiming 8 domains takes ~2–4 ms p50, with no duplicates across two claimers. Also yielded autovacuum and restart-query guidance. | Bucketed random pick (Q27, Q39) |
| [03-layout-drl](03-layout-drl) | Can Leiden + DrL lay out 1M sites daily? | Leiden, yes: 12 s. DrL, no: it didn't finish in 38 min (est. 1.5–2 h). | DrL dropped (Q42) |
| [04-layout-alternatives](04-layout-alternatives) | Which large-graph layout works at 1M? | Graph t-SNE had the best neighbor recall by 3–4×: 97 s seeded, ~2% daily movement. Stress/MDS, FM³ and ForceAtlas2 were unusable at this scale. | Graph t-SNE, seeded (Q42) |
| [05-layout-3d](05-layout-3d) | Is a 3D map worth it? | 3D t-SNE took 18 min at 1M with lower recall than 2D. 3D UMAP took 17 s, the globe 55 s. User studies favor 2D on flat screens. | 2D map; 3D stars later (Q43) |
| [06-toy-and-viewers](06-toy-and-viewers) | Why t-SNE over spectral? Explore the layouts by hand | An 800-site toy comparison (`spectral_vs_tsne.png`), spectral in 2D/3D/16D, 3D UMAP, an Embedding Atlas builder and a deck.gl 3D viewer. | Supports Q42/Q43 |

**The metrics used across the layout spikes:**
- **Purity:** the share of a dot's 10 nearest dots that are in its own community.
- **Neighbor recall:** the share of a site's linked sites among its k nearest dots (k = degree, capped at 30). Random placement within the right community scores 0.012 at 1M.
- **Edge-length ratio:** mean link length ÷ mean distance between random pairs.

## Research notes

These are the online research tasks from the review, with their main sources.

**Database choice (Q21).** The workload is shaped like a graph but doesn't need a graph engine: tiny writes, 1–2-hop reads, and the analytics run in igraph. PostgreSQL + CloudNativePG won on online backups, a mature Rust client, `COPY` export speed and disk-based scaling.
- Neo4j Community's backups are offline only ([docs](https://neo4j.com/docs/operations-manual/current/backup-restore/offline-backup/)).
- Memgraph and FalkorDB are in-memory, about 50–60 GB at 10M sites ([Memgraph sizing](https://memgraph.com/docs/fundamentals/storage-memory-usage)).
- Kùzu was archived in October 2025 ([The Register](https://www.theregister.com/software/2025/10/14/kuzudb-graph-database-abandoned-community-mulls-options/1142229)).
- Apache AGE is thinly maintained ([discussion](https://github.com/apache/age/discussions/2305)).

**Storing and serving positions (Q41.5).**
- "DB as the source of truth, cached artifact served" is standard GIS practice (e.g. the [Martin](https://maplibre.org/martin/architecture/) tile server).
- Point-map tools ship immutable Arrow or Parquet files, tiled at scale ([deepscatter](https://github.com/nomic-ai/deepscatter), [Embedding Atlas](https://apple.github.io/embedding-atlas/)).
- Postgres partitions avoid the churn of daily bulk loads ([docs](https://www.postgresql.org/docs/current/ddl-partitioning.html)).
- deck.gl handles about 1M points at 60 fps ([performance guide](https://deck.gl/docs/developer-guide/performance)).

**Layout methods (Q42).**
- Graph neighbor embedding had the best neighbor recall on a 727k-node graph ([Böhm et al., TMLR 2025](https://arxiv.org/abs/2503.23822)); it runs on [openTSNE](https://opentsne.readthedocs.io/).
- Published results show sfdp at 5,745 s on a 4M-node graph ([t-FDP](https://arxiv.org/html/2303.03964)).
- The Internet Map (2012) used a custom force simulation that ran for weeks ([Habr](https://habr.com/ru/articles/148351/)).

**3D (Q43).**
- openTSNE's fast FFT mode is 2D-only, and FIt-SNE, tsnecuda and cuML TSNE are too.
- SG-t-SNE-Π does 3D ([paper](https://arxiv.org/abs/1906.05582)).
- anvaka's [pm](https://github.com/anvaka/pm) renders 1.1M-node 3D galaxies.
- The usability evidence favors 2D without stereo or VR, e.g. [Cockburn & McKenzie](https://www.csse.canterbury.ac.nz/andrew.cockburn/papers/ijhcs2D3D.pdf) on spatial memory.
