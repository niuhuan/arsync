use crate::common::{
    check_passbook_password, create_passbook_password, delete_remote_file, find_passbook_folder,
    list_local_folder_file, list_remote_folder_file,
};
use crate::config::adrive_client_for_config;
use crate::custom_crypto::{decrypt_file_name, encrypt_file_name, encryptor_from_key};
use alipan::response::AdriveOpenFile;
use alipan::AdriveOpenFileType::Folder;
use alipan::{AdriveClient, AdriveOpenFilePartInfoCreate, AdriveOpenFileType, CheckNameMode};
use anyhow::Context;
use chrono::{Local, TimeZone};
use clap::{arg, Command};
use sha1::Digest;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncReadExt;

pub const COMMAND_NAME: &str = "up";

pub fn command() -> Command {
    Command::new(COMMAND_NAME).args(args())
}

fn args() -> Vec<clap::Arg> {
    vec![
        arg!(-s --source <SOURCE_PATH> "local source uri, like `file:///tmp/Backups`"),
        arg!(-t --target <CONFIG_FILE_PATH> "remote target uri, like `adrive://drive_id/file_path`"),
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
    if !"file".eq(source_url.scheme()) {
        return Err(anyhow::anyhow!("原路径必须是file协议"));
    }
    let metadata = tokio::fs::metadata(source_path)
        .await
        .with_context(|| format!("原路径未找到 : {}", source_path))?;
    if !metadata.is_dir() {
        return Err(anyhow::anyhow!("原路径必须是文件夹"));
    }
    if !"adrive".eq(target_url.scheme()) {
        return Err(anyhow::anyhow!("目标路径必须是adrive协议"));
    }
    let target_sp = target_path.split('/').collect::<Vec<&str>>();
    if target_sp.len() < 3 || target_sp[0] != "" {
        return Err(anyhow::anyhow!(
            "目标路径必须是 `adrive:///{{DriveID}}/{{文件夹路径}}`"
        ));
    }
    let drive_id = target_sp[1].to_owned();
    let folder_path = "/".to_owned() + &target_sp[2..].join("/").to_owned();
    // url解码
    let folder_path = urlencoding::decode(&folder_path)?.to_string();
    let client = adrive_client_for_config().await?;
    let folder_info = client
        .adrive_open_file_get_by_path()
        .await
        .drive_id(drive_id.clone())
        .file_path(folder_path)
        .request()
        .await?;
    if !Folder.eq(&folder_info.r#type) {
        return Err(anyhow::anyhow!("目标路径必须是一个文件夹"));
    }
    let password: Option<&String> = args.get_one("password");
    let password = password.map(|p| p.clone());
    let mut sync_password: Option<Vec<u8>> = None;
    if let Some(password) = password {
        let (passbook, other_files) =
            find_passbook_folder(&client, drive_id.clone(), folder_info.file_id.clone()).await?;
        if let Some(passbook) = passbook {
            // 核实密码对不对
            sync_password =
                Some(check_passbook_password(Arc::clone(&client), passbook, password).await?);
        } else {
            if other_files.is_empty() {
                // 创建password
                sync_password = Some(
                    create_passbook_password(
                        &client,
                        drive_id.clone(),
                        folder_info.file_id.clone(),
                        password,
                    )
                    .await?,
                );
            } else {
                return Err(anyhow::anyhow!(
                    "文件夹不为空，且无密码，请删除文件重新同步或不使用密码"
                ));
            }
        }
    }
    up_sync_folder(
        source_path.to_owned(),
        Arc::clone(&client),
        drive_id.clone(),
        folder_info.file_id,
        sync_password.clone(),
    )
    .await?;
    Ok(())
}

#[async_recursion::async_recursion]
async fn up_sync_folder(
    source_path: String,
    client: Arc<AdriveClient>,
    drive_id: String,
    folder_id: String,
    sync_password: Option<Vec<u8>>,
) -> anyhow::Result<()> {
    println!("向云端同步 : {}", source_path);
    // 读取本地的文件
    let metadata_list = list_local_folder_file(&source_path).await?;
    // 读取远端文件
    let mut open_file_list =
        list_remote_folder_file(&client, drive_id.clone(), folder_id.clone()).await?;
    // 1. 删掉日期不一样的，名字不存在的
    // 整理一个本地留存的文件和修改日期的map
    let mut has_deleted = false;
    let mut local_folder_list = Vec::new();
    let mut local_file_update_map = HashMap::<String, chrono::DateTime<Local>>::new();
    for (pb, m) in &metadata_list {
        let name = pb
            .file_name()
            .with_context(|| "文件名为空(1)")?
            .to_str()
            .with_context(|| "文件名为空(2)")?
            .to_string();
        if name.is_empty() {
            return Err(anyhow::anyhow!("文件名未空(3)"));
        }
        if m.is_file() {
            let updated_at = chrono::DateTime::from(m.modified().unwrap());
            local_file_update_map.insert(name, updated_at);
        } else if m.is_dir() {
            local_folder_list.push(name);
        }
    }
    for x in &open_file_list {
        let mut delete = true;
        let mut name = x.name.clone();
        let mut error_file_name = false;
        if let Some(sync_password) = &sync_password {
            if let Ok(n) = decrypt_file_name(&name, sync_password) {
                name = n;
            } else {
                println!(
                    "删除云端文件 : {}/{}  (文件名解密失败)",
                    source_path, x.name
                );
                error_file_name = true;
            }
        }
        if !error_file_name {
            match x.r#type {
                AdriveOpenFileType::File => {
                    if let Some(date) = local_file_update_map.get(&name) {
                        if x.updated_at.timestamp() >= date.timestamp() {
                            delete = false;
                        } else {
                            println!(
                                "删除云端文件 : {}/{} (云端文件更新时间比本地更早)",
                                source_path, name
                            )
                        }
                    } else {
                        println!(
                            "删除云端文件 : {}/{} (本地对应文件已经删除)",
                            source_path, name
                        )
                    }
                }
                Folder => {
                    if local_folder_list.contains(&name) {
                        delete = false;
                    } else {
                        println!("删除云端文件 : {}/{}/", source_path, name)
                    }
                }
            }
        }
        if delete {
            has_deleted = true;
            delete_remote_file(Arc::clone(&client), x.drive_id.clone(), x.file_id.clone()).await?;
        }
    }
    if has_deleted {
        open_file_list =
            list_remote_folder_file(&client, drive_id.clone(), folder_id.clone()).await?;
    }
    // 上传不存在的
    let open_file_name_obj_map = open_file_list
        .iter()
        .map(|x| (x.name.clone(), x.clone()))
        .collect::<HashMap<String, AdriveOpenFile>>();
    for (pb, m) in &metadata_list {
        let name = pb
            .file_name()
            .with_context(|| "file name is invalid")?
            .to_str()
            .with_context(|| "file name is invalid")?
            .to_string();
        let mut remote_name = name.clone();
        if let Some(password) = &sync_password {
            remote_name = encrypt_file_name(&name, password)?;
        }
        if m.is_file() {
            if open_file_name_obj_map.contains_key(&remote_name) {
                continue;
            }
            up_sync_file(
                pb.to_str()
                    .with_context(|| "file name is invalid")?
                    .to_string(),
                m,
                Arc::clone(&client),
                drive_id.clone(),
                folder_id.clone(),
                remote_name,
                sync_password.clone(),
            )
            .await?;
        } else if m.is_dir() {
            let remote_dir_id = if let Some(obj) = open_file_name_obj_map.get(&remote_name) {
                obj.file_id.clone()
            } else {
                client
                    .adrive_open_file_create()
                    .await
                    .check_name_mode(CheckNameMode::Refuse)
                    .drive_id(drive_id.as_str())
                    .parent_file_id(folder_id.as_str())
                    .name(remote_name.as_str())
                    .r#type(AdriveOpenFileType::Folder)
                    .request()
                    .await?
                    .file_id
            };
            up_sync_folder(
                pb.to_str()
                    .with_context(|| "file name is invalid")?
                    .to_string(),
                Arc::clone(&client),
                drive_id.clone(),
                remote_dir_id,
                sync_password.clone(),
            )
            .await?;
        }
    }
    Ok(())
}

async fn up_sync_file(
    source_path: String,
    m: &std::fs::Metadata,
    client: Arc<AdriveClient>,
    drive_id: String,
    folder_id: String,
    file_name: String,
    sync_password: Option<Vec<u8>>,
) -> anyhow::Result<()> {
    println!("上传至云端 : {}", source_path);
    let md = m
        .modified()
        .with_context(|| "modified is empty")?
        .duration_since(std::time::UNIX_EPOCH)?;
    let md = chrono::Utc
        .timestamp_opt(md.as_secs() as i64, md.subsec_nanos())
        .unwrap();
    let cd = m
        .created()
        .with_context(|| "created is empty")?
        .duration_since(std::time::UNIX_EPOCH)?;
    let cd = chrono::Utc
        .timestamp_opt(cd.as_secs() as i64, cd.subsec_nanos())
        .unwrap();
    let (sha1, size) = sum_file(source_path.as_str(), &sync_password).await?;
    let parts = vec![AdriveOpenFilePartInfoCreate { part_number: 1 }];
    let result = client
        .adrive_open_file_create()
        .await
        .check_name_mode(CheckNameMode::Refuse)
        .drive_id(drive_id.as_str())
        .parent_file_id(folder_id.as_str())
        .name(file_name.as_str())
        .r#type(AdriveOpenFileType::File)
        .size(size as i64)
        .content_hash_name("sha1")
        .content_hash(sha1)
        .local_modified_at(md)
        .local_created_at(cd)
        .part_info_list(parts)
        .request()
        .await?;
    if result.rapid_upload {
        return Ok(());
    }
    if result.exist {
        return Err(anyhow::anyhow!("文件已存在"));
    }
    let url = result.part_info_list[0].upload_url.clone();
    put_file(source_path.as_str(), &sync_password, url.as_str()).await?;
    client
        .adrive_open_file_complete()
        .await
        .drive_id(result.drive_id.as_str())
        .file_id(result.file_id.as_str())
        .upload_id(
            result
                .upload_id
                .with_context(|| "upload_id is empty")?
                .as_str(),
        )
        .request()
        .await?;
    Ok(())
}

async fn sum_file(
    file_path: &str,
    sync_password: &Option<Vec<u8>>,
) -> anyhow::Result<(String, u64)> {
    if let Some(sync_password) = sync_password {
        Ok(password_sha1(file_path, sync_password).await?)
    } else {
        Ok((
            sha1_file(file_path).await?,
            tokio::fs::metadata(file_path).await?.len(),
        ))
    }
}

async fn password_sha1(file_path: &str, password: &[u8]) -> anyhow::Result<(String, u64)> {
    let file = tokio::fs::File::open(file_path)
        .await
        .with_context(|| format!("读取文件失败: {}", file_path))?;
    let mut hasher = sha1::Sha1::new();
    let mut encryptor = encryptor_from_key(password)?;
    let mut reader = tokio::io::BufReader::new(file);
    let mut buffer = [0u8; 1 << 20];
    let mut size = 0;
    let mut position = 0;
    loop {
        let n = reader.read(&mut buffer[position..]).await?;
        position += n;
        if n == 0 {
            let b = encryptor
                .encrypt_last(&buffer[..position])
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            hasher.update(b.as_slice());
            size += b.len();
            break;
        }
        if position == buffer.len() {
            position = 0;
            let b = encryptor
                .encrypt_next(&buffer[..n])
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            hasher.update(b.as_slice());
            size += b.len();
        }
    }
    let result = hasher.finalize();
    Ok((hex::encode(result), size as u64))
}

async fn sha1_file(file: &str) -> anyhow::Result<String> {
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    let file = tokio::fs::File::open(file).await?;
    let mut reader = tokio::io::BufReader::new(file);
    let mut buffer = [0u8; 1 << 10];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let result = hasher.finalize();
    Ok(hex::encode(result))
}

async fn put_file(
    file_path: &str,
    sync_password: &Option<Vec<u8>>,
    url: &str,
) -> anyhow::Result<()> {
    if let Some(sync_password) = sync_password {
        put_file_with_password(file_path, sync_password, url).await
    } else {
        put_file_without_password(file_path, url).await
    }
}

async fn put_file_with_password(file_path: &str, password: &[u8], url: &str) -> anyhow::Result<()> {
    let (sender, body) = PutResource::channel_resource();
    let request = reqwest::Client::new().put(url).body(body).send();
    let cp = sender.clone();
    let read_file_back = async move {
        let result = put_steam_with_password(cp, file_path, password).await;
        if let Err(e) = result {
            let _ = sender.send(Err(e)).await;
        }
    };
    let (send, _read) = tokio::join!(request, read_file_back);
    send?.error_for_status()?;
    Ok(())
}

async fn put_file_without_password(file_path: &str, url: &str) -> anyhow::Result<()> {
    let (sender, body) = PutResource::channel_resource();
    let request = reqwest::Client::new().put(url).body(body).send();
    let cp = sender.clone();
    let read_file_back = async move {
        let result = put_steam(cp, file_path).await;
        if let Err(e) = result {
            let _ = sender.send(Err(e)).await;
        }
    };
    let (send, _read) = tokio::join!(request, read_file_back);
    send?.error_for_status()?;
    Ok(())
}

async fn put_steam(
    sender: tokio::sync::mpsc::Sender<anyhow::Result<Vec<u8>>>,
    path: &str,
) -> anyhow::Result<()> {
    let mut buffer = vec![0u8; 1 << 10];
    let file = tokio::fs::File::open(path).await?;
    let mut reader = tokio::io::BufReader::new(file);
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        sender.send(Ok(buffer[..n].to_vec())).await?;
    }
    Ok(())
}

async fn put_steam_with_password(
    sender: tokio::sync::mpsc::Sender<anyhow::Result<Vec<u8>>>,
    path: &str,
    password: &[u8],
) -> anyhow::Result<()> {
    let mut buffer = vec![0u8; 1 << 20];
    let file = tokio::fs::File::open(path).await?;
    let mut reader = tokio::io::BufReader::new(file);
    let mut encryptor = encryptor_from_key(password)?;
    let mut position = 0;
    loop {
        let n = reader.read(&mut buffer[position..]).await?;
        position += n;
        if n == 0 {
            let enc = encryptor.encrypt_last(&buffer[..position]);
            match enc {
                Ok(vec) => {
                    sender.send(Ok(vec)).await?;
                }
                Err(err) => {
                    sender.send(Err(anyhow::anyhow!("{}", err))).await?;
                    return Err(anyhow::anyhow!("{}", err));
                }
            }
            break;
        }
        if position == buffer.len() {
            position = 0;
            match encryptor.encrypt_next(&buffer[..]) {
                Ok(vec) => {
                    sender.send(Ok(vec)).await?;
                }
                Err(err) => {
                    sender.send(Err(anyhow::anyhow!("{}", err))).await?;
                    return Err(anyhow::anyhow!("{}", err));
                }
            }
        }
    }
    Ok(())
}

use reqwest::Body;
use tokio::sync::mpsc::Sender;

pub struct PutResource {
    pub agent: Arc<reqwest::Client>,
    pub url: String,
    pub resource: Body,
}

impl PutResource {
    pub async fn put(self) -> anyhow::Result<()> {
        let text = self
            .agent
            .request(reqwest::Method::PUT, self.url.as_str())
            .body(self.resource)
            .send()
            .await?
            .text()
            .await?;
        println!("{}", text);
        Ok(())
    }
}

impl PutResource {
    pub async fn file_resource(path: &str) -> anyhow::Result<Body> {
        let file = tokio::fs::read(path).await?;
        Ok(Body::from(file))
    }

    pub fn channel_resource() -> (Sender<anyhow::Result<Vec<u8>>>, Body) {
        let (sender, receiver) = tokio::sync::mpsc::channel::<anyhow::Result<Vec<u8>>>(16);
        let body = Body::wrap_stream(tokio_stream::wrappers::ReceiverStream::new(receiver));
        (sender, body)
    }

    pub fn bytes_body(bytes: Vec<u8>) -> Body {
        Body::from(bytes)
    }
}
