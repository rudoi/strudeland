//! Local MCP adapter. The SDK owns stdio, initialization, discovery, and request framing.
use std::io::Write;
use std::path::PathBuf;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use strudeland_project::InstrumentLibrary;

use super::instruments::{Apply, Instruments};
use super::{EXIT_APP_ERROR, EXIT_OK, Failure, current_dir};
use crate::transport::SocketTransport;

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Source {
    Library,
    Project,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct List {
    /// Choose the global instrument library or the pins belonging to one project.
    source: Source,
    /// Project directory for project pins. Defaults to the enclosing project, then the app's.
    project: Option<PathBuf>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Create {
    id: String,
    /// A definition such as {"schema":1,"parameters":{},"voices":[{"name":"lead",
    /// "chain":[{"control":"s","value":"sine"}]}]}. A control value can refer to a
    /// parameter default by name using {"param":"cutoff"}.
    definition: Map<String, Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Pin {
    id: String,
    /// Project directory. Defaults to the enclosing project, then the app's.
    project: Option<PathBuf>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Update {
    id: String,
    project: Option<PathBuf>,
    /// Current project pin fingerprint returned by instrument_list.
    expect: String,
}

struct InstrumentServer {
    socket: Option<PathBuf>,
    timeout: u64,
}

impl InstrumentServer {
    fn serve_stdio(self) -> Result<(), Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let server = self.serve(rmcp::transport::stdio()).await?;
            if let rmcp::service::QuitReason::JoinError(error) = server.waiting().await? {
                return Err(error.into());
            }
            Ok(())
        })
    }

    async fn perform<T: Serialize>(
        &self,
        action: impl FnOnce(&mut Instruments<'_>) -> Result<T, Failure> + Send + 'static,
    ) -> CallToolResult {
        let socket = self.socket.clone();
        let timeout = self.timeout;
        // Both the store and control client are blocking. Reconnect for each call, so a
        // restarted app is available to the same MCP session.
        let result = tokio::task::spawn_blocking(move || {
            let mut transport = SocketTransport::new(socket, timeout);
            let mut instruments = Instruments {
                transport: &mut transport,
                cwd: &current_dir,
                library: &InstrumentLibrary::open_default,
            };
            action(&mut instruments)
                .map(|value| json!(value))
                .map_err(|e| e.message())
        })
        .await;
        match result {
            Ok(Ok(value)) => CallToolResult::structured(value),
            Ok(Err(message)) => CallToolResult::error(vec![ContentBlock::text(message)]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        }
    }
}

#[tool_router]
impl InstrumentServer {
    #[tool(
        description = "List the directory, IDs and fingerprints of global library entries or project pins."
    )]
    async fn instrument_list(&self, Parameters(args): Parameters<List>) -> CallToolResult {
        self.perform(move |ops| match args.source {
            Source::Library => ops.library_entries(),
            Source::Project => ops.project_entries(args.project.as_deref()),
        })
        .await
    }

    #[tool(
        description = "Create a global library instrument from its definition. An existing ID is refused."
    )]
    async fn instrument_create(&self, Parameters(args): Parameters<Create>) -> CallToolResult {
        self.perform(move |ops| ops.create(&args.id, &Value::Object(args.definition).to_string()))
            .await
    }

    #[tool(description = "Pin a global library instrument to the project for the first time.")]
    async fn instrument_pin(&self, Parameters(args): Parameters<Pin>) -> CallToolResult {
        self.perform(move |ops| ops.pin(&args.id, args.project.as_deref()))
            .await
    }

    #[tool(
        description = "Wrap a complete expression in the active editor with a pinned instrument. Leaves the edit unsaved and returns the new range."
    )]
    async fn instrument_apply(&self, Parameters(args): Parameters<Apply>) -> CallToolResult {
        self.perform(move |ops| ops.apply(args)).await
    }

    #[tool(
        description = "Replace a project pin with the current library definition if its fingerprint matches expect."
    )]
    async fn instrument_update(&self, Parameters(args): Parameters<Update>) -> CallToolResult {
        self.perform(move |ops| ops.update(&args.id, args.project.as_deref(), &args.expect))
            .await
    }
}

#[tool_handler(router = Self::tool_router(), name = "strudeland")]
impl ServerHandler for InstrumentServer {}

pub(super) fn serve(socket: Option<PathBuf>, timeout: u64, stderr: &mut dyn Write) -> i32 {
    let server = InstrumentServer { socket, timeout };
    match server.serve_stdio() {
        Ok(()) => EXIT_OK,
        Err(error) => {
            let _ = writeln!(stderr, "error: MCP server: {error}");
            EXIT_APP_ERROR
        }
    }
}
