mod args;
mod auth;
mod dispatch;
mod logging;
mod mcp;
mod output;
mod service;

pub async fn run() -> anyhow::Result<()> {
    dispatch::run().await
}
