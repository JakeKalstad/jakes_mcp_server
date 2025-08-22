use anyhow::{Context, Result, anyhow};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
};

#[derive(Parser, Debug, Clone)]
struct Args {
    /// Restrict all file operations under this directory.
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum Id {
    Num(i64),
    Str(String),
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Id>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}
#[derive(Debug, Serialize)]
struct RpcResponse<'a> {
    jsonrpc: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let mut reader = BufReader::new(stdin);
    let mut writer = stdout;

    let mut line = String::new();
    while reader.read_line(&mut line).await? > 0 {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }

        let req: RpcRequest = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                // If we can’t parse, emit a JSON-RPC error without id.
                let resp = RpcResponse {
                    jsonrpc: "2.0",
                    id: None,
                    result: None,
                    error: Some(RpcError {
                        code: -32700,
                        message: format!("Parse error: {e}"),
                        data: None,
                    }),
                };
                writer
                    .write_all(serde_json::to_string(&resp)?.as_bytes())
                    .await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                line.clear();
                continue;
            }
        };

        let result = handle_request(&args, &req).await;
        let response = match result {
            Ok(val) => RpcResponse {
                jsonrpc: "2.0",
                id: req.id.as_ref(),
                result: Some(val),
                error: None,
            },
            Err(err) => RpcResponse {
                jsonrpc: "2.0",
                id: req.id.as_ref(),
                result: None,
                error: Some(RpcError {
                    code: -32000,
                    message: err.to_string(),
                    data: None,
                }),
            },
        };

        writer
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        line.clear();
    }

    Ok(())
}

async fn handle_request(args: &Args, req: &RpcRequest) -> Result<serde_json::Value> {
    match req.method.as_str() {
        // MCP handshake: return server info & capabilities
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "serverInfo": {"name": "jake-mcp-rs", "version": "0.1.0"},
            "capabilities": {
                "tools": {},
                "resources": {},
            }
        })),

        "tools/list" => Ok(json!({
        "tools": [
            {
                "name": "list_dir",
                "description": "List files and directories under a given path (relative to server root)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "recursive": {"type": "boolean", "default": false}
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }
            },
            {
                "name": "read_file",
                "description": "Read a file as UTF-8 text (relative to server root)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "offset": {"type": "integer", "minimum": 0},
                        "length": {"type": "integer", "minimum": 0}
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }
            },
            {
                "name": "write_file",
                "description": "Write UTF-8 text to a file (create or overwrite)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"},
                        "create": {"type": "boolean", "default": true},
                        "append": {"type": "boolean", "default": false}
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }
            },
            {
                "name": "unshare_exec",
                "description": "Run a binary in isolated Linux namespaces using unshare",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "binary": {"type": "string"},
                        "args": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["binary"],
                    "additionalProperties": false
                }
            }
        ]
        })),
        
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing params.name"))?;
            let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
            let root = args.get("root").and_then(|v| v.as_str()).unwrap_or("");
            let root_path = Path::new(root);

            match name {
                "list_dir" => tool_list_dir(&args, root_path).await,
                "read_file" => tool_read_file(&args, root_path).await,
                "write_file" => tool_write_file(&args, root_path).await,
                "unshare_exec" => tool_unshare_exec(&args, root_path).await,
                other => Err(anyhow!("Unknown tool: {other}")),
            }
        }

        "resources/list" | "prompts/list" => Ok(json!({
            "resources": [],
            "next": null
        })),

        _ => Err(anyhow!("Method not implemented: {}", req.method)),
    }
}

async fn tool_unshare_exec(params: &serde_json::Value, root: &Path) -> Result<serde_json::Value> {
    let binary = params.get("binary").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("binary required"))?;
    let args = params.get("args").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    // Optional: map working directory under root
    let cwd = root.join("sandbox");
    tokio::fs::create_dir_all(&cwd).await?;

    // Spawn unshare command
    let output = std::process::Command::new("unshare")
        .arg("--uts")
        .arg("--ipc")
        .arg("--net")
        .arg("--pid")
        .arg("--fork")
        .arg("--user")
        .arg(binary)
        .args(&args)
        .current_dir(&cwd)
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code
    }))
}

fn resolve_under_root(root: &Path, rel: &str) -> Result<PathBuf> {
    let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf()); // fallback if root doesn't exist
    let joined = base.join(rel);

    // Only canonicalize if path exists
    let canonical = if joined.exists() {
        joined.canonicalize()?
    } else {
        joined
    };

    if !canonical.starts_with(&base) {
        anyhow::bail!("path escapes root: {}", canonical.display());
    }

    Ok(canonical)
}

async fn tool_list_dir(params: &serde_json::Value, root: &Path) -> Result<serde_json::Value> {
    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("list_dir.path is required"))?;
    let recursive = params
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let base = resolve_under_root(root, path)?;
    let mut entries = Vec::new();

    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        let mut rd = fs::read_dir(&dir)
            .await
            .with_context(|| format!("read_dir {}", dir.display()))?;
        while let Some(entry) = rd.next_entry().await? {
            let md = entry.metadata().await?;
            let is_dir = md.is_dir();
            let p = entry.path();
            entries.push(json!({
            "path": p.display().to_string(),
            "is_dir": is_dir,
            "len": md.len()
            }));
            if recursive && is_dir {
                stack.push(p);
            }
        }
    }

    Ok(json!({
    "content": [{"type": "json", "json": entries }]
    }))
}

async fn tool_read_file(params: &serde_json::Value, root: &Path) -> Result<serde_json::Value> {
    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("read_file.path is required"))?;

    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let length = params.get("length").and_then(|v| v.as_u64());

    let full = resolve_under_root(root, path)?;
    let data = fs::read(&full)
        .await
        .with_context(|| format!("read {}", full.display()))?;

    let slice: &[u8] = if let Some(len) = length {
        let end = offset.saturating_add(len as usize).min(data.len());
        &data[offset.min(data.len())..end]
    } else {
        &data[offset.min(data.len())..]
    };
    let text = String::from_utf8_lossy(slice).to_string();

    Ok(json!({
    "content": [{"type": "text", "text": text }]
    }))
}

async fn tool_write_file(params: &serde_json::Value, root: &Path) -> Result<serde_json::Value> {
    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("write_file.path is required"))?;
    let content = params
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("write_file.content is required"))?;
    let append = params
        .get("append")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let full = resolve_under_root(root, path)?;
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).await.ok();
    }

    if append {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full)
            .await?;
        f.write_all(content.as_bytes()).await?;
    } else {
        fs::write(&full, content.as_bytes()).await?;
    }

    Ok(json!({
"content": [{"type": "text", "text": format!("wrote {} bytes to {}", content.len(), path)}]}))
}
