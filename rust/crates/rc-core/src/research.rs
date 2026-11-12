//! Models (+ reasoning effort) used for web research (`src/main/coach/research.ts`).
//! Set from the active profile at runtime (the app calls the setters):
//!  - deep (patch brief, lane matchup): once per patch / game → quality first.
//!  - fast (runes, draft roles): inside champion select → speed first.
//! `RC_RESEARCH_MODEL` overrides both defaults, like in the TS app.

use crate::llm::profiles::Pick;
use crate::llm::Effort;
use std::sync::RwLock;

struct Models {
    deep: Pick,
    fast: Pick,
}

static MODELS: RwLock<Option<Models>> = RwLock::new(None);

fn defaults() -> Models {
    let env = std::env::var("RC_RESEARCH_MODEL").ok().filter(|s| !s.is_empty());
    Models {
        deep: Pick { model: env.clone().unwrap_or_else(|| "claude-opus-5-5".into()), effort: Effort::Low },
        fast: Pick { model: env.unwrap_or_else(|| "gpt-6.1-terra".into()), effort: Effort::High },
    }
}

fn read<T>(f: impl FnOnce(&Models) -> T) -> T {
    {
        let g = MODELS.read().unwrap();
        if let Some(m) = g.as_ref() {
            return f(m);
        }
    }
    let mut g = MODELS.write().unwrap();
    f(g.get_or_insert_with(defaults))
}

pub fn research_model() -> String {
    read(|m| m.deep.model.clone())
}
pub fn research_effort() -> Effort {
    read(|m| m.deep.effort)
}
pub fn fast_research_model() -> String {
    read(|m| m.fast.model.clone())
}
pub fn fast_research_effort() -> Effort {
    read(|m| m.fast.effort)
}

/// Empty model = no change; effort None keeps the current effort.
pub fn set_research_model(model: &str, effort: Option<Effort>) {
    if model.is_empty() {
        return;
    }
    read(|_| ());
    let mut g = MODELS.write().unwrap();
    let m = g.as_mut().unwrap();
    m.deep = Pick { model: model.into(), effort: effort.unwrap_or(m.deep.effort) };
}

pub fn set_fast_research_model(model: &str, effort: Option<Effort>) {
    if model.is_empty() {
        return;
    }
    read(|_| ());
    let mut g = MODELS.write().unwrap();
    let m = g.as_mut().unwrap();
    m.fast = Pick { model: model.into(), effort: effort.unwrap_or(m.fast.effort) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setters_keep_effort_and_ignore_empty() {
        set_fast_research_model("claude-sonnet-5-5", Some(Effort::Medium));
        set_fast_research_model("", Some(Effort::Max));
        set_fast_research_model("gpt-6.1-sol", None);
        assert_eq!(fast_research_model(), "gpt-6.1-sol");
        assert_eq!(fast_research_effort(), Effort::Medium);
    }
}
