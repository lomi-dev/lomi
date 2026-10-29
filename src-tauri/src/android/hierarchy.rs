//! Bounded UI Automator projection. XML declarations never enable DTD/entity IO.
use lomi_control_protocol::{android::*, ErrorCode};
use quick_xml::{events::Event, Reader, XmlVersion};
use std::collections::BTreeMap;

fn bounded(value: &str, bytes: usize) -> String {
    let mut end = value.len().min(bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end]
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
fn bounds(value: &str) -> Option<[i32; 4]> {
    let values: Vec<_> = value
        .split(['[', ']', ','])
        .filter(|s| !s.is_empty())
        .map(str::parse::<i32>)
        .collect::<Result<_, _>>()
        .ok()?;
    let values: [i32; 4] = values.try_into().ok()?;
    if values.iter().any(|v| v.unsigned_abs() > 32767)
        || values[0] > values[2]
        || values[1] > values[3]
    {
        return None;
    }
    Some(values)
}

pub(super) fn parse(
    xml: &str,
    input: &AndroidSnapshotInput,
    snapshot_id: String,
    hardware: [u32; 2],
) -> Result<AndroidSnapshot, ErrorCode> {
    let invalid = ErrorCode::UnsupportedCapability;
    if xml.len() > 262144 || xml.is_empty() {
        return Err(invalid);
    }
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Option<u16>> = Vec::new();
    let mut seen_root = false;
    let mut ended_root = false;
    let mut total_nodes = 0usize;
    let mut snapshot = AndroidSnapshot {
        workspace_id: input.workspace_id.clone(), panel_id: input.panel_id.clone(), device_id: input.device_id.clone(), generation: input.generation.clone(), snapshot_id,
        captured_at_millis: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_| invalid)?.as_millis().to_string(),
        hardware_display: hardware, rotation: 0, coordinate_space: "rotated_display".into(), nodes: Vec::new(), truncated: false,
        limitations: vec!["UI Automator can omit secure screens, canvas content and parts of WebViews; absence is not proof of absence.".into(), "Password and editable field values are omitted. Bounds use the rotated screen, not preview pixels. Node IDs are observation-local and are not input targets.".into()],
    };
    let mut budget = serde_json::to_vec(&snapshot).map_err(|_| invalid)?.len();
    if budget > input.max_bytes as usize {
        return Err(ErrorCode::ResourceExhausted);
    }
    loop {
        let event = reader.read_event().map_err(|_| invalid)?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let name = element.name();
                if stack.len() >= 64 || ended_root {
                    return Err(invalid);
                }
                let mut attrs = BTreeMap::new();
                for attr in element.attributes() {
                    let attr = attr.map_err(|_| invalid)?;
                    if attrs.len() >= 32 || attr.value.len() > 16384 {
                        return Err(invalid);
                    }
                    attrs.insert(
                        std::str::from_utf8(attr.key.as_ref())
                            .map_err(|_| invalid)?
                            .to_string(),
                        attr.normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|_| invalid)?
                            .into_owned(),
                    );
                }
                let get = |name: &str| attrs.get(name).map(String::as_str).unwrap_or("");
                let id = if name.as_ref() == b"hierarchy" {
                    if seen_root || !stack.is_empty() {
                        return Err(invalid);
                    }
                    seen_root = true;
                    snapshot.rotation = get("rotation").parse().map_err(|_| invalid)?;
                    if snapshot.rotation > 3 {
                        return Err(invalid);
                    }
                    None
                } else if name.as_ref() == b"node" && seen_root && !stack.is_empty() {
                    total_nodes += 1;
                    if total_nodes > 4096 {
                        return Err(invalid);
                    }
                    let omitted = get("password") == "true"
                        || get("class").ends_with("EditText")
                        || get("editable") == "true";
                    let node = AndroidNode {
                        id: snapshot.nodes.len() as u16 + 1,
                        parent: stack.last().copied().flatten(),
                        class: bounded(get("class"), 256),
                        resource_id: bounded(get("resource-id"), 256),
                        package: bounded(get("package"), 256),
                        description: if get("password") == "true" {
                            String::new()
                        } else {
                            bounded(get("content-desc"), 512)
                        },
                        text: if omitted {
                            String::new()
                        } else {
                            bounded(get("text"), 512)
                        },
                        bounds: bounds(get("bounds")),
                        enabled: get("enabled") == "true",
                        clickable: get("clickable") == "true",
                        focused: get("focused") == "true",
                        scrollable: get("scrollable") == "true",
                        checked: get("checked") == "true",
                        value_omitted: omitted,
                    };
                    let size = serde_json::to_vec(&node).map_err(|_| invalid)?.len() + 1;
                    if snapshot.nodes.len() >= input.max_nodes as usize
                        || budget + size > input.max_bytes as usize
                    {
                        snapshot.truncated = true;
                        None
                    } else {
                        budget += size;
                        let id = node.id;
                        snapshot.nodes.push(node);
                        Some(id)
                    }
                } else {
                    return Err(invalid);
                };
                if !empty {
                    stack.push(id);
                } else if name.as_ref() == b"hierarchy" {
                    ended_root = true;
                }
            }
            Event::End(_) => {
                if stack.pop().is_none() {
                    return Err(invalid);
                }
                if stack.is_empty() {
                    ended_root = true;
                }
            }
            Event::Decl(_) if !seen_root => {}
            Event::Text(text) if text.iter().all(u8::is_ascii_whitespace) => {}
            Event::Eof => break,
            _ => return Err(invalid),
        }
    }
    if !seen_root || !ended_root || !stack.is_empty() {
        return Err(invalid);
    }
    Ok(snapshot)
}
