"""Build high-precision offline teacher labels; never used as tracker input.

Usage: py scripts/dev/prepare-replay-labels.py SESSION [SESSION ...]
Writes detections-truth.json and label-quality.json in each session. Raw caches
stay intact. Temporal support may use the next frame because these are labels,
not observations available to the player.
"""
import argparse
import bisect
import json
import math
from collections import Counter, defaultdict
from pathlib import Path


def distance(a, b):
    return math.hypot(a['x'] - b['x'], a['y'] - b['y'])


def supported(a, b):
    dt = abs(a[0] - b[0]) / 1000
    # Generous movement envelope, including fast champions and icon jitter.
    return 0 < dt <= 2 and distance(a[1], b[1]) <= .035 + .055 * dt


def prepare(folder):
    red = json.loads((folder / 'detections-red.json').read_text())
    blue = json.loads((folder / 'detections-frames.json').read_text())
    blue = {int(k): v for k, v in blue.items()}
    blue_times = sorted(blue)
    candidates = defaultdict(list)
    counts = Counter()
    for t in sorted(map(int, red)):
        for d in red[str(t)]:
            if d['t'] != 'CHAOS':
                continue
            counts['raw'] += 1
            if d['s'] < .75:
                counts['lowScore'] += 1
                continue
            candidates[d['c']].append((t, d))
    out = {k: [] for k in red}
    for sequence in candidates.values():
        for i, (t, d) in enumerate(sequence):
            neighbours = sequence[max(0, i-1):i] + sequence[i+1:i+2]
            if not any(supported((t, d), b) for b in neighbours):
                counts['noTemporalSupport'] += 1
                continue
            j = bisect.bisect_left(blue_times, t)
            near = blue_times[max(0, j-1):j+1]
            bt = min(near, key=lambda x: abs(x-t))
            contradictory = abs(bt-t) <= 300 and any(
                b['t'] == 'CHAOS' and b['c'] == d['c'] and b['s'] >= .85
                and distance(d, b) > .07 for b in blue[bt])
            if contradictory:
                counts['conflictsWithVisibleEnemy'] += 1
                continue
            out[str(t)].append(d)
            counts['accepted'] += 1
    report = dict(counts)
    report.update(minScore=.75, supportSeconds=2, maxTimeAlignmentMs=300,
                  acceptedFraction=counts['accepted']/max(counts['raw'], 1))
    (folder / 'detections-truth.json').write_text(json.dumps(out), encoding='utf8')
    (folder / 'label-quality.json').write_text(json.dumps(report, indent=2), encoding='utf8')
    return report


def self_test():
    p = lambda x, y: {'x': x, 'y': y}
    assert supported((1000, p(.5, .5)), (1800, p(.52, .5)))
    assert not supported((1000, p(.1, .1)), (1800, p(.9, .9)))
    assert not supported((1000, p(.5, .5)), (4000, p(.5, .5)))
    assert not supported((1000, p(.5, .5)), (1000, p(.5, .5)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('sessions', nargs='*', type=Path)
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print('Label consistency tests passed')
    for folder in args.sessions:
        print(folder.name, json.dumps(prepare(folder)))
