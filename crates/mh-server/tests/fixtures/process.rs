use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Fixture {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    MockAgent,
    McpBridge {
        #[arg(long)]
        url: String,
        #[arg(long)]
        session: String,
        #[arg(long)]
        caller: String,
    },
}

// Release packaging builds only `meta-harness`. CI uses this fixture for subprocess tests,
// including on hosted Windows runners whose process tokens are already elevated.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Fixture::parse().command {
        Commands::MockAgent => {
            mh_acp::mock_agent::run_mock_acp_agent_stdio().await;
            Ok(())
        }
        Commands::McpBridge {
            url,
            session,
            caller,
        } => mh_server::mcp_bridge::run_mcp_bridge(url, session, caller).await,
    }
}
