"""Replay proxy: off-lane enemies walking into 0.13-map reach of the player.
Not a human annotation of gank intent, and never a frame-level accuracy score.
"""
import json, math, pathlib, collections, argparse

def dist(a,b):return math.hypot(a['x']-b['x'],a['y']-b['y'])
def on_lane(p):
 return (p['x']>.82 and p['y']>.42) or (p['y']>.82 and p['x']>.42) or (p['x']<.18 and p['y']<.58) or (p['y']<.18 and p['x']<.58) or abs(p['x']+p['y']-1)<.1
def load(path,my_role='UTILITY',approach_hold=1.5):
 data=[]
 for line in pathlib.Path(path).read_text().splitlines():
  try:f=json.loads(line)
  except json.JSONDecodeError:continue
  me=f['me']
  if not me or me['score']<.75:continue
  if math.hypot(me['x']-.05,me['y']-.95)<.18 or math.hypot(me['x']-.95,me['y']-.05)<.18:continue
  for e in f['enemies']:
   if e['role']==my_role or {e['role'],my_role}<={'BOTTOM','UTILITY'}:continue
   data.append(dict(t=f['t'],me=me,enemy=e,lane=on_lane(me)))
 by=collections.defaultdict(list)
 for r in data:by[r['enemy']['c']].append(r)
 for rs in by.values():
  start=last=None
  for r in rs:
   e=r['enemy'];raw=e.get('approach',False)
   if raw:
    if last is None or r['t']-last>1.8:start=r['t']
    last=r['t'];e['approach']=r['t']-start>=approach_hold
   else:start=last=None
 return by

def events(by):
 out=[]
 for c,rs in by.items():
  last=-100
  for i,r in enumerate(rs):
   t=r['t'];e=r['enemy'];q=e['truth']
   if t<120 or t-last<25 or e['dead'] or not q or q['score']<.75 or dist(q,r['me'])>.13:continue
   before=[a for a in rs[max(0,i-70):i] if 2<=t-a['t']<=18 and a['enemy']['truth'] and not a['enemy']['dead']]
   far=[a for a in before if dist(a['enemy']['truth'],a['me'])>=.22]
   if not far:continue
   a=far[-1];old=a['enemy']['truth']
   # The enemy approached; merely walking into a stationary enemy is not a gank.
   if dist(old,a['me'])-dist(q,a['me'])<.04:continue
   if dist(old,q)>.055*(t-a['t'])+.04:continue
   future=[b for b in rs[i+1:i+10] if .5<=b['t']-t<=2 and b['enemy']['truth'] and dist(b['enemy']['truth'],b['me'])<.15]
   if not future:continue
   hidden=[b for b in before if 2<=t-b['t']<=8 and b['enemy']['blueScore'] is None and b['enemy']['truthVisible'] is not None and b['enemy']['truthVisible']<.52]
   if not hidden:continue
   out.append(dict(c=c,t=t,lane=r['lane'],warningOpportunities=len(hidden)));last=t
 return sorted(out,key=lambda x:x['t'])

def risk(r,threshold=.4,rise=.15,max_age=120):
 e=r['enemy'];age=r['t']-e['fogSince'] if e['fogSince'] is not None else 0
 if e.get('approach') and not e['dead']:return True
 if e['state']!='fog' or e['estimate'] is None or e['dead'] or age<(1.5 if e['role']=='JUNGLE' else 4):return False
 if (e['mass'] or 0)>.6:return True
 f=e['forecast'];cur=e['forecastCurrent'] or 0
 return bool(f and 1<=f[1]<=6 and age<=max_age and cur<.6 and f[0]>=threshold and f[0]-cur>=rise)

def score(by,threshold=.4,rise=.15,max_age=120,persistence=0):
 ev=events(by);episodes=[];history=[]
 for c,rs in by.items():
  start=last=None;active=False
  for r in rs:
   if risk(r,threshold,rise,max_age):
    if last is None or r['t']-last>2:start=r['t'];active=False
    last=r['t']
    if r['t']-start>=persistence:
     history.append((c,r['t']))
     if not active:episodes.append(dict(c=c,t=r['t'],end=r['t']));active=True
     else:episodes[-1]['end']=r['t']
 compact=episodes
 for event in ev:
  hits=[t for c,t in history if c==event['c'] and 2<=event['t']-t<=8]
  event.update(warned=bool(hits),lead=event['t']-min(hits) if hits else None)
 confirmed=unknown=0;fps=[]
 for w in compact:
  rs=by[w['c']];after=[r for r in rs if w['t']<=r['t']<=w['end']+8 and r['enemy']['truth'] and not r['enemy']['dead']]
  if any(dist(r['enemy']['truth'],r['me'])<=.13 for r in after):confirmed+=1
  elif len(after)>=3 and max(r['t'] for r in after)-min(r['t'] for r in after)>=4:fps.append(w)
  else:unknown+=1
 n=len(ev);hit=sum(e['warned'] for e in ev)
 return dict(events=ev,eventsN=n,warned=hit,recall=hit/n if n else None,warningEpisodes=len(compact),confirmed=confirmed,falseWarnings=len(fps),unscorableWarnings=unknown,precision=confirmed/(confirmed+len(fps)) if confirmed+len(fps) else None,falseWarningEvents=fps,policy=dict(threshold=threshold,rise=rise,maxAge=max_age,persistence=persistence))

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('trace');p.add_argument('--role',default='UTILITY');p.add_argument('--threshold',type=float,default=.4);p.add_argument('--rise',type=float,default=.15);p.add_argument('--max-age',type=float,default=120);p.add_argument('--persistence',type=float,default=0);p.add_argument('--approach-hold',type=float,default=1.5);p.add_argument('--out');a=p.parse_args()
 r=score(load(a.trace,a.role,a.approach_hold),a.threshold,a.rise,a.max_age,a.persistence)
 r['policy']['approachHoldSeconds']=a.approach_hold
 s=json.dumps(r,indent=2)
 if a.out:pathlib.Path(a.out).write_text(s)
 print(s)
