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
            intent: description.to_string(),
            status: TaskStatus::Pending,
            role, // Assign the specific specialist
            result: None,
            depends_on: dependencies,
        };
        self.tasks.push(task);
        id
    }
}