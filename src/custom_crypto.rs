use aead::consts::U12;
use aead::stream::{Decryptor, DecryptorBE32, Encryptor, EncryptorBE32, StreamBE32};
use aead::KeyInit;
use aes_gcm::aes::Aes256;
use aes_gcm::{Aes256Gcm, AesGcm};
use base64::Engine;

pub fn encryptor_from_key(
    password: &[u8],
) -> anyhow::Result<Encryptor<AesGcm<Aes256, U12>, StreamBE32<AesGcm<Aes256, U12>>>> {
    let key_bytes = md5::compute(password).0;
    let key_hex = hex::encode(key_bytes);
    let key = key_hex.as_bytes();
    let nonce_slice = &key_bytes[0..7];
    let cipher = Aes256Gcm::new_from_slice(key)?;
    let encryptor = EncryptorBE32::from_aead(cipher, nonce_slice.into());
    Ok(encryptor)
}

pub fn encrypt_buff(buff: &[u8], password: &[u8]) -> anyhow::Result<Vec<u8>> {
    let encryptor = encryptor_from_key(password)?;
    let final_vec = encryptor
        .encrypt_last(buff)
        .map_err(|e| anyhow::anyhow!("加密时出错: {}", e))?;
    Ok(final_vec)
}

pub fn encrypt_buff_to_base64(buff: &[u8], password: &[u8]) -> anyhow::Result<String> {
    let final_vec = encrypt_buff(buff, password)?;
    let final_base64 = base64::prelude::BASE64_URL_SAFE.encode(final_vec.as_slice());
    Ok(final_base64)
}

pub fn encrypt_file_name(file_name: &str, password: &[u8]) -> anyhow::Result<String> {
    Ok(encrypt_buff_to_base64(file_name.as_bytes(), password)?)
}

pub fn decryptor_from_key(
    password: &[u8],
) -> anyhow::Result<Decryptor<AesGcm<Aes256, U12>, StreamBE32<AesGcm<Aes256, U12>>>> {
    let key_bytes = md5::compute(password).0;
    let key_hex = hex::encode(key_bytes);
    let key = key_hex.as_bytes();
    let nonce_slice = &key_bytes[0..7];
    let cipher = Aes256Gcm::new_from_slice(key)?;
    let decryptor = DecryptorBE32::from_aead(cipher, nonce_slice.into());
    Ok(decryptor)
}

pub fn decrypt_buff(buff: &[u8], password: &[u8]) -> anyhow::Result<Vec<u8>> {
    let encryptor = decryptor_from_key(password)?;
    let final_buff = encryptor
        .decrypt_last(buff)
        .map_err(|e| anyhow::anyhow!("解密时出错(1): {}", e))?;
    Ok(final_buff)
}

pub fn decrypt_base64(base64_str: &str, password: &[u8]) -> anyhow::Result<Vec<u8>> {
    let final_vec = base64::prelude::BASE64_URL_SAFE.decode(base64_str.as_bytes())?;
    decrypt_buff(final_vec.as_slice(), password)
}

pub fn decrypt_file_name(file_name: &str, password: &[u8]) -> anyhow::Result<String> {
    decrypt_base64(file_name, password)
        .and_then(|v| String::from_utf8(v).map_err(|e| anyhow::anyhow!("解码时出错: {}", e)))
}

#[test]
pub fn test_encrypt_decrypt() {
    let password = decrypt_base64("YVY-359tgDPNDJsyaoEC_Ay0qEcZ5PlwddCnslO4xvkGcocEjM9M6e367GDfN4oP21wCYAMb2Cq532MylqhLWCVz1USKpv6Rk6NBJE_C-rE=", "isonlypass".as_bytes()).unwrap();

    let src_file = "/Volumes/DATA/Downloads/sdksInstallers/B/FFmpeg-master.zip";
    let enc_file = "/Volumes/DATA/Downloads/sdksInstallers1/FFmpeg-master.zip.enc";
    let dec_file = "/Volumes/DATA/Downloads/sdksInstallers1/FFmpeg-master.zip.dec.zip";

    println!("encryptor");
    let mut encryptor = encryptor_from_key(password.as_slice()).unwrap();
    let mut src_file = std::fs::File::open(src_file).unwrap();
    let mut target_file = std::fs::File::create(enc_file).unwrap();
    let mut buff = [0u8; 1 << 10];
    while let Some(n) = src_file.read(&mut buff).ok() {
        if n == 0 {
            break;
        }
        println!("{}", n);
        let encrypted_buff = encryptor.encrypt_next(&buff[..n]).unwrap();
        target_file.write_all(&encrypted_buff).unwrap();
    }
    let encrypted_buff = encryptor.encrypt_last(Vec::<u8>::new().as_slice()).unwrap();
    target_file.write_all(&encrypted_buff).unwrap();
    target_file.flush().unwrap();
    drop(src_file);
    drop(target_file);

    println!("decryptor");
    let mut decryptor = decryptor_from_key(password.as_slice()).unwrap();
    let mut enc_file = std::fs::File::open(enc_file).unwrap();
    let mut target_file = std::fs::File::create(dec_file).unwrap();
    let mut buff = [0u8; (1 << 10) + 16];
    while let Some(n) = enc_file.read(&mut buff).ok() {
        if n == 0 {
            break;
        }
        println!("{}", n);
        let decrypted_buff = decryptor.decrypt_next(&buff[..n]).unwrap();
        target_file.write_all(&decrypted_buff).unwrap();
    }
    let decrypted_buff = decryptor.decrypt_last(Vec::<u8>::new().as_slice()).unwrap();
    target_file.write_all(&decrypted_buff).unwrap();
    target_file.flush().unwrap();
    drop(enc_file);
    drop(target_file);
}
