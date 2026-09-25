use super::{
    attachments,
    preferences::Connection,
    store::{Conversation, Start, Store},
};
use serde_json::{json, Value};
use std::path::Path;

const MAX_PARTS: usize = 1024;
const MAX_PROVIDER_METADATA_BYTES: usize = 64 * 1024;
const MAX_TOOL_INPUT_BYTES: usize = 64 * 1024;
const MAX_TOOL_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct Context {
    pub payload: Value,
    pub attachments: Vec<attachments::Attachment>,
}

/// Build exactly the provider-bound context without reading credentials or
/// starting a process. Approval and actual generation share these checks.
pub(super) fn context(
    store: &Store,
    root: &Path,
    input: &Start,
    loaded: &Conversation,
    connection: &Connection,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Context, String> {
    check()?;
    let context = store.preview(input);
    let mut disclosed_attachments = std::collections::BTreeMap::new();
    let payload=context.and_then(|messages|{
            let mut result=Vec::new();
            let mut context_bytes = loaded.config.system.len();
            for (message, attachment_ids) in messages {
                check()?;
                if message.parts_version!=1{return Err("This message uses an unsupported parts version. Start a new conversation.".into());}
                let mut parts=Vec::new();
                for part in message.parts.as_array().ok_or("Invalid stored message.")? {
                    match part["type"].as_str() {
                        Some("text") | Some("reasoning") => {
                            let kind = part["type"].as_str().unwrap();
                            let text = part["text"].as_str().ok_or("Invalid stored response text.")?;
                            let mut preserved = json!({"type":kind,"text":text});
                            if let Some(id) = part.get("id").filter(|id| !id.is_null()) {
                                if !id.is_string() || id.as_str().unwrap().len() > 256 {
                                    return Err("Invalid stored response block ID.".into());
                                }
                                preserved["id"] = id.clone();
                            }
                            if let Some(state) = part.get("state").filter(|state| !state.is_null()) {
                                if !matches!(state.as_str(), Some("streaming" | "done")) {
                                    return Err("Invalid stored response state.".into());
                                }
                                preserved["state"] = state.clone();
                            }
                            if let Some(metadata) = bounded_metadata(part.get("providerMetadata"))? {
                                preserved["providerMetadata"] = metadata;
                            }
                            parts.push(preserved);
                        }
                        Some("step-start") => parts.push(json!({"type":"step-start"})),
                        Some("dynamic-tool") => parts.push(preserve_tool_part(part)?),
                        _ => return Err("This message contains unsupported content. Start a new conversation.".into()),
                    }
                }
                for id in attachment_ids {
                    check()?;
                    let (attachment,bytes)=attachments::read_object(store,root,&id)?;
                    disclosed_attachments.entry(id).or_insert_with(|| attachment.clone());
                    if attachment.mime!="text/plain" {
                        if !super::backend::capability(&connection.provider, &loaded.config.model, "images") { return Err("Image input is not verified for this model. Choose a supported model or remove the images.".into()); }
                        parts.push(json!({"type":"file","mediaType":attachment.mime,"filename":attachment.name,"url":format!("data:{};base64,{}",attachment.mime,super::process::encode(&bytes))}));
                        continue;
                    }
                    let text=std::str::from_utf8(&bytes).map_err(|_|"Attachment encoding changed.")?.trim_start_matches('\u{feff}');
                    parts.push(json!({"type":"text","text":format!("\nAttached file: {}\n{}",attachment.name,text)}));
                }
                if parts.len() > MAX_PARTS || result.len() >= 2000 {return Err("The conversation exceeds supported message limits. Start a new conversation.".into());}
                let value = json!({"id":message.id,"role":message.role,"parts":parts});
                context_bytes += serde_json::to_vec(&value).map_err(|_| "Invalid message.")?.len();
                if context_bytes > 40*1024*1024 { return Err("The conversation exceeds the 40 MiB context limit.".into()); }
                result.push(value);
            }
            Ok(json!({"provider":connection.provider,"model":loaded.config.model,"assistantId":input.assistant_id,"messages":result,"system":loaded.config.system,"maxOutputTokens":loaded.config.max_output_tokens}))
        })?;
    let mut payload = payload;
    if let Some(temperature) = loaded.config.temperature {
        if !super::backend::capability(&connection.provider, &loaded.config.model, "temperature") {
            return Err(
                "Temperature is not verified for this model. Reset it to the default.".into(),
            );
        }
        payload["temperature"] = json!(temperature);
    }
    if serde_json::to_vec(&payload)
        .map_err(|_| "Invalid context.")?
        .len()
        > 40 * 1024 * 1024
    {
        return Err("The conversation exceeds the 40 MiB context limit.".into());
    }
    check()?;
    Ok(Context {
        payload,
        attachments: disclosed_attachments.into_values().collect(),
    })
}

fn bounded_metadata(value: Option<&Value>) -> Result<Option<Value>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if !value.is_object()
        || serde_json::to_vec(value)
            .map_err(|_| "Invalid provider metadata.")?
            .len()
            > MAX_PROVIDER_METADATA_BYTES
    {
        return Err("Provider metadata exceeds its limit.".into());
    }
    Ok(Some(value.clone()))
}

fn preserve_tool_part(part: &Value) -> Result<Value, String> {
    let name = part["toolName"]
        .as_str()
        .ok_or("Invalid stored tool name.")?;
    let call_id = part["toolCallId"]
        .as_str()
        .ok_or("Invalid stored tool call ID.")?;
    if name.is_empty()
        || name.len() > 100
        || !super::process::valid_id(call_id)
        || part["providerExecuted"] == true
    {
        return Err("This message contains an unsupported tool call.".into());
    }
    let state = part["state"].as_str().ok_or("Invalid stored tool state.")?;
    if !matches!(
        state,
        "input-available" | "output-available" | "output-error"
    ) {
        return Err("This message contains an unsupported tool state.".into());
    }
    let input = part
        .get("input")
        .or_else(|| part.get("rawInput"))
        .cloned()
        .unwrap_or(Value::Null);
    let input_bytes = serde_json::to_vec(&input).map_err(|_| "Invalid stored tool input.")?;
    if input_bytes.len() > MAX_TOOL_INPUT_BYTES {
        return Err("This message contains invalid tool arguments.".into());
    }
    if state != "output-error" && (!input.is_object() || !lomi_mcp::chat::contains_tool(name)) {
        return Err("This message contains an unsupported tool call.".into());
    }
    let mut preserved = json!({
        "type":"dynamic-tool",
        "toolName":name,
        "toolCallId":call_id,
        "state":state,
        "input":input,
    });
    match state {
        "output-available" => {
            let output = part.get("output").ok_or("Missing stored tool result.")?;
            let encoded = serde_json::to_vec(output).map_err(|_| "Invalid stored tool result.")?;
            if encoded.len() > MAX_TOOL_OUTPUT_BYTES
                || !output.is_object()
                || !output["content"].is_array()
                || output
                    .get("isError")
                    .is_some_and(|value| !value.is_boolean())
            {
                return Err("This message contains an invalid or oversized MCP result.".into());
            }
            preserved["output"] = output.clone();
        }
        "output-error" => {
            let error = part["errorText"]
                .as_str()
                .filter(|error| error.len() <= 8192)
                .ok_or("Invalid stored tool error.")?;
            preserved["errorText"] = json!(error);
        }
        _ => (),
    }
    for key in ["callProviderMetadata", "resultProviderMetadata"] {
        if let Some(metadata) = bounded_metadata(part.get(key))? {
            preserved[key] = metadata;
        }
    }
    Ok(preserved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_tool_parts_roundtrip_provider_metadata_without_wire_only_fields() {
        let input = json!({
            "type":"dynamic-tool",
            "toolCallId":"tool-1",
            "toolName":"lomi_workspace_list",
            "state":"input-available",
            "input":{},
            "dynamic":true,
            "callProviderMetadata":{"google":{"thoughtSignature":"call-signature"}}
        });
        let input = preserve_tool_part(&input).unwrap();
        assert_eq!(input["state"], "input-available");
        assert!(input.get("dynamic").is_none());
        assert_eq!(
            input["callProviderMetadata"]["google"]["thoughtSignature"],
            "call-signature"
        );

        let output = json!({
            "type":"dynamic-tool",
            "toolCallId":"tool-1",
            "toolName":"lomi_workspace_list",
            "state":"output-available",
            "input":{},
            "output":{"content":[{"type":"text","text":"{}"}],"structuredContent":{"kind":"workspaces"}},
            "callProviderMetadata":{"google":{"thoughtSignature":"call-signature"}},
            "resultProviderMetadata":{"google":{"thoughtSignature":"result-signature"}}
        });
        let output = preserve_tool_part(&output).unwrap();
        assert_eq!(output["output"]["structuredContent"]["kind"], "workspaces");
        assert_eq!(
            output["callProviderMetadata"]["google"]["thoughtSignature"],
            "call-signature"
        );
        assert_eq!(
            output["resultProviderMetadata"]["google"]["thoughtSignature"],
            "result-signature"
        );
    }

    #[test]
    fn tool_input_errors_are_kept_without_becoming_executable_calls() {
        let failed = json!({
            "type":"dynamic-tool",
            "toolCallId":"tool-2",
            "toolName":"hallucinated_tool",
            "state":"output-error",
            "input":"invalid raw arguments",
            "errorText":"invalid arguments",
            "resultProviderMetadata":{"anthropic":{"signature":"opaque"}}
        });
        let context = preserve_tool_part(&failed).unwrap();
        assert_eq!(context["toolName"], "hallucinated_tool");
        assert_eq!(context["input"], "invalid raw arguments");
        assert_eq!(context["state"], "output-error");
        assert!(context.get("dynamic").is_none());
        assert!(preserve_tool_part(&json!({
            "type":"dynamic-tool",
            "toolCallId":"tool-3",
            "toolName":"lomi_status",
            "state":"input-available",
            "input":{},
            "providerExecuted":true
        }))
        .is_err());
    }
}
