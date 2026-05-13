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
        #[allow(dead_code)]
        summary: String,
        structured_data_payload: serde_json::Value,
        #[serde(default)]
        design_tokens: Option<DesignTokens>,
        #[serde(default)]
        verification_logs: Vec<String>,
    }

    let args: FinalizeArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse finalize_mission_state arguments: {}", e))?;

    match &args.structured_data_payload {
        serde_json::Value::Null => {
            return Err(
                "finalize_mission_state: structured_data_payload must not be null".to_string(),
            );
        }
        serde_json::Value::Object(map) if map.is_empty() => {
            return Err(
                "finalize_mission_state: structured_data_payload must not be empty".to_string(),
            );
        }
        _ => {}
    }

    // Use provided tokens or fall back to defaults; clamp glass_intensity to [0,1].
    let mut tokens = args.design_tokens.unwrap_or_default();
    tokens.glass_intensity = tokens.glass_intensity.clamp(0.0, 1.0);
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
    };

    serde_json::to_string(&state)
        .map_err(|e| format!("Failed to serialize MissionState: {}", e))
}
