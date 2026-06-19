//! JSON Schema generation for tool definitions
//!
//! Automatically generates OpenAI-compatible JSON Schema definitions from Rust structs
//! using the `schemars` crate.

use schemars::schema::{InstanceType, Schema, SchemaObject, StringValidation};
use schemars::{schema_for, JsonSchema};
use serde_json::{json, Value};

use crate::error::{Error, Result};

/// Sanitize a tool name to be compatible with provider APIs (Google, OpenAI).
///
/// Google requires: `[a-zA-Z_][a-zA-Z0-9_.:-]{0,127}`
/// This function replaces invalid characters with `_`, ensures the name starts
/// with a letter or underscore, and truncates to 128 characters.
pub fn sanitize_tool_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == ':' || ch == '-' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    // Ensure starts with letter or underscore
    if result.is_empty()
        || (!result.as_bytes()[0].is_ascii_alphabetic() && result.as_bytes()[0] != b'_')
    {
        result.insert(0, '_');
    }
    // Truncate to 128 chars
    if result.len() > 128 {
        result.truncate(128);
    }
    result
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSchema {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: sanitize_tool_name(&name.into()),
            description: description.into(),
            parameters,
        }
    }

    pub fn from_type<T: JsonSchema>(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let parameters = schema_for!(T);
        let parameters_value = serde_json::to_value(&parameters).unwrap_or_else(|_| {
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            })
        });

        Self {
            name: sanitize_tool_name(&name.into()),
            description: description.into(),
            parameters: parameters_value,
        }
    }

    /// Validate arguments against the schema
    pub fn validate_args(&self, args: &Value) -> Result<()> {
        // Basic validation - check if args is an object
        if !args.is_object() {
            return Err(Error::InvalidToolArgs {
                name: self.name.clone(),
                details: "Arguments must be a JSON object".to_string(),
            });
        }

        // For now, we do basic structural validation
        // A full JSON Schema validator would be more robust
        if let Some(required) = self.parameters.get("required").and_then(|r| r.as_array()) {
            for req_field in required {
                if let Some(key) = req_field.as_str() {
                    if args.get(key).is_none() {
                        return Err(Error::InvalidToolArgs {
                            name: self.name.clone(),
                            details: format!("Missing required field: {}", key),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

/// Generate an OpenAI-compatible tools array from a list of ToolSchemas
pub fn to_openai_tools(schemas: &[ToolSchema]) -> Value {
    let tools: Vec<Value> = schemas
        .iter()
        .map(|schema| {
            json!({
                "type": "function",
                "function": {
                    "name": schema.name,
            "description": schema.description,
                    "parameters": schema.parameters
                }
            })
        })
        .collect();

    json!({ "tools": tools })
}

/// Schema generator for creating custom schemas programmatically
pub struct SchemaGenerator;

impl SchemaGenerator {
    /// Create a schema for a string parameter
    pub fn string_param(name: &str, description: &str) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            string: Some(Box::new(StringValidation {
                max_length: None,
                min_length: None,
                pattern: None,
            })),
            metadata: Some(Box::new(schemars::schema::Metadata {
                id: None,
                title: Some(name.to_string()),
                description: Some(description.to_string()),
                default: None,
                deprecated: false,
                read_only: false,
                write_only: false,
                examples: vec![],
            })),
            ..Default::default()
        })
    }

    /// Create a schema for an integer parameter
    pub fn integer_param(name: &str, description: &str) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Integer.into()),
            metadata: Some(Box::new(schemars::schema::Metadata {
                id: None,
                title: Some(name.to_string()),
                description: Some(description.to_string()),
                default: None,
                deprecated: false,
                read_only: false,
                write_only: false,
                examples: vec![],
            })),
            ..Default::default()
        })
    }

    /// Create a schema for a boolean parameter
    pub fn boolean_param(name: &str, description: &str) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Boolean.into()),
            metadata: Some(Box::new(schemars::schema::Metadata {
                id: None,
                title: Some(name.to_string()),
                description: Some(description.to_string()),
                default: None,
                deprecated: false,
                read_only: false,
                write_only: false,
                examples: vec![],
            })),
            ..Default::default()
        })
    }

    /// Create a schema for an array parameter
    pub fn array_param(name: &str, description: &str, items: Schema) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Array.into()),
            array: Some(Box::new(schemars::schema::ArrayValidation {
                items: Some(items.into()),
                ..Default::default()
            })),
            metadata: Some(Box::new(schemars::schema::Metadata {
                id: None,
                title: Some(name.to_string()),
                description: Some(description.to_string()),
                default: None,
                deprecated: false,
                read_only: false,
                write_only: false,
                examples: vec![],
            })),
            ..Default::default()
        })
    }

    /// Create a schema for an object parameter
    pub fn object_param(name: &str, description: &str, properties: Vec<(&str, Schema)>) -> Schema {
        let mut obj = schemars::schema::ObjectValidation::default();
        for (prop_name, prop_schema) in properties {
            obj.properties.insert(prop_name.to_string(), prop_schema);
        }

        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Object.into()),
            object: Some(Box::new(obj)),
            metadata: Some(Box::new(schemars::schema::Metadata {
                id: None,
                title: Some(name.to_string()),
                description: Some(description.to_string()),
                default: None,
                deprecated: false,
                read_only: false,
                write_only: false,
                examples: vec![],
            })),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;

    #[derive(JsonSchema)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct TestParams {
        query: String,
        limit: Option<i32>,
    }

    #[test]
    fn test_schema_generation() {
        let schema = ToolSchema::from_type::<TestParams>("test_tool", "A test tool");

        assert_eq!(schema.name, "test_tool");
        assert_eq!(schema.description, "A test tool");
        assert!(schema.parameters.is_object());
    }

    #[test]
    fn test_openai_tools_format() {
        let schemas = vec![
            ToolSchema::from_type::<TestParams>("tool1", "First tool"),
            ToolSchema::from_type::<TestParams>("tool2", "Second tool"),
        ];

        let tools = to_openai_tools(&schemas);

        assert!(tools.get("tools").is_some());
        let tools_array = tools.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools_array.len(), 2);
    }
}
