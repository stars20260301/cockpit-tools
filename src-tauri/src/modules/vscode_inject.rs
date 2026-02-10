//! VS Code GitHub Copilot token injection module (Windows only)
//!
//! Enables one-click Copilot account switching in VS Code by directly
//! writing auth sessions into VS Code's state.vscdb database.
//!
//! ## How it works
//!
//! VS Code stores extension secrets in a SQLite database (`state.vscdb`)
//! using Chromium v10 encryption format:
//!   1. A master AES-256 key is stored in `Local State`, protected by Windows DPAPI
//!   2. Secrets are encrypted with AES-256-GCM using the master key
//!
//! This module reads the master key via DPAPI (current user only),
//! decrypts the existing GitHub auth sessions, replaces the token
//! with the target account's token, re-encrypts, and writes back.
//!
//! **Security note**: This operates on the current user's own data using
//! their own DPAPI key. No privilege escalation or cross-user access is involved.

use std::path::PathBuf;

use base64::{engine::general_purpose, Engine as _};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use aes_gcm::aead::{Aead, AeadCore, OsRng};
use aes_gcm::aead::generic_array::GenericArray;

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HLOCAL, LocalFree};
#[cfg(target_os = "windows")]
use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

use rusqlite::Connection;

/// Get the path to VS Code's state.vscdb
pub fn get_vscode_db_path() -> Result<PathBuf, String> {
    let appdata = std::env::var("APPDATA")
        .map_err(|_| "Cannot read APPDATA environment variable".to_string())?;
    let path = PathBuf::from(appdata)
        .join("Code")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb");
    if path.exists() {
        Ok(path)
    } else {
        Err(format!("VS Code database not found: {}", path.display()))
    }
}

/// Read and decrypt the AES-256 master key from VS Code's Local State file
#[cfg(target_os = "windows")]
pub fn get_encryption_key() -> Result<Vec<u8>, String> {
    let path = get_local_state_path()?;
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read Local State: {}", e))?;

    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse Local State JSON: {}", e))?;

    let encrypted_key_b64 = json["os_crypt"]["encrypted_key"]
        .as_str()
        .ok_or("Cannot find os_crypt.encrypted_key in Local State")?;

    let encrypted_key_bytes = general_purpose::STANDARD
        .decode(encrypted_key_b64)
        .map_err(|e| format!("Base64 decode failed for encrypted_key: {}", e))?;

    if encrypted_key_bytes.len() < 6 {
        return Err("encrypted_key data too short".to_string());
    }

    let prefix = String::from_utf8_lossy(&encrypted_key_bytes[..5]);
    if prefix != "DPAPI" {
        return Err(format!("encrypted_key prefix is not DPAPI, got: {}", prefix));
    }

    let dpapi_blob = &encrypted_key_bytes[5..];
    let key = dpapi_decrypt(dpapi_blob)?;
    if key.len() != 32 {
        return Err(format!("Decrypted AES key has unexpected length: {}", key.len()));
    }
    Ok(key)
}

/// Decrypt Chromium v10 format data using AES-256-GCM
pub fn decrypt_v10(key: &[u8], encrypted: &[u8]) -> Result<Vec<u8>, String> {
    if encrypted.len() < 31 {
        return Err("Encrypted data too short".to_string());
    }
    if &encrypted[..3] != b"v10" {
        return Err(format!("Not v10 format, prefix: {:?}", &encrypted[..3]));
    }

    let nonce_bytes = &encrypted[3..15];
    let ciphertext = &encrypted[15..];

    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES-GCM decryption failed: {}", e))
}

/// Encrypt data into Chromium v10 format using AES-256-GCM
pub fn encrypt_v10(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("AES-GCM encryption failed: {}", e))?;

    let mut result = Vec::with_capacity(3 + 12 + ciphertext.len());
    result.extend_from_slice(b"v10");
    result.extend_from_slice(nonce.as_slice());
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Inject a Copilot account's GitHub token into VS Code's auth storage.
///
/// This replaces the `user:email` scoped session in the
/// `vscode.github-authentication` secret with the provided token,
/// effectively switching the logged-in GitHub account for Copilot.
#[cfg(target_os = "windows")]
pub fn inject_copilot_token(
    username: &str,
    token: &str,
    github_user_id: Option<&str>,
) -> Result<String, String> {
    let key = get_encryption_key()?;
    let db_path = get_vscode_db_path()?;
    let conn = Connection::open(&db_path)
        .map_err(|e| format!("Failed to open VS Code database: {}", e))?;

    let secret_key = r#"secret://{"extensionId":"vscode.github-authentication","key":"github.auth"}"#;
    let existing: Option<String> = match conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?",
        [secret_key],
        |row| row.get(0),
    ) {
        Ok(val) => Some(val),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(format!("Failed to query github.auth from database: {}", e)),
    };

    let new_sessions =
        build_github_auth_sessions(existing.as_deref(), &key, username, token, github_user_id)?;

    let sessions_json = serde_json::to_string(&new_sessions)
        .map_err(|e| format!("Failed to serialize sessions: {}", e))?;
    let encrypted = encrypt_v10(&key, sessions_json.as_bytes())?;

    let buffer_json = serde_json::json!({
        "type": "Buffer",
        "data": encrypted
    });
    let buffer_str = serde_json::to_string(&buffer_json)
        .map_err(|e| format!("Failed to serialize Buffer: {}", e))?;

    let tx = conn.unchecked_transaction()
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    tx.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        [secret_key, &buffer_str.as_str()],
    )
    .map_err(|e| format!("Failed to write github.auth: {}", e))?;

    tx.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        ["github.copilot-github", username],
    )
    .map_err(|e| format!("Failed to write github.copilot-github: {}", e))?;

    tx.commit()
        .map_err(|e| format!("Failed to commit transaction: {}", e))?;

    Ok(format!("Successfully injected {} into VS Code", username))
}

#[cfg(target_os = "windows")]
fn get_local_state_path() -> Result<PathBuf, String> {
    let appdata =
        std::env::var("APPDATA").map_err(|_| "Cannot read APPDATA environment variable".to_string())?;
    let path = PathBuf::from(appdata).join("Code").join("Local State");
    if path.exists() {
        Ok(path)
    } else {
        Err(format!("VS Code Local State not found: {}", path.display()))
    }
}

#[cfg(target_os = "windows")]
fn dpapi_decrypt(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };

        CryptUnprotectData(
            &mut input,
            None,
            None,
            None,
            None,
            0,
            &mut output,
        )
        .map_err(|_| "DPAPI CryptUnprotectData call failed".to_string())?;

        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(HLOCAL(output.pbData as *mut _));
        Ok(result)
    }
}

fn build_github_auth_sessions(
    existing_encrypted_value: Option<&str>,
    key: &[u8],
    username: &str,
    token: &str,
    github_user_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut sessions: Vec<serde_json::Value> = if let Some(value) = existing_encrypted_value {
        let buffer: serde_json::Value = serde_json::from_str(value)
            .map_err(|e| format!("Failed to parse existing secret: {}", e))?;
        let data_arr = buffer["data"]
            .as_array()
            .ok_or("Secret data is not in Buffer format")?;
        let mut encrypted_bytes: Vec<u8> = Vec::with_capacity(data_arr.len());
        for (idx, v) in data_arr.iter().enumerate() {
            let n = v
                .as_u64()
                .ok_or_else(|| format!("Secret data element at index {} is not a non-negative integer", idx))?;
            if n > 255 {
                return Err(format!(
                    "Secret data element at index {} is out of range ({} > 255)",
                    idx, n
                ));
            }
            encrypted_bytes.push(n as u8);
        }

        let decrypted = decrypt_v10(key, &encrypted_bytes)?;
        let json_str = String::from_utf8(decrypted)
            .map_err(|e| format!("Decrypted data is not valid UTF-8: {}", e))?;
        serde_json::from_str(&json_str)
            .map_err(|e| format!("Decrypted github.auth is not a valid sessions array: {}", e))?
    } else {
        Vec::new()
    };

    let user_id = github_user_id.unwrap_or("0");
    let new_session = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "scopes": ["user:email"],
        "accessToken": token,
        "account": {
            "label": username,
            "id": user_id
        }
    });

    let mut replaced = false;
    for session in sessions.iter_mut() {
        if let Some(scopes) = session["scopes"].as_array() {
            let has_user_email = scopes.iter().any(|s| s.as_str() == Some("user:email"));
            if has_user_email {
                *session = new_session.clone();
                replaced = true;
                break;
            }
        }
    }
    if !replaced {
        sessions.push(new_session);
    }

    Ok(serde_json::Value::Array(sessions))
}
