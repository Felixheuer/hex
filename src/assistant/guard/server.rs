//! `hex assistant mcp --spec …`: the guard as an MCP stdio server. Tool
//! lists and results pass through as agent-desktop wrote them; the guard
//! decides each call (`proxy`).

use std::sync::{Arc, Mutex};

use color_eyre::eyre::{Result, eyre};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};

use super::child::Desktop;
use super::proxy::Guard;

struct Server<D: Desktop + 'static> {
    guard: Arc<Mutex<Guard<D>>>,
    tools: Vec<Tool>,
    instructions: String,
}

impl<D: Desktop + 'static> ServerHandler for Server<D> {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("agent-desktop", crate::VERSION)
                    .with_title("Hex desktop guard"),
            )
            .with_instructions(self.instructions.clone())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let guard = self.guard.clone();
        let name = request.name.to_string();
        let args = request.arguments.unwrap_or_default();
        let value = tokio::task::spawn_blocking(move || {
            guard
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .call(&name, args)
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let result: CallToolResult = serde_json::from_value(value)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(result.into())
    }
}

/// Serves `guard` over stdin/stdout until the agent disconnects.
pub fn serve<D: Desktop + 'static>(mut guard: Guard<D>) -> Result<()> {
    let tools = guard
        .tools()
        .map_err(|why| eyre!("agent-desktop didn't list its tools: {why}"))?
        .into_iter()
        .filter_map(|tool| serde_json::from_value::<Tool>(tool).ok())
        .collect();
    let mut instructions = guard.desktop().instructions().unwrap_or_default();
    if !instructions.is_empty() {
        instructions.push_str("\n\n");
    }
    instructions.push_str(&guard.rules());
    let server = Server {
        guard: Arc::new(Mutex::new(guard)),
        tools,
        instructions,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| eyre!("MCP session failed to start: {error}"))?;
        service.waiting().await?;
        Ok(())
    })
}
