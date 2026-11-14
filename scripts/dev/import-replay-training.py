"""Import prepared replay observations/truth, retaining backups of replaced files.

Use --model only while Rift Coach is stopped: it loads weights into memory.
The prepared directory must contain manifest.json, observations/ and replay-truth/.
"""
import argparse
import base64
import datetime
import gzip
import hashlib
import json
import math
import os
from pathlib import Path


def atomic_write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + '.import-tmp')
    temp.write_bytes(data)
    os.replace(temp, path)


def import_data(prepared, data_dir, model=None):
    manifest = json.loads((prepared/'manifest.json').read_text(encoding='utf8'))
    seen = set()
    files = []
    for entry in manifest['games']:
        gid = entry['gameId']
        assert type(gid) is int and gid > 0 and gid not in seen, 'duplicate/invalid game id'
        seen.add(gid)
        name = str(gid)+'.json.gz'
        raw = (prepared/'observations'/name).read_bytes()
        assert hashlib.sha256(raw).hexdigest() == entry['observationSha256']
        log = json.loads(gzip.decompress(raw))
        assert log['gameId'] == gid and len(log['frames']) == entry['frames'] >= 120
        assert log['team'] in ('ORDER','CHAOS') and log['enemyTeam'] in ('ORDER','CHAOS') and log['team']!=log['enemyTeam']
        for f in log['frames']:
            assert math.isfinite(f['t']) and f['t']>=0
            assert len(base64.b64decode(f['m'],validate=True))==128
            assert len(f['e'])==5
            for c,role,dead,speed,gold in f['e']:
                assert isinstance(c,str) and isinstance(role,str) and type(dead) is int and dead in (0,1)
                assert math.isfinite(speed) and speed>0 and math.isfinite(gold) and gold>=0
            for c,x,y in f['d']:
                assert math.isfinite(x) and math.isfinite(y) and 0<=x<=1 and 0<=y<=1 and any(e[0]==c for e in f['e'])
        assert all(b['t']>a['t'] for a,b in zip(log['frames'],log['frames'][1:]))
        truth = (prepared/'replay-truth'/name).read_bytes()
        assert hashlib.sha256(truth).hexdigest() == entry['truthSha256']
        assert isinstance(json.loads(gzip.decompress(truth)),dict)
        files.extend([(Path('observations')/name,raw),(Path('replay-truth')/name,truth)])
    if model is not None:
        raw = model.read_bytes()
        weights = json.loads(raw)
        assert weights['version']==1 and weights['score']['n']>0
        assert all(math.isfinite(v) and .2<=v<=5 for row in weights['multipliers'].values() for v in row.values())
        files.append((Path('fog-model.json'),raw))
    files.append((Path('replay-imports.json'),json.dumps(manifest,indent=2).encode('utf8')))
    root = data_dir/'model'
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    backup = root/'backups'/('replay-import-'+stamp)
    for relative, raw in files:
        target = root/relative
        if target.exists():
            atomic_write(backup/relative,target.read_bytes())
        atomic_write(target,raw)
    return dict(importedGameIds=sorted(seen),observedGames=len(list((root/'observations').glob('*.json.gz'))),
                modelActivated=model is not None,backup=str(backup))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('prepared',type=Path)
    p.add_argument('--data-dir',type=Path,default=Path(os.environ['APPDATA'])/'Rift Coach')
    p.add_argument('--model',type=Path)
    a=p.parse_args()
    print(json.dumps(import_data(a.prepared,a.data_dir,a.model),indent=2))
