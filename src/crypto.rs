use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, Generate, KeyInit},
};
use base64::{Engine, engine::general_purpose::STANDARD};

const NONCE_LEN: usize = 12;

pub struct Cipher(Aes256Gcm);

impl Cipher {
    pub fn from_base64_key(key: &str) -> Self {
        let key = STANDARD
            .decode(key.trim())
            .expect("SETTINGS_ENCRYPTION_KEY must be valid base64");
        let cipher = Aes256Gcm::new_from_slice(&key)
            .expect("SETTINGS_ENCRYPTION_KEY must decode to 32 bytes");
        Self(cipher)
    }

    pub fn encrypt(&self, plaintext: &str) -> String {
        let nonce = Nonce::generate();
        let ciphertext = self
            .0
            .encrypt(&nonce, plaintext.as_bytes())
            .expect("AES-GCM encryption failed");

        let mut payload = nonce.to_vec();
        payload.extend_from_slice(&ciphertext);
        STANDARD.encode(payload)
    }

    pub fn decrypt(&self, value: &str) -> Result<String, String> {
        let payload = STANDARD
            .decode(value)
            .map_err(|_| "encrypted value is not valid base64".to_string())?;
        if payload.len() < NONCE_LEN {
            return Err("encrypted value is too short".to_string());
        }

        let (nonce, ciphertext) = payload.split_at(NONCE_LEN);
        let nonce = Nonce::try_from(nonce).map_err(|_| "invalid nonce".to_string())?;
        let plaintext = self.0.decrypt(&nonce, ciphertext).map_err(|_| {
            "failed to decrypt value (was SETTINGS_ENCRYPTION_KEY changed?)".to_string()
        })?;

        String::from_utf8(plaintext).map_err(|_| "decrypted value is not UTF-8".to_string())
    }
}
