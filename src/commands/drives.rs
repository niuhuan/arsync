use crate::commands::access_token_loader::AccessTokenStore;
use crate::config;
use alipan::{OAuthClient, OAuthClientAccessTokenManager};
use clap::Command;
use std::sync::Arc;

pub const COMMAND_NAME: &str = "drives";

pub fn command() -> Command {
    Command::new(COMMAND_NAME).args(args())
}

fn args() -> Vec<clap::Arg> {
    vec![]
}

pub(crate) async fn run_sub_command(_args: &clap::ArgMatches) -> anyhow::Result<()> {
    let app_config = config::get_app_config().await?;
    let client = alipan::AdriveClient::default()
        .set_client_id(app_config.client_id.clone())
        .await
        .set_access_token_loader(Box::new(OAuthClientAccessTokenManager {
            oauth_client: OAuthClient::default()
                .set_client_id(app_config.client_id.as_str())
                .await
                .set_client_secret(app_config.client_secret.as_str())
                .await
                .into(),
            access_token_store: Arc::new(Box::new(AccessTokenStore {})),
        }))
        .await;
    let info = client.adrive_user_get_drive_info().await.request().await?;
    println!("default drive id : {}", info.default_drive_id);
    Ok(())
}
