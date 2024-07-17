use crate::config;
use alipan::{
    AccessToken, AdriveClient, OAuthClient, OAuthClientAccessTokenManager,
    OAuthClientAccessTokenStore,
};
use anyhow::Context;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_derive::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub app: AppConfig,
    pub access_token: AccessToken,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub client_id: String,
    pub client_secret: String,
}

static CONFIG_PATH_CELL: OnceCell<String> = OnceCell::new();
static CONFIG_CELL: OnceCell<RwLock<Config>> = OnceCell::new();

pub async fn save_config() -> anyhow::Result<()> {
    let config = CONFIG_CELL
        .get()
        .ok_or_else(|| anyhow::anyhow!("配置文件未加载"))?;
    let config = config.read().await;
    let config = toml::to_string(&*config)?;
    tokio::fs::write(
        CONFIG_PATH_CELL
            .get()
            .with_context(|| "config cell not set")?
            .as_str(),
        config,
    )
    .await?;
    Ok(())
}

pub async fn load_config() -> anyhow::Result<()> {
    let config = tokio::fs::read_to_string(
        CONFIG_PATH_CELL
            .get()
            .with_context(|| "config cell not set")?
            .as_str(),
    )
    .await?;
    let config: Config = toml::from_str(&config)?;
    CONFIG_CELL
        .set(RwLock::new(config))
        .map_err(|_| anyhow::anyhow!("配置文件重复加载"))?;
    Ok(())
}

pub async fn new_config() -> anyhow::Result<()> {
    let config = Config::default();
    CONFIG_CELL
        .set(RwLock::new(config.clone()))
        .map_err(|_| anyhow::anyhow!("配置文件重复加载"))?;
    Ok(())
}

pub async fn get_config() -> anyhow::Result<Config> {
    Ok(CONFIG_CELL
        .get()
        .ok_or_else(|| anyhow::anyhow!("配置文件未加载"))?
        .read()
        .await
        .clone())
}

pub async fn set_app_config(app_config: AppConfig) -> anyhow::Result<()> {
    let mut config = CONFIG_CELL
        .get()
        .ok_or_else(|| anyhow::anyhow!("配置文件未加载"))?
        .write()
        .await;
    config.app = app_config;
    drop(config);
    save_config().await?;
    Ok(())
}

pub async fn get_app_config() -> anyhow::Result<AppConfig> {
    Ok(get_config().await?.app)
}

pub async fn set_path(config_path: &str) -> anyhow::Result<()> {
    CONFIG_PATH_CELL
        .set(config_path.to_string())
        .map_err(|_| anyhow::anyhow!("配置路径重复设置"))?;

    let data = tokio::fs::metadata(config_path).await.ok();
    if let Some(_data) = data {
        match config::load_config().await {
            Err(_) => {
                config::new_config().await?;
                config::save_config().await?;
            }
            _ => {}
        };
    } else {
        config::new_config().await?;
        config::save_config().await?;
    }
    Ok(())
}

pub async fn set_access_token(access_token: AccessToken) -> anyhow::Result<()> {
    let mut config = CONFIG_CELL
        .get()
        .ok_or_else(|| anyhow::anyhow!("配置文件未加载"))?
        .write()
        .await;
    config.access_token = access_token;
    drop(config);
    save_config().await?;
    Ok(())
}

pub async fn get_access_token() -> anyhow::Result<AccessToken> {
    Ok(get_config().await?.access_token)
}

#[derive(Debug)]
pub struct ConfigAccessTokenStore();

impl ConfigAccessTokenStore {
    pub fn new() -> Self {
        ConfigAccessTokenStore()
    }
}

#[async_trait]
impl OAuthClientAccessTokenStore for ConfigAccessTokenStore {
    async fn get_access_token(&self) -> anyhow::Result<Option<AccessToken>> {
        Ok(Some(get_access_token().await?))
    }

    async fn set_access_token(&self, access_token: AccessToken) -> anyhow::Result<()> {
        set_access_token(access_token).await
    }
}

pub async fn adrive_client_for_config() -> anyhow::Result<Arc<AdriveClient>> {
    let app_config = get_app_config().await?;
    let client = AdriveClient::default()
        .set_client_id(app_config.client_id.clone())
        .await
        .set_access_token_loader(Box::new(OAuthClientAccessTokenManager {
            oauth_client: Arc::new(
                OAuthClient::default()
                    .set_client_id(app_config.client_id.clone())
                    .await
                    .set_client_secret(app_config.client_secret.clone())
                    .await,
            ),
            access_token_store: Arc::new(Box::new(ConfigAccessTokenStore {})),
        }))
        .await;
    Ok(client.into())
}
