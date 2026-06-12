//! Lua-defined tool definitions extracted from plugin tables.
//!
//! Plugins declare tools in their `init.lua` via a simplified DSL:
//!
//! ```lua
//! M.tools = {
//!     {
//!         name = "judgment_failed",
//!         description = "Call when the response fails evaluation",
//!         parameters = {
//!             { name = "message", type = "string", description = "Why it failed" },
//!         },
//!         handler = function(ctx, args) ... end,
//!     },
//! }
//! ```
//!
//! The simplified parameter DSL is expanded to full JSON Schema on the Rust side.
//! The `PluginToolDef` struct stores the raw tool metadata alongside a Lua registry
//! key for the handler function. The domain layer converts these into `ToolDefinition`
//! instances when registering with the tools actor.

/// Whether a plugin tool is available globally or only in the session it's attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolScope {
    /// Available in all sessions.
    Global,
    /// Available only in the session the plugin is attached to.
    Attached,
}

impl Default for ToolScope {
    fn default() -> Self {
        Self::Attached
    }
}
use mlua::RegistryKey;
use serde::{Deserialize, Serialize};

/// A tool defined by a Lua plugin.
pub struct PluginToolDef {
    /// Tool name (e.g., "judgment_passed").
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema for the tool's input parameters (expanded from simplified DSL).
    pub parameters: serde_json::Value,
    /// Registry key for the Lua handler function in the plugin's Lua state.
    pub handler_key: RegistryKey,
    /// Plugin name that owns this tool.
    pub plugin_name: String,
    /// Whether this tool is global or session-attached.
    pub scope: ToolScope,
}

/// Send-safe metadata for a plugin-defined tool, used to communicate tool
/// definitions from the plugin thread back to the domain layer.
///
/// Unlike `PluginToolDef`, this does not contain a Lua registry key and can
/// be sent across thread boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolMetadata {
    /// Tool name (e.g., "judgment_passed").
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub parameters: serde_json::Value,
    /// Plugin name that owns this tool.
    pub plugin_name: String,
    /// Whether this tool is global or session-attached.
    pub scope: ToolScope,
}

impl PluginToolDef {
    /// Convert to send-safe metadata for crossing thread boundaries.
    pub fn to_metadata(&self) -> PluginToolMetadata {
        PluginToolMetadata {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
            plugin_name: self.plugin_name.clone(),
            scope: self.scope.clone(),
        }
    }
}

/// Intermediate representation of a simplified parameter DSL entry read from Lua.
pub(crate) struct LuaParamDef {
    /// Parameter name.
    name: String,
    /// Parameter type (e.g., "string", "number", "boolean").
    param_type: String,
    /// Optional human-readable description.
    description: Option<String>,
}

/// Expands simplified parameter DSL entries to full JSON Schema.
///
/// Input: `[{ name = "msg", type = "string", description = "Why" }]`
/// Output: `{ type: "object", properties: { msg: { type: "string", description: "Why" } }, required: ["msg"] }`
pub(crate) fn expand_parameters_to_schema(params: &[LuaParamDef]) -> serde_json::Value {
    use serde_json::{Map, Value};

    if params.is_empty() {
        return serde_json::json!({
            "type": "object",
            "properties": {},
        });
    }

    let mut properties = Map::new();
    let mut required = Vec::new();

    for param in params {
        let mut prop = Map::new();
        prop.insert("type".to_owned(), Value::String(param.param_type.clone()));
        if let Some(ref desc) = param.description {
            prop.insert("description".to_owned(), Value::String(desc.clone()));
        }
        properties.insert(param.name.clone(), Value::Object(prop));
        required.push(Value::String(param.name.clone()));
    }

    serde_json::json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": required,
    })
}

/// Extracts tool definitions from a plugin's returned Lua table.
///
/// Reads the `tools` field (an array of tool entries). For each entry:
/// - `name` (string) — tool name
/// - `description` (string) — human-readable description
/// - `parameters` (array of `{ name, type, description? }`) — simplified DSL
/// - `handler` (function) — Lua function to call when the LLM invokes this tool
///
/// Returns a list of parsed tool definitions with handler registry keys.
/// Malformed entries are logged and skipped.
pub fn extract_tools(
    lua: &mlua::Lua,
    plugin_table: &mlua::Table,
    plugin_name: &str,
) -> Vec<PluginToolDef> {
    let tools_value: mlua::Value = match plugin_table.get("tools") {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let tools_table: mlua::Table = match tools_value {
        mlua::Value::Table(t) => t,
        _ => return Vec::new(),
    };

    tools_table
        .sequence_values::<mlua::Table>()
        .filter_map(|pair| {
            let entry = match pair {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        plugin = plugin_name,
                        err = %e,
                        "skipping malformed tool entry in plugin"
                    );
                    return None;
                }
            };
            extract_single_tool(lua, &entry, plugin_name)
        })
        .collect()
}

fn extract_single_tool(
    lua: &mlua::Lua,
    entry: &mlua::Table,
    plugin_name: &str,
) -> Option<PluginToolDef> {
    let name: String = entry.get("name").ok()?;
    let description: String = entry.get("description").unwrap_or_default();
    let handler: mlua::Function = entry.get("handler").ok()?;

    // Extract simplified parameter DSL.
    let params = extract_params(entry);
    let parameters = expand_parameters_to_schema(&params);

    // Extract scope (defaults to Attached for safety).
    let scope: ToolScope = match entry.get::<String>("scope") {
        Ok(s) if s == "global" => ToolScope::Global,
        Ok(s) if s == "attached" => ToolScope::Attached,
        Ok(other) => {
            tracing::warn!(
                plugin = plugin_name,
                tool = name,
                scope = %other,
                "unknown tool scope, defaulting to attached"
            );
            ToolScope::default()
        }
        Err(_) => ToolScope::default(),
    };

    let handler_key = lua.create_registry_value(handler).ok()?;

    Some(PluginToolDef {
        name,
        description,
        parameters,
        handler_key,
        plugin_name: plugin_name.to_owned(),
        scope,
    })
}

fn extract_params(entry: &mlua::Table) -> Vec<LuaParamDef> {
    let params_value: mlua::Value = match entry.get("parameters") {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let params_table: mlua::Table = match params_value {
        mlua::Value::Table(t) => t,
        _ => return Vec::new(),
    };

    params_table
        .sequence_values::<mlua::Table>()
        .filter_map(|pair| pair.ok())
        .filter_map(|t| {
            let name: String = t.get("name").ok()?;
            let param_type: String = t.get("type").ok()?;
            let description: Option<String> = t.get("description").ok();
            Some(LuaParamDef {
                name,
                param_type,
                description,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        reason = "test code"
    )]

    use super::*;

    #[test]
    fn expand_empty_params_produces_empty_object_schema() {
        // Given no parameters.
        let schema = expand_parameters_to_schema(&[]);

        // Then the schema is an empty object type.
        assert_eq!(schema["type"], "object");
        assert_eq!(
            schema["properties"].as_object().map(serde_json::Map::len),
            Some(0)
        );
    }

    #[test]
    fn expand_single_param_produces_required_field() {
        // Given a single parameter.
        let params = [LuaParamDef {
            name: "message".to_owned(),
            param_type: "string".to_owned(),
            description: Some("Why it failed".to_owned()),
        }];

        // When expanding.
        let schema = expand_parameters_to_schema(&params);

        // Then the property has type and description.
        let props = schema["properties"].as_object().expect("properties");
        let msg = &props["message"];
        assert_eq!(msg["type"], "string");
        assert_eq!(msg["description"], "Why it failed");

        // And the field is required.
        let required = schema["required"].as_array().expect("required");
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "message");
    }

    #[test]
    fn expand_param_without_description_omits_description() {
        // Given a parameter with no description.
        let params = [LuaParamDef {
            name: "count".to_owned(),
            param_type: "number".to_owned(),
            description: None,
        }];

        // When expanding.
        let schema = expand_parameters_to_schema(&params);

        // Then the property has no description field.
        let props = schema["properties"].as_object().expect("properties");
        let count = &props["count"];
        assert_eq!(count["type"], "number");
        assert!(!count.as_object().expect("prop").contains_key("description"));
    }

    #[test]
    fn extract_tools_from_lua_table() {
        // Given a Lua plugin table with a tools field.
        let lua = mlua::Lua::new();
        let source = r#"
            local M = {}
            M.tools = {
                {
                    name = "greet",
                    description = "Say hello",
                    parameters = {
                        { name = "name", type = "string", description = "Who to greet" },
                    },
                    handler = function(ctx, args) return "hello " .. args.name end,
                },
            }
            return M
        "#;
        let table: mlua::Table = lua.load(source).eval().expect("eval");

        // When extracting tools.
        let tools = extract_tools(&lua, &table, "test_plugin");

        // Then one tool is extracted.
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "greet");
        assert_eq!(tools[0].description, "Say hello");
        assert_eq!(tools[0].plugin_name, "test_plugin");

        // And the parameters were expanded.
        let params = &tools[0].parameters;
        assert_eq!(params["type"], "object");
        let props = params["properties"].as_object().expect("properties");
        assert!(props.contains_key("name"));
    }

    #[test]
    fn extract_tools_returns_empty_when_no_tools_field() {
        // Given a plugin table without a tools field.
        let lua = mlua::Lua::new();
        let table: mlua::Table = lua.load("return {}").eval().expect("eval");

        // When extracting tools.
        let tools = extract_tools(&lua, &table, "no_tools");

        // Then no tools are extracted.
        assert!(tools.is_empty());
    }

    #[test]
    fn extract_tools_skips_malformed_entries() {
        // Given a plugin table with one good and one bad tool entry.
        let lua = mlua::Lua::new();
        let source = r#"
            local M = {}
            M.tools = {
                { name = "good", description = "A good tool", handler = function() end },
                { description = "Missing name and handler" },
            }
            return M
        "#;
        let table: mlua::Table = lua.load(source).eval().expect("eval");

        // When extracting tools.
        let tools = extract_tools(&lua, &table, "mixed");

        // Then only the good tool is extracted.
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "good");
    }

    #[test]
    fn tool_handler_is_callable_via_registry_key() {
        // Given a tool with a handler function.
        let lua = mlua::Lua::new();
        let source = r#"
            local M = {}
            M.tools = {
                {
                    name = "echo",
                    description = "Echo input",
                    handler = function(ctx, args) return args.text end,
                },
            }
            return M
        "#;
        let table: mlua::Table = lua.load(source).eval().expect("eval");
        let tools = extract_tools(&lua, &table, "echo_plugin");

        // When calling the handler via its registry key.
        let func: mlua::Function = lua
            .registry_value(&tools[0].handler_key)
            .expect("get handler");
        let args = lua.create_table().expect("args");
        args.set("text", "hello").expect("set arg");
        let result: String = func.call((mlua::Value::Nil, args)).expect("call");

        // Then the handler returns the expected result.
        assert_eq!(result, "hello");
    }
}

impl From<PluginToolMetadata> for crate::feat::plugin_system::PluginToolMetadata {
    fn from(value: PluginToolMetadata) -> Self {
        Self {
            name: value.name,
            description: value.description,
            parameters: value.parameters,
            plugin_name: value.plugin_name,
            scope: value.scope.into(),
        }
    }
}

impl From<ToolScope> for crate::feat::plugin_system::ToolScope {
    fn from(value: ToolScope) -> Self {
        match value {
            ToolScope::Global => Self::Global,
            ToolScope::Attached => Self::Attached,
        }
    }
}
