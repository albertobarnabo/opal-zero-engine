use crate::protocol::{DesignTokens, MissionState};

/// Execute the `finalize_mission_state` tool.
///
/// The Analyst passes a `structured_data_payload` (free-form JSON object),
/// `design_tokens` (visual theme), an optional `summary`, and optional
/// `verification_logs`.  Missing or invalid `design_tokens` fall back to the
/// default minimalist theme rather than failing the mission.
pub fn finalize_mission_state(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct FinalizeArgs {
        #[serde(default)]
        summary: String,
        #[serde(default)]
        structured_data_payload: serde_json::Value,
        #[serde(default)]
        design_tokens: Option<DesignTokens>,
        #[serde(default)]
        verification_logs: Vec<String>,
        #[serde(default)]
        suggested_widgets: Vec<String>,
    }

    // Parse leniently — if the whole JSON is malformed, that's the only hard error.
    let mut args: FinalizeArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse finalize_mission_state arguments: {}", e))?;

    // If the model forgot structured_data_payload, salvage the call by promoting
    // all other top-level string/number fields into it rather than failing.
    if matches!(args.structured_data_payload, serde_json::Value::Null | serde_json::Value::Object(_))
        && args.structured_data_payload.as_object().map(|m| m.is_empty()).unwrap_or(true)
    {
        if let Ok(raw) = serde_json::from_str::<serde_json::Value>(arguments) {
            if let Some(obj) = raw.as_object() {
                let salvaged: serde_json::Map<String, serde_json::Value> = obj
                    .iter()
                    .filter(|(k, v)| {
                        !matches!(k.as_str(), "summary" | "verification_logs" | "suggested_widgets" | "design_tokens" | "structured_data_payload")
                            && matches!(v, serde_json::Value::String(_) | serde_json::Value::Number(_) | serde_json::Value::Object(_) | serde_json::Value::Array(_))
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if !salvaged.is_empty() {
                    args.structured_data_payload = serde_json::Value::Object(salvaged);
                } else {
                    // Nothing to salvage — surface a visible notice on the dashboard
                    // so the user knows why the grid is empty rather than seeing a blank screen.
                    let mut fallback = serde_json::Map::new();
                    fallback.insert("summary".into(), serde_json::Value::String(args.summary.clone()));
                    fallback.insert(
                        "_notice".into(),
                        serde_json::Value::String(
                            "The agent did not populate structured_data_payload. \
                             Refine this mission for better results.".into(),
                        ),
                    );
                    args.structured_data_payload = serde_json::Value::Object(fallback);
                }
            }
        }
    }

    // Use provided tokens or fall back to defaults; clamp ranges.
    let mut tokens = args.design_tokens.unwrap_or_default();
    tokens.glass_intensity  = tokens.glass_intensity.clamp(0.0, 1.0);
    tokens.surface_opacity  = tokens.surface_opacity.clamp(0.0, 1.0);
    tokens.border_radius    = tokens.border_radius.min(48);
    if tokens.primary_accent.is_empty() {
        tokens.primary_accent = "#6366f1".to_string();
    }

    let mut logs = args.verification_logs;
    if !tokens.primary_accent.starts_with('#') {
        logs.push(format!(
            "design_tokens.primary_accent '{}' is not a valid hex color — using default.",
            tokens.primary_accent
        ));
        tokens.primary_accent = "#6366f1".to_string();
    }
    logs.push("Mission state finalized.".to_string());

    let state = MissionState {
        intent_resolved: true,
        data_payload: args.structured_data_payload,
        verification_logs: logs,
        design_tokens: tokens,
        suggested_widgets: args.suggested_widgets,
    };

    serde_json::to_string(&state)
        .map_err(|e| format!("Failed to serialize MissionState: {}", e))
}
