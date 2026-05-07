use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum AgentRole {
    WebSearcher,
    Analyst,
    Planner,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Task {
    pub id: Uuid,
    pub intent: String,
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
            AgentRole::WebSearcher => "Web Searcher",
            AgentRole::Analyst => "Analyst",
            AgentRole::Planner => "Planner",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ContextBus {
    pub data: HashMap<String, String>, // Shared key-value store
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameters,
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
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["operation".to_string(), "values".to_string()],
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
            parameters: ToolParameters {
                param_type: "object".to_string(),
                properties,
                required: vec!["query".to_string()],
            },
        }
    }
}
