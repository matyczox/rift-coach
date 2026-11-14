"""Regression tests for replay labels, ensemble construction and warning metrics."""
import importlib.util
import json
import math
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


labels = module('prepare-replay-labels')
metrics = module('eval-gank-approaches')
trainer = module('train-replay-tracker')


class ReplayTrainingTests(unittest.TestCase):
    def cache(self, folder, points, blue=None):
        red = {str(t): [dict(c='Enemy', t='CHAOS', x=x, y=y, s=s)] for t, x, y, s in points}
        (folder/'detections-red.json').write_text(json.dumps(red))
        (folder/'detections-frames.json').write_text(json.dumps(blue or {str(t): [] for t, *_ in points}))

    def test_weak_teacher_is_removed_and_raw_cache_is_untouched(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            self.cache(p, [(0,.8,.8,.95),(1000,.81,.8,.94),(2000,.1,.1,.5)])
            before = (p/'detections-red.json').read_bytes()
            report = labels.prepare(p)
            self.assertEqual(report['accepted'], 2)
            self.assertEqual(report['lowScore'], 1)
            self.assertEqual(before, (p/'detections-red.json').read_bytes())

    def test_isolated_high_score_teleport_is_not_a_teacher(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            self.cache(p, [(0,.2,.2,.95),(1000,.9,.9,.96),(2000,.21,.2,.95)])
            labels.prepare(p)
            self.assertEqual(json.loads((p/'detections-truth.json').read_text())['1000'], [])

    def test_conflicting_visible_identity_is_not_used_as_truth(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d)
            blue = {'0': [dict(c='Enemy',t='CHAOS',x=.1,y=.1,s=.98)], '1000': []}
            self.cache(p, [(0,.8,.8,.95),(1000,.8,.8,.95)], blue)
            report = labels.prepare(p)
            self.assertEqual(report['conflictsWithVisibleEnemy'], 1)

    def test_equal_game_ensemble_does_not_depend_on_label_count(self):
        def model(weight, count):
            return dict(version=1,multipliers={'JUNGLE|early':{'camp':weight}},
                        score=dict(n=count,logModel=0,logBaseline=0,hits=0),recent=[])
        self.assertAlmostEqual(trainer.ensemble([model(4,1000),model(.25,10)])['multipliers']['JUNGLE|early']['camp'],1)
        self.assertAlmostEqual(trainer.ensemble([model(4,1000),model(1,10)])['multipliers']['JUNGLE|early']['camp'],math.sqrt(2))

    def test_approach_confirmation_requires_duration_and_resets_after_a_turn(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'trace.jsonl'
            frames=[]
            for t, raw in [(100,True),(100.8,True),(101.6,True),(101.7,False),(101.8,True)]:
                frames.append(dict(t=t,me=dict(x=.5,y=.5,score=.95),enemies=[dict(c='Enemy',role='JUNGLE',approach=raw)]))
            p.write_text('\n'.join(json.dumps(f) for f in frames))
            flags=[r['enemy']['approach'] for r in metrics.load(p)['Enemy']]
            self.assertEqual(flags,[False,False,True,False,False])

    def test_caitlyn_kaisa_and_their_support_are_not_bot_lane_ganks(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'trace.jsonl'
            enemies=[dict(c=c,role=role,approach=True) for c,role in
                     [('Caitlyn','BOTTOM'),("Kai'Sa",'BOTTOM'),('Sona','UTILITY')]]
            frames=[dict(t=t,me=dict(x=.85,y=.88,score=.95),enemies=enemies) for t in [200,201,202]]
            p.write_text('\n'.join(json.dumps(f) for f in frames))
            by=metrics.load(p,'UTILITY')
            self.assertEqual(metrics.score(by)['eventsN'],0)
            self.assertEqual(metrics.score(by)['warningEpisodes'],0)

    def test_warning_is_confirmed_by_contact_after_its_last_refresh(self):
        rows=[]
        for t in [200,202,204,206,208,210]:
            rows.append(dict(t=t,me=dict(x=.5,y=.5),lane=True,enemy=dict(c='Enemy',role='JUNGLE',
                state='fog',fogSince=190,estimate={},dead=False,mass=.8 if t<=204 else 0,
                forecast=None,forecastCurrent=0,blueScore=None,truthVisible=0,
                truth=dict(x=.6 if t==210 else .9,y=.5 if t==210 else .9,score=.95))))
        result=metrics.score({'Enemy':rows})
        self.assertEqual(result['confirmed'],1)
        self.assertEqual(result['falseWarnings'],0)


if __name__ == '__main__':
    unittest.main()
