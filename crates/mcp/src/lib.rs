use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolFilter {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDefinition {
    pub config: McpServerConfig,
    #[serde(default)]
    pub filter: ToolFilter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpStartupStatus {
    Starting,
    Ready,
    Failed { error: String },
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpStartupUpdateEvent {
    pub server_name: String,
    pub status: McpStartupStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpStartupFailure {
    pub server_name: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpStartupCompleteEvent {
    pub ready: Vec<String>,
    pub failed: Vec<McpStartupFailure>,
    pub cancelled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDescriptor {
    pub server_name: String,
    pub tool_name: String,
    pub qualified_name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceDescriptor {
    pub server_name: String,
    pub uri: String,
    pub description: Option<String>,
}

pub trait McpManagedClient: Send + Sync {
    fn list_tools(&self) -> Result<Vec<McpToolDescriptor>>;
    fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value>;
    fn list_resources(&self) -> Result<Vec<McpResourceDescriptor>>;
    fn read_resource(&self, uri: &str) -> Result<Value>;
}

#[derive(Debug, Default)]
pub struct InMemoryMcpClient {
    tools: HashMap<String, Value>,
    resources: HashMap<String, Value>,
}

impl InMemoryMcpClient {
    pub fn with_tool(mut self, name: &str, sample_result: Value) -> Self {
        self.tools.insert(name.to_string(), sample_result);
        self
    }

    pub fn with_resource(mut self, uri: &str, data: Value) -> Self {
        self.resources.insert(uri.to_string(), data);
        self
    }
}

impl McpManagedClient for InMemoryMcpClient {
    fn list_tools(&self) -> Result<Vec<McpToolDescriptor>> {
        Ok(self
            .tools
            .keys()
            .map(|name| McpToolDescriptor {
                server_name: "in-memory".to_string(),
                tool_name: name.clone(),
                qualified_name: name.clone(),
                description: None,
            })
            .collect())
    }

    fn call_tool(&self, tool_name: &str, _arguments: Value) -> Result<Value> {
        self.tools
            .get(tool_name)
            .cloned()
            .with_context(|| format!("tool '{tool_name}' not found"))
    }

    fn list_resources(&self) -> Result<Vec<McpResourceDescriptor>> {
        Ok(self
            .resources
            .keys()
            .map(|uri| McpResourceDescriptor {
                server_name: "in-memory".to_string(),
                uri: uri.clone(),
                description: None,
            })
            .collect())
    }

    fn read_resource(&self, uri: &str) -> Result<Value> {
        self.resources
            .get(uri)
            .cloned()
            .with_context(|| format!("resource '{uri}' not found"))
    }
}

#[derive(Default)]
pub struct McpManager {
    configs: HashMap<String, (McpServerConfig, ToolFilter)>,
    clients: HashMap<String, Box<dyn McpManagedClient>>,
}

impl McpManager {
    pub fn register_server(
        &mut self,
        config: McpServerConfig,
        filter: ToolFilter,
        client: Box<dyn McpManagedClient>,
    ) {
        self.clients.insert(config.name.clone(), client);
        self.configs.insert(config.name.clone(), (config, filter));
    }

    pub fn start_all<F>(&self, mut emit: F) -> McpStartupCompleteEvent
    where
        F: FnMut(McpStartupUpdateEvent),
    {
        let mut ready = Vec::new();
        let mut failed = Vec::new();
        let mut cancelled = Vec::new();
        for (server_name, (cfg, _)) in &self.configs {
            if !cfg.enabled {
                emit(McpStartupUpdateEvent {
                    server_name: server_name.clone(),
                    status: McpStartupStatus::Cancelled,
                });
                cancelled.push(server_name.clone());
                continue;
            }
            emit(McpStartupUpdateEvent {
                server_name: server_name.clone(),
                status: McpStartupStatus::Starting,
            });
            if self.clients.contains_key(server_name) {
                emit(McpStartupUpdateEvent {
                    server_name: server_name.clone(),
                    status: McpStartupStatus::Ready,
                });
                ready.push(server_name.clone());
            } else {
                let error = "client not registered".to_string();
                emit(McpStartupUpdateEvent {
                    server_name: server_name.clone(),
                    status: McpStartupStatus::Failed {
                        error: error.clone(),
                    },
                });
                failed.push(McpStartupFailure {
                    server_name: server_name.clone(),
                    error,
                });
            }
        }
        McpStartupCompleteEvent {
            ready,
            failed,
            cancelled,
        }
    }

    pub fn stop_server(&mut self, server_name: &str) -> Result<()> {
        self.clients
            .remove(server_name)
            .with_context(|| format!("server '{server_name}' is not running"))?;
        Ok(())
    }

    pub fn unregister_server(&mut self, server_name: &str) -> Result<()> {
        let had_config = self.configs.remove(server_name).is_some();
        self.clients.remove(server_name);
        if !had_config {
            bail!("server '{server_name}' is not registered");
        }
        Ok(())
    }

    pub fn list_tools(&self) -> Result<Vec<McpToolDescriptor>> {
        let mut out = Vec::new();
        for (server_name, (_, filter)) in &self.configs {
            let Some(client) = self.clients.get(server_name) else {
                continue;
            };
            let tools = client.list_tools()?;
            for tool in tools {
                if !allowed_by_filter(&tool.tool_name, filter) {
                    continue;
                }
                let qualified_name = qualify_tool_name(server_name, &tool.tool_name);
                out.push(McpToolDescriptor {
                    server_name: server_name.clone(),
                    tool_name: tool.tool_name,
                    qualified_name,
                    description: tool.description,
                });
            }
        }
        Ok(out)
    }

    pub fn call_tool(&self, server_name: &str, tool_name: &str, arguments: Value) -> Result<Value> {
        let client = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{server_name}' not available"))?;
        client.call_tool(tool_name, arguments)
    }

    pub fn call_qualified_tool(
        &self,
        qualified_tool_name: &str,
        arguments: Value,
    ) -> Result<Value> {
        let (server_name, tool_name) = parse_qualified_tool_name(qualified_tool_name)
            .with_context(|| format!("invalid qualified MCP tool name: {qualified_tool_name}"))?;
        self.call_tool(&server_name, &tool_name, arguments)
    }

    pub fn list_resources(&self) -> Result<Vec<McpResourceDescriptor>> {
        let mut out = Vec::new();
        for server_name in self.configs.keys() {
            let Some(client) = self.clients.get(server_name) else {
                continue;
            };
            for mut resource in client.list_resources()? {
                resource.server_name = server_name.clone();
                out.push(resource);
            }
        }
        Ok(out)
    }

    pub fn read_resource(&self, server_name: &str, uri: &str) -> Result<Value> {
        let client = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{server_name}' not available"))?;
        client.read_resource(uri)
    }

    pub fn update_sandbox_state(&self, sandbox_mode: &str, cwd: &str) -> Result<Vec<Value>> {
        let mut notices = Vec::new();
        for server_name in self.configs.keys() {
            notices.push(json!({
                "server_name": server_name,
                "method": "codex/sandbox-state/update",
                "params": {
                    "sandbox_mode": sandbox_mode,
                    "cwd": cwd
                }
            }));
        }
        Ok(notices)
    }
}

fn default_true() -> bool {
    true
}

fn allowed_by_filter(name: &str, filter: &ToolFilter) -> bool {
    if filter.deny.iter().any(|pattern| pattern == name) {
        return false;
    }
    if filter.allow.is_empty() {
        return true;
    }
    filter.allow.iter().any(|pattern| pattern == name)
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn qualify_tool_name(server: &str, tool: &str) -> String {
    let mut name = format!(
        "mcp__{}__{}",
        sanitize_component(server),
        sanitize_component(tool)
    );
    if name.len() > 64 {
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        let hash = format!("{:x}", hasher.finish());
        name.truncate(48);
        name.push('_');
        name.push_str(&hash[..12]);
    }
    name
}

fn parse_qualified_tool_name(value: &str) -> Result<(String, String)> {
    let Some(stripped) = value.strip_prefix("mcp__") else {
        bail!("missing mcp__ prefix");
    };
    let mut split = stripped.splitn(2, "__");
    let server = split
        .next()
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .context("missing server segment")?;
    let tool = split
        .next()
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .context("missing tool segment")?;
    Ok((server, tool))
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    jsonrpc: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug)]
struct JsonRpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ToolsListParams {
    #[serde(default)]
    server: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct ResourcesListParams {
    #[serde(default)]
    server: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResourcesReadParams {
    #[serde(default)]
    server: Option<String>,
    uri: String,
}

#[derive(Debug, Deserialize)]
struct ServerRegisterParams {
    server: McpServerConfig,
    #[serde(default)]
    filter: ToolFilter,
    #[serde(default = "default_true")]
    start: bool,
}

#[derive(Debug, Deserialize)]
struct ServerNameParams {
    name: String,
}

struct StdioMcpState {
    manager: McpManager,
    definitions: HashMap<String, McpServerDefinition>,
    running: HashMap<String, bool>,
    lifecycle_state: String,
}

pub fn run_stdio_server(
    initial_definitions: Vec<McpServerDefinition>,
) -> Result<Vec<McpServerDefinition>> {
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut state = build_stdio_state(initial_definitions);

    for line in stdin.lock().lines() {
        let line = line.context("failed to read stdio line")?;
        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                let msg = jsonrpc_error(
                    None,
                    JsonRpcError::parse_error(format!("invalid json: {err}")),
                );
                writeln!(stdout, "{msg}")?;
                stdout.flush()?;
                continue;
            }
        };

        if request
            .jsonrpc
            .as_deref()
            .is_some_and(|version| version != "2.0")
        {
            let response = jsonrpc_error(
                request.id,
                JsonRpcError::invalid_request("jsonrpc version must be 2.0"),
            );
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
            continue;
        }

        let response = match dispatch_stdio_request(&mut state, &request.method, request.params) {
            Ok((result, should_exit)) => {
                let payload = jsonrpc_result(request.id, result);
                writeln!(stdout, "{payload}")?;
                stdout.flush()?;
                if should_exit {
                    break;
                }
                continue;
            }
            Err(err) => jsonrpc_error(request.id, err),
        };

        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }

    state.lifecycle_state = "stopped".to_string();
    let _ = writeln!(stderr, "deepseek-mcp stdio server exited");
    let mut definitions: Vec<McpServerDefinition> = state.definitions.into_values().collect();
    definitions.sort_by(|a, b| a.config.name.cmp(&b.config.name));
    Ok(definitions)
}

fn build_stdio_state(initial_definitions: Vec<McpServerDefinition>) -> StdioMcpState {
    let mut manager = McpManager::default();
    let mut definitions = HashMap::new();
    let mut running = HashMap::new();

    for definition in initial_definitions {
        let name = definition.config.name.clone();
        let should_start = definition.config.enabled;
        definitions.insert(name.clone(), definition.clone());
        if should_start {
            manager.register_server(
                definition.config.clone(),
                definition.filter.clone(),
                default_stdio_client(&name),
            );
            running.insert(name, true);
        } else {
            running.insert(name, false);
        }
    }

    StdioMcpState {
        manager,
        definitions,
        running,
        lifecycle_state: "running".to_string(),
    }
}

fn default_stdio_client(server_name: &str) -> Box<dyn McpManagedClient> {
    let health_uri = format!("mcp://{server_name}/health");
    let capabilities_uri = format!("mcp://{server_name}/capabilities");
    Box::new(
        InMemoryMcpClient::default()
            .with_tool(
                "health",
                json!({
                    "status": "ok",
                    "server_name": server_name
                }),
            )
            .with_tool(
                "capabilities",
                json!({
                    "tools": ["health", "capabilities"],
                    "resources": [health_uri.clone(), capabilities_uri.clone()]
                }),
            )
            .with_resource(
                &health_uri,
                json!({
                    "status": "ok",
                    "server_name": server_name
                }),
            )
            .with_resource(
                &capabilities_uri,
                json!({
                    "server_name": server_name,
                    "methods": [
                        "tools/list",
                        "tools/call",
                        "resources/list",
                        "resources/read",
                        "server/list",
                        "server/register",
                        "server/start",
                        "server/stop",
                        "server/unregister"
                    ]
                }),
            ),
    )
}

fn default_rpc_methods() -> Vec<&'static str> {
    vec![
        "initialize",
        "healthz",
        "capabilities",
        "tools/list",
        "tools/call",
        "resources/list",
        "resources/read",
        "server/list",
        "server/register",
        "server/start",
        "server/stop",
        "server/unregister",
        "shutdown",
    ]
}

fn lifecycle_snapshot(state: &StdioMcpState) -> Value {
    let mut servers: Vec<Value> = state
        .definitions
        .iter()
        .map(|(name, definition)| {
            let is_running = state.running.get(name).copied().unwrap_or(false);
            json!({
                "name": name,
                "enabled": definition.config.enabled,
                "running": is_running,
                "command": definition.config.command.clone(),
                "args": definition.config.args.clone(),
            })
        })
        .collect();
    servers.sort_by(|a, b| {
        let a_name = a.get("name").and_then(Value::as_str).unwrap_or_default();
        let b_name = b.get("name").and_then(Value::as_str).unwrap_or_default();
        a_name.cmp(b_name)
    });

    let running_count = state.running.values().filter(|running| **running).count();
    json!({
        "status": state.lifecycle_state,
        "servers": servers,
        "counts": {
            "defined": state.definitions.len(),
            "running": running_count
        }
    })
}

fn params_or_object(params: Value) -> Value {
    if params.is_null() { json!({}) } else { params }
}

fn parse_params<T: DeserializeOwned>(params: Value) -> std::result::Result<T, JsonRpcError> {
    serde_json::from_value(params).map_err(|err| JsonRpcError::invalid_params(err.to_string()))
}

fn parse_server_from_uri(uri: &str) -> Option<String> {
    let stripped = uri.strip_prefix("mcp://")?;
    let server = stripped.split('/').next()?;
    if server.is_empty() {
        None
    } else {
        Some(server.to_string())
    }
}

fn dispatch_stdio_request(
    state: &mut StdioMcpState,
    method: &str,
    params: Value,
) -> std::result::Result<(Value, bool), JsonRpcError> {
    match method {
        "initialize" | "capabilities" => Ok((
            json!({
                "server": "deepseek-mcp",
                "transport": "stdio",
                "methods": default_rpc_methods(),
                "lifecycle": lifecycle_snapshot(state)
            }),
            false,
        )),
        "healthz" => Ok((
            json!({
                "status": "ok",
                "service": "deepseek-mcp",
                "transport": "stdio",
                "lifecycle": lifecycle_snapshot(state)
            }),
            false,
        )),
        "tools/list" => {
            let parsed: ToolsListParams = parse_params(params_or_object(params))?;
            let mut tools = state
                .manager
                .list_tools()
                .map_err(|err| JsonRpcError::internal(err.to_string()))?;
            if let Some(server) = parsed.server {
                tools.retain(|tool| tool.server_name == server);
            }
            Ok((json!({ "tools": tools }), false))
        }
        "tools/call" => {
            let parsed: ToolsCallParams = parse_params(params_or_object(params))?;
            let ToolsCallParams {
                name,
                tool,
                server,
                arguments,
            } = parsed;
            let tool_name = name
                .or(tool)
                .context("missing tool name")
                .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
            let arguments = if arguments.is_null() {
                json!({})
            } else {
                arguments
            };
            let result = if tool_name.starts_with("mcp__") {
                state
                    .manager
                    .call_qualified_tool(&tool_name, arguments)
                    .map_err(|err| JsonRpcError::internal(err.to_string()))?
            } else {
                let server = server
                    .context("missing server for unqualified tool")
                    .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
                state
                    .manager
                    .call_tool(&server, &tool_name, arguments)
                    .map_err(|err| JsonRpcError::internal(err.to_string()))?
            };
            Ok((json!({ "result": result }), false))
        }
        "resources/list" => {
            let parsed: ResourcesListParams = parse_params(params_or_object(params))?;
            let mut resources = state
                .manager
                .list_resources()
                .map_err(|err| JsonRpcError::internal(err.to_string()))?;
            if let Some(server) = parsed.server {
                resources.retain(|resource| resource.server_name == server);
            }
            Ok((json!({ "resources": resources }), false))
        }
        "resources/read" => {
            let parsed: ResourcesReadParams = parse_params(params_or_object(params))?;
            let ResourcesReadParams { server, uri } = parsed;
            let server_name = server
                .or_else(|| parse_server_from_uri(&uri))
                .context("missing server for resource read")
                .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
            let value = state
                .manager
                .read_resource(&server_name, &uri)
                .map_err(|err| JsonRpcError::internal(err.to_string()))?;
            Ok((json!({ "resource": value }), false))
        }
        "server/list" | "servers/list" => {
            Ok((json!({ "lifecycle": lifecycle_snapshot(state) }), false))
        }
        "server/register" | "servers/register" => {
            let parsed: ServerRegisterParams = parse_params(params_or_object(params))?;
            let name = parsed.server.name.clone();
            if name.trim().is_empty() {
                return Err(JsonRpcError::invalid_params(
                    "server.name must not be empty",
                ));
            }

            if state.definitions.contains_key(&name) {
                let _ = state.manager.unregister_server(&name);
            }
            state.definitions.insert(
                name.clone(),
                McpServerDefinition {
                    config: parsed.server.clone(),
                    filter: parsed.filter.clone(),
                },
            );
            let should_run = parsed.start && parsed.server.enabled;
            if should_run {
                state.manager.register_server(
                    parsed.server.clone(),
                    parsed.filter.clone(),
                    default_stdio_client(&name),
                );
            }
            state.running.insert(name, should_run);
            Ok((json!({ "lifecycle": lifecycle_snapshot(state) }), false))
        }
        "server/start" | "servers/start" => {
            let parsed: ServerNameParams = parse_params(params_or_object(params))?;
            let definition = state
                .definitions
                .get(&parsed.name)
                .cloned()
                .with_context(|| format!("server '{}' is not defined", parsed.name))
                .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
            if !definition.config.enabled {
                return Err(JsonRpcError::invalid_params(format!(
                    "server '{}' is disabled",
                    parsed.name
                )));
            }
            if !state.running.get(&parsed.name).copied().unwrap_or(false) {
                state.manager.register_server(
                    definition.config.clone(),
                    definition.filter.clone(),
                    default_stdio_client(&parsed.name),
                );
                state.running.insert(parsed.name, true);
            }
            Ok((json!({ "lifecycle": lifecycle_snapshot(state) }), false))
        }
        "server/stop" | "servers/stop" => {
            let parsed: ServerNameParams = parse_params(params_or_object(params))?;
            if state.running.get(&parsed.name).copied().unwrap_or(false) {
                state
                    .manager
                    .stop_server(&parsed.name)
                    .map_err(|err| JsonRpcError::internal(err.to_string()))?;
            }
            state.running.insert(parsed.name, false);
            Ok((json!({ "lifecycle": lifecycle_snapshot(state) }), false))
        }
        "server/unregister" | "servers/unregister" => {
            let parsed: ServerNameParams = parse_params(params_or_object(params))?;
            if state.definitions.remove(&parsed.name).is_none() {
                return Err(JsonRpcError::invalid_params(format!(
                    "server '{}' is not defined",
                    parsed.name
                )));
            }
            let _ = state.manager.unregister_server(&parsed.name);
            state.running.remove(&parsed.name);
            Ok((json!({ "lifecycle": lifecycle_snapshot(state) }), false))
        }
        "shutdown" => {
            state.lifecycle_state = "shutting_down".to_string();
            Ok((
                json!({
                    "ok": true,
                    "lifecycle": lifecycle_snapshot(state)
                }),
                true,
            ))
        }
        _ => Err(JsonRpcError::method_not_found(method)),
    }
}

fn jsonrpc_result(id: Option<Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": result
    })
}

fn jsonrpc_error(id: Option<Value>, err: JsonRpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": err.code,
            "message": err.message,
            "data": err.data
        }
    })
}

impl JsonRpcError {
    fn parse_error(message: impl Into<String>) -> Self {
        Self {
            code: -32700,
            message: message.into(),
            data: None,
        }
    }

    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            data: None,
        }
    }

    fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("unsupported method: {method}"),
            data: None,
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            data: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config(name: &str, enabled: bool) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
            enabled,
        }
    }

    // --- InMemoryMcpClient ---

    #[test]
    fn in_memory_client_lists_tools() {
        let client = InMemoryMcpClient::default()
            .with_tool("ping", json!({"pong": true}))
            .with_tool("echo", json!({"ok": true}));
        let tools = client.list_tools().unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().all(|t| t.server_name == "in-memory"));
    }

    #[test]
    fn in_memory_client_calls_existing_tool() {
        let client = InMemoryMcpClient::default().with_tool("ping", json!({"pong": true}));
        let result = client.call_tool("ping", json!({})).unwrap();
        assert_eq!(result, json!({"pong": true}));
    }

    #[test]
    fn in_memory_client_call_missing_tool_returns_error() {
        let client = InMemoryMcpClient::default();
        let err = client.call_tool("nonexistent", json!({})).unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn in_memory_client_lists_resources() {
        let client = InMemoryMcpClient::default()
            .with_resource("mcp://test/health", json!({"status": "ok"}));
        let resources = client.list_resources().unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].uri, "mcp://test/health");
    }

    #[test]
    fn in_memory_client_reads_resource() {
        let client =
            InMemoryMcpClient::default().with_resource("mcp://test/status", json!({"up": true}));
        let val = client.read_resource("mcp://test/status").unwrap();
        assert_eq!(val, json!({"up": true}));
    }

    #[test]
    fn in_memory_client_read_missing_resource_returns_error() {
        let client = InMemoryMcpClient::default();
        let err = client.read_resource("mcp://test/missing").unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    // --- ToolFilter ---

    #[test]
    fn filter_allow_empty_permits_all() {
        let filter = ToolFilter {
            allow: vec![],
            deny: vec![],
        };
        assert!(allowed_by_filter("anything", &filter));
    }

    #[test]
    fn filter_deny_blocks_exact_match() {
        let filter = ToolFilter {
            allow: vec![],
            deny: vec!["blocked".to_string()],
        };
        assert!(!allowed_by_filter("blocked", &filter));
        assert!(allowed_by_filter("allowed", &filter));
    }

    #[test]
    fn filter_allow_restricts_to_listed() {
        let filter = ToolFilter {
            allow: vec!["ping".to_string()],
            deny: vec![],
        };
        assert!(allowed_by_filter("ping", &filter));
        assert!(!allowed_by_filter("other", &filter));
    }

    #[test]
    fn filter_deny_takes_precedence_over_allow() {
        let filter = ToolFilter {
            allow: vec!["ping".to_string()],
            deny: vec!["ping".to_string()],
        };
        assert!(!allowed_by_filter("ping", &filter));
    }

    // --- qualify / parse tool names ---

    #[test]
    fn qualify_tool_name_basic() {
        let name = qualify_tool_name("my-server", "read_file");
        assert_eq!(name, "mcp__my_server__read_file");
    }

    #[test]
    fn qualify_tool_name_sanitizes_special_chars() {
        let name = qualify_tool_name("Server.One!", "tool@2");
        assert!(name.starts_with("mcp__"));
        assert!(!name.contains('.'));
        assert!(!name.contains('!'));
        assert!(!name.contains('@'));
    }

    #[test]
    fn qualify_tool_name_truncates_long_names() {
        let long_server = "a".repeat(40);
        let long_tool = "b".repeat(40);
        let name = qualify_tool_name(&long_server, &long_tool);
        assert!(name.len() <= 64, "qualified name too long: {}", name.len());
    }

    #[test]
    fn parse_qualified_tool_name_round_trips() {
        let qualified = qualify_tool_name("myserver", "mytool");
        let (server, tool) = parse_qualified_tool_name(&qualified).unwrap();
        assert_eq!(server, "myserver");
        assert_eq!(tool, "mytool");
    }

    #[test]
    fn parse_qualified_tool_name_rejects_missing_prefix() {
        assert!(parse_qualified_tool_name("no_prefix__server__tool").is_err());
    }

    #[test]
    fn parse_qualified_tool_name_rejects_missing_segments() {
        assert!(parse_qualified_tool_name("mcp__").is_err());
        assert!(parse_qualified_tool_name("mcp__server").is_err());
    }

    // --- sanitize_component ---

    #[test]
    fn sanitize_component_lowercases_and_replaces() {
        assert_eq!(sanitize_component("Hello-World"), "hello_world");
        assert_eq!(sanitize_component("foo_bar"), "foo_bar");
        assert_eq!(sanitize_component("A.B!C"), "a_b_c");
    }

    // --- parse_server_from_uri ---

    #[test]
    fn parse_server_from_uri_extracts_server() {
        assert_eq!(
            parse_server_from_uri("mcp://myserver/health"),
            Some("myserver".to_string())
        );
    }

    #[test]
    fn parse_server_from_uri_returns_none_for_invalid() {
        assert!(parse_server_from_uri("http://notmcp/path").is_none());
        assert!(parse_server_from_uri("mcp:///no-server").is_none());
    }

    // --- McpManager ---

    #[test]
    fn manager_register_and_list_tools() {
        let mut manager = McpManager::default();
        let client = InMemoryMcpClient::default().with_tool("greet", json!({"hi": true}));
        manager.register_server(
            sample_config("test-srv", true),
            ToolFilter::default(),
            Box::new(client),
        );
        let tools = manager.list_tools().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "greet");
        assert!(tools[0].qualified_name.starts_with("mcp__"));
    }

    #[test]
    fn manager_call_tool_directly() {
        let mut manager = McpManager::default();
        let client = InMemoryMcpClient::default().with_tool("echo", json!({"echoed": true}));
        manager.register_server(
            sample_config("srv", true),
            ToolFilter::default(),
            Box::new(client),
        );
        let result = manager.call_tool("srv", "echo", json!({})).unwrap();
        assert_eq!(result, json!({"echoed": true}));
    }

    #[test]
    fn manager_call_qualified_tool() {
        let mut manager = McpManager::default();
        let client = InMemoryMcpClient::default().with_tool("greet", json!({"hello": true}));
        manager.register_server(
            sample_config("demo", true),
            ToolFilter::default(),
            Box::new(client),
        );
        let result = manager
            .call_qualified_tool("mcp__demo__greet", json!({}))
            .unwrap();
        assert_eq!(result, json!({"hello": true}));
    }

    #[test]
    fn manager_call_tool_missing_server_errors() {
        let manager = McpManager::default();
        assert!(manager.call_tool("ghost", "ping", json!({})).is_err());
    }

    #[test]
    fn manager_stop_server_removes_client() {
        let mut manager = McpManager::default();
        let client = InMemoryMcpClient::default().with_tool("a", json!(null));
        manager.register_server(
            sample_config("s1", true),
            ToolFilter::default(),
            Box::new(client),
        );
        manager.stop_server("s1").unwrap();
        assert!(manager.call_tool("s1", "a", json!({})).is_err());
    }

    #[test]
    fn manager_stop_nonexistent_server_errors() {
        let mut manager = McpManager::default();
        assert!(manager.stop_server("nope").is_err());
    }

    #[test]
    fn manager_unregister_server_removes_config_and_client() {
        let mut manager = McpManager::default();
        let client = InMemoryMcpClient::default();
        manager.register_server(
            sample_config("s2", true),
            ToolFilter::default(),
            Box::new(client),
        );
        manager.unregister_server("s2").unwrap();
        assert!(manager.list_tools().unwrap().is_empty());
    }

    #[test]
    fn manager_unregister_nonexistent_server_errors() {
        let mut manager = McpManager::default();
        assert!(manager.unregister_server("missing").is_err());
    }

    #[test]
    fn manager_list_and_read_resources() {
        let mut manager = McpManager::default();
        let client =
            InMemoryMcpClient::default().with_resource("mcp://res/data", json!({"key": "value"}));
        manager.register_server(
            sample_config("res-srv", true),
            ToolFilter::default(),
            Box::new(client),
        );
        let resources = manager.list_resources().unwrap();
        assert_eq!(resources.len(), 1);

        let value = manager.read_resource("res-srv", "mcp://res/data").unwrap();
        assert_eq!(value["key"], "value");
    }

    #[test]
    fn manager_read_resource_missing_server_errors() {
        let manager = McpManager::default();
        assert!(manager.read_resource("noserver", "mcp://x").is_err());
    }

    #[test]
    fn manager_start_all_reports_ready_failed_cancelled() {
        let mut manager = McpManager::default();
        let client = InMemoryMcpClient::default();
        manager.register_server(
            sample_config("enabled", true),
            ToolFilter::default(),
            Box::new(client),
        );
        // Add a disabled config (no client)
        manager.configs.insert(
            "disabled".to_string(),
            (sample_config("disabled", false), ToolFilter::default()),
        );

        let mut events = Vec::new();
        let result = manager.start_all(|evt| events.push(evt));

        assert!(result.ready.contains(&"enabled".to_string()));
        assert!(result.cancelled.contains(&"disabled".to_string()));
        assert!(!events.is_empty());
    }

    #[test]
    fn manager_update_sandbox_state_produces_notices() {
        let mut manager = McpManager::default();
        manager.configs.insert(
            "s".to_string(),
            (sample_config("s", true), ToolFilter::default()),
        );
        let notices = manager.update_sandbox_state("strict", "/tmp").unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0]["method"], "codex/sandbox-state/update");
    }

    #[test]
    fn manager_tool_filter_applied_on_list() {
        let mut manager = McpManager::default();
        let client = InMemoryMcpClient::default()
            .with_tool("allowed_tool", json!(null))
            .with_tool("blocked_tool", json!(null));
        let filter = ToolFilter {
            allow: vec![],
            deny: vec!["blocked_tool".to_string()],
        };
        manager.register_server(sample_config("filtered", true), filter, Box::new(client));
        let tools = manager.list_tools().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "allowed_tool");
    }

    // --- McpServerConfig / serde ---

    #[test]
    fn mcp_server_config_round_trips() {
        let config = McpServerConfig {
            name: "test".to_string(),
            command: "node".to_string(),
            args: vec!["server.js".to_string()],
            env: HashMap::from([("PORT".to_string(), "3000".to_string())]),
            enabled: true,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert!(parsed.enabled);
        assert_eq!(parsed.env.get("PORT").unwrap(), "3000");
    }

    #[test]
    fn mcp_startup_status_serde() {
        let ready = McpStartupStatus::Ready;
        let json = serde_json::to_string(&ready).unwrap();
        assert!(json.contains("ready"));

        let failed = McpStartupStatus::Failed {
            error: "timeout".to_string(),
        };
        let json = serde_json::to_string(&failed).unwrap();
        assert!(json.contains("timeout"));
    }

    // --- JsonRpcError codes ---

    #[test]
    fn jsonrpc_error_codes() {
        assert_eq!(JsonRpcError::parse_error("x").code, -32700);
        assert_eq!(JsonRpcError::invalid_request("x").code, -32600);
        assert_eq!(JsonRpcError::method_not_found("m").code, -32601);
        assert_eq!(JsonRpcError::invalid_params("x").code, -32602);
        assert_eq!(JsonRpcError::internal("x").code, -32603);
    }

    #[test]
    fn jsonrpc_result_structure() {
        let result = jsonrpc_result(Some(json!(1)), json!({"ok": true}));
        assert_eq!(result["jsonrpc"], "2.0");
        assert_eq!(result["id"], 1);
        assert_eq!(result["result"]["ok"], true);
    }

    #[test]
    fn jsonrpc_error_structure() {
        let err = jsonrpc_error(Some(json!(2)), JsonRpcError::internal("fail"));
        assert_eq!(err["jsonrpc"], "2.0");
        assert_eq!(err["id"], 2);
        assert_eq!(err["error"]["code"], -32603);
        assert_eq!(err["error"]["message"], "fail");
    }

    #[test]
    fn jsonrpc_null_id_when_missing() {
        let result = jsonrpc_result(None, json!({}));
        assert!(result["id"].is_null());
    }
}
