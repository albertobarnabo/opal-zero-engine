use crate::protocol::MissionState;

/// Execute the `finalize_mission_state` tool.
///
/// The Analyst passes a `structured_data_payload` (free-form JSON object),
/// an optional `summary`, and optional `verification_logs`.
/// Design-token and widget-hint fields from the LLM are intentionally ignored
/// here — they are presentation concerns handled by the server layer.
pub fn finalize_mission_state(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct FinalizeArgs {
        #[serde(default)]
        summary: String,
        #[serde(default)]
        structured_data_payload: serde_json::Value,
        #[serde(default)]
        verification_logs: Vec<String>,
        // Accept (and pass through) suggested_widgets and design_tokens so the
        // server layer can extract them from the raw result JSON.  They are not
        // stored in MissionState — they live only on the wire representation.
        #[serde(default)]
        suggested_widgets: Vec<String>,
        #[serde(default)]
        design_tokens: serde_json::Value,
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

    let mut logs = args.verification_logs;
    logs.push("Mission state finalized.".to_string());

    let state = MissionState {
        intent_resolved: true,
        data_payload: args.structured_data_payload,
        verification_logs: logs,
    };

    // Serialize the core MissionState, then inject the presentation-layer fields
    // (suggested_widgets, design_tokens) into the JSON so the server layer can
    // extract them at the SSE boundary without them ever being part of MissionState.
    let mut json_val = serde_json::to_value(&state)
        .map_err(|e| format!("Failed to serialize MissionState: {}", e))?;

    if let Some(obj) = json_val.as_object_mut() {
        if !args.suggested_widgets.is_empty() {
            obj.insert(
                "suggested_widgets".into(),
                serde_json::Value::Array(
                    args.suggested_widgets
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );
        }
        if !args.design_tokens.is_null() {
            obj.insert("design_tokens".into(), args.design_tokens);
        }
    }

    serde_json::to_string(&json_val)
        .map_err(|e| format!("Failed to serialize task result: {}", e))
}
