"""Read-only replay audit. Diagnostics only; never used to tune weights."""
import argparse
import bisect
import collections
import json
import math
from pathlib import Path
import zlib
from astra_guard import GAMES


def audit(appdata):
    root = Path(appdata) / "Rift Coach/recordings"
    out = []
    for game in GAMES:
        folder = root / game
        raw = (folder / "api.jsonl.gz").read_bytes()
        decoded = bytearray()
        while raw:
            inflater = zlib.decompressobj(31)
            decoded.extend(inflater.decompress(raw))
            raw = inflater.unused_data
        api = [json.loads(line) for line in decoded.splitlines() if line.strip()]
        roles, phases = collections.Counter(), collections.Counter()
        for row in api:
            for player in row["d"].get("allPlayers", []):
                if player.get("team") == "CHAOS":
                    roles[player.get("position", "")] += 1
            t = row["o"] / 1000
            phases["early" if t < 840 else "mid" if t < 1500 else "late"] += 1
        caches = {s: {int(k): v for k, v in json.loads(
            (folder / f"detections-{s}.json").read_text()).items()} for s in ["frames", "red"]}
        red_times = sorted(caches["red"])
        errors = []
        aligned = 0
        for t, detections in caches["frames"].items():
            if t < 30000:
                continue
            i = bisect.bisect_left(red_times, t)
            near = red_times[max(0, i-1): min(len(red_times), i+1)]
            rt = min(near, key=lambda r: abs(r-t))
            if abs(rt-t) > 300:
                continue
            aligned += 1
            for det in detections:
                if det["t"] != "CHAOS":
                    continue
                truth = next((d for d in caches["red"][rt]
                              if d["t"] == "CHAOS" and d["c"] == det["c"]), None)
                if truth:
                    errors.append(math.hypot(det["x"]-truth["x"], det["y"]-truth["y"]))
        errors.sort()
        out.append({"game": game, "api_rows": len(api),
            "first_game_time": api[0]["d"]["gameData"]["gameTime"],
            "last_game_time": api[-1]["d"]["gameData"]["gameTime"],
            "role_counts": dict(roles), "phase_api_rows": dict(phases),
            "frames": {s: len(v) for s, v in caches.items()},
            "aligned_blue_frames_ge30s": aligned,
            "visible_cross_view_pairs": len(errors),
            "visible_cross_view_mean_pct": sum(errors)/len(errors)*100,
            "visible_cross_view_median_pct": errors[len(errors)//2]*100,
            "visible_cross_view_p90_pct": errors[int(len(errors)*.9)]*100,
            "visible_cross_view_gt5pct_fraction": sum(e > .05 for e in errors)/len(errors),
            "visible_cross_view_gt10pct_fraction": sum(e > .1 for e in errors)/len(errors)})
    return out


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--appdata", required=True)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    result = audit(args.appdata)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False, indent=2))
