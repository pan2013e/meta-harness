#[tokio::main]
async fn main() {
    mh_acp::mock_agent::run_mock_acp_agent_stdio().await;
}
