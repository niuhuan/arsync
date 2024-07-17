use crate::custom_crypto::{
    decrypt_base64, decrypt_file_name, encrypt_buff_to_base64, encrypt_file_name,
};
use alipan::response::AdriveOpenFile;
use alipan::{AdriveClient, AdriveOpenFilePartInfoCreate, AdriveOpenFileType, CheckNameMode};
use anyhow::Context;
use reqwest::Body;
use serde_derive::{Deserialize, Serialize};
use std::fs::Metadata;
use std::path::PathBuf;
use std::sync::Arc;

pub async fn find_passbook_folder(
    client: &Arc<AdriveClient>,
    drive_id: String,
    folder_id: String,
) -> anyhow::Result<(Option<AdriveOpenFile>, Vec<AdriveOpenFile>)> {
    let mut passbook: Option<AdriveOpenFile> = None;
    let mut open_file_list: Vec<AdriveOpenFile> = vec![];
    let mut list = client
        .adrive_open_file_list()
        .await
        .drive_id(drive_id.clone())
        .parent_file_id(folder_id.clone())
        .request()
        .await?;
    for x in list.items {
        if x.name.eq("passbook") {
            passbook = Some(x);
            continue;
        }
        open_file_list.push(x);
    }
    while list.next_marker.is_some() {
        list = client
            .adrive_open_file_list()
            .await
            .drive_id(drive_id.clone())
            .parent_file_id(folder_id.clone())
            .marker(list.next_marker.unwrap())
            .request()
            .await?;
        for x in list.items {
            if x.name.eq("passbook") {
                passbook = Some(x);
                continue;
            }
            open_file_list.push(x);
        }
    }
    Ok((passbook, open_file_list))
}

pub async fn list_remote_folder_file(
    client: &Arc<AdriveClient>,
    drive_id: String,
    folder_id: String,
) -> anyhow::Result<Vec<AdriveOpenFile>> {
    let mut open_file_list: Vec<AdriveOpenFile> = vec![];
    let mut list = client
        .adrive_open_file_list()
        .await
        .drive_id(drive_id.clone())
        .parent_file_id(folder_id.clone())
        .request()
        .await?;
    for x in list.items {
        if x.name.eq("passbook") {
            continue;
        }
        open_file_list.push(x);
    }
    while list.next_marker.is_some() {
        list = client
            .adrive_open_file_list()
            .await
            .drive_id(drive_id.clone())
            .parent_file_id(folder_id.clone())
            .marker(list.next_marker.unwrap())
            .request()
            .await?;
        for x in list.items {
            if x.name.eq("passbook") {
                continue;
            }
            open_file_list.push(x);
        }
    }
    Ok(open_file_list)
}

pub async fn list_local_folder_file(
    source_path: &String,
) -> anyhow::Result<Vec<(PathBuf, Metadata)>> {
    let mut entries = tokio::fs::read_dir(&source_path)
        .await
        .with_context(|| format!("read dir failed: {}", source_path))?;
    let mut metadata_list = vec![];
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        metadata_list.push((entry.path(), metadata));
    }
    Ok(metadata_list)
}

#[derive(Serialize, Deserialize)]
pub struct Passbook {
    pub key_encrypted: String,
    pub test_encrypted: String,
}

pub async fn check_passbook_password(
    client: Arc<AdriveClient>,
    passbook: AdriveOpenFile,
    password: String,
) -> anyhow::Result<Vec<u8>> {
    let file_down_url = client
        .adrive_open_file_get_download_url()
        .await
        .file_id(passbook.file_id)
        .drive_id(passbook.drive_id)
        .request()
        .await?;
    let download_buff = download_file_to_buff(file_down_url.url).await?;
    let passbook: Passbook = toml::from_str(&download_buff)?;
    let test_decrypted = decrypt_file_name(&passbook.test_encrypted, password.as_bytes())?;
    if test_decrypted.eq("test") {
        return Ok(decrypt_base64(
            &passbook.key_encrypted,
            password.as_bytes(),
        )?);
    }
    return Err(anyhow::anyhow!("密码不正确"));
}

pub async fn download_file_to_buff(url: String) -> anyhow::Result<String> {
    let resp = ::reqwest::get(url).await?;
    let resp = resp.text().await?;
    Ok(resp)
}

pub async fn create_passbook_password(
    client: &Arc<AdriveClient>,
    drive_id: String,
    folder_id: String,
    password: String,
) -> anyhow::Result<Vec<u8>> {
    let key = random_string(64);
    let passbook = Passbook {
        key_encrypted: encrypt_buff_to_base64(key.as_slice(), password.as_bytes())?,
        test_encrypted: encrypt_file_name("test", &password.as_bytes())?,
    };
    let passbook = toml::to_string(&passbook)?;
    let parts = vec![AdriveOpenFilePartInfoCreate { part_number: 1 }];
    let file = client
        .adrive_open_file_create()
        .await
        .check_name_mode(CheckNameMode::Refuse)
        .drive_id(drive_id.clone())
        .parent_file_id(folder_id.clone())
        .r#type(AdriveOpenFileType::File)
        .name("passbook")
        .size(passbook.len() as i64)
        .part_info_list(parts)
        .request()
        .await?;
    reqwest::Client::new()
        .put(file.part_info_list[0].upload_url.as_str())
        .body(Body::from(passbook))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    client
        .adrive_open_file_complete()
        .await
        .drive_id(file.drive_id.clone())
        .file_id(file.file_id.clone())
        .upload_id(file.upload_id.clone())
        .request()
        .await?;
    Ok(key)
}

fn random_string(len: usize) -> Vec<u8> {
    use rand::distributions::Alphanumeric;
    use rand::{thread_rng, Rng};
    let buff = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .collect::<Vec<u8>>();
    buff
}
