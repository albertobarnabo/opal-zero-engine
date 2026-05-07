use crate::protocol::{UIBlueprint, UIComponent};

const VALID_TYPES: &[&str] = &["MetricCard", "ComparisonTable", "StatusBadge", "Timeline"];

/// Execute the `build_dynamic_ui` tool.
///
/// The LLM passes `summary` (text description of findings) and `components`
/// (an array of `{component_type, props}` objects it designed).  We validate
/// the component types, construct a [`UIBlueprint`], and return its JSON so
/// it can be stored in the ContextBus and surfaced in `MissionComplete`.
pub fn build_dynamic_ui(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct UiArgs {
        #[allow(dead_code)]
        summary: String,
        components: Vec<ComponentArg>,
    }

    #[derive(serde::Deserialize)]
    struct ComponentArg {
        component_type: String,
        props: serde_json::Value,
    }

    let args: UiArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse build_dynamic_ui arguments: {}", e))?;

    let components: Vec<UIComponent> = args
        .components
        .into_iter()
        .filter(|c| VALID_TYPES.contains(&c.component_type.as_str()))
        .map(|c| UIComponent {
            component_type: c.component_type,
            props: c.props,
        })
        .collect();

    if components.is_empty() {
        return Err(
            "build_dynamic_ui requires at least one valid component \
             (MetricCard, ComparisonTable, StatusBadge, Timeline)"
                .to_string(),
        );
    }

    let blueprint = UIBlueprint { components };

    serde_json::to_string(&blueprint)
        .map_err(|e| format!("Failed to serialize UIBlueprint: {}", e))
}
