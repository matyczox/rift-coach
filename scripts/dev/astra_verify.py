"""Verify lineage, read-only inputs and exact identity-model reproducibility."""
import argparse
from pathlib import Path
from astra_experiment import Experiment, GAMES, REPO, read, write, sha

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--appdata", required=True)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    e = Experiment(args.appdata, args.out, 2)
    results = read(REPO / "model/astra-results.json")
    e.baseline()
    checked = 0
    for path in (e.out / "runs").glob("*.json"):
        identity = read(path)["identity"]
        if not identity["training"]:
            assert identity["game"] not in identity["lineage"], path
            assert identity["exe_sha256"] == e.exe_sha, path
            checked += 1
    best = REPO / "model/astra-best.json"
    assert sha(best) == results["best_sha256"]
    equality = {}
    if results["final_choice"] == "none":
        assert read(best)["multipliers"] == {}
        summaries = e.parallel([lambda g=g: e.run_eval(g, f"verify-identity-{GAMES.index(g)}", best)
                                for g in GAMES])
        for game, summary in zip(GAMES, summaries):
            equality[game] = summary == e.outer["none", game]
        assert all(equality.values()), equality
    assert e.fingerprint() == read(e.out / "provenance.json")
    summary = {"lineage_checks_passed": checked,
               "neutral_model_exactly_matches_no_model": equality,
               "input_manifest_unchanged": True,
               "best_sha256": sha(best), "exe_sha256": e.exe_sha}
    write(e.out / "verification.json", summary)
    write(REPO / "model/astra-verification.json", summary)
    print(summary)
