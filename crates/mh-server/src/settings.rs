use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use mh_core::{AppConfig, GlobalPersona, NotificationPreferences};
use mh_store::{StoreError, settings::AppPreferences};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState,
    routes::{ApiResult, err_resp},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsFile {
    version: u32,
    preferences: AppPreferences,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
}

pub async fn export_settings(State(state): State<AppState>) -> ApiResult<SettingsFile> {
    let preferences = state
        .switch
        .store
        .export_preferences()
        .map_err(store_error)?;
    Ok(Json(SettingsFile {
        version: 1,
        preferences,
        theme: None,
    }))
}

pub async fn notification_preferences(
    State(state): State<AppState>,
) -> ApiResult<NotificationPreferences> {
    state
        .switch
        .store
        .notification_preferences()
        .map(Json)
        .map_err(store_error)
}

pub async fn save_notification_preferences(
    State(state): State<AppState>,
    Json(prefs): Json<NotificationPreferences>,
) -> ApiResult<NotificationPreferences> {
    state
        .switch
        .store
        .set_notification_preferences(prefs)
        .map_err(store_error)?;
    Ok(Json(prefs))
}

pub async fn import_settings(
    State(state): State<AppState>,
    Json(mut file): Json<SettingsFile>,
) -> ApiResult<SettingsFile> {
    let config = state.switch.config.write().await;
    file.validate(&config).map_err(invalid)?;
    state
        .switch
        .store
        .import_preferences(&file.preferences)
        .map_err(store_error)?;
    for pref in &file.preferences.quota_preferences {
        if config.adapters.contains_key(&pref.adapter_id) {
            state
                .switch
                .quota
                .configure_adapter(
                    &pref.adapter_id,
                    pref.plan_label.clone(),
                    pref.exact_probe_opt_in,
                    pref.manual_5h_pct,
                    pref.manual_weekly_pct,
                )
                .await;
        }
    }
    file.preferences = state
        .switch
        .store
        .export_preferences()
        .map_err(store_error)?;
    Ok(Json(file))
}

pub async fn save_persona(
    State(state): State<AppState>,
    Json(mut persona): Json<GlobalPersona>,
) -> ApiResult<Value> {
    let cfg = state.switch.config.read().await;
    normalize_persona(&mut persona, &cfg).map_err(invalid)?;
    if !cfg.adapters.contains_key(&persona.adapter_id) {
        return Err(invalid("Choose a configured harness for this persona."));
    }
    state
        .switch
        .store
        .save_global_persona(&persona)
        .map_err(store_error)?;
    Ok(Json(json!({ "persona": persona })))
}

pub async fn delete_persona(
    State(state): State<AppState>,
    Path(handle): Path<String>,
) -> ApiResult<Value> {
    state
        .switch
        .store
        .delete_global_persona(&handle)
        .map_err(store_error)?;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn add_persona_to_session(
    State(state): State<AppState>,
    Path((id, handle)): Path<(String, String)>,
) -> ApiResult<Value> {
    let member = state
        .switch
        .store
        .add_global_persona_to_session(&id, &handle.to_ascii_lowercase())
        .map_err(|error| match error {
            StoreError::Conflict(message) => err_resp(StatusCode::CONFLICT, message),
            error => store_error(error),
        })?;
    Ok(Json(json!({ "member": member })))
}

fn invalid(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    err_resp(StatusCode::BAD_REQUEST, message)
}

fn store_error(error: StoreError) -> (StatusCode, Json<Value>) {
    err_resp(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn normalize_persona(persona: &mut GlobalPersona, config: &AppConfig) -> Result<(), &'static str> {
    persona.handle = persona
        .handle
        .trim()
        .trim_start_matches('@')
        .to_ascii_lowercase();
    if persona.handle.is_empty()
        || persona.handle.len() > 48
        || !persona
            .handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err("Use a persona handle of up to 48 letters, numbers, hyphens, or underscores.");
    }
    if matches!(persona.handle.as_str(), "all" | "orchestrator")
        || config
            .adapters
            .keys()
            .any(|id| id.eq_ignore_ascii_case(&persona.handle))
    {
        return Err("That handle is reserved by Meta-Harness or a configured harness.");
    }
    persona.display_name = persona.display_name.trim().to_owned();
    if persona.display_name.is_empty() {
        persona.display_name.clone_from(&persona.handle);
    }
    persona.persona_prompt = persona.persona_prompt.trim().to_owned();
    if !valid_text(&persona.adapter_id, 128)
        || !valid_text(&persona.display_name, 128)
        || persona.persona_prompt.len() > 16_000
        || persona.persona_prompt.contains('\0')
    {
        return Err("The persona's harness, display name, or guidance is invalid.");
    }
    normalize_choice(&mut persona.active_model, 256)?;
    normalize_choice(&mut persona.active_effort, 64)?;
    Ok(())
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn normalize_choice(value: &mut Option<String>, max: usize) -> Result<(), &'static str> {
    *value = value
        .take()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if value.as_ref().is_some_and(|value| !valid_text(value, max)) {
        return Err("A model, effort, or quota label is invalid.");
    }
    Ok(())
}

impl SettingsFile {
    fn validate(&mut self, config: &AppConfig) -> Result<(), &'static str> {
        if self.version != 1 {
            return Err("This settings file version is unsupported.");
        }
        if self.theme.as_deref().is_some_and(|theme| {
            !matches!(
                theme,
                "obsidian" | "tokyo-night" | "catppuccin" | "daylight"
            )
        }) {
            return Err("The settings file specifies an unknown theme.");
        }

        let mut handles = HashSet::new();
        for persona in &mut self.preferences.global_personas {
            normalize_persona(persona, config)?;
            if !handles.insert(persona.handle.clone()) {
                return Err("The settings file contains duplicate personas.");
            }
        }
        let mut defaults = HashSet::new();
        for pref in &mut self.preferences.adapter_defaults {
            if !valid_text(&pref.adapter_id, 128) || !defaults.insert(pref.adapter_id.clone()) {
                return Err("The settings file contains invalid or duplicate harness defaults.");
            }
            normalize_choice(&mut pref.default_model, 256)?;
            normalize_choice(&mut pref.default_effort, 64)?;
        }
        let mut quotas = HashSet::new();
        for pref in &mut self.preferences.quota_preferences {
            if !valid_text(&pref.adapter_id, 128)
                || !quotas.insert(pref.adapter_id.clone())
                || [pref.manual_5h_pct, pref.manual_weekly_pct]
                    .into_iter()
                    .flatten()
                    .any(|pct| !(0.0..=100.0).contains(&pct))
            {
                return Err("The settings file contains invalid or duplicate quota preferences.");
            }
            normalize_choice(&mut pref.plan_label, 128)?;
        }
        Ok(())
    }
}
