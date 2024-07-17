use crate::config;
use async_trait::async_trait;

#[derive(Debug, Clone, Default)]
pub struct AccessTokenStore {}

#[async_trait]
impl alipan::OAuthClientAccessTokenStore for AccessTokenStore {
    async fn get_access_token(&self) -> anyhow::Result<Option<alipan::AccessToken>> {
        Ok(Some(config::get_access_token().await?))
    }

    async fn set_access_token(&self, access_token: alipan::AccessToken) -> anyhow::Result<()> {
        config::set_access_token(access_token).await?;
        Ok(())
    }
}
