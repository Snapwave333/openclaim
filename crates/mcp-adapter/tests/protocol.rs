//! MCP stdio protocol tests (MDR §14.9): initialize negotiation, tool listing,
//! resource reads, hostile arguments, and failure when no daemon answers.

use openclaim_cli_core::client::Client;
use openclaim_core::Paths;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn client() -> Client {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_dir(dir.keep());
    Client {
        endpoint: openclaim_ipc::Endpoint::for_paths(&paths),
        paths,
        timeout: std::time::Duration::from_secs(2),
        autostart: false,
    }
}

async fn roundtrip(messages: Vec<Value>) -> Vec<Value> {
    let (client_in, server_in) = tokio::io::duplex(1 << 20);
    let (server_out, client_out) = tokio::io::duplex(1 << 20);
    let task = tokio::spawn(openclaim_mcp_adapter::serve(client(), None, server_in, server_out));
    let mut w = client_in;
    for m in messages {
        w.write_all(format!("{m}\n").as_bytes()).await.unwrap();
    }
    drop(w);
    let mut lines = BufReader::new(client_out).lines();
    let mut out = Vec::new();
    while let Ok(Some(l)) = lines.next_line().await {
        if !l.trim().is_empty() {
            out.push(serde_json::from_str(&l).unwrap());
        }
    }
    task.await.unwrap().unwrap();
    out
}

#[tokio::test]
async fn initialize_tools_and_resources() {
    let out = roundtrip(vec![
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2099-01-01", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "openclaim://capabilities"}}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "ping"}),
        json!({"jsonrpc": "2.0", "id": 6, "method": "nope"}),
        json!({"jsonrpc": "2.0", "id": 7}),
    ])
    .await;
    assert_eq!(
        out[0]["result"]["protocolVersion"], "2025-06-18",
        "unknown version falls back to the newest supported"
    );
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "openclaim");
    let tools = out[1]["result"]["tools"].as_array().unwrap();
    assert!(tools.len() >= 18);
    assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
    assert!(tools.iter().any(|t| t["name"] == "evaluate_match" && t["annotations"]["readOnlyHint"] == false));
    assert!(!tools.iter().any(|t| t["name"].as_str().unwrap().contains("submit")));
    assert_eq!(out[2]["result"]["resources"].as_array().unwrap().len(), 4);
    let caps: Value = serde_json::from_str(out[3]["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(caps["model_dependency"], "none");
    assert_eq!(out[4]["result"], json!({}));
    assert_eq!(out[5]["error"]["code"], -32601);
    assert_eq!(out[6]["error"]["code"], -32601, "a message without a method is method-not-found, not a crash");
}

#[tokio::test]
async fn tool_calls_without_a_daemon_are_errors_not_hangs() {
    let out = roundtrip(vec![
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "discover_opportunities", "arguments": {"q": "x", "__proto__": {"evil": true}, "approval_id": 42}}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "no_such_tool", "arguments": {}}}),
    ])
    .await;
    assert_eq!(out[0]["result"]["isError"], true);
    assert_eq!(out[0]["result"]["structuredContent"]["error"]["code"], "UNAVAILABLE");
    assert_eq!(out[1]["error"]["code"], -32602);
}
