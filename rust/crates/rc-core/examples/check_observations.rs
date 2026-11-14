//! Validate imported replay observations using the same decoder as the app.
use base64::Engine;
fn main() {
    for file in std::env::args().skip(1) {
        let log=rc_core::self_train::load_obs(std::path::Path::new(&file)).expect("observation log");
        assert!(log.game_id.is_some() && log.frames.len()>=120);
        assert!(log.frames.windows(2).all(|f|f[1].t>f[0].t));
        for f in &log.frames {
            assert_eq!(base64::engine::general_purpose::STANDARD.decode(&f.m).unwrap().len(),128);
            assert!(f.e.len()==5 && f.e.iter().all(|(_,_,dead,ms,gold)|*dead<=1 && ms.is_finite() && *ms>0.0 && gold.is_finite() && *gold>=0.0));
            assert!(f.d.iter().all(|(c,x,y)|x.is_finite() && y.is_finite() && (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y) && f.e.iter().any(|e|e.0==*c)));
        }
        println!("{}",serde_json::json!({"gameId":log.game_id,"frames":log.frames.len(),"first":log.frames.first().unwrap().t,"last":log.frames.last().unwrap().t,"valid":true}));
    }
}
