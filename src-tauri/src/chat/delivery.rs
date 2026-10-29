use serde_json::{json, Value};
use std::collections::VecDeque;

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_PARTS: usize = 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_TOOL_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Delivery acknowledgements never govern persistence or provider cancellation.
/// A stalled view receives one resync notice; only the latest snapshot is kept.
pub struct Delivery {
    pub epoch: u64,
    pub sequence: u64,
    pub snapshot: Value,
    pub terminal: Option<Value>,
    outstanding: VecDeque<(u64, usize)>,
    bytes: usize,
    pub response_bytes: usize,
    text_bytes: usize,
    limit: usize,
    paused: bool,
}
impl Delivery {
    pub fn new(assistant: &str, limit: usize) -> Self {
        Self {
            epoch: 0,
            sequence: 0,
            snapshot: json!({"message":{"id":assistant,"role":"assistant","parts":[]},"blocks":{}}),
            terminal: None,
            outstanding: VecDeque::new(),
            bytes: 0,
            response_bytes: 0,
            text_bytes: 0,
            limit,
            paused: false,
        }
    }

    pub fn subscribe(&mut self) -> Value {
        self.epoch += 1;
        self.outstanding.clear();
        self.bytes = 0;
        self.paused = false;
        json!({"type":"snapshot","epoch":self.epoch,"sequence":self.sequence,"snapshot":self.snapshot,"terminal":self.terminal})
    }
    pub fn ack(&mut self, epoch: u64, sequence: u64) {
        if epoch != self.epoch || sequence > self.sequence {
            return;
        }
        while self
            .outstanding
            .front()
            .is_some_and(|(seq, _)| *seq <= sequence)
        {
            self.bytes -= self.outstanding.pop_front().unwrap().1;
        }
    }
    pub fn chunk(&mut self, sequence: u64, chunk: &Value) -> Result<Option<Value>, String> {
        if sequence <= self.sequence {
            return Err("Out-of-order AI event.".into());
        }
        let size = serde_json::to_vec(chunk)
            .map_err(|_| "Invalid response.")?
            .len();
        if self.response_bytes.saturating_add(size) > MAX_RESPONSE_BYTES {
            return Err("Response exceeds the size limit.".into());
        }
        self.response_bytes += size;
        self.sequence = sequence;
        match chunk["type"].as_str() {
            Some("text-start" | "reasoning-start") => {
                let id = chunk["id"].as_str().ok_or("Missing block ID.")?;
                if self.snapshot["blocks"].get(id).is_some() {
                    return Err("Duplicate block ID.".into());
                }
                let kind = if chunk["type"] == "text-start" {
                    "text"
                } else {
                    "reasoning"
                };
                let parts = self.snapshot["message"]["parts"]
                    .as_array_mut()
                    .ok_or("Invalid snapshot.")?;
                let index = parts.len();
                if index >= MAX_PARTS {
                    return Err("Too many response blocks.".into());
                }
                let mut part = json!({"type":kind,"text":"","state":"streaming"});
                if let Some(metadata) = metadata(chunk, "providerMetadata")? {
                    part["providerMetadata"] = metadata;
                }
                parts.push(part);
                self.snapshot["blocks"][id] = json!({"index":index,"type":kind,"open":true});
            }
            Some("text-delta" | "reasoning-delta" | "text-end" | "reasoning-end") => {
                let id = chunk["id"].as_str().ok_or("Missing block ID.")?;
                let block = &self.snapshot["blocks"][id];
                if block["open"] != true {
                    return Err("Missing open response block.".into());
                }
                let index = block["index"].as_u64().ok_or("Invalid response block.")? as usize;
                let part = &mut self.snapshot["message"]["parts"][index];
                if let Some(delta) = chunk["delta"].as_str() {
                    let text = part["text"].as_str().ok_or("Invalid response text.")?;
                    if self.text_bytes.saturating_add(delta.len()) > MAX_TEXT_BYTES {
                        return Err("Response exceeds the size limit.".into());
                    }
                    self.text_bytes += delta.len();
                    part["text"] = Value::String(format!("{text}{delta}"));
                    if let Some(metadata) = metadata(chunk, "providerMetadata")? {
                        part["providerMetadata"] = metadata;
                    }
                } else {
                    part["state"] = json!("done");
                    if let Some(metadata) = metadata(chunk, "providerMetadata")? {
                        part["providerMetadata"] = metadata;
                    }
                    self.snapshot["blocks"][id]["open"] = json!(false);
                }
            }
            Some("start") => (),
            Some("start-step") => {
                let parts = self.snapshot["message"]["parts"]
                    .as_array_mut()
                    .ok_or("Invalid snapshot.")?;
                if parts.len() >= MAX_PARTS {
                    return Err("Too many response blocks.".into());
                }
                parts.push(json!({"type":"step-start"}));
            }
            Some("tool-input-available") => self.tool_input(chunk)?,
            Some("tool-output-available") => self.tool_output(chunk, false)?,
            Some("tool-output-error") => self.tool_output(chunk, true)?,
            Some("tool-input-error") => self.tool_input_error(chunk)?,
            _ => return Err("Unsupported AI stream part.".into()),
        }
        if self.paused {
            return Ok(None);
        }
        let bytes = serde_json::to_vec(chunk)
            .map_err(|_| "Invalid chunk.")?
            .len();
        if self.bytes + bytes > self.limit || self.outstanding.len() >= 4096 {
            self.paused = true;
            self.outstanding.clear();
            self.bytes = 0;
            return Ok(Some(
                json!({"type":"resync","epoch":self.epoch,"sequence":self.sequence}),
            ));
        }
        self.bytes += bytes;
        self.outstanding.push_back((sequence, bytes));
        Ok(Some(
            json!({"type":"chunk","epoch":self.epoch,"sequence":self.sequence,"chunk":chunk}),
        ))
    }

    fn tool_input(&mut self, chunk: &Value) -> Result<(), String> {
        let id = chunk["toolCallId"]
            .as_str()
            .ok_or("Missing tool call ID.")?;
        if !super::process::valid_id(id)
            || chunk["toolName"]
                .as_str()
                .is_none_or(|name| name.is_empty() || name.len() > 100)
            || !chunk["input"].is_object()
            || serde_json::to_vec(&chunk["input"])
                .map_err(|_| "Invalid tool input.")?
                .len()
                > 64 * 1024
            || chunk["dynamic"] != true
        {
            return Err("Invalid dynamic tool input.".into());
        }
        let call_metadata = metadata(chunk, "providerMetadata")?;
        let parts = self.snapshot["message"]["parts"]
            .as_array_mut()
            .ok_or("Invalid snapshot.")?;
        if parts.len() >= MAX_PARTS || parts.iter().any(|part| part["toolCallId"] == id) {
            return Err("Duplicate or excessive tool call.".into());
        }
        let mut part = json!({
            "type":"dynamic-tool",
            "toolCallId":id,
            "toolName":chunk["toolName"],
            "input":chunk["input"],
            "state":"input-available",
        });
        if let Some(metadata) = call_metadata {
            part["callProviderMetadata"] = metadata;
        }
        parts.push(part);
        Ok(())
    }

    fn tool_output(&mut self, chunk: &Value, failed: bool) -> Result<(), String> {
        let id = chunk["toolCallId"]
            .as_str()
            .ok_or("Missing tool call ID.")?;
        if !super::process::valid_id(id) || chunk["dynamic"] != true {
            return Err("Invalid dynamic tool result.".into());
        }
        if failed {
            if chunk["errorText"]
                .as_str()
                .is_none_or(|text| text.len() > 8192)
            {
                return Err("Invalid tool error text.".into());
            }
        } else {
            let output = &chunk["output"];
            let size = serde_json::to_vec(output)
                .map_err(|_| "Invalid tool output.")?
                .len();
            if size > MAX_TOOL_OUTPUT_BYTES {
                return Err("Tool output exceeds 8 MiB.".into());
            }
            if !output.is_object()
                || !output["content"].is_array()
                || output
                    .get("isError")
                    .is_some_and(|value| !value.is_boolean())
            {
                return Err("Invalid MCP tool result.".into());
            }
        }
        let result_metadata = metadata(chunk, "providerMetadata")?;
        let parts = self.snapshot["message"]["parts"]
            .as_array_mut()
            .ok_or("Invalid snapshot.")?;
        let part = parts
            .iter_mut()
            .find(|part| part["type"] == "dynamic-tool" && part["toolCallId"] == id)
            .ok_or("Tool result has no matching input.")?;
        if part["state"] != "input-available" {
            return Err("Duplicate tool result.".into());
        }
        if failed {
            part["state"] = json!("output-error");
            part["errorText"] = chunk["errorText"].clone();
        } else {
            part["state"] = json!("output-available");
            part["output"] = chunk["output"].clone();
        }
        if let Some(metadata) = result_metadata {
            part["resultProviderMetadata"] = metadata;
        }
        Ok(())
    }

    fn tool_input_error(&mut self, chunk: &Value) -> Result<(), String> {
        let id = chunk["toolCallId"]
            .as_str()
            .ok_or("Missing tool call ID.")?;
        let name = chunk["toolName"].as_str().ok_or("Missing tool name.")?;
        let input = chunk.get("input").cloned().unwrap_or(Value::Null);
        let error_text = chunk["errorText"]
            .as_str()
            .filter(|text| text.len() <= 8192)
            .ok_or("Invalid tool input error.")?;
        if !super::process::valid_id(id)
            || name.is_empty()
            || name.len() > 100
            || serde_json::to_vec(&input)
                .map_err(|_| "Invalid tool input.")?
                .len()
                > 64 * 1024
            || chunk["dynamic"] != true
        {
            return Err("Invalid tool input error.".into());
        }
        let result_metadata = metadata(chunk, "providerMetadata")?;
        let parts = self.snapshot["message"]["parts"]
            .as_array_mut()
            .ok_or("Invalid snapshot.")?;
        if parts.len() >= MAX_PARTS || parts.iter().any(|part| part["toolCallId"] == id) {
            return Err("Duplicate or excessive tool call.".into());
        }
        let mut part = json!({
            "type":"dynamic-tool",
            "toolCallId":id,
            "toolName":name,
            "input":input,
            "state":"output-error",
            "errorText":error_text,
        });
        if let Some(metadata) = result_metadata {
            part["resultProviderMetadata"] = metadata;
        }
        parts.push(part);
        Ok(())
    }
}

fn metadata(chunk: &Value, key: &str) -> Result<Option<Value>, String> {
    let Some(value) = chunk.get(key).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if !value.is_object()
        || serde_json::to_vec(value)
            .map_err(|_| "Invalid provider metadata.")?
            .len()
            > MAX_METADATA_BYTES
    {
        return Err("Provider metadata exceeds its limit.".into());
    }
    Ok(Some(value.clone()))
}
