use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use aes_gcm::aead::rand_core::RngCore;
use fred::prelude::*;
use serde::{Deserialize, Serialize};

const REDIS_KEY: &str = "sigmacode:server_key";
const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedKey {
    nonce: String,
    ciphertext: String,
}

fn derive_key(master_hex: &str) -> Result<[u8; 32], anyhow::Error> {
    let bytes = hex::decode(master_hex)?;
    if bytes.len() != 32 {
        anyhow::bail!("ENCRYPTION_KEY must be 64 hex chars (32 bytes)");
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

pub fn encrypt_key(plaintext: &str, master_key_hex: &str) -> Result<String, anyhow::Error> {
    let key = derive_key(master_key_hex)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow::anyhow!("Cipher init failed: {}", e))?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

    let enc = EncryptedKey {
        nonce: hex::encode(nonce_bytes),
        ciphertext: hex::encode(ciphertext),
    };
    Ok(serde_json::to_string(&enc)?)
}

pub fn decrypt_key(encrypted_json: &str, master_key_hex: &str) -> Result<String, anyhow::Error> {
    let key = derive_key(master_key_hex)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow::anyhow!("Cipher init failed: {}", e))?;

    let enc: EncryptedKey = serde_json::from_str(encrypted_json)?;
    let nonce_bytes = hex::decode(&enc.nonce)?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = hex::decode(&enc.ciphertext)?;

    let plaintext = cipher.decrypt(nonce, ciphertext.as_ref())
        .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?;
    Ok(String::from_utf8(plaintext)?)
}

pub async fn store_key_in_redis(
    server_key: &str,
    master_key_hex: &str,
    redis_url: &str,
) -> Result<(), anyhow::Error> {
    let encrypted = encrypt_key(server_key, master_key_hex)?;

    let config = RedisConfig::from_url(redis_url)?;
    let client = RedisClient::new(config, None, None, None);
    client.init().await?;

    let _: () = client.set(REDIS_KEY, &encrypted, None, None, false).await?;
    tracing::info!("Server key stored in Redis (encrypted)");
    client.quit().await?;
    Ok(())
}

pub async fn load_key_from_redis(
    master_key_hex: &str,
    redis_url: &str,
) -> Result<String, anyhow::Error> {
    let config = RedisConfig::from_url(redis_url)?;
    let client = RedisClient::new(config, None, None, None);
    client.init().await?;

    let encrypted: String = client.get(REDIS_KEY).await?;
    let key = decrypt_key(&encrypted, master_key_hex)?;
    client.quit().await?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let master = "02163eb0a4d05fd3265610aa4b6df4812be1d9a8510b7badea13bb625df5496d";
        let plaintext = "my-super-secret-server-key-12345";

        let encrypted = encrypt_key(plaintext, master).unwrap();
        let decrypted = decrypt_key(&encrypted, master).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_different_nonces() {
        let master = "02163eb0a4d05fd3265610aa4b6df4812be1d9a8510b7badea13bb625df5496d";

        let enc1 = encrypt_key("test", master).unwrap();
        let enc2 = encrypt_key("test", master).unwrap();

        let e1: EncryptedKey = serde_json::from_str(&enc1).unwrap();
        let e2: EncryptedKey = serde_json::from_str(&enc2).unwrap();
        assert_ne!(e1.nonce, e2.nonce);

        assert_eq!(decrypt_key(&enc1, master).unwrap(), "test");
        assert_eq!(decrypt_key(&enc2, master).unwrap(), "test");
    }
}
