//! `cafleet server` — the only entry point to the HTTP server (SPEC §6.8,
//! A7): the built-in axum server, single process, no auto-reload, defaults
//! read from settings at command-definition time and shown in `--help`.

use clap::Args;

use crate::config::Settings;
use crate::error::CafleetError;

/// The `--help` defaults mirror exactly the two settings bindings the flags
/// override; a malformed `CAFLEET_BROKER_PORT` still fails loudly in
/// `Settings::from_env` before serving — the rendered default is
/// display-only.
fn default_host() -> String {
    std::env::var("CAFLEET_BROKER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn default_port() -> u16 {
    std::env::var("CAFLEET_BROKER_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8000)
}

#[derive(Args)]
pub struct ServerArgs {
    /// Bind address.
    #[arg(long, default_value_t = default_host())]
    host: String,
    /// Bind port.
    #[arg(long, default_value_t = default_port())]
    port: u16,
}

pub fn run(settings: &Settings, args: ServerArgs) -> Result<(), CafleetError> {
    super::helpers::schema_guard(settings)?;
    let app = crate::webui::create_app(&settings.database_url)?;
    let address = format!("{}:{}", args.host, args.port);
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| CafleetError::App(format!("cannot start the async runtime: {e}")))?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(&address)
            .await
            .map_err(|e| CafleetError::App(format!("cannot bind {address}: {e}")))?;
        axum::serve(listener, app)
            .await
            .map_err(|e| CafleetError::App(format!("server error: {e}")))
    })
}
