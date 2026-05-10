use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

// ── UI Blueprint ──────────────────────────────────────────────────────────────

/// A single renderable UI primitive produced by the `build_dynamic_ui` tool.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UIComponent {
    pub component_type: String,
    pub props: serde_json::Value,
}

/// The structured dashboard output that replaces verbose text for data-rich missions.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct UIBlueprint {
    pub components: Vec<UIComponent>,
}

// ── Agent roles ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum AgentRole {
    WebSearcher,
    Analyst,
    Planner,
    /// Executes Python code for complex math, data transformation, or analysis.
    Coder,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Task {
    pub id: Uuid,
    /// Full natural-language task description.
    pub intent: String,
    /// Short, sanitized key used as the ContextBus entry for this task's result.
    /// Derived from the first few significant words of `intent`, so it is both
    /// human-readable and location-aware (e.g. `find_hotels_in_seoul` vs
    /// `find_hotels_in_rome`).
    pub slug: String,
    pub status: TaskStatus,
    pub role: AgentRole,
    pub result: Option<String>,
    pub depends_on: Vec<Uuid>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl AgentRole {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRole::WebSearcher => "WebSearcher",
            AgentRole::Analyst => "Analyst",
            AgentRole::Planner => "Planner",
            AgentRole::Coder => "Coder",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ContextBus {
    pub data: HashMap<String, String>,
}

impl ContextBus {
    /// Wipe all stored results. Call this before re-running a mission to
    /// guarantee no stale data from a previous execution leaks through.
    pub fn clear(&mut self) {
        self.data.clear();
    }
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameters,
    /// Whether the tool dispatch is async (e.g. `web_search`).
    /// Loaded from manifests; skipped when serialising for the OpenAI API.
    #[serde(default, skip_serializing)]
    pub is_async: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolParameters {
    #[serde(rename = "type")]
    pub param_type: String,
    pub properties: HashMap<String, ParameterProperty>,
    pub required: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParameterProperty {
    #[serde(rename = "type")]
    pub prop_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<ParameterProperty>>,
}

impl Tool {
    pub fn calculator() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "operation".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some("The operation to perform: 'add', 'subtract', 'multiply', or 'divide'".to_string()),
                items: None,
            },
        );
        properties.insert(
            "values".to_string(),
            ParameterProperty {
                prop_type: "array".to_string(),
                description: Some("Array of numbers to perform the operation on".to_string()),
                items: Some(Box::new(ParameterProperty {
                    prop_type: "number".to_string(),
                    description: None,
                    items: None,
                })),
            },
        );

        Tool {
            name: "calculator".to_string(),
            description: "Performs mathematical calculations (add, subtract, multiply, divide)".to_string(),
            is_async: false,
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["operation".to_string(), "values".to_string()],
            },
        }
    }

    pub fn write_file() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "filename".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some("Name of the file to write (e.g. 'trip_report.md'). Must not contain path separators.".to_string()),
                items: None,
            },
        );
        properties.insert(
            "content".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some("The full text content to write into the file.".to_string()),
                items: None,
            },
        );

        Tool {
            name: "write_file".to_string(),
            description: "Writes text content to a file inside the output/ directory. Use this to persist reports or summaries to disk.".to_string(),
            is_async: false,
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["filename".to_string(), "content".to_string()],
            },
        }
    }

    pub fn web_search() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "query".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some("The search query to execute".to_string()),
                items: None,
            },
        );

        Tool {
            name: "web_search".to_string(),
            description: "Searches the web for information".to_string(),
            is_async: true,
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["query".to_string()],
            },
        }
    }

    pub fn python_interpreter() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "code".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some(
                    "The Python 3 code to execute. Must use only the standard library. \
                     Use print() to emit results. Do not use file I/O or shell commands."
                        .to_string(),
                ),
                items: None,
            },
        );

        Tool {
            name: "python_interpreter".to_string(),
            description: "Executes a Python 3 snippet and returns its stdout output. \
                          Use this for complex calculations, statistical analysis, \
                          data transformation, or any logic the calculator cannot express."
                .to_string(),
            is_async: false,
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["code".to_string()],
            },
        }
    }

    pub fn build_dynamic_ui() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "summary".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some(
                    "A brief plain-text summary of all mission findings.".to_string(),
                ),
                items: None,
            },
        );
        properties.insert(
            "components".to_string(),
            ParameterProperty {
                prop_type: "array".to_string(),
                description: Some(
                    "Array of UI components. Use these types and their props:\n\
                     - MetricCard: { title, value, subtitle?, unit?, trend?: \"up\"|\"down\" }\n\
                     - ComparisonTable: { title?, headers: string[], rows: (string|number)[][] }\n\
                     - StatusBadge: { label, status: \"info\"|\"success\"|\"warning\"|\"error\", description? }\n\
                     - Timeline: { title?, steps: { label, description?, time?, status?: \"completed\"|\"current\"|\"upcoming\" }[] }"
                    .to_string(),
                ),
                items: Some(Box::new(ParameterProperty {
                    prop_type: "object".to_string(),
                    description: None,
                    items: None,
                })),
            },
        );

        Tool {
            name: "build_dynamic_ui".to_string(),
            description:
                "Transform long text findings into a scannable dashboard. \
                 Call this with a brief summary of the mission and an array of components. \
                 Focus on high-density data: extract every number, comparison, and status into \
                 a dedicated component. Minimise raw prose — the goal is a data-first layout."
                .to_string(),
            is_async: false,
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["summary".to_string(), "components".to_string()],
            },
        }
    }

    /// Hydrate a `Tool` from a JSON manifest file on disk.
    ///
    /// The manifest must be valid JSON matching the `Tool` serde shape:
    /// `name`, `description`, `is_async` (bool), and `parameters` (JSON Schema).
    pub fn from_manifest(path: &std::path::Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read manifest {:?}: {}", path, e))?;
        serde_json::from_str::<Tool>(&contents)
            .map_err(|e| format!("Failed to parse manifest {:?}: {}", path, e))
    }
}

// ── Streaming events ──────────────────────────────────────────────────────────

/// Emitted through the mission channel as work progresses.
/// `serde(tag = "type")` puts the variant name into the JSON payload so the
/// frontend can switch on it without a separate SSE `event:` header lookup.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MissionUpdate {
    TaskStarted {
        slug: String,
        role: String,
        intent: String,
    },
    TaskCompleted {
        slug: String,
        role: String,
        result: String,
    },
    TaskFailed {
        slug: String,
        role: String,
    },
    GovernorExpand {
        new_task_count: usize,
        descriptions: Vec<String>,
    },
    MissionComplete {
        intent: String,
        task_count: usize,
        expanded_task_count: usize,
        mission_id: String,
        layout_hint: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        ui_blueprint: Option<UIBlueprint>,
    },
    MissionFailed {
        error: String,
    },
}

impl MissionUpdate {
    /// Returns the SSE `event:` field value for this variant.
    pub fn event_name(&self) -> &'static str {
        match self {
            MissionUpdate::TaskStarted { .. }    => "task_started",
            MissionUpdate::TaskCompleted { .. }  => "task_completed",
            MissionUpdate::TaskFailed { .. }     => "task_failed",
            MissionUpdate::GovernorExpand { .. } => "governor_expand",
            MissionUpdate::MissionComplete { .. } => "mission_complete",
            MissionUpdate::MissionFailed { .. }  => "mission_failed",
        }
    }
}
