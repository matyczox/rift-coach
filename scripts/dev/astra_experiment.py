"""Reproducible, pre-specified nested LOO experiment; no production writes.

Run astra_guard.py build and baseline first. All results include immutable
input hashes. Training scores are never used as evaluation scores.
"""
from __future__ import annotations
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import subprocess
import threading
import zlib
from astra_guard import REPO, GAMES, guarded_run, data_env

EMPTY = {"version": 1, "multipliers": {}, "score": {"n": 0, "logModel": 0,
         "logBaseline": 0, "hits": 0}, "recent": []}
BINS = [5, 15, 30, 60]
VARIANTS = ["none", "priors", "zero+train", "priors+train", "astra", "astra+train",
            "bag-zero-a025", "bag-zero-a050", "bag-zero-a100", "bag-astra-a050", "nested"]
BAGS = {"bag-zero-a025": ("zero", .25), "bag-zero-a050": ("zero", .5),
        "bag-zero-a100": ("zero", 1.), "bag-astra-a050": ("astra", .5)}
PRINT_LOCK = threading.Lock()


def read(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def write(path, obj):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def clean_model(model):
    out = json.loads(json.dumps(EMPTY))
    out["multipliers"] = model["multipliers"]
    for row in out["multipliers"].values():
        for value in row.values():
            assert math.isfinite(value) and .2 <= value <= 5
    return out


def blend(start, fitted, alpha):
    """Equal-game geometric aggregation, shrink toward starting prior in log space."""
    out = json.loads(json.dumps(EMPTY))
    rows = sorted(set(start["multipliers"]).union(*(m["multipliers"] for m in fitted)))
    for row in rows:
        kinds = sorted(set(start["multipliers"].get(row, {})).union(
            *(m["multipliers"].get(row, {}) for m in fitted)))
        out["multipliers"][row] = {}
        for kind in kinds:
            s = start["multipliers"].get(row, {}).get(kind, 1.)
            avg = sum(math.log(m["multipliers"].get(row, {}).get(kind, 1.))
                      for m in fitted) / len(fitted)
            out["multipliers"][row][kind] = min(5., max(.2,
                math.exp((1 - alpha) * math.log(s) + alpha * avg)))
    return clean_model(out)


def aggregate(summaries):
    rows = {}
    for lo in BINS:
        bucket = [next(b for b in s["buckets"] if b["lo"] == lo) for s in summaries]
        n = sum(b["n"] for b in bucket)
        rows[lo] = {"n": n,
                    "errModel": sum(b["n"] * b["errModel"] for b in bucket) / n,
                    "errBase": sum(b["n"] * b["errBase"] for b in bucket) / n,
                    "macro": sum(b["errModel"] for b in bucket) / len(bucket)}
    return rows


def same_support(candidates, base):
    for a, b in zip(candidates, base):
        a, b = aggregate([a]), aggregate([b])
        for lo in BINS:
            if a[lo]["n"] != b[lo]["n"] or abs(a[lo]["errBase"] - b[lo]["errBase"]) > 1e-12:
                return False
    return True


def select(candidates, ordered):
    base = aggregate(candidates["none"])
    diagnostics = {}
    eligible = []
    for name in ordered:
        rows = aggregate(candidates[name])
        gains = {lo: base[lo]["errModel"] - rows[lo]["errModel"] for lo in BINS}
        support_ok = same_support(candidates[name], candidates["none"])
        passes = support_ok and gains[5] >= 0 and all(gains[lo] >= .01 for lo in [15, 30, 60])
        score = sum(w * rows[lo]["errModel"] for w, lo in [(1, 15), (2, 30), (3, 60)]) / 6
        diagnostics[name] = {"gain_pp": {str(k): v * 100 for k, v in gains.items()},
                             "same_support": support_ok, "eligible": passes, "score": score}
        if passes:
            eligible.append((score, ordered.index(name), name))
    return (min(eligible)[2] if eligible else "none"), diagnostics


class Experiment:
    def __init__(self, appdata, out, workers):
        self.env = data_env(appdata)
        self.out = out.resolve()
        self.workers = workers
        self.exe = REPO / "rust/target/release/examples/eval_tracker.exe"
        self.exe_sha = sha(self.exe)
        self.protocol = read(REPO / "model/astra-protocol.json")
        self.starts = {"zero": EMPTY, "astra": read(REPO / "model/astra-v1.json")}
        self.fixed = {"none": None, "priors": REPO / "model/priors-v1.json",
                      "astra": REPO / "model/astra-v1.json"}
        self.singles = {}
        self.outer = {}
        self.models = {}
        self.inner = {}

    def parallel(self, tasks):
        with ThreadPoolExecutor(max_workers=self.workers) as pool:
            return list(pool.map(lambda f: f(), tasks))

    def run_eval(self, game, name, model=None, training=False, lineage=()):
        assert training or game not in lineage, (game, lineage)
        assert sha(self.exe) == self.exe_sha, "Evaluator binary changed during experiment"
        path = self.out / "runs" / f"{name}.json"
        env = self.env.copy()
        input_sha = sha(model) if model else None
        if model:
            env["MODEL"] = str(Path(model).resolve())
        if training:
            env["TRAIN"] = "1"
        identity = {"game": game, "model_in_sha256": input_sha,
                    "training": training, "lineage": list(lineage), "exe_sha256": self.exe_sha}
        if path.exists() and not training:
            cached = read(path)
            assert cached["identity"] == identity, f"Stale cached result: {path}"
            return cached["summary"]
        output = guarded_run([str(self.exe), game], path.with_suffix(".log"), env)
        matches = [json.loads(s[8:]) for s in output.splitlines() if s.startswith("SUMMARY ")]
        assert len(matches) == 1, path
        assert sha(self.exe) == self.exe_sha
        if not training and model:
            assert sha(model) == input_sha, "Evaluation modified model"
        result = {"identity": identity, "summary": matches[0],
                  "model_out_sha256": sha(model) if model else None}
        write(path, result)
        with PRINT_LOCK:
            print("DONE", name, "train" if training else "test", flush=True)
        return matches[0]

    def train(self, origin, games, name):
        target = self.out / "models" / (name + ".json")
        marker = target.with_suffix(".done.json")
        start = self.starts[origin]
        identity = {"origin": start, "games": list(games), "exe_sha256": self.exe_sha}
        if marker.exists():
            saved = read(marker)
            assert saved["identity"] == identity and saved["sha256"] == sha(target)
            return target
        write(target, start)
        for i, game in enumerate(games):
            self.run_eval(game, f"train-{name}-{i}", target, True, games[:i])
        write(marker, {"identity": identity, "sha256": sha(target)})
        return target

    def bag(self, name, games, target):
        origin, alpha = BAGS[name]
        models = [read(self.singles[origin, g]) for g in games]
        write(target, blend(self.starts[origin], models, alpha))
        return target

    def fingerprint(self):
        root = Path(self.env["APPDATA"]) / "Rift Coach"
        recordings = {}
        for game in GAMES:
            folder = root / "recordings" / game
            # Read-only inventory covers even files not consumed by the evaluator.
            files = [(p.relative_to(folder).as_posix(), p.stat().st_size, p.stat().st_mtime_ns)
                     for p in sorted(folder.rglob("*")) if p.is_file()]
            hashes = {name: sha(folder / name) for name in
                      ["api.jsonl.gz", "detections-frames.json", "detections-red.json", "meta.json"]}
            recordings[game] = {"files": files, "sha256": hashes}
        version = (root / "ddragon/latest.txt").read_text().strip()
        code = list((REPO / "rust/crates/rc-vision/src/tracker").glob("*.rs"))
        code += [REPO / "rust/crates/rc-vision/examples/eval_tracker.rs",
                 REPO / "rust/crates/rc-vision/examples/util/session.rs"]
        return {"appdata": self.env["APPDATA"], "recordings": recordings,
                "ddragon": {"version": version, "sha256": sha(root / f"ddragon/{version}-pl_PL.json")},
                "exe_sha256": self.exe_sha,
                "code_sha256": {p.relative_to(REPO).as_posix(): sha(p) for p in code},
                "priors_sha256": sha(REPO / "model/priors-v1.json"),
                "astra_sha256": sha(REPO / "model/astra-v1.json"),
                "protocol_sha256": sha(REPO / "model/astra-protocol.json")}

    def baseline(self):
        log = (self.out / "baseline.log").read_text(encoding="utf-8")
        for line in log.splitlines():
            match = re.match(r"TEST (\S+) (\S+) (\{.*\})$", line)
            if match:
                game, variant, text = match.groups()
                self.outer[variant, game] = json.loads(text)
        assert len(self.outer) == 12
        for v in self.protocol["baseline_variants"]:
            for g in GAMES:
                assert (v, g) in self.outer

    def execute(self):
        before = self.fingerprint()
        provenance = self.out / "provenance.json"
        if provenance.exists():
            assert read(provenance) == before, "Inputs changed since previous run"
        else:
            write(provenance, before)
        self.baseline()
        # Per-game fitted models are reused only where that game belongs to TRAIN.
        jobs = [(o, g) for o in ["zero", "astra"] for g in GAMES]
        results = self.parallel([lambda o=o, g=g: self.train(o, [g], f"single-{o}-{GAMES.index(g)}")
                                 for o, g in jobs])
        self.singles.update(zip(jobs, results))
        for game in GAMES:
            train_games = [g for g in GAMES if g != game]
            self.models["astra+train", game] = self.train("astra", train_games,
                                                         f"sequential-astra-{GAMES.index(game)}")
            for v in BAGS:
                self.models[v, game] = self.bag(v, train_games,
                    self.out / "models" / f"outer-{v}-{GAMES.index(game)}.json")
        jobs = [(v, g) for v in ["astra", "astra+train", *BAGS] for g in GAMES]
        results = self.parallel([lambda v=v, g=g: self.run_eval(g,
            f"outer-{v}-{GAMES.index(g)}", self.fixed[v] if v == "astra" else self.models[v, g],
            lineage=[] if v == "astra" else [t for t in GAMES if t != g]) for v, g in jobs])
        self.outer.update(zip(jobs, results))
        # All six directed inner train/validation pairs. Test identity is not used
        # in fitting, shrinkage, candidate definitions or the inner selector.
        jobs = [(v, t, val) for v in BAGS for t in GAMES for val in GAMES if t != val]
        inner_models = {}
        for v, t, val in jobs:
            inner_models[v, t, val] = self.bag(v, [t], self.out / "models" /
                f"inner-{v}-train{GAMES.index(t)}-val{GAMES.index(val)}.json")
        results = self.parallel([lambda v=v, t=t, val=val: self.run_eval(val,
            f"inner-{v}-train{GAMES.index(t)}-val{GAMES.index(val)}", inner_models[v, t, val],
            lineage=[t]) for v, t, val in jobs])
        self.inner.update(zip(jobs, results))
        decisions = {}
        ordered = self.protocol["nested_candidates"]
        for test in GAMES:
            train_games = [g for g in GAMES if g != test]
            options = {}
            for v in ordered:
                if v in self.fixed:
                    options[v] = [self.outer[v, g] for g in train_games]
                else:
                    a, b = train_games
                    options[v] = [self.inner[v, b, a], self.inner[v, a, b]]
            choice, details = select(options, ordered)
            self.outer["nested", test] = self.outer[choice, test]
            decisions[test] = {"choice": choice, "inner_training_games": train_games,
                               "diagnostics": details}
        final_options = {v: [self.outer[v, g] for g in GAMES] for v in ordered}
        choice, details = select(final_options, ordered)
        final_path = REPO / "model/astra-best.json"
        if choice == "none":
            # Explicit fallback: the identity model exactly implements no learned weights.
            write(final_path, EMPTY)
        elif choice in self.fixed:
            write(final_path, clean_model(read(self.fixed[choice])))
        else:
            self.bag(choice, GAMES, final_path)
        after = self.fingerprint()
        assert before == after, "Read-only experiment inputs changed during execution"
        write(self.out / "input-integrity.json", {"unchanged": True})
        combined = {"outer": [{"variant": v, "test": g, "summary": self.outer[v, g]}
                                for v in VARIANTS for g in GAMES],
                    "nested_decisions": decisions, "final_choice": choice,
                    "final_diagnostics": details, "best_sha256": sha(final_path),
                    "aggregate": {v: aggregate([self.outer[v, g] for g in GAMES]) for v in VARIANTS},
                    "support": {v: same_support([self.outer[v, g] for g in GAMES],
                                                [self.outer["none", g] for g in GAMES]) for v in VARIANTS}}
        write(REPO / "model/astra-results.json", combined)
        write(self.out / "results.json", combined)
        self.tables(combined)
        print("FINAL CHOICE", choice, flush=True)

    def tables(self, results):
        lines = ["| Wariant | 5–15 s | 15–30 s | 30–60 s | 60–120 s |",
                 "|---|---:|---:|---:|---:|"]
        for v in VARIANTS:
            rows = results["aggregate"][v]
            lines.append("| " + v + " | " + " | ".join(f"{rows[lo]['errModel']*100:.2f}" for lo in BINS) + " |")
        base = results["aggregate"]["none"]
        lines += ["| Ostatnia widziana pozycja | " + " | ".join(f"{base[lo]['errBase']*100:.2f}" for lo in BINS) + " |",
                  "| Liczba próbek | " + " | ".join(str(base[lo]['n']) for lo in BINS) + " |"]
        (self.out / "aggregate-table.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
        lines = ["| Mecz | Wariant | 5–15 s | 15–30 s | 30–60 s | 60–120 s |",
                 "|---|---|---:|---:|---:|---:|"]
        for g in GAMES:
            for v in VARIANTS:
                rows = aggregate([self.outer[v, g]])
                lines.append(f"| G{GAMES.index(g)+1} | {v} | " + " | ".join(f"{rows[lo]['errModel']*100:.2f}" for lo in BINS) + " |")
            rows = aggregate([self.outer["none", g]])
            lines.append(f"| G{GAMES.index(g)+1} | Ostatnia pozycja | " + " | ".join(f"{rows[lo]['errBase']*100:.2f}" for lo in BINS) + " |")
        (self.out / "per-game-table.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--appdata", required=True)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--workers", type=int, default=2, choices=[1, 2])
    args = parser.parse_args()
    Experiment(args.appdata, args.out, args.workers).execute()
