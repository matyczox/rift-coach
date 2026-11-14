"""Train equal-game ensembles and evaluate only on held-out matches.

py scripts/dev/train-replay-tracker.py --out work/training --trace SESSION SESSION ...
Build eval_tracker first; prepare-replay-labels.py must have been run on sessions.
The final ensemble excludes no games, so its training score is never a test score.
"""
import argparse
import json
import math
import os
import subprocess
from pathlib import Path


def ensemble(models):
    result = dict(version=1, multipliers={}, score={k: sum(m['score'][k] for m in models)
                  for k in ('n', 'logModel', 'logBaseline', 'hits')}, recent=[])
    for context in set().union(*(m['multipliers'] for m in models)):
        kinds = set().union(*(m['multipliers'].get(context, {}) for m in models))
        result['multipliers'][context] = {
            k: math.exp(.5 * sum(math.log(m['multipliers'].get(context, {}).get(k, 1))
                               for m in models) / len(models)) for k in kinds}
    result['recent'] = sorted([r for m in models for r in m['recent']],
                              key=lambda r: r['at'], reverse=True)[:200]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('sessions', nargs='+', type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--trace', action='store_true')
    parser.add_argument('--exe', type=Path, default=Path(__file__).resolve().parents[2] /
                        'rust/target/release/examples/eval_tracker.exe')
    args = parser.parse_args()
    assert len(args.sessions) >= 2, 'at least two distinct matches required'
    assert len({p.resolve() for p in args.sessions}) == len(args.sessions), 'duplicate sessions'
    args.out.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    for k in ('MODEL', 'TRAIN', 'TRACE', 'MY_CHAMPION'):
        env.pop(k, None)
    env['TRUTH'] = 'truth'
    empty = dict(version=1, multipliers={}, score=dict(n=0, logModel=0, logBaseline=0, hits=0), recent=[])
    neutral = args.out / 'neutral.json'
    neutral.write_text(json.dumps(empty))

    def run(label, session, model, train=False):
        e = env.copy()
        e['MODEL'] = str(model.resolve())
        meta = json.loads((session / 'meta.json').read_text())
        e['MY_CHAMPION'] = meta.get('champion') or 'Leona'
        if train:
            e['TRAIN'] = '1'
        elif args.trace:
            e['TRACE'] = str((args.out / (label + '.jsonl')).resolve())
        proc = subprocess.run([str(args.exe.resolve()), str(session.resolve())],
                              env=e, text=True, capture_output=True)
        (args.out / (label + '.log')).write_text(proc.stdout, encoding='utf8')
        (args.out / (label + '.err')).write_text(proc.stderr, encoding='utf8')
        if proc.returncode:
            raise RuntimeError(label + ': ' + proc.stderr[-1000:])
        print(label + ' complete', flush=True)
        return next(json.loads(l[8:]) for l in proc.stdout.splitlines() if l.startswith('SUMMARY '))

    models = []
    for i, session in enumerate(args.sessions):
        assert (session / 'detections-truth.json').exists(), 'prepare high-quality labels first'
        model = args.out / f'train-{i}.json'
        model.write_text(json.dumps(empty))
        run(f'train-{i}', session, model, train=True)
        models.append(json.loads(model.read_text()))
    report = dict(recipe='equal-game geometric mean, half-log shrinkage', absenceHz=env.get('ABSENCE_HZ','default'), folds=[])
    for i, session in enumerate(args.sessions):
        fold = args.out / f'fold-{i}.json'
        fold.write_text(json.dumps(ensemble([m for j, m in enumerate(models) if i != j]), indent=2))
        report['folds'].append(dict(session=session.name,
                                   neutral=run(f'neutral-{i}', session, neutral),
                                   trained=run(f'heldout-{i}', session, fold)))
        (args.out / 'cross-validation.json').write_text(json.dumps(report, indent=2))
    (args.out / 'candidate.json').write_text(json.dumps(ensemble(models), indent=2))
    print('Cross-validation and final candidate complete', flush=True)


if __name__ == '__main__':
    main()
