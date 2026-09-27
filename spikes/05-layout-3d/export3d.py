"""python export3d.py RESULT_NAME FOLDER LABEL [globe] -> ../mock-map/3d/FOLDER/{positions.f32,meta.json}"""
import sys, os, json, numpy as np
name, folder, label = sys.argv[1:4]; globe = len(sys.argv) > 4
r = json.load(open(f"results/g1m_{name}.json")); xyz = np.load(f"data/g1m_{name}.npy").astype(np.float64)
if globe:
    xyz = xyz / np.linalg.norm(xyz, axis=1, keepdims=True) * 100          # sphere of radius 100
else:
    xyz -= np.median(xyz, 0); xyz = xyz / np.percentile(np.abs(xyz), 99.5) * 100
out = os.path.join("..", "mock-map", "3d", folder); os.makedirs(out, exist_ok=True)
xyz.astype("<f4").tofile(os.path.join(out, "positions.f32"))
json.dump({"n": int(r["N"]), "seconds": r["layout_s"], "purity": r["purity"], "recall": r["recall"], "label": label},
          open(os.path.join(out, "meta.json"), "w"))
print(out, os.path.getsize(os.path.join(out, "positions.f32")), open(os.path.join(out, "meta.json")).read())
