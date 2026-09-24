use super::{
    attachments,
    preferences::Connection,
    store::{Conversation, Start, Store},
};
use serde_json::{json, Value};
use std::path::Path;

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
                    if part["type"]=="text"{parts.push(json!({"type":"text","text":part["text"]}));}
                    else if part["type"]!="reasoning"{return Err("This message contains unsupported content. Start a new conversation.".into());}
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
                if parts.len() > 100 || result.len() >= 2000 {return Err("The conversation exceeds supported message limits. Start a new conversation.".into());}
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
