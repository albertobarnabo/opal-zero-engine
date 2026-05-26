//! # Axion Safety Layer
//!
//! Three components that address the failure patterns identified through
//! τ-bench evaluation.  All three are general-purpose — they apply to any
//! agentic task, not just benchmarks.
//!
//! ## Components
//!
//! | Component              | Prevents                                              |
//! |------------------------|-------------------------------------------------------|
//! | [`PreWriteGate`]       | Irreversible actions taken without authorization      |
//! | [`grounding_suffix`]   | Hallucinated numbers / facts not in tool results      |
//! | [`RequirementTracker`] | Silently dropped requirements in multi-part requests  |

use crate::engine::{AiProvider, ToolResponse};

// ─────────────────────────────────────────────────────────────────────────────
// 1. PRE-WRITE GATE
// ─────────────────────────────────────────────────────────────────────────────
//
// Intercepts every tool call before execution and vetoes write operations
// that do not clearly align with the task intent.
//
// Write tools are tools whose side-effects are externally visible and cannot
// be trivially undone: sending email, mutating a database, making HTTP write
// requests, or running arbitrary code.
//
// Read tools (web_search, fetch_page, rss_reader, read_csv, extract_pdf_text,
// vision, diff, calculator) are never blocked.

/// Tools that are unconditionally treated as writes.
const UNCONDITIONAL_WRITE_TOOLS: &[&str] = &[
    "send_email",
    "memory_persist",
    "write_file",
    "generate_document",
    "build_dynamic_ui",
];

/// Tools that *may* be writes depending on their arguments.
const CONDITIONAL_WRITE_TOOLS: &[&str] = &[
    "http_request",
    "sqlite_query",
    "python_interpreter",
];

/// Returns `true` if the tool name identifies an irreversible write operation.
///
/// For conditional write tools the caller must additionally inspect the
/// `arguments` JSON — use [`is_write_invocation`] instead of this function
/// when the arguments are available.
pub fn is_write_tool(name: &str) -> bool {
    UNCONDITIONAL_WRITE_TOOLS.contains(&name) || CONDITIONAL_WRITE_TOOLS.contains(&name)
}

/// Returns `true` when this specific tool call (name + arguments) constitutes
/// a write operation.
///
/// * Unconditional write tools → always `true`.
/// * `http_request` → `true` only for POST / PUT / PATCH / DELETE methods.
/// * `sqlite_query` → `true` only for non-SELECT statements.
/// * `python_interpreter` → always `true` (arbitrary execution).
pub fn is_write_invocation(name: &str, arguments: &str) -> bool {
    if UNCONDITIONAL_WRITE_TOOLS.contains(&name) {
        return true;
    }

    match name {
        "http_request" => {
            // Check for write HTTP methods in the arguments JSON.
            let lower = arguments.to_lowercase();
            ["\"post\"", "\"put\"", "\"patch\"", "\"delete\""]
                .iter()
                .any(|m| lower.contains(m))
        }
        "sqlite_query" => {
            let lower = arguments.to_lowercase();
            ["insert", "update", "delete", "drop", "create", "alter", "replace"]
                .iter()
                .any(|kw| lower.contains(kw))
        }
        "python_interpreter" => true,
        _ => false,
    }
}

/// Outcome of the pre-write compliance check.
#[derive(Debug)]
pub enum WriteDecision {
    /// Proceed — the write is clearly within scope.
    Authorized,
    /// Block — the write was not sanctioned by the task intent.
    ///
    /// Contains a human-readable reason that will be stored as the task
    /// failure message so the Governor can diagnose and retry.
    Denied(String),
}

/// Ask the LLM whether the pending write action is authorized given the task intent.
///
/// This is intentionally a *lightweight* check (single prompt, no tool calls) so
/// it adds minimal latency.  The default is to allow — the gate only blocks when
/// the model is confident the action is out of scope.
///
/// Returns [`WriteDecision::Authorized`] on any LLM / parsing failure so
/// transient errors never silently drop legitimate actions.
pub async fn check_write_authorization(
    tool_name: &str,
    arguments: &str,
    task_intent: &str,
    provider: &dyn AiProvider,
) -> WriteDecision {
    let args_preview = &arguments[..arguments.len().min(400)];

    let prompt = format!(
        "You are the Axion pre-write compliance gate.\n\
         \n\
         An AI agent is about to execute a WRITE / SIDE-EFFECT action.\n\
         \n\
         Tool:      {tool_name}\n\
         Arguments: {args_preview}\n\
         Task:      {task_intent}\n\
         \n\
         Authorize the action ONLY if ALL of the following hold:\n\
         1. The action is directly required to complete the stated task.\n\
         2. The scope of the write matches what the task asks for — no broader.\n\
         3. The action does not affect resources not mentioned in the task.\n\
         \n\
         When uncertain, DENY.  A denied action causes the agent to reconsider;\
         it does not end the mission.\n\
         \n\
         Respond with valid JSON only — no prose:\n\
         {{\"authorized\": true, \"reason\": \"<one sentence>\"}}"
    );

    match provider.generate_response(&prompt, None).await {
        Ok(ToolResponse::Text(text)) => {
            if let Some(json_str) = crate::governor::extract_json(&text) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    let authorized = val["authorized"].as_bool().unwrap_or(true);
                    let reason = val["reason"]
                        .as_str()
                        .unwrap_or("no reason given")
                        .to_string();
                    if authorized {
                        tracing::debug!(
                            tool = tool_name,
                            reason,
                            "PreWriteGate: authorized"
                        );
                        return WriteDecision::Authorized;
                    } else {
                        tracing::warn!(
                            tool = tool_name,
                            reason,
                            "PreWriteGate: DENIED"
                        );
                        return WriteDecision::Denied(format!(
                            "PreWriteGate blocked {tool_name}: {reason}"
                        ));
                    }
                }
            }
            // Parsing failed — default to allow.
            tracing::debug!(tool = tool_name, "PreWriteGate: parse failed — defaulting to authorized");
            WriteDecision::Authorized
        }
        _ => {
            // LLM unavailable — default to allow.
            tracing::debug!(tool = tool_name, "PreWriteGate: LLM unavailable — defaulting to authorized");
            WriteDecision::Authorized
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. RESPONSE GROUNDING
// ─────────────────────────────────────────────────────────────────────────────
//
// A prompt suffix injected into every agent's system prompt.
// Instructs the model to source all factual claims directly from tool results
// rather than re-deriving them from memory — preventing the class of failures
// where the agent does the right work but reports wrong numbers.

/// System-prompt suffix that enforces response grounding.
///
/// Append this to every role's system prompt via [`system_prompt_for_role`].
pub fn grounding_suffix() -> &'static str {
    "\n\
     ═══ GROUNDING RULE (mandatory) ═══\n\
     Every number, date, name, status, or factual claim in your response MUST\n\
     come directly from a tool result in this conversation.\n\
     • DO NOT re-calculate values you already retrieved — read and report them.\n\
     • If you cannot point to the tool result that supports a claim, do not\n\
       state the claim.  Say \"I could not confirm X\" instead.\n\
     • Arithmetic summaries (totals, differences, averages) must show the\n\
       operands sourced from tool results before stating the result.\n"
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. REQUIREMENT TRACKER
// ─────────────────────────────────────────────────────────────────────────────
//
// Decomposes a user intent into explicit, checkable sub-requirements at plan
// time, then verifies all are addressed before the mission is declared complete.
//
// This prevents the failure mode where a multi-part request is partially handled
// and the agent signs off confidently on incomplete work.

/// A single tracked requirement extracted from the user's intent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Requirement {
    /// Short identifier (e.g. "calculate_gift_card_total").
    pub id: String,
    /// Human-readable description of what must be accomplished.
    pub description: String,
    /// Whether this requirement has been addressed in a task result.
    pub satisfied: bool,
}

/// Extracts a structured checklist of sub-requirements from a free-form intent.
///
/// Called by the planner once per mission.  The result is stored in the
/// `ContextBus` under [`CTX_REQUIREMENTS`] as JSON.
///
/// Returns an empty vec if the intent is simple (single-requirement) or if
/// the LLM cannot parse distinct sub-requirements — in which case the mission
/// proceeds without explicit tracking (same behaviour as before this feature).
pub async fn extract_requirements(
    intent: &str,
    provider: &dyn AiProvider,
) -> Vec<Requirement> {
    let prompt = format!(
        "You are analyzing a user request to identify its distinct sub-requirements.\n\
         \n\
         Request: {intent}\n\
         \n\
         List every independent deliverable or action the user is asking for.\n\
         A simple single-action request should produce exactly one requirement.\n\
         \n\
         Respond with valid JSON only:\n\
         {{\"requirements\": [\n\
           {{\"id\": \"short_snake_case_id\", \"description\": \"what must be done\"}}\n\
         ]}}"
    );

    match provider.generate_response(&prompt, None).await {
        Ok(ToolResponse::Text(text)) => {
            if let Some(json_str) = crate::governor::extract_json(&text) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    if let Some(arr) = val["requirements"].as_array() {
                        let reqs: Vec<Requirement> = arr
                            .iter()
                            .filter_map(|r| {
                                let id = r["id"].as_str()?.to_string();
                                let description = r["description"].as_str()?.to_string();
                                Some(Requirement { id, description, satisfied: false })
                            })
                            .collect();
                        if !reqs.is_empty() {
                            tracing::debug!(
                                count = reqs.len(),
                                "RequirementTracker: extracted requirements"
                            );
                            return reqs;
                        }
                    }
                }
            }
            vec![]
        }
        _ => vec![],
    }
}

/// Checks which requirements are satisfied given the completed task results.
///
/// Returns `(satisfied_count, total_count, unmet_descriptions)`.
/// When `total == 0` (no requirements were extracted), returns `(0, 0, vec![])`,
/// which the caller interprets as "tracking not applicable — proceed".
pub async fn check_requirements(
    requirements: &[Requirement],
    task_results: &str,
    intent: &str,
    provider: &dyn AiProvider,
) -> (usize, usize, Vec<String>) {
    if requirements.is_empty() {
        return (0, 0, vec![]);
    }

    let req_list = requirements
        .iter()
        .map(|r| format!("- [{}] {}", r.id, r.description))
        .collect::<Vec<_>>()
        .join("\n");

    let results_preview = &task_results[..task_results.len().min(2_000)];

    let prompt = format!(
        "You are a completeness checker for an AI agent mission.\n\
         \n\
         Original intent: {intent}\n\
         \n\
         Requirements checklist:\n\
         {req_list}\n\
         \n\
         Completed task results (excerpt):\n\
         {results_preview}\n\
         \n\
         For each requirement, mark it satisfied (true) if the results clearly\n\
         address it — even if imperfectly.  Mark it unsatisfied (false) only if\n\
         there is no evidence it was handled.\n\
         \n\
         Respond with valid JSON only:\n\
         {{\"results\": [\n\
           {{\"id\": \"requirement_id\", \"satisfied\": true, \"evidence\": \"...\"}}\n\
         ]}}"
    );

    match provider.generate_response(&prompt, None).await {
        Ok(ToolResponse::Text(text)) => {
            if let Some(json_str) = crate::governor::extract_json(&text) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    if let Some(arr) = val["results"].as_array() {
                        let mut satisfied = 0;
                        let mut unmet = vec![];

                        for r in requirements {
                            let entry = arr.iter().find(|e| e["id"].as_str() == Some(&r.id));
                            let is_satisfied = entry
                                .and_then(|e| e["satisfied"].as_bool())
                                .unwrap_or(true); // default to satisfied if not found

                            if is_satisfied {
                                satisfied += 1;
                            } else {
                                unmet.push(r.description.clone());
                            }
                        }

                        let total = requirements.len();
                        if !unmet.is_empty() {
                            tracing::warn!(
                                satisfied,
                                total,
                                unmet = ?unmet,
                                "RequirementTracker: unmet requirements detected"
                            );
                        }
                        return (satisfied, total, unmet);
                    }
                }
            }
            // Parse failure — assume all satisfied.
            (requirements.len(), requirements.len(), vec![])
        }
        _ => (requirements.len(), requirements.len(), vec![]),
    }
}

/// ContextBus key under which the serialized `Vec<Requirement>` is stored.
pub const CTX_REQUIREMENTS: &str = "__requirements";

/// Serialize requirements to JSON for ContextBus storage.
pub fn serialize_requirements(reqs: &[Requirement]) -> String {
    serde_json::to_string(reqs).unwrap_or_default()
}

/// Deserialize requirements from ContextBus JSON.
pub fn deserialize_requirements(json: &str) -> Vec<Requirement> {
    serde_json::from_str(json).unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_tool_classification() {
        assert!(is_write_tool("send_email"));
        assert!(is_write_tool("memory_persist"));
        assert!(is_write_tool("http_request"));
        assert!(is_write_tool("sqlite_query"));
        assert!(is_write_tool("python_interpreter"));

        assert!(!is_write_tool("web_search"));
        assert!(!is_write_tool("fetch_page"));
        assert!(!is_write_tool("rss_reader"));
        assert!(!is_write_tool("calculator"));
        assert!(!is_write_tool("vision"));
        assert!(!is_write_tool("diff"));
        assert!(!is_write_tool("read_csv"));
    }

    #[test]
    fn http_request_method_detection() {
        let get_args  = r#"{"method": "GET", "url": "https://example.com"}"#;
        let post_args = r#"{"method": "POST", "url": "https://example.com", "body": "{}"}"#;
        let del_args  = r#"{"method": "DELETE", "url": "https://example.com/1"}"#;

        assert!(!is_write_invocation("http_request", get_args));
        assert!(is_write_invocation("http_request", post_args));
        assert!(is_write_invocation("http_request", del_args));
    }

    #[test]
    fn sqlite_write_detection() {
        let select = r#"{"query": "SELECT * FROM users"}"#;
        let insert = r#"{"query": "INSERT INTO users (name) VALUES ('Alice')"}"#;
        let update = r#"{"query": "UPDATE users SET active = 1 WHERE id = 5"}"#;
        let drop   = r#"{"query": "DROP TABLE temp_data"}"#;

        assert!(!is_write_invocation("sqlite_query", select));
        assert!(is_write_invocation("sqlite_query", insert));
        assert!(is_write_invocation("sqlite_query", update));
        assert!(is_write_invocation("sqlite_query", drop));
    }

    #[test]
    fn python_always_write() {
        assert!(is_write_invocation("python_interpreter", r#"{"code": "print(1+1)"}"#));
        assert!(is_write_invocation("python_interpreter", r#"{"code": "x = 1"}"#));
    }

    #[test]
    fn requirement_serialization_roundtrip() {
        let reqs = vec![
            Requirement { id: "r1".into(), description: "do A".into(), satisfied: false },
            Requirement { id: "r2".into(), description: "do B".into(), satisfied: true  },
        ];
        let json    = serialize_requirements(&reqs);
        let decoded = deserialize_requirements(&json);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].id, "r1");
        assert_eq!(decoded[1].satisfied, true);
    }

    #[test]
    fn empty_requirements_serialization() {
        let json    = serialize_requirements(&[]);
        let decoded = deserialize_requirements(&json);
        assert!(decoded.is_empty());
    }
}
