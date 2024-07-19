use crate::common::{
    check_passbook_password, delete_remote_file, find_passbook_folder, list_local_folder_file,
    list_remote_folder_file,
};
use crate::config::adrive_client_for_config;
use crate::custom_crypto::{decrypt_file_name, decryptor_from_key};
use alipan::{AdriveClient, AdriveOpenFileType};
use anyhow::Context;
use chrono::TimeZone;
use clap::{arg, Command};
use futures_util::stream::TryStreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;

pub const COMMAND_NAME: &str = "down";

pub fn command() -> Command {
    Command::new(COMMAND_NAME).args(args())
}

fn args() -> Vec<clap::Arg> {
    vec![
        arg!(-s --source <SOURCE_PATH> "remote target uri, like `adrive://drive_id/file_path`"),
        arg!(-t --target <CONFIG_FILE_PATH> "local source uri, like `file:///tmp/Backups`"),
        arg!(-p --password <PASSWORD> "password for adrive folder encryption").required(false),
    ]
}

pub(crate) async fn run_sub_command(args: &clap::ArgMatches) -> anyhow::Result<()> {
    let source: &String = args
        .get_one("source")
        .with_context(|| "source is required")?;
    let target: &String = args
        .get_one("target")
        .with_context(|| "target is required")?;
    let source_url =
        url::Url::parse(source).with_context(|| format!("source url is invalid: {}", source))?;
    let target_url =
        url::Url::parse(target).with_context(|| format!("target url is invalid: {}", target))?;
    let source_path = source_url.path();
    let target_path = target_url.path();
    if !"adrive".eq(source_url.scheme()) {
        return Err(anyhow::anyhow!("原路径必须是adrive协议"));
    }
    if !"file".eq(target_url.scheme()) {
        return Err(anyhow::anyhow!("目标路径必须是file协议"));
    }
    let source_sp = source_path.split('/').collect::<Vec<&str>>();
    if source_sp.len() < 3 || source_sp[0] != "" {
        return Err(anyhow::anyhow!(
            "目标路径必须是 `adrive:///{{DriveID}}/{{文件夹路径}}`"
        ));
    }
    let drive_id = source_sp[1].to_owned();
    let folder_path = "/".to_owned() + &source_sp[2..].join("/").to_owned();
    let folder_path = urlencoding::decode(&folder_path)?.to_string();
    let client = adrive_client_for_config().await?;
    let folder_info = client
        .adrive_open_file_get_by_path()
        .await
        .drive_id(drive_id.clone())
        .file_path(folder_path)
        .request()
        .await?;
    if !AdriveOpenFileType::Folder.eq(&folder_info.r#type) {
        return Err(anyhow::anyhow!("原路径必须是一个文件夹"));
    }
    let metadata = tokio::fs::metadata(target_path)
        .await
        .with_context(|| format!("目标路径未找到 : {}", target_path))?;
    if !metadata.is_dir() {
        return Err(anyhow::anyhow!("目标路径必须是文件夹"));
    }
    // 验证密码
    let mut sync_password: Option<Vec<u8>> = None;
    let password: Option<&String> = args.get_one("password");
    let (passbook, _other_files) =
        find_passbook_folder(&client, drive_id.clone(), folder_info.file_id.clone()).await?;
    if let Some(passbook) = passbook {
        if let Some(password) = password {
            sync_password = Some(
                check_passbook_password(Arc::clone(&client), passbook, password.clone()).await?,
            );
        } else {
            return Err(anyhow::anyhow!("需要密码"));
        }
    } else {
        if let Some(_password) = password {
            return Err(anyhow::anyhow!("云端无密码"));
        }
    }
    down_sync_folder(
        Arc::clone(&client),
        drive_id.clone(),
        folder_info.file_id,
        sync_password.clone(),
        target_path.to_owned(),
    )
    .await?;
    Ok(())
}

#[async_recursion::async_recursion]
async fn down_sync_folder(
    client: Arc<AdriveClient>,
    drive_id: String,
    folder_id: String,
    sync_password: Option<Vec<u8>>,
    target_path: String,
) -> anyhow::Result<()> {
    // 读取远端文件
    let mut open_file_list =
        list_remote_folder_file(&client, drive_id.clone(), folder_id.clone()).await?;
    // 读取本地的文件
    let mut metadata_list = list_local_folder_file(&target_path).await?;
    //
    let mut remote_delete = false;
    let mut local_delete = false;
    // 1. 删掉日期不一样的，名字不存在的
    let mut remote_file_date_map = HashMap::new();
    let mut remote_folder_list = Vec::new();
    for x in &open_file_list {
        let mut name = x.name.clone();
        if let Some(sync_password) = &sync_password {
            match decrypt_file_name(name.as_str(), sync_password) {
                Ok(decrypt_name) => {
                    name = decrypt_name;
                }
                Err(_) => {
                    println!("解密文件名失败, 删除文件: {}", name);
                    remote_delete = true;
                    delete_remote_file(Arc::clone(&client), drive_id.clone(), x.file_id.clone())
                        .await?;
                    continue;
                }
            }
        }
        match x.r#type {
            AdriveOpenFileType::File => {
                remote_file_date_map.insert(name, x.updated_at);
            }
            AdriveOpenFileType::Folder => {
                remote_folder_list.push(name);
            }
        }
    }
    for (p, m) in &metadata_list {
        let mut delete = true;
        let file_name = p
            .file_name()
            .with_context(|| format!("文件名解析失败: {:?}", p))?
            .to_str()
            .with_context(|| format!("文件名解析失败: {:?}", p))?
            .to_string();
        if m.is_dir() {
            if remote_folder_list.contains(&file_name) {
                delete = false;
            }
        } else if m.is_file() {
            if let Some(date) = remote_file_date_map.get(&file_name) {
                let md = m
                    .modified()
                    .with_context(|| "modified is empty")?
                    .duration_since(std::time::UNIX_EPOCH)?;
                let md = chrono::Utc
                    .timestamp_opt(md.as_secs() as i64, md.subsec_nanos())
                    .unwrap();
                if md.timestamp() >= date.timestamp() {
                    delete = false;
                }
            }
        }
        if delete {
            println!("删除: {:?}", p);
            local_delete = true;
            if m.is_file() {
                tokio::fs::remove_file(p).await?;
            } else if m.is_dir() {
                tokio::fs::remove_dir_all(p).await?;
            }
        }
    }
    // 2. 下载不存在的
    // 读取远端文件
    if remote_delete {
        open_file_list =
            list_remote_folder_file(&client, drive_id.clone(), folder_id.clone()).await?;
    }
    // 读取本地的文件
    if local_delete {
        metadata_list = list_local_folder_file(&target_path).await?;
    }
    // 下载
    // 阿里云盘限制：一分钟最多获取10次下载链接
    let mut local_name_list = Vec::new();
    for (p, _m) in &metadata_list {
        local_name_list.push(
            p.file_name()
                .with_context(|| format!("文件名解析失败: {:?}", p))?
                .to_str()
                .with_context(|| format!("文件名解析失败: {:?}", p))?
                .to_string(),
        )
    }
    for x in open_file_list {
        // todo
        let mut name = x.name.clone();
        if let Some(sync_password) = &sync_password {
            match decrypt_file_name(name.as_str(), sync_password) {
                Ok(decrypt_name) => {
                    name = decrypt_name;
                }
                Err(_) => {
                    println!("解密文件名失败, 删除文件: {}", name);
                    delete_remote_file(Arc::clone(&client), drive_id.clone(), x.file_id.clone())
                        .await?;
                    continue;
                }
            }
        }
        let path = std::path::Path::new(&target_path).join(&name);
        let path_string = path.to_str().unwrap().to_string();
        match x.r#type {
            AdriveOpenFileType::File => {
                if !local_name_list.contains(&name) {
                    down_file(
                        Arc::clone(&client),
                        drive_id.clone(),
                        x.file_id.clone(),
                        sync_password.clone(),
                        path_string,
                    )
                    .await?;
                }
            }
            AdriveOpenFileType::Folder => {
                if !local_name_list.contains(&name) {
                    tokio::fs::create_dir_all(path).await?;
                }
                down_sync_folder(
                    Arc::clone(&client),
                    drive_id.clone(),
                    x.file_id.clone(),
                    sync_password.clone(),
                    path_string,
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn down_file(
    client: Arc<AdriveClient>,
    drive_id: String,
    file_id: String,
    sync_password: Option<Vec<u8>>,
    local_file_path: String,
) -> anyhow::Result<()> {
    println!("down file: {}", local_file_path);
    let path_tmp = format!("{}.tmp", local_file_path);
    let url = client
        .adrive_open_file_get_download_url()
        .await
        .drive_id(drive_id)
        .file_id(file_id)
        .request()
        .await?
        .url;
    if let Some(sync_password) = sync_password {
        down_to_file_with_password(url, path_tmp.as_str(), sync_password).await?;
    } else {
        down_to_file(url, path_tmp.as_str()).await?;
    }
    move_file(path_tmp.as_str(), local_file_path.as_str()).await?;
    Ok(())
}

async fn move_file(from: &str, to: &str) -> anyhow::Result<()> {
    tokio::fs::rename(from, to).await?;
    Ok(())
}

async fn down_to_file(url: String, path: &str) -> anyhow::Result<()> {
    let mut stream = reqwest::get(url).await?.error_for_status()?.bytes_stream();
    let mut file = tokio::fs::File::create(path).await?;
    while let Some(item) = stream.next().await {
        file.write_all(&item?).await?;
    }
    file.flush().await?;
    Ok(())
}

async fn down_to_file_with_password(
    url: String,
    path: &str,
    sync_password: Vec<u8>,
) -> anyhow::Result<()> {
    let stream = reqwest::get(url)
        .await?
        .error_for_status()?
        .bytes_stream()
        .map_err(convert_err);
    let mut reader = StreamReader::new(stream);
    let mut file = tokio::fs::File::create(path).await?;
    let mut decryptor = decryptor_from_key(sync_password.as_slice())?;
    let mut buffer = [0u8; (1 << 20) + 16];
    let mut position = 0;
    loop {
        let n = reader.read(&mut buffer[position..]).await?;
        position += n;
        if n == 0 {
            let item = decryptor
                .decrypt_last(&buffer[..position])
                .map_err(|e| anyhow::anyhow!("解密时出错(3): {}", e))?;
            file.write_all(&item).await?;
            file.flush().await?;
            break;
        }
        if position == buffer.len() {
            position = 0;
            let item = decryptor
                .decrypt_next(&buffer[..])
                .map_err(|e| anyhow::anyhow!("解密时出错(2): {}", e))?;
            file.write_all(&item).await?;
        }
    }
    Ok(())
}

fn convert_err(err: reqwest::Error) -> std::io::Error {
    std::io::Error::other(err)
}
