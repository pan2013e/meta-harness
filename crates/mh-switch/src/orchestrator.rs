use mh_acp::{AdapterProbeStatus, QuotaTracker};
use mh_core::{AppConfig, DiscoveredConfigOption, SessionMember};
use mh_store::Store;

/// Builds a compact (<250-token) `<orchestrator_telemetry>` context header injected into
/// Orchestrator turns so it can pick executor, model, and reasoning effort intelligently.
pub async fn build_orchestrator_telemetry_block(
    store: &Store,
    config: &AppConfig,
    quota: &QuotaTracker,
    project_id: &str,
    members: &[SessionMember],
    discovered_options: &[(String, Vec<DiscoveredConfigOption>)],
    user_query: &str,
) -> String {
    let snapshots = quota.all_snapshots(config).await;
    let jobs = store
        .recall_jobs(project_id, Some(user_query), 5)
        .unwrap_or_default();
    let facts = store.list_facts(project_id, 5).unwrap_or_default();

    let mut out = String::from("<orchestrator_telemetry>\n");

    out.push_str("  <members>\n");
    for m in members {
        out.push_str(&format!(
            "    - handle=@{} adapter={} role={:?} model={} effort={}\n",
            m.handle,
            m.adapter_id,
            m.role,
            m.active_model.as_deref().unwrap_or("default"),
            m.active_effort.as_deref().unwrap_or("default"),
        ));
    }
    out.push_str("  </members>\n");

    out.push_str("  <quotas>\n");
    for q in &snapshots {
        let p5 = q
            .window_5h_remaining_pct
            .map(|p| format!("{p:.0}%"))
            .unwrap_or_else(|| "n/a".into());
        let pw = q
            .weekly_remaining_pct
            .map(|p| format!("{p:.0}%"))
            .unwrap_or_else(|| "n/a".into());
        let limited = q.rate_limited_until.is_some();
        out.push_str(&format!(
            "    - adapter={} 5h_rem={} weekly_rem={} rate_limited={}\n",
            q.adapter_id, p5, pw, limited
        ));
    }
    out.push_str("  </quotas>\n");

    out.push_str("  <adapter_profiles>\n");
    for (id, cfg) in &config.adapters {
        if !cfg.enabled {
            continue;
        }
        let live_models = discovered_options
            .iter()
            .find(|(h, _)| h == id)
            .map(|(_, opts)| format_opts_summary(opts))
            .unwrap_or_else(|| "pending-lazy-spawn".into());
        out.push_str(&format!(
            "    - adapter={} strengths=[{}] latency={} options=[{}]\n",
            id,
            cfg.characteristics.strengths.join(","),
            cfg.characteristics.latency_tier,
            live_models
        ));
    }
    out.push_str("  </adapter_profiles>\n");

    if !jobs.is_empty() || !facts.is_empty() {
        out.push_str("  <episodic_memory>\n");
        for j in &jobs {
            out.push_str(&format!(
                "    - job({}@{}) [{}]: {} (modules: {})\n",
                j.member_handle,
                j.model.as_deref().unwrap_or("default"),
                j.outcome,
                truncate_str(&j.curated_summary, 140),
                j.touched_modules.join(", ")
            ));
        }
        for f in &facts {
            out.push_str(&format!(
                "    - fact[{}]: {}\n",
                f.category,
                truncate_str(&f.summary, 120)
            ));
        }
        out.push_str("  </episodic_memory>\n");
    }

    out.push_str(
        "  <instructions>\n\
         For trivial questions or repo lookups, answer directly. \
         For coding work, call `orchestrator_dispatch` choosing the best member(s), `model`, and `effort` \
         (`low` for simple edits, `medium` for features/tests, `high` for complex architecture/debugging; \
         avoid adapters with 5h_rem < 15% or rate_limited=true). \
         Use your own harness brand's executor member if it is the strongest fit.\n\
         </instructions>\n",
    );
    out.push_str("</orchestrator_telemetry>\n");
    out
}

pub fn build_conflict_escalation_prompt(
    member_handle: &str,
    member_branch: &str,
    conflicting_files: &[String],
    diff_preview: &str,
    turn_id: &str,
) -> String {
    format!(
        "<merge_conflict_escalation>\n\
         Executor @{member_handle} (branch `{member_branch}`, turn `{turn_id}`) produced a Git merge conflict \
         in the session integration worktree.\n\
         Conflicting files: {}\n\
         Inspect peer trajectories if helpful (`trajectory_read`), resolve all conflict markers cleanly in your working directory, \
         stage and `git commit --no-edit` to complete the merge, and then call `memory_curate_job` summarizing the job.\n\
         <conflict_diff>\n{}\n</conflict_diff>\n\
         </merge_conflict_escalation>",
        conflicting_files.join(", "),
        truncate_str(diff_preview, 8000)
    )
}

pub struct MemoryCurationPromptInput<'a> {
    pub member_handle: &'a str,
    pub adapter_id: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub turn_id: &'a str,
    pub task_brief: &'a str,
    pub touched_modules: &'a [String],
    pub reply_summary: &'a str,
}

pub fn build_memory_curation_prompt(input: MemoryCurationPromptInput<'_>) -> String {
    format!(
        "<memory_curation_request>\n\
         A job by @{} (adapter={}, model={}, effort={}, turn_id={}) completed and merged cleanly.\n\
         Task brief: {}\n\
         Touched modules: {}\n\
         Executor reply excerpt: {}\n\
         Call `memory_curate_job` once with a crisp 1-2 sentence `curated_summary` capturing who did what job, \
         which modules changed, and any notable model/harness outcome (and optionally `project_fact`).\n\
         </memory_curation_request>",
        input.member_handle,
        input.adapter_id,
        input.model.unwrap_or("default"),
        input.effort.unwrap_or("default"),
        input.turn_id,
        truncate_str(input.task_brief, 240),
        if input.touched_modules.is_empty() {
            "none".into()
        } else {
            input.touched_modules.join(", ")
        },
        truncate_str(input.reply_summary, 400),
    )
}

pub fn pick_fallback_executor(
    members: &[SessionMember],
    quotas: &[mh_core::QuotaSnapshot],
    probes: &[AdapterProbeStatus],
    exclude_adapter: Option<&str>,
) -> Option<SessionMember> {
    let mut best: Option<(f32, SessionMember)> = None;
    for m in members {
        if exclude_adapter.is_some_and(|ex| ex == m.adapter_id) {
            continue;
        }
        let installed = probes
            .iter()
            .find(|p| p.adapter_id == m.adapter_id)
            .map(|p| p.installed)
            .unwrap_or(true);
        if !installed {
            continue;
        }
        let q = quotas.iter().find(|q| q.adapter_id == m.adapter_id);
        if q.is_some_and(|snap| snap.rate_limited_until.is_some()) {
            continue;
        }
        let score = q
            .and_then(|snap| snap.window_5h_remaining_pct)
            .unwrap_or(50.0);
        if best.as_ref().is_none_or(|(b_score, _)| score > *b_score) {
            best = Some((score, m.clone()));
        }
    }
    best.map(|(_, m)| m)
}

fn format_opts_summary(opts: &[DiscoveredConfigOption]) -> String {
    opts.iter()
        .map(|o| {
            let vals: Vec<&str> = o.choices.iter().take(6).map(|c| c.value.as_str()).collect();
            format!("{}:{}", o.id, vals.join("|"))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn truncate_str(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars).collect();
        out.push_str("...");
        out
    }
}
