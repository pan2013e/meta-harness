use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use mh_core::{AdapterModelCatalog, ConfigOptionChoice, ModelChoiceDetail, QuotaWindow};
use tokio::process::Command;

use crate::platform::{ProcessTreeGuard, prepare_isolated_command, resolve_command};

pub(crate) async fn run_agy(
    command: &str,
    profile: Option<&Path>,
    args: &[&str],
) -> Option<String> {
    let args: Vec<_> = args.iter().map(|arg| (*arg).to_string()).collect();
    let (program, args) = resolve_command(command, &args).ok()?;
    let mut process = Command::new(program);
    process
        .args(args)
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(path) = profile {
        process.env("GEMINI_HOME", dunce::simplified(path));
    }
    prepare_isolated_command(&mut process);
    let child = process.spawn().ok()?;
    let mut guard = ProcessTreeGuard::attach(&child);
    let result = tokio::time::timeout(Duration::from_secs(15), child.wait_with_output()).await;
    guard.kill_tree();
    let output = result.ok()?.ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn model_catalog(text: &str, adapter_id: &str) -> Option<AdapterModelCatalog> {
    let mut details: Vec<ModelChoiceDetail> = Vec::new();
    let mut efforts: Vec<ConfigOptionChoice> = Vec::new();
    for row in text.lines() {
        let Some((id, label)) = row.trim().split_once('\t') else {
            continue;
        };
        if id.is_empty()
            || label.is_empty()
            || !id
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || b"-._".contains(&ch))
        {
            continue;
        }
        let effort = id.rsplit_once('-').filter(|(_, effort)| {
            matches!(
                *effort,
                "low" | "medium" | "high" | "xhigh" | "max" | "minimal" | "none"
            )
        });
        let key = effort.map_or(id, |(base, _)| base);
        let title = if effort.is_some() {
            label
                .rsplit_once(" (")
                .filter(|(_, suffix)| suffix.ends_with(')'))
                .map_or(label, |(title, _)| title)
        } else {
            label
        };
        let detail = if let Some(index) = details.iter().position(|model| model.value == key) {
            &mut details[index]
        } else {
            details.push(ModelChoiceDetail {
                value: key.into(),
                label: title.into(),
                description: String::new(),
                is_default: false,
                default_effort: None,
                supported_efforts: Vec::new(),
            });
            details.last_mut()?
        };
        if let Some((_, value)) = effort {
            let choice = ConfigOptionChoice {
                value: value.into(),
                label: match value {
                    "xhigh" => "Extra High".into(),
                    _ => format!("{}{}", value[..1].to_ascii_uppercase(), &value[1..]),
                },
            };
            if !detail
                .supported_efforts
                .iter()
                .any(|existing| existing.value == value)
            {
                detail.supported_efforts.push(choice.clone());
            }
            if !efforts.iter().any(|existing| existing.value == value) {
                efforts.push(choice);
            }
        }
    }
    if details.is_empty() {
        return None;
    }
    Some(AdapterModelCatalog {
        adapter_id: adapter_id.into(),
        models: details
            .iter()
            .map(|model| ConfigOptionChoice {
                value: model.value.clone(),
                label: model.label.clone(),
            })
            .collect(),
        model_details: details,
        efforts,
        default_context_window: 1_000_000,
    })
}

pub(crate) fn quota_windows(text: &str) -> Vec<QuotaWindow> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.trim().split('\t');
            let group = fields.next()?.trim();
            if group.is_empty() {
                return None;
            }
            let (kind, minutes) = match fields.next()?.trim() {
                "Five Hour Limit Remaining" => ("5h", 300),
                "Weekly Limit Remaining" => ("Weekly", 10_080),
                _ => return None,
            };
            let remaining_pct: f32 = fields.next()?.trim().strip_suffix('%')?.parse().ok()?;
            if !(0.0..=100.0).contains(&remaining_pct) {
                return None;
            }
            let resets_at = fields
                .next()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value.trim()).ok())
                .map(|date| date.timestamp_millis())
                .filter(|timestamp| *timestamp > 0);
            Some(QuotaWindow {
                id: format!("{group}/{minutes}"),
                label: format!("{group} · {kind}"),
                remaining_pct,
                resets_at,
                window_mins: Some(minutes),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_groups_cli_variants_into_acp_model_and_effort_choices() {
        let catalog = model_catalog("gemini-4-neon-high\tGemini 4 Neon (High)\ngemini-4-neon-medium\tGemini 4 Neon (Medium)\norchestrator-neon\tOrchestrator (Neon)", "antigravity").unwrap();
        assert_eq!(catalog.models[0].value, "gemini-4-neon");
        assert_eq!(
            catalog.model_details[0].supported_efforts[1].value,
            "medium"
        );
        assert_eq!(catalog.models[1].value, "orchestrator-neon");
    }

    #[test]
    fn usage_keeps_provider_groups_and_drops_epoch_reset() {
        let windows = quota_windows(
            "Gemini Models\tWeekly Limit Remaining\t83.5%\t2026-12-03T08:00:00Z\nClaude and GPT models\tFive Hour Limit Remaining\t100%\t1970-01-01T00:00:00Z\nignored\tWeekly Limit Remaining\tNaN",
        );
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].remaining_pct, 83.5);
        assert_eq!(windows[1].window_mins, Some(300));
        assert_eq!(windows[1].resets_at, None);
    }
}
