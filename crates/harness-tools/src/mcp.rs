use crate::ToolExecutionResult;
use harness_net::{NetworkGate, PrivacyMode};
use harness_providers::ToolSpec;
use harness_store::McpServerEntry;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDiscoveredTool {
    pub server_id: String,
    pub server_name: String,
    pub tool_name: String,
    pub namespaced_name: String,
    pub description: String,
    pub input_schema: Value,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpProbeResult {
    pub server_id: String,
    pub server_name: String,
    pub ok: bool,
    pub server_info: String,
    pub tools: Vec<McpDiscoveredTool>,
    pub error: Option<String>,
    pub latency_ms: u64,
}

pub fn sanitize_mcp_slug(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "server".into()
    } else {
        trimmed.chars().take(24).collect()
    }
}

pub fn namespaced_mcp_tool(server_name: &str, tool_name: &str) -> String {
    format!(
        "mcp__{}__{}",
        sanitize_mcp_slug(server_name),
        tool_name.trim()
    )
}

/// Split a shell-like command line into `(program, args)` respecting quotes.
pub fn split_command_line(raw: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = raw.trim().chars().peekable();

    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                }
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        parts.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

fn resolve_stdio_command(entry: &McpServerEntry) -> anyhow::Result<(String, Vec<String>)> {
    if !entry.command.trim().is_empty() {
        return Ok((entry.command.trim().to_string(), entry.args.clone()));
    }
    let parts = split_command_line(&entry.target);
    if parts.is_empty() {
        anyhow::bail!("MCP server '{}' has no command configured.", entry.name);
    }
    Ok((parts[0].clone(), parts[1..].to_vec()))
}

fn resolve_http_url(entry: &McpServerEntry) -> anyhow::Result<String> {
    let url = if !entry.url.trim().is_empty() {
        entry.url.trim()
    } else {
        entry.target.trim()
    };
    if url.is_empty() {
        anyhow::bail!("MCP HTTP server '{}' has no URL configured.", entry.name);
    }
    Ok(url.to_string())
}

fn configure_no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

async fn write_jsonrpc_line(
    stdin: &mut tokio::process::ChildStdin,
    val: Value,
) -> anyhow::Result<()> {
    let line = format!("{}\n", val);
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_jsonrpc_response(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    expected_id: u64,
) -> anyhow::Result<Value> {
    let mut skipped = 0usize;
    while skipped < 40 {
        let Some(line) = reader.next_line().await? else {
            anyhow::bail!("MCP server closed stdout unexpectedly before replying.");
        };
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            skipped += 1;
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            if val.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
                if let Some(err) = val.get("error") {
                    anyhow::bail!(
                        "MCP JSON-RPC error: {}",
                        err.get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or(&err.to_string())
                    );
                }
                return Ok(val.get("result").cloned().unwrap_or(Value::Null));
            }
        }
        skipped += 1;
    }
    anyhow::bail!("MCP server did not return a matching JSON-RPC response.")
}

struct WarmStdioSession {
    signature: String,
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    server_info: String,
    next_id: u64,
}

type PoolEntry = Arc<Mutex<Option<WarmStdioSession>>>;
static STDIO_POOL: OnceLock<Mutex<HashMap<String, PoolEntry>>> = OnceLock::new();

fn stdio_pool() -> &'static Mutex<HashMap<String, PoolEntry>> {
    STDIO_POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn get_pool_slot(server_id: &str) -> PoolEntry {
    let mut map = stdio_pool().lock().await;
    map.entry(server_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(None)))
        .clone()
}

pub async fn evict_mcp_server_session(server_id: &str) {
    let slot = {
        let mut map = stdio_pool().lock().await;
        map.remove(server_id)
    };
    if let Some(entry_mutex) = slot {
        let mut guard = entry_mutex.lock().await;
        if let Some(mut sess) = guard.take() {
            let _ = sess.child.kill().await;
        }
    }
}

async fn spawn_and_initialize_stdio_session(
    entry: &McpServerEntry,
    signature: String,
    timeout_secs: u64,
) -> anyhow::Result<WarmStdioSession> {
    let (program, args) = resolve_stdio_command(entry)?;

    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::scrub_sensitive_child_env(&mut cmd);

    for (k, v) in &entry.env {
        cmd.env(k, v);
    }
    configure_no_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "Failed to launch MCP server '{}' ({}): {e}",
            entry.name,
            program
        )
    })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to open stdin for MCP server '{}'", entry.name))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to open stdout for MCP server '{}'", entry.name))?;
    let mut reader = BufReader::new(stdout).lines();

    let init_fut = async {
        write_jsonrpc_line(
            &mut stdin,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "cinder", "version": "4.5.0" }
                }
            }),
        )
        .await?;

        let init_res = read_jsonrpc_response(&mut reader, 1).await?;
        let server_name = init_res["serverInfo"]["name"]
            .as_str()
            .unwrap_or(&entry.name);
        let server_ver = init_res["serverInfo"]["version"].as_str().unwrap_or("");
        let server_info = if server_ver.is_empty() {
            server_name.to_string()
        } else {
            format!("{server_name} v{server_ver}")
        };

        write_jsonrpc_line(
            &mut stdin,
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        )
        .await?;

        Ok::<String, anyhow::Error>(server_info)
    };

    match tokio::time::timeout(Duration::from_secs(timeout_secs), init_fut).await {
        Ok(Ok(server_info)) => Ok(WarmStdioSession {
            signature,
            child,
            stdin,
            reader,
            server_info,
            next_id: 2,
        }),
        Ok(Err(e)) => {
            let _ = child.kill().await;
            Err(e)
        }
        Err(_) => {
            let _ = child.kill().await;
            anyhow::bail!(
                "MCP server '{}' initialize timed out after {timeout_secs}s.",
                entry.name
            )
        }
    }
}

async fn stdio_jsonrpc_exchange(
    entry: &McpServerEntry,
    followup_method: &str,
    followup_params: Value,
    timeout_secs: u64,
) -> anyhow::Result<(String, Value)> {
    let pool_key = if entry.id.trim().is_empty() {
        entry.name.clone()
    } else {
        entry.id.clone()
    };
    let signature = format!("{}|{:?}|{}|{:?}", entry.command, entry.args, entry.target, entry.env);
    let slot = get_pool_slot(&pool_key).await;
    let mut guard = slot.lock().await;

    for attempt in 0..2 {
        let needs_spawn = match guard.as_mut() {
            Some(sess) if sess.signature == signature => {
                !matches!(sess.child.try_wait(), Ok(None))
            }
            _ => true,
        };
        if needs_spawn {
            if let Some(mut old) = guard.take() {
                let _ = old.child.kill().await;
            }
            let fresh =
                spawn_and_initialize_stdio_session(entry, signature.clone(), timeout_secs).await?;
            *guard = Some(fresh);
        }

        let sess = guard.as_mut().expect("warm MCP session just spawned");
        let req_id = sess.next_id;
        sess.next_id += 1;

        let rpc_fut = async {
            write_jsonrpc_line(
                &mut sess.stdin,
                json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "method": followup_method,
                    "params": followup_params
                }),
            )
            .await?;
            let result = read_jsonrpc_response(&mut sess.reader, req_id).await?;
            Ok::<(String, Value), anyhow::Error>((sess.server_info.clone(), result))
        };

        match tokio::time::timeout(Duration::from_secs(timeout_secs), rpc_fut).await {
            Ok(Ok(res)) => return Ok(res),
            Ok(Err(e)) => {
                if let Some(mut dead) = guard.take() {
                    let _ = dead.child.kill().await;
                }
                if attempt == 1 {
                    return Err(e);
                }
            }
            Err(_) => {
                if let Some(mut timed_out) = guard.take() {
                    let _ = timed_out.child.kill().await;
                }
                if attempt == 1 {
                    anyhow::bail!(
                        "MCP server '{}' timed out after {timeout_secs}s.",
                        entry.name
                    );
                }
            }
        }
    }

    unreachable!("loop returns on attempt 1")
}

fn parse_streamable_http_json(body: &str, expected_id: u64) -> anyhow::Result<Value> {
    let trimmed = body.trim();
    if trimmed.starts_with('{') {
        let val: Value = serde_json::from_str(trimmed)?;
        if let Some(err) = val.get("error") {
            anyhow::bail!("MCP HTTP error: {err}");
        }
        return Ok(val.get("result").cloned().unwrap_or(Value::Null));
    }
    // Parse SSE `data: {...}` lines returned by Streamable HTTP servers
    for line in trimmed.lines() {
        let l = line.trim();
        if let Some(json_str) = l.strip_prefix("data:") {
            if let Ok(val) = serde_json::from_str::<Value>(json_str.trim()) {
                if val.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
                    if let Some(err) = val.get("error") {
                        anyhow::bail!("MCP HTTP error: {err}");
                    }
                    return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        }
    }
    anyhow::bail!("Could not parse MCP Streamable HTTP response.")
}

async fn http_jsonrpc_exchange(
    gate: &NetworkGate,
    entry: &McpServerEntry,
    followup_method: &str,
    followup_params: Value,
    timeout_secs: u64,
) -> anyhow::Result<(String, Value)> {
    let url = resolve_http_url(entry)?;

    let init_req = gate
        .request(Method::POST, &url, "mcp_http_initialize")?
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "cinder", "version": "3.1.0" }
            }
        }));

    let init_resp = tokio::time::timeout(Duration::from_secs(timeout_secs), init_req.send())
        .await
        .map_err(|_| anyhow::anyhow!("MCP HTTP initialize timed out"))??;

    let session_id = init_resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if !init_resp.status().is_success() {
        let status = init_resp.status();
        let text = init_resp.text().await.unwrap_or_default();
        anyhow::bail!("MCP HTTP initialize failed ({status}): {text}");
    }
    let init_body = init_resp.text().await?;
    let init_res = parse_streamable_http_json(&init_body, 1)?;
    let server_info = format!(
        "{} {}",
        init_res["serverInfo"]["name"]
            .as_str()
            .unwrap_or(&entry.name),
        init_res["serverInfo"]["version"].as_str().unwrap_or("")
    )
    .trim()
    .to_string();

    // Send notifications/initialized (best-effort)
    if let Ok(mut notif) = gate.request(Method::POST, &url, "mcp_http_notify") {
        notif = notif
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json");
        if let Some(ref sid) = session_id {
            notif = notif.header("Mcp-Session-Id", sid);
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            notif
                .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
                .send(),
        )
        .await;
    }

    let mut call_req = gate
        .request(Method::POST, &url, "mcp_http_rpc")?
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json");
    if let Some(ref sid) = session_id {
        call_req = call_req.header("Mcp-Session-Id", sid);
    }

    let call_resp = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        call_req
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": followup_method,
                "params": followup_params
            }))
            .send(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("MCP HTTP request timed out"))??;

    if !call_resp.status().is_success() {
        let status = call_resp.status();
        let text = call_resp.text().await.unwrap_or_default();
        anyhow::bail!("MCP HTTP '{followup_method}' failed ({status}): {text}");
    }
    let body = call_resp.text().await?;
    let result = parse_streamable_http_json(&body, 2)?;
    Ok((server_info, result))
}

/// Probe an MCP server (`initialize` + `tools/list`) and return its live tool manifest.
pub async fn probe_mcp_server(gate: &NetworkGate, entry: &McpServerEntry) -> McpProbeResult {
    let start = Instant::now();
    if entry.needs_network && gate.mode() == PrivacyMode::StrictLocal {
        return McpProbeResult {
            server_id: entry.id.clone(),
            server_name: entry.name.clone(),
            ok: false,
            server_info: String::new(),
            tools: Vec::new(),
            error: Some(
                "Blocked by Strict Local Mode (this MCP server accesses the internet). Switch to Hybrid Mode to use it."
                    .into(),
            ),
            latency_ms: start.elapsed().as_millis() as u64,
        };
    }

    let res = match entry.transport.to_ascii_lowercase().as_str() {
        "http" | "streamable_http" => {
            http_jsonrpc_exchange(gate, entry, "tools/list", json!({}), 12).await
        }
        _ => stdio_jsonrpc_exchange(entry, "tools/list", json!({}), 15).await,
    };

    match res {
        Ok((server_info, val)) => {
            let mut tools = Vec::new();
            if let Some(arr) = val["tools"].as_array() {
                for t in arr {
                    if let Some(tname) = t["name"].as_str() {
                        let schema = t
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                        let desc = t["description"]
                            .as_str()
                            .unwrap_or("External MCP tool")
                            .to_string();
                        tools.push(McpDiscoveredTool {
                            server_id: entry.id.clone(),
                            server_name: entry.name.clone(),
                            tool_name: tname.to_string(),
                            namespaced_name: namespaced_mcp_tool(&entry.name, tname),
                            description: format!("[MCP: {}] {}", entry.name, desc),
                            input_schema: schema,
                            read_only: entry.read_only,
                        });
                    }
                }
            }
            McpProbeResult {
                server_id: entry.id.clone(),
                server_name: entry.name.clone(),
                ok: true,
                server_info,
                tools,
                error: None,
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
        Err(e) => McpProbeResult {
            server_id: entry.id.clone(),
            server_name: entry.name.clone(),
            ok: false,
            server_info: String::new(),
            tools: Vec::new(),
            error: Some(e.to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Discover tools across all enabled MCP servers (respecting PrivacyMode).
pub async fn discover_enabled_mcp_tools(
    gate: &NetworkGate,
    servers: &[McpServerEntry],
) -> Vec<McpDiscoveredTool> {
    let mut all = Vec::new();
    for srv in servers {
        if !srv.enabled {
            continue;
        }
        if srv.needs_network && gate.mode() == PrivacyMode::StrictLocal {
            continue;
        }
        let probe = probe_mcp_server(gate, srv).await;
        if probe.ok {
            all.extend(probe.tools);
        }
    }
    all
}

pub fn mcp_tools_to_specs(tools: &[McpDiscoveredTool]) -> Vec<ToolSpec> {
    tools
        .iter()
        .map(|t| ToolSpec {
            name: t.namespaced_name.clone(),
            description: t.description.clone(),
            parameters_schema: t.input_schema.clone(),
        })
        .collect()
}

/// Invoke a tool on an MCP server (`tools/call`) and format output for the agent loop.
pub async fn call_mcp_tool(
    gate: &NetworkGate,
    entry: &McpServerEntry,
    tool_name: &str,
    arguments: Value,
) -> ToolExecutionResult {
    if entry.needs_network && gate.mode() == PrivacyMode::StrictLocal {
        return ToolExecutionResult {
            output: format!(
                "Strict Local Mode blocked MCP server '{}' (requires network). Switch to Hybrid Mode to allow outbound MCP calls.",
                entry.name
            ),
            diff: None,
            file_path: None,
            is_error: true,
        };
    }

    let params = json!({
        "name": tool_name,
        "arguments": if arguments.is_object() { arguments } else { json!({}) }
    });

    let res = match entry.transport.to_ascii_lowercase().as_str() {
        "http" | "streamable_http" => {
            http_jsonrpc_exchange(gate, entry, "tools/call", params, 30).await
        }
        _ => stdio_jsonrpc_exchange(entry, "tools/call", params, 30).await,
    };

    match res {
        Ok((_, val)) => {
            let is_err = val["isError"].as_bool().unwrap_or(false);
            let mut text_parts = Vec::new();
            if let Some(content) = val["content"].as_array() {
                for item in content {
                    if let Some(txt) = item["text"].as_str() {
                        text_parts.push(txt.to_string());
                    } else {
                        text_parts.push(item.to_string());
                    }
                }
            }
            let output = if text_parts.is_empty() {
                serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())
            } else {
                text_parts.join("\n")
            };
            ToolExecutionResult {
                output,
                diff: None,
                file_path: None,
                is_error: is_err,
            }
        }
        Err(e) => ToolExecutionResult {
            output: format!("MCP tool error ({}::{}): {e}", entry.name, tool_name),
            diff: None,
            file_path: None,
            is_error: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_command_lines_cleanly() {
        let parts = split_command_line(r#""C:\Program Files\Python\python.exe" -m mcp_server_time --local-timezone UTC"#);
        assert_eq!(
            parts,
            vec![
                r"C:\Program Files\Python\python.exe",
                "-m",
                "mcp_server_time",
                "--local-timezone",
                "UTC"
            ]
        );
    }

    #[test]
    fn parses_sse_streamable_http_response() {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"ping\"}]}}\n\n";
        let val = parse_streamable_http_json(sse, 2).unwrap();
        assert_eq!(val["tools"][0]["name"].as_str(), Some("ping"));
    }

    #[test]
    fn namespaces_mcp_tools_deterministically() {
        assert_eq!(
            namespaced_mcp_tool("Time & Zone MCP", "get_current_time"),
            "mcp__time_zone_mcp__get_current_time"
        );
    }
}
