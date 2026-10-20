//! Stdio language server used by the LSP attach test. It is this package's own binary.

use std::io::{self, Read, Write};

fn main() {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    while let Some(body) = read_frame(&mut input) {
        let message: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let method = message.get("method").and_then(|value| value.as_str());
        let id = message.get("id").cloned();
        match (method, id) {
            (Some("initialize"), Some(id)) => {
                write_result(
                    &mut output,
                    id,
                    serde_json::json!({ "capabilities": { "hoverProvider": true } }),
                );
            }
            (Some("textDocument/hover"), Some(id)) => {
                write_result(
                    &mut output,
                    id,
                    serde_json::json!({
                        "contents": { "kind": "plaintext", "value": "fixture-hover-token" }
                    }),
                );
            }
            (Some("shutdown"), Some(id)) => {
                write_result(&mut output, id, serde_json::Value::Null);
            }
            (Some("exit"), _) => break,
            (_, Some(id)) => write_result(&mut output, id, serde_json::Value::Null),
            _ => {}
        }
    }
}

fn read_frame(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if input.read(&mut byte).ok()? == 0 {
            return None;
        }
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&header);
    let length = text
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())?;
    let mut body = vec![0u8; length];
    input.read_exact(&mut body).ok()?;
    Some(body)
}

fn write_result(output: &mut impl Write, id: serde_json::Value, result: serde_json::Value) {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let bytes = body.to_string().into_bytes();
    let _ = write!(output, "Content-Length: {}\r\n\r\n", bytes.len());
    let _ = output.write_all(&bytes);
    let _ = output.flush();
}
