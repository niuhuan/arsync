use crate::config;
use alipan::GrantType;
use clap::{arg, Command};
use serde_json::json;
use std::collections::HashMap;
use std::convert::Infallible;
use warp::http::Response;
use warp::hyper::Body;
use warp::{Filter, Reply};

pub const COMMAND_NAME: &str = "config";

pub fn command() -> Command {
    Command::new(COMMAND_NAME).args(args())
}

fn args() -> Vec<clap::Arg> {
    vec![arg!(--port <PASSWORD> "端口 默认是58080").required(false)]
}

pub(crate) async fn run_sub_command(args: &clap::ArgMatches) -> anyhow::Result<()> {
    // run warp server
    let port = args
        .get_one::<String>("port")
        .map_or("58080", |v| v.as_str())
        .parse::<u16>()?;
    run_warp_server(port).await?;
    Ok(())
}

async fn run_warp_server(port: u16) -> anyhow::Result<()> {
    println!(
        "打开网页配置app以及账户信息： http://localhost:{}/html/index.html",
        port
    );
    println!("配置完成后，请按 Ctrl+C 停止服务");
    let routes = index().or(api());
    warp::serve(routes).run(([127, 0, 0, 1], port)).await;
    Ok(())
}

fn index() -> impl Filter<Extract = impl Reply, Error = warp::Rejection> + Clone {
    let mut static_resource_map = HashMap::<&str, &str>::new();
    static_resource_map.insert("index.html", include_str!("../../html/index.html"));
    let static_resource_map = static_resource_map;
    warp::path!("html" / String)
        .and(warp::get())
        .map(move |file_name: String| {
            if let Some(resource) = static_resource_map.get(file_name.as_str()) {
                let mime = match file_name.split('.').last() {
                    Some("html") => "text/html",
                    Some("css") => "text/css",
                    Some("js") => "application/javascript",
                    _ => "text/plain",
                };
                warp::reply::with_header(warp::reply::html(*resource), "Content-Type", mime)
                    .into_response()
            } else {
                warp::reply::with_status(
                    warp::reply::html("Not Found"),
                    warp::http::StatusCode::NOT_FOUND,
                )
                .into_response()
            }
        })
}

fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    get_client_info()
        .or(save_client_info())
        .or(oauth_authorize())
}

fn get_client_info() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("api" / "client_info")
        .and(warp::get())
        .and_then(get_client_info_body)
}

async fn get_client_info_body() -> Result<impl warp::Reply, Infallible> {
    map_err(get_client_info_body_inner().await)
}

async fn get_client_info_body_inner() -> anyhow::Result<Response<Body>> {
    let config = config::get_config().await?;
    Ok(warp::reply::json(&config.app).into_response())
}

fn map_err(result: anyhow::Result<Response<Body>>) -> Result<impl warp::Reply, Infallible> {
    match result {
        Ok(reply) => Ok(reply),
        Err(err) => Ok(warp::reply::with_status(
            warp::reply::html(format!("Error: {}", err)),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        )
        .into_response()),
    }
}

fn save_client_info() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("api" / "client_info")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(save_client_info_body)
}

async fn save_client_info_body(
    app_config: config::AppConfig,
) -> Result<impl warp::Reply, Infallible> {
    map_err(save_client_info_body_inner(app_config).await)
}

async fn save_client_info_body_inner(
    app_config: config::AppConfig,
) -> anyhow::Result<Response<Body>> {
    config::set_app_config(app_config.clone()).await?;

    let oauth_client = alipan::OAuthClient::default()
        .set_client_id(app_config.client_id.as_str())
        .await
        .set_client_secret(app_config.client_secret.as_str())
        .await;

    let url = oauth_client
        .oauth_authorize()
        .await
        .redirect_uri("http://localhost:58080/oauth_authorize")
        .scope("user:base,file:all:read,file:all:write,album:shared:read")
        .build()?;

    Ok(warp::reply::json(&json!({
        "url": url,
    }))
    .into_response())
}

fn oauth_authorize() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("oauth_authorize")
        .and(warp::get())
        .and(warp::query::<HashMap<String, String>>())
        .and_then(oauth_authorize_body)
}

async fn oauth_authorize_body(
    query: HashMap<String, String>,
) -> Result<impl warp::Reply, Infallible> {
    map_err(oauth_authorize_body_inner(query).await)
}

async fn oauth_authorize_body_inner(
    query: HashMap<String, String>,
) -> anyhow::Result<Response<Body>> {
    let code = query
        .get("code")
        .ok_or_else(|| anyhow::anyhow!("code 未找到"))?;

    let app_config = config::get_app_config().await?;

    let oauth_client = alipan::OAuthClient::default()
        .set_client_id(app_config.client_id.as_str())
        .await
        .set_client_secret(app_config.client_secret.as_str())
        .await;

    let raw_token = oauth_client
        .oauth_access_token()
        .await
        .grant_type(GrantType::AuthorizationCode)
        .code(code.as_str())
        .request()
        .await?;

    let access_token = alipan::AccessToken::wrap_oauth_token(raw_token);
    config::set_access_token(access_token.clone()).await?;

    Ok(warp::reply::html("认证成功，请关闭此页面").into_response())
}
