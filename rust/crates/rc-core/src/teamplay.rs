//! Select one reachable ally to protect using the same fight estimate as the HUD.
use crate::intel::game_intel::GameIntel;
use serde::Serialize;

pub struct SupportCandidate {
    pub champion: String,
    pub position: Option<(f64,f64)>,
    pub distance: f64,
    pub p_me: f64,
    pub threatened: bool,
    pub dive: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct SupportDecision {
    pub champion: String,
    pub text: String,
    pub position: Option<(f64,f64)>,
    pub eta_sec: f64,
    pub local: bool,
    pub urgent: bool,
    pub approach: bool,
}

pub fn choose_support(intel: &GameIntel, candidates: &[SupportCandidate]) -> Option<SupportDecision> {
    let me = intel.me.as_ref().filter(|m| !m.is_dead)?;
    let my_hp = intel.my_stats.as_ref().filter(|s| s.max_hp>0.0).map_or(1.0,|s|s.hp/s.max_hp);
    let mut ranked = Vec::new();
    for c in candidates {
        if !c.threatened || !c.p_me.is_finite() { continue; }
        let Some(ally) = intel.players.iter().find(|p| p.is_ally && !p.is_me && !p.is_dead && p.champion==c.champion) else { continue };
        let eta = c.distance * 14870.0 / me.move_speed.max(250.0);
        if eta>6.0 { continue; }
        // A zero HP reading of an alive portrait is not evidence of imminent death.
        let hp = ally.hp_pct.filter(|h| *h>0.0 && *h<=1.0)
            .filter(|_| ally.hp_seen_at.is_some_and(|at| (0.0..=3.0).contains(&(intel.game_time-at))));
        if hp.is_some_and(|h| h<0.12) && eta>2.0 { continue; }
        let carry = match ally.role.as_str() { "BOTTOM"=>0.8,"MIDDLE"=>0.5,_=>0.0 };
        let score = hp.map_or(0.0,|h|(1.0-h)*2.0) + carry - eta*0.12;
        let local = c.distance<=0.12;
        let approach = c.p_me>=0.4 && my_hp>=0.25 && !c.dive;
        let text = match (local,approach) {
            (true,true)=>format!("Osłaniaj {}",c.champion),
            (true,false)=>format!("Wycofaj się z {}",c.champion),
            (false,true)=>format!("Pomóż {} · dojście {:.0} s",c.champion,eta.ceil()),
            (false,false)=>format!("Nie idź do {} — zbyt niebezpiecznie",c.champion),
        };
        ranked.push((score,SupportDecision { champion:c.champion.clone(), text, position:c.position, eta_sec:eta,local,
            urgent:hp.is_some_and(|h| h<0.4),approach }));
    }
    ranked.sort_by(|a,b|b.0.total_cmp(&a.0).then(a.1.champion.cmp(&b.1.champion)));
    ranked.into_iter().next().map(|(_,d)|d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::matchup::tests::{intel,player};
    fn candidate(c:&str,d:f64,p:f64)->SupportCandidate { SupportCandidate { champion:c.into(),position:Some((0.5,0.5)),distance:d,p_me:p,threatened:true,dive:false } }
    #[test]
    fn protect_nearby_adc_and_react_to_health_changes() {
        let mut i=intel(vec![player("Leona",true,true,"UTILITY",10),player("Draven",true,false,"BOTTOM",10),player("Akali",true,false,"MIDDLE",10)]);
        let c=[candidate("Draven",0.02,0.6),candidate("Akali",0.04,0.6)];
        assert_eq!(choose_support(&i,&c).unwrap().text,"Osłaniaj Draven");
        i.players[2].hp_pct=Some(0.2);i.players[2].hp_seen_at=Some(i.game_time);
        assert_eq!(choose_support(&i,&c).unwrap().text,"Osłaniaj Akali");
        i.players[2].is_dead=true;
        assert_eq!(choose_support(&i,&c).unwrap().champion,"Draven");
    }
    #[test]
    fn no_suicide_or_help_across_the_map() {
        let i=intel(vec![player("Leona",true,true,"UTILITY",10),player("Draven",true,false,"BOTTOM",10)]);
        assert!(choose_support(&i,&[candidate("Draven",0.4,0.8)]).is_none());
        let d=choose_support(&i,&[candidate("Draven",0.03,0.2)]).unwrap();
        assert!(!d.approach);assert_eq!(d.text,"Wycofaj się z Draven");
        let mut c=candidate("Draven",0.03,0.9);c.dive=true;
        assert!(!choose_support(&i,&[c]).unwrap().approach);
    }
    #[test]
    fn no_threat_no_call_and_stale_hp_does_not_pick_target() {
        let mut i=intel(vec![player("Leona",true,true,"UTILITY",10),player("Draven",true,false,"BOTTOM",10),player("Akali",true,false,"MIDDLE",10)]);
        i.players[2].hp_pct=Some(0.01);i.players[2].hp_seen_at=Some(i.game_time-20.0);
        assert_eq!(choose_support(&i,&[candidate("Draven",0.02,0.6),candidate("Akali",0.02,0.6)]).unwrap().champion,"Draven");
        let mut c=candidate("Draven",0.02,0.6);c.threatened=false;
        assert!(choose_support(&i,&[c]).is_none());
    }
}
