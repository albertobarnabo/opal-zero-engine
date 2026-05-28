use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

// ── Human-in-the-loop signalling ──────────────────────────────────────────────

/// Magic prefix written to a task's `result` field by the `feedback` tool.
/// `validate_mission` scans for this prefix to detect a requested human pause.
pub const AWAITING_FEEDBACK_PREFIX: &str = "__AWAITING_FEEDBACK__: ";

// ── ContextBus well-known keys ────────────────────────────────────────────────

/// ContextBus key for the caller-supplied JSON output schema.
/// Written before the mission starts; read by the Analyst and State Finalizer
/// to enforce the required `structured_data_payload` shape.
pub const CTX_OUTPUT_SCHEMA: &str = "__output_schema";

/// ContextBus key for the cross-mission persistent memory summary.
/// Injected at mission start (and refreshed on resume) from the memory store.
pub const CTX_GLOBAL_MEMORY: &str = "__global_memory";

/// ContextBus key that holds the human user's feedback text on resume.
/// Its presence is also used as a guard to skip re-detection of the
/// awaiting-feedback signal in `check_code_gates`.
pub const CTX_USER_FEEDBACK: &str = "user_feedback";

/// ContextBus key for the question the agent asked the user before pausing.
/// Stored alongside `CTX_USER_FEEDBACK` so the refinement task has full context.
pub const CTX_FEEDBACK_QUESTION: &str = "awaiting_feedback_question";

/// ContextBus key for the structured payload from a prior mission run.
/// Injected during `refine_mission` and `run_mission_with_history` so the
/// Analyst can build on previous results rather than starting blind.
pub const CTX_PRIOR_MISSION_STATE: &str = "prior_mission_state";

/// ContextBus key for the human-readable age of the prior mission run.
/// Example value: `"47 hours ago (2026-05-24T10:30:00Z)"`.
/// Lets the Analyst express confidence in how current the prior data is.
pub const CTX_PRIOR_RUN_TIMESTAMP: &str = "__prior_run_timestamp";

/// Returned by `run_mission` when the mission is paused awaiting human input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeRequest {
    /// ID of the persisted snapshot (`missions/<id>.json`).
    pub mission_id: String,
    /// The question the agent wants the user to answer.
    pub question: String,
}

// ── Mission State ─────────────────────────────────────────────────────────────

/// The canonical final output of an OpalZero mission, produced by `finalize_mission_state`.
///
/// This is a raw, client-agnostic fact payload.  The web frontend transforms it
/// into UI components via its `ApplicationMapper`; other clients (CLI, API) can
/// consume `data_payload` directly without any rendering logic.
///
/// UI presentation concerns (design tokens, widget hints) are intentionally
/// absent here — they are applied by the server or client layer, not the kernel.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct MissionState {
    /// Whether the mission fully resolved the original intent.
    pub intent_resolved: bool,
    /// Free-form JSON object whose keys and value types are determined by the
    /// Analyst agent.  Keys should be descriptive (e.g. `cheapest_flight`,
    /// `hotel_options`, `total_cost`) and values can be scalars, arrays, or
    /// nested objects.
    pub data_payload: serde_json::Value,
    /// Human-readable notes the agent logged during synthesis (quality checks,
    /// data sources, caveats).
    #[serde(default)]
    pub verification_logs: Vec<String>,
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
    /// Slugs of tasks that must reach `Completed` before this task is eligible
    /// to run.  An empty vec means "no prerequisites — run immediately".
    /// Using `#[serde(default)]` keeps older mission snapshots (which either
    /// omit the field or carry UUID strings from the previous schema) safe to
    /// deserialise — unknown strings simply won't match any slug and are ignored.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Tool names that must not be offered to the agent for this task.
    /// Used by the re-planner to steer repair tasks away from previously
    /// failing tools (e.g. `python_interpreter` after a Coder failure).
    #[serde(default)]
    pub excluded_tools: Vec<String>,
    /// Explicit allowlist of tool names the Planner declared for this task.
    ///
    /// When `Some`, the Dispatcher narrows the role's default tool set to only
    /// these names before applying `excluded_tools`.  When `None` (the default),
    /// the full role tool set is offered.
    ///
    /// This lets the Planner give each task the minimal tool scope it actually
    /// needs — a WebSearcher that only needs `web_search` won't be distracted
    /// by `calculator`, `diff`, or `sqlite_query`.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,

    /// JSON-encoded output schema the Planner declared for this task.
    ///
    /// Object whose keys are the specific data points this task must contribute.
    /// Values are type hints ("string", "number", …) for documentation.
    ///
    /// For **WebSearcher** tasks the Dispatcher injects these as "REQUIRED DATA
    /// POINTS" in the prompt so the agent knows what specific facts to surface.
    ///
    /// For **Analyst** tasks it serves as the schema-enforcement contract when
    /// no global `CTX_OUTPUT_SCHEMA` was provided by the client.  The global
    /// schema always takes priority.
    #[serde(default)]
    pub output_schema: Option<String>,

    /// Research manifest that short-circuited this task's execution.
    ///
    /// Set by the dispatcher when a TOML manifest match succeeds and the
    /// manifest path produces a result (bypassing the multi-turn Exa loop).
    /// `None` means the task ran via the normal LLM tool-call path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_id: Option<String>,
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
/// A directory the Wasm executor should expose to the guest module.
///
/// `host` is a path on the OpalZero host (relative to CWD at runtime).
/// `guest` is the absolute path the Wasm code will see (e.g. `"/missions"`).
/// `readonly` restricts the preopen to read-only `DirPerms::READ | FilePerms::READ`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WasmPreopen {
    pub host: String,
    pub guest: String,
    #[serde(default = "bool_true")]
    pub readonly: bool,
}

fn bool_true() -> bool {
    true
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
    /// Host directories to expose to the Wasm module via WASI preopens.
    /// Loaded from manifests; skipped when serialising for the OpenAI API.
    #[serde(default, skip_serializing)]
    pub preopens: Vec<WasmPreopen>,
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
            preopens: vec![],
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
            preopens: vec![],
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
            preopens: vec![],
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
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["code".to_string()],
            },
        }
    }

    pub fn finalize_mission_state() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "summary".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some("A brief plain-text summary of all mission findings.".to_string()),
                items: None,
            },
        );
        properties.insert(
            "structured_data_payload".to_string(),
            ParameterProperty {
                prop_type: "object".to_string(),
                description: Some(
                    "MANDATORY JSON object containing all mission findings. \
                     Use descriptive keys (e.g. 'cheapest_flight', 'hotel_options', \
                     'total_cost'). Values may be strings, numbers, arrays, or nested \
                     objects. Focus on completeness and accuracy — the client renders \
                     this automatically."
                    .to_string(),
                ),
                items: None,
            },
        );
        properties.insert(
            "verification_logs".to_string(),
            ParameterProperty {
                prop_type: "array".to_string(),
                description: Some("Optional quality notes, data sources, or caveats.".to_string()),
                items: Some(Box::new(ParameterProperty {
                    prop_type: "string".to_string(),
                    description: None,
                    items: None,
                })),
            },
        );

        Tool {
            name: "finalize_mission_state".to_string(),
            description:
                "REQUIRED final-step tool. Submit all mission findings as a structured JSON \
                 state payload. Do not write plain text — call this tool exactly once with \
                 a complete structured_data_payload that captures every fact, number, \
                 comparison, and status the mission discovered."
                .to_string(),
            is_async: false,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["summary".to_string(), "structured_data_payload".to_string()],
            },
        }
    }

    pub fn feedback() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "question".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some(
                    "The question or decision you want the user to answer before work continues \
                     (e.g. 'The dashboard is ready — does the layout look correct?')."
                        .to_string(),
                ),
                items: None,
            },
        );
        properties.insert(
            "context".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some(
                    "Optional summary of what was accomplished so far, to give the user \
                     enough context to answer the question."
                        .to_string(),
                ),
                items: None,
            },
        );

        Tool {
            name: "feedback".to_string(),
            description:
                "Pause the mission and ask the human user a question. Use when you need \
                 approval, clarification, or a decision before continuing. The mission will \
                 resume once the user responds."
                    .to_string(),
            is_async: false,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["question".to_string()],
            },
        }
    }

    pub fn vision() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "file_path".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some(
                    "File name of the image in the uploads directory (e.g. 'chart.png'). \
                     Do not include directory separators or path prefixes."
                        .to_string(),
                ),
                items: None,
            },
        );
        properties.insert(
            "prompt".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some(
                    "Specific analysis question or instruction (e.g. \
                     'What data is shown in this chart?', 'Extract all visible text'). \
                     Defaults to a general description if omitted."
                        .to_string(),
                ),
                items: None,
            },
        );

        Tool {
            name: "vision".to_string(),
            description:
                "Analyze an image file and return a detailed textual description or data \
                 summary. Use for charts, photos, diagrams, and screenshots stored in the \
                 uploads/ directory."
                    .to_string(),
            is_async: true,
            preopens: vec![WasmPreopen {
                host: "uploads".to_string(),
                guest: "/uploads".to_string(),
                readonly: true,
            }],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["file_path".to_string()],
            },
        }
    }

    pub fn memory_persist() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "key".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some(
                    "Unique identifier for this memory entry (alphanumeric, underscores, \
                     hyphens, max 64 chars). Example: 'user_industry', 'preferred_chart_style'"
                        .to_string(),
                ),
                items: None,
            },
        );
        properties.insert(
            "value".to_string(),
            ParameterProperty {
                prop_type: "string".to_string(),
                description: Some(
                    "The fact or context to remember. Be concise — max 2000 chars."
                        .to_string(),
                ),
                items: None,
            },
        );

        Tool {
            name: "memory_persist".to_string(),
            description:
                "Write a named fact to the persistent cross-mission memory store. \
                 Use this to save important findings, user preferences, or recurring context \
                 that should be available in future OpalZero missions."
                    .to_string(),
            is_async: false,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["key".to_string(), "value".to_string()],
            },
        }
    }

    pub fn http_request() -> Self {
        let mut properties = HashMap::new();
        properties.insert("url".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Full URL including scheme (https://...).".to_string()),
            items: None,
        });
        properties.insert("method".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("HTTP method: GET, POST, PUT, PATCH, DELETE. Default: GET.".to_string()),
            items: None,
        });
        properties.insert("headers".to_string(), ParameterProperty {
            prop_type: "object".to_string(),
            description: Some("Optional request headers as key/value pairs (e.g. Authorization).".to_string()),
            items: None,
        });
        properties.insert("body".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Optional request body. Serialize JSON to a string before passing.".to_string()),
            items: None,
        });
        Tool {
            name: "http_request".to_string(),
            description:
                "Make an HTTP GET or POST request to any URL and return the response status, \
                 headers, and body. Use this to call REST APIs, webhooks, or read JSON/text \
                 endpoints that are not covered by other tools. Response body is capped at 512 KB."
                    .to_string(),
            is_async: true,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["url".to_string()],
            },
        }
    }

    pub fn fetch_page() -> Self {
        let mut properties = HashMap::new();
        properties.insert("url".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Full URL of the webpage to fetch (https://...).".to_string()),
            items: None,
        });
        properties.insert("prompt".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Optional hint about what you are looking for on the page.".to_string()),
            items: None,
        });
        Tool {
            name: "fetch_page".to_string(),
            description:
                "Fetch a webpage and return its readable text content with HTML stripped. \
                 Use this to read a specific URL you already know — a pricing page, docs page, \
                 job listing, product page, or any web resource web_search does not fully index. \
                 Output is capped at 8 000 characters."
                    .to_string(),
            is_async: true,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["url".to_string()],
            },
        }
    }

    pub fn sqlite_query() -> Self {
        let mut properties = HashMap::new();
        properties.insert("query".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some(
                "SQL statement to execute (SELECT, CREATE TABLE, INSERT, UPDATE, DELETE). \
                 Use ? placeholders for bound parameters."
                    .to_string(),
            ),
            items: None,
        });
        properties.insert("params".to_string(), ParameterProperty {
            prop_type: "array".to_string(),
            description: Some(
                "Optional positional parameters to bind to ? placeholders in the query."
                    .to_string(),
            ),
            items: Some(Box::new(ParameterProperty {
                prop_type: "string".to_string(),
                description: None,
                items: None,
            })),
        });
        Tool {
            name: "sqlite_query".to_string(),
            description:
                "Run a SQL query against an embedded SQLite database scoped to this mission. \
                 Use CREATE TABLE + INSERT to store structured findings, then SELECT to analyse \
                 them. The database persists across tasks within the same mission. \
                 SELECT results are returned as a JSON array of row objects (max 500 rows)."
                    .to_string(),
            is_async: false,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["query".to_string()],
            },
        }
    }

    pub fn read_csv() -> Self {
        let mut properties = HashMap::new();
        properties.insert("filename".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Filename in the uploads/ directory (e.g. \"sales.csv\").".to_string()),
            items: None,
        });
        properties.insert("delimiter".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Field delimiter — use \",\" for CSV (default) or \"\\t\" for TSV.".to_string()),
            items: None,
        });
        Tool {
            name: "read_csv".to_string(),
            description:
                "Parse a CSV or TSV file into a structured JSON array of row objects keyed by \
                 column header. Use this instead of read_file when you need to reason about \
                 specific columns, filter rows, or load data into sqlite_query."
                    .to_string(),
            is_async: false,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["filename".to_string()],
            },
        }
    }

    pub fn rss_reader() -> Self {
        let mut properties = HashMap::new();
        properties.insert("url".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Full URL of the RSS or Atom feed.".to_string()),
            items: None,
        });
        properties.insert("limit".to_string(), ParameterProperty {
            prop_type: "integer".to_string(),
            description: Some("Max items to return (1–20, default 20).".to_string()),
            items: None,
        });
        Tool {
            name: "rss_reader".to_string(),
            description:
                "Fetch and parse an RSS or Atom feed. Returns the feed title, description, and \
                 up to 20 recent items with title, link, summary, and published date. Use for \
                 news monitoring, release tracking, and blog aggregation missions."
                    .to_string(),
            is_async: true,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["url".to_string()],
            },
        }
    }

    pub fn diff() -> Self {
        let mut properties = HashMap::new();
        properties.insert("original".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("The original (before) text.".to_string()),
            items: None,
        });
        properties.insert("modified".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("The modified (after) text.".to_string()),
            items: None,
        });
        properties.insert("context_lines".to_string(), ParameterProperty {
            prop_type: "integer".to_string(),
            description: Some("Lines of context around each change (default 3).".to_string()),
            items: None,
        });
        Tool {
            name: "diff".to_string(),
            description:
                "Compute a unified text diff between two strings. Returns lines added, removed, \
                 unchanged, and a standard unified diff. Use in comparison missions: price changes, \
                 document revisions, before/after content changes."
                    .to_string(),
            is_async: false,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["original".to_string(), "modified".to_string()],
            },
        }
    }

    pub fn extract_pdf_text() -> Self {
        let mut properties = HashMap::new();
        properties.insert("filename".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("PDF filename in the uploads/ directory (e.g. \"report.pdf\").".to_string()),
            items: None,
        });
        Tool {
            name: "extract_pdf_text".to_string(),
            description:
                "Extract readable text from a PDF file in the uploads/ directory. Returns the \
                 text content, page count, and character count. Output is capped at 32 KB. \
                 Use this for research missions that involve uploaded documents, contracts, \
                 reports, or papers."
                    .to_string(),
            is_async: false,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["filename".to_string()],
            },
        }
    }

    pub fn send_email() -> Self {
        let mut properties = HashMap::new();
        properties.insert("to".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Recipient email address.".to_string()),
            items: None,
        });
        properties.insert("subject".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Email subject line.".to_string()),
            items: None,
        });
        properties.insert("body".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Plain-text email body with the mission findings.".to_string()),
            items: None,
        });
        properties.insert("reply_to".to_string(), ParameterProperty {
            prop_type: "string".to_string(),
            description: Some("Optional reply-to email address.".to_string()),
            items: None,
        });
        Tool {
            name: "send_email".to_string(),
            description:
                "Send an email with mission findings to a specified address. Use this to deliver \
                 results asynchronously — summaries, reports, alerts, or notifications. \
                 SMTP credentials are read from server environment variables; never ask the \
                 user for passwords."
                    .to_string(),
            is_async: true,
            preopens: vec![],
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["to".to_string(), "subject".to_string(), "body".to_string()],
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
        /// Serialized mission output.  Typed as a free-form JSON value so the
        /// server layer can augment it with presentation-layer fields (e.g.
        /// design tokens, widget hints) before sending to the frontend, without
        /// those fields leaking into the kernel's `MissionState` type.
        #[serde(skip_serializing_if = "Option::is_none")]
        mission_state: Option<serde_json::Value>,
    },
    MissionFailed {
        error: String,
    },
    /// The mission is paused and waiting for a human response before continuing.
    MissionPaused {
        /// The question the Governor wants the user to answer.
        question: String,
        /// Snapshot ID so the caller can persist/resume the mission.
        mission_id: String,
    },
    /// Structured performance and quality metrics emitted once per mission.
    ///
    /// Sent immediately after `MissionComplete` or `MissionFailed` so that
    /// the server / SSE stream can forward it to the frontend or log it.
    Metrics(crate::metrics::MissionMetrics),
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
            MissionUpdate::MissionPaused { .. }  => "mission_paused",
            MissionUpdate::Metrics(_)            => "metrics",
        }
    }
}
