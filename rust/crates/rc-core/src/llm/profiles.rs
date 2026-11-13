//! Model profiles per setup (`src/main/coach/profiles.ts`, docs/MODELE.md).

use super::{is_codex_model, Effort};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Live,
    Death,
    Review,
    Build,
    Research,
    Draft,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pick {
    pub model: String,
    pub effort: Effort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileName {
    Mixed,
    Claude,
    Codex,
}

const fn p(model: &'static str, effort: Effort) -> (&'static str, Effort) {
    (model, effort)
}

/// [live, death, review, build, research, draft]
type Row = [(&'static str, Effort); 6];
const MIXED: Row = [
    p("gpt-6.1-terra", Effort::High),
    p("claude-opus-5-5", Effort::Low),
    p("claude-opus-5-5", Effort::Medium),
    // Build: Opus low (2026-11-13 test, Ahri vs Sion/Urgot/Wukong/Ashe/Milio: Opus gave Zhonya ->
    // Shadowflame -> Rabadon in 6 s; Terra / Sol xhigh both pushed Liandry's + Void Staff).
    p("claude-opus-5-5", Effort::Low),
    p("claude-opus-5-5", Effort::Low),
    p("gpt-6.1-terra", Effort::High),
];
const CLAUDE: Row = [
    p("claude-sonnet-5-5", Effort::Medium),
    p("claude-opus-5-5", Effort::Low),
    p("claude-opus-5-5", Effort::Medium),
    p("claude-opus-5-5", Effort::Low),
    p("claude-opus-5-5", Effort::Low),
    p("claude-sonnet-5-5", Effort::Medium),
];
const CODEX: Row = [
    p("gpt-6.1-terra", Effort::High),
    p("gpt-6.1-sol", Effort::Low),
    p("gpt-6.1-sol", Effort::Medium),
    p("gpt-6.1-terra", Effort::High),
    p("gpt-6.1-terra", Effort::High),
    p("gpt-6.1-terra", Effort::High),
];

fn idx(t: Task) -> usize {
    match t {
        Task::Live => 0,
        Task::Death => 1,
        Task::Review => 2,
        Task::Build => 3,
        Task::Research => 4,
        Task::Draft => 5,
    }
}

pub fn profile(name: ProfileName, task: Task) -> Pick {
    let row = match name {
        ProfileName::Mixed => &MIXED,
        ProfileName::Claude => &CLAUDE,
        ProfileName::Codex => &CODEX,
    };
    let (m, e) = row[idx(task)];
    Pick { model: m.into(), effort: e }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Backends {
    pub claude: bool,
    pub codex: bool,
}

/// Claude Code logged in? Codex app installed?
pub async fn detect_backends() -> Backends {
    let (c, x) = tokio::join!(super::claude::auth_status(), super::codex::resolve_codex());
    Backends { claude: c.logged_in, codex: x.is_some() }
}

pub fn profile_for(b: Backends) -> Option<ProfileName> {
    match (b.claude, b.codex) {
        (true, true) => Some(ProfileName::Mixed),
        (true, false) => Some(ProfileName::Claude),
        (false, true) => Some(ProfileName::Codex),
        _ => None,
    }
}

/// Keep the configured model if its backend exists (with the benchmark's effort for it);
/// otherwise the available profile's pick.
pub fn usable_model(configured: &str, task: Task, b: Backends) -> Option<Pick> {
    let ok = if is_codex_model(configured) { b.codex } else { b.claude };
    if ok {
        let same = [&MIXED, &CLAUDE, &CODEX].iter().map(|r| r[idx(task)]).find(|(m, _)| *m == configured);
        return Some(Pick { model: configured.into(), effort: same.map(|x| x.1).unwrap_or(Effort::Low) });
    }
    profile_for(b).map(|n| profile(n, task))
}

/// index.ts `pick`: usable model or the configured one at low effort.
pub fn pick(configured: &str, task: Task, b: Backends) -> Pick {
    usable_model(configured, task, b).unwrap_or(Pick { model: configured.into(), effort: Effort::Low })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: Backends = Backends { claude: true, codex: true };
    const CL: Backends = Backends { claude: true, codex: false };
    const CX: Backends = Backends { claude: false, codex: true };
    const NONE: Backends = Backends { claude: false, codex: false };

    #[test]
    fn keeps_configured_with_benchmark_effort() {
        assert_eq!(usable_model("gpt-6.1-terra", Task::Live, BOTH), Some(Pick { model: "gpt-6.1-terra".into(), effort: Effort::High }));
        assert_eq!(usable_model("claude-opus-5-5", Task::Review, CL).unwrap().effort, Effort::Medium);
        // Same model, different task -> effort from that task's column.
        assert_eq!(usable_model("claude-opus-5-5", Task::Death, BOTH).unwrap().effort, Effort::Low);
        // Unknown model for the task -> low.
        assert_eq!(usable_model("claude-haiku-5-5", Task::Live, CL).unwrap().effort, Effort::Low);
    }

    #[test]
    fn falls_back_to_available_profile() {
        assert_eq!(usable_model("gpt-6.1-terra", Task::Live, CL).unwrap().model, "claude-sonnet-5-5");
        assert_eq!(usable_model("claude-opus-5-5", Task::Death, CX).unwrap(), Pick { model: "gpt-6.1-sol".into(), effort: Effort::Low });
        assert_eq!(usable_model("claude-opus-5-5", Task::Death, NONE), None);
        assert_eq!(pick("claude-opus-5-5", Task::Death, NONE), Pick { model: "claude-opus-5-5".into(), effort: Effort::Low });
    }
}
