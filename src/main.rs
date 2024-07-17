mod arsync;
mod commands;
mod common;
mod config;
mod custom_crypto;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    arsync::run_command().await
}
