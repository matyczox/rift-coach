"""Run offline experiments; suspend only our child process tree while LoL runs.

Requires Python 3.11+ and psutil. Does not control the game or Rift Coach.
"""
from __future__ import annotations
import os
from pathlib import Path
import subprocess
import time
import psutil

REPO = Path(__file__).resolve().parents[2]
GAMES = ["replay-4027208890", "replay-unknown-2026-11-12T09-21-59",
         "replay-unknown-2026-11-12T01-27-38"]


def game_running():
    return any((p.info["name"] or "").lower() == "league of legends.exe"
               for p in psutil.process_iter(["name"]))


def guarded_run(command, log, env=None):
    log = Path(log)
    log.parent.mkdir(parents=True, exist_ok=True)
    waiting = False
    while game_running():
        if not waiting:
            print("PAUSED: League of Legends.exe is running", flush=True)
            waiting = True
        time.sleep(1)
    if waiting:
        print("RESUMED: game exited", flush=True)
    print("RUN", subprocess.list2cmdline([str(x) for x in command]), flush=True)
    with log.open("w", encoding="utf-8") as output:
        proc = subprocess.Popen(command, cwd=REPO, env=env, stdout=output,
                                stderr=subprocess.STDOUT,
                                creationflags=subprocess.CREATE_NO_WINDOW)
        root = psutil.Process(proc.pid)
        suspended = {}
        try:
            while proc.poll() is None:
                if game_running():
                    if not suspended:
                        print("PAUSED: suspending experiment process tree", flush=True)
                    try:
                        # Stop the parent before children to prevent new launches.
                        children = [root] + root.children(recursive=True)
                    except psutil.NoSuchProcess:
                        children = []
                    for child in children:
                        if child.pid not in suspended:
                            try:
                                child.suspend()
                                suspended[child.pid] = child
                            except psutil.NoSuchProcess:
                                pass
                elif suspended:
                    for child in reversed(list(suspended.values())):
                        try:
                            child.resume()
                        except psutil.NoSuchProcess:
                            pass
                    suspended.clear()
                    print("RESUMED: experiment process tree", flush=True)
                time.sleep(0.25)
        finally:
            if proc.poll() is None:
                for child in reversed(root.children(recursive=True)):
                    try:
                        child.kill()
                    except psutil.NoSuchProcess:
                        pass
                proc.kill()
                proc.wait()
        if proc.returncode:
            raise RuntimeError(f"Command failed ({proc.returncode}): {log}\n" +
                               log.read_text(encoding="utf-8", errors="replace")[-6000:])
    return log.read_text(encoding="utf-8", errors="replace")


def data_env(appdata):
    env = os.environ.copy()
    for name in ("MODEL", "TRAIN", "NO_ITEMS", "CAMPS", "TRUTH"):
        env.pop(name, None)
    env["APPDATA"] = str(Path(appdata).resolve())
    env["TRUTH"] = "red"
    return env


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("stage", choices=["build", "baseline"])
    parser.add_argument("--appdata", required=True)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    env = data_env(args.appdata)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    if args.stage == "build":
        guarded_run(["powershell", "-ExecutionPolicy", "Bypass", "-File",
                     "rust/app/cargo-gnu.ps1", "build", "--release", "-p", "rc-vision",
                     "--example", "eval_tracker", "--example", "detect_cache"],
                    args.out / "build.log", env)
    else:
        for game in GAMES:
            session = Path(env["APPDATA"]) / "Rift Coach" / "recordings" / game
            for name in ("api.jsonl.gz", "detections-frames.json", "detections-red.json"):
                if not (session / name).is_file():
                    raise FileNotFoundError(session / name)
        # Original script removes TEMP/rc-loo. Isolate it in a new, owned scratch
        # parent, and check the final absolute removal target before invoking bash.
        scratch = args.out / "baseline-scratch"
        scratch.mkdir(exist_ok=False)
        removal_target = (scratch / "rc-loo").resolve()
        assert removal_target.is_relative_to(args.out) and not removal_target.exists()
        env["TEMP"] = scratch.as_posix()
        text = guarded_run([r"C:\Program Files\Git\bin\bash.exe",
                            "scripts/dev/loo-tracker.sh"], args.out / "baseline.log", env)
        lines = [line for line in text.splitlines() if line.startswith("TEST ")]
        if len(lines) != 12 or any('"buckets"' not in line for line in lines):
            raise RuntimeError("Incomplete baseline output; inspect baseline.log")
        print(text, flush=True)
