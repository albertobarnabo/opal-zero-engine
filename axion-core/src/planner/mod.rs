use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::protocol::{AgentRole, ContextBus, Task, TaskStatus};

#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub original_intent: String,
    pub tasks: Vec<Task>,
    pub context: ContextBus,
}

impl Plan {
    pub fn new(intent: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            original_intent: intent.to_string(),
            tasks: Vec::new(),
            context: ContextBus::default(),
        }
    }

    pub fn add_task(&mut self, description: &str, dependencies: Vec<Uuid>, role: AgentRole) -> Uuid {
        let id = Uuid::new_v4();
        let task = Task {
            id,
            slug: make_slug(description),
            intent: description.to_string(),
            status: TaskStatus::Pending,
            role,
            result: None,
            depends_on: dependencies,
        };
        self.tasks.push(task);
        id
    }
}

/// Derive a short, stable, location-aware key from a task intent.
///
/// Takes up to 8 significant words (skipping common stop-words), lowercases
/// them, strips non-alphanumeric characters, and joins with underscores.
/// Ensures uniqueness within a plan because callers include the location in
/// their intent strings (e.g. "Find hotels in Seoul").
fn make_slug(intent: &str) -> String {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "in", "on", "at", "to", "for", "of", "and", "or",
        "is", "are", "be", "this", "that", "it", "with", "from", "by",
        "return", "exactly", "using", "use", "report", "fact",
    ];

    let words: Vec<String> = intent
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| !STOP_WORDS.contains(&w.as_str()) && w.len() > 1)
        .take(6)
        .collect();

    if words.is_empty() {
        return uuid::Uuid::new_v4().to_string()[..8].to_string();
    }
    words.join("_")
}