//! P0 wire fixture; an example target, never part of the shipped helper.
use lomi_control_protocol::{EmptyInput, ErrorCode, ToolError, MAX_FRAME_BYTES};
use lomi_mcp::bounded_stdio::BoundedStdin;
use rmcp::{model::*, service::RequestContext, ErrorData, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProbeResult {
    control_api_version: String,
    fixture: bool,
    width: u32,
    height: u32,
}

struct Probe;

fn catalog() -> Vec<Tool> {
    ["probe_image", "probe_error"]
        .into_iter()
        .map(|name| {
            let tool = Tool::new(
                name,
                "Isolated P0 protocol fixture; does not control Lomi.",
                serde_json::Map::new(),
            )
            .with_input_schema::<EmptyInput>()
            .with_annotations(
                ToolAnnotations::new()
                    .read_only(true)
                    .destructive(false)
                    .idempotent(true)
                    .open_world(false),
            );
            if name == "probe_image" {
                tool.with_output_schema::<ProbeResult>()
            } else {
                tool.with_output_schema::<ToolError>()
            }
        })
        .collect()
}

impl ServerHandler for Probe {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "lomi-protocol-probe",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "P0 qualification fixture only. No app resources, grants, shell or paid calls.",
            )
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        catalog().into_iter().find(|tool| tool.name == name)
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.and_then(|r| r.cursor).is_some() {
            return Err(ErrorData::invalid_params("Invalid catalog cursor", None));
        }
        Ok(ListToolsResult {
            tools: catalog(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let _: EmptyInput = serde_json::from_value(serde_json::Value::Object(
            request.arguments.unwrap_or_default(),
        ))
        .map_err(|_| ErrorData::invalid_params("Expected an empty object", None))?;
        let result = match request.name.as_ref() {
            "probe_error" => CallToolResult::structured_error(
                serde_json::to_value(ToolError::new(
                    ErrorCode::TargetNotFound,
                    "Fixture target does not exist",
                ))
                .unwrap(),
            ),
            "probe_image" => {
                let mut result = CallToolResult::structured(
                    serde_json::to_value(ProbeResult {
                        control_api_version: "1.0".into(),
                        fixture: true,
                        width: 1,
                        height: 1,
                    })
                    .unwrap(),
                );
                result.content.push(ContentBlock::image("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=", "image/png"));
                result
            }
            _ => return Err(ErrorData::invalid_params("Unknown fixture tool", None)),
        };
        Ok(result.into())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = Probe
        .serve((
            BoundedStdin::new(tokio::io::stdin(), MAX_FRAME_BYTES),
            tokio::io::stdout(),
        ))
        .await?;
    service.waiting().await?;
    Ok(())
}
