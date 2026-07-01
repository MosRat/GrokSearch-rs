use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Map<String, Value>,
}

pub fn tools() -> Vec<ToolSpec> {
    tools_list_json()["tools"]
        .as_array()
        .expect("tools_list is an array")
        .iter()
        .map(tool_from_value)
        .collect()
}

fn tool_from_value(value: &Value) -> ToolSpec {
    ToolSpec {
        name: value["name"].as_str().expect("tool name").to_string(),
        description: value["description"]
            .as_str()
            .expect("tool description")
            .to_string(),
        input_schema: value["inputSchema"]
            .as_object()
            .expect("input schema")
            .clone(),
    }
}

pub(crate) const TOOLS_SPEC_JSON: &str = include_str!("../spec/tools.json");

pub fn tools_list_json() -> Value {
    serde_json::from_str(TOOLS_SPEC_JSON).expect("embedded tools spec JSON must be valid")
}
