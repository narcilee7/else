//! Connector 凭据的私有存储、Scope 校验与 OAuth refresh。
//!
//! macOS 默认写入系统 Keychain；其他平台或显式 `ELSE_CREDENTIAL_BACKEND=file`
//! 时使用 Runtime 目录下 0600 的私有凭据文件。凭据本体永远不进入 Ledger。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use else_protocol::connector::ConnectorCredential;
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("credential not found: {0}")]
    NotFound(String),
    #[error("credential storage: {0}")]
    Storage(String),
    #[error("credential scope: {0}")]
    Scope(String),
    #[error("credential expired and cannot be refreshed: {0}")]
    Expired(String),
    #[error("credential refresh: {0}")]
    Refresh(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SecretError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialBackend {
    Keychain,
    PrivateFile,
}

pub struct CredentialStore {
    backend: CredentialBackend,
    directory: PathBuf,
    service: String,
}

impl CredentialStore {
    /// 生产默认：macOS Keychain；没有系统 Keychain 时失败关闭，不静默降级。
    pub fn open(runtime_dir: &Path) -> Result<Self> {
        let backend = match std::env::var("ELSE_CREDENTIAL_BACKEND").as_deref() {
            Ok("file") => CredentialBackend::PrivateFile,
            Ok("keychain") if cfg!(target_os = "macos") => CredentialBackend::Keychain,
            Ok("keychain") => {
                return Err(SecretError::Storage(
                    "Keychain backend is only available on macOS".into(),
                ))
            }
            Ok(other) => {
                return Err(SecretError::Storage(format!(
                    "unknown ELSE_CREDENTIAL_BACKEND={other}"
                )))
            }
            _ if cfg!(target_os = "macos") => CredentialBackend::Keychain,
            _ => CredentialBackend::PrivateFile,
        };
        Self::with_backend(runtime_dir, backend)
    }

    pub fn with_backend(runtime_dir: &Path, backend: CredentialBackend) -> Result<Self> {
        let directory = runtime_dir.join("credentials");
        if backend == CredentialBackend::PrivateFile {
            fs::create_dir_all(&directory)?;
            restrict_directory(&directory)?;
        }
        let canonical = runtime_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime_dir.to_path_buf());
        Ok(Self {
            backend,
            directory,
            service: format!(
                "else.connector.credentials.{:016x}",
                stable_hash(&canonical.to_string_lossy())
            ),
        })
    }

    pub fn backend(&self) -> CredentialBackend {
        self.backend
    }

    pub fn put(&self, reference: &str, credential: &ConnectorCredential) -> Result<()> {
        validate_reference(reference)?;
        let serialized = serde_json::to_vec(credential)?;
        match self.backend {
            CredentialBackend::PrivateFile => {
                let path = self.file_path(reference);
                write_private_file(&path, &serialized)?;
                Ok(())
            }
            CredentialBackend::Keychain => self.put_keychain(reference, &serialized),
        }
    }

    pub fn get(&self, reference: &str) -> Result<ConnectorCredential> {
        validate_reference(reference)?;
        let serialized = match self.backend {
            CredentialBackend::PrivateFile => fs::read(self.file_path(reference))
                .map_err(|error| map_file_read_error(reference, error))?,
            CredentialBackend::Keychain => self.get_keychain(reference)?,
        };
        serde_json::from_slice(&serialized).map_err(SecretError::from)
    }

    pub fn delete(&self, reference: &str) -> Result<()> {
        validate_reference(reference)?;
        match self.backend {
            CredentialBackend::PrivateFile => match fs::remove_file(self.file_path(reference)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            },
            CredentialBackend::Keychain => self.delete_keychain(reference),
        }
    }

    /// 在交给 Adapter 前校验 Scope，并在临近过期时刷新。
    pub fn resolve(
        &self,
        reference: &str,
        required_scopes: &[String],
    ) -> Result<ConnectorCredential> {
        let mut credential = self.get(reference)?;
        credential
            .validate_scopes(required_scopes)
            .map_err(SecretError::Scope)?;
        if credential
            .expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now() + Duration::seconds(60))
        {
            credential = refresh_credential(credential)?;
            self.put(reference, &credential)?;
        }
        Ok(credential)
    }

    fn file_path(&self, reference: &str) -> PathBuf {
        self.directory.join(format!("{reference}.json"))
    }

    #[cfg(target_os = "macos")]
    fn put_keychain(&self, reference: &str, serialized: &[u8]) -> Result<()> {
        security_framework::passwords::set_generic_password(&self.service, reference, serialized)
            .map_err(|error| SecretError::Storage(error.to_string()))
    }

    #[cfg(not(target_os = "macos"))]
    fn put_keychain(&self, _reference: &str, _serialized: &[u8]) -> Result<()> {
        Err(SecretError::Storage(
            "Keychain backend is unavailable on this platform".into(),
        ))
    }

    #[cfg(target_os = "macos")]
    fn get_keychain(&self, reference: &str) -> Result<Vec<u8>> {
        security_framework::passwords::get_generic_password(&self.service, reference)
            .map_err(|error| SecretError::Storage(error.to_string()))
    }

    #[cfg(not(target_os = "macos"))]
    fn get_keychain(&self, _reference: &str) -> Result<Vec<u8>> {
        Err(SecretError::Storage(
            "Keychain backend is unavailable on this platform".into(),
        ))
    }

    #[cfg(target_os = "macos")]
    fn delete_keychain(&self, reference: &str) -> Result<()> {
        match security_framework::passwords::delete_generic_password(&self.service, reference) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(error) => Err(SecretError::Storage(error.to_string())),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn delete_keychain(&self, _reference: &str) -> Result<()> {
        Err(SecretError::Storage(
            "Keychain backend is unavailable on this platform".into(),
        ))
    }
}

fn refresh_credential(mut credential: ConnectorCredential) -> Result<ConnectorCredential> {
    let config = credential
        .refresh
        .clone()
        .ok_or_else(|| SecretError::Expired("no OAuth refresh configuration".into()))?;
    if !config.token_url.starts_with("https://")
        && !config.token_url.starts_with("http://127.0.0.1:")
        && !config.token_url.starts_with("http://localhost:")
        && !config.token_url.starts_with("http://[::1]:")
    {
        return Err(SecretError::Refresh(
            "OAuth token_url must use HTTPS (loopback HTTP is allowed for tests)".into(),
        ));
    }
    let refresh_token = credential
        .refresh_token
        .clone()
        .ok_or_else(|| SecretError::Expired("OAuth credential has no refresh token".into()))?;
    let mut body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        form_encode(&refresh_token),
        form_encode(&config.client_id)
    );
    if let Some(client_secret) = &config.client_secret {
        body.push_str(&format!("&client_secret={}", form_encode(client_secret)));
    }
    if !credential.scopes.is_empty() {
        body.push_str(&format!(
            "&scope={}",
            form_encode(&credential.scopes.join(" "))
        ));
    }
    let response = ureq::post(&config.token_url)
        .set("Accept", "application/json")
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&body)
        .map_err(|error| SecretError::Refresh(error.to_string()))?;
    let payload: Value = serde_json::from_str(
        &response
            .into_string()
            .map_err(|error| SecretError::Refresh(error.to_string()))?,
    )?;
    let access_token = payload
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| SecretError::Refresh("refresh response has no access_token".into()))?;
    credential.access_token = access_token.into();
    credential.refresh_token = payload
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(credential.refresh_token);
    credential.token_type = payload
        .get("token_type")
        .and_then(Value::as_str)
        .unwrap_or("bearer")
        .into();
    credential.expires_at = payload
        .get("expires_in")
        .and_then(Value::as_i64)
        .map(|seconds| Utc::now() + Duration::seconds(seconds.max(1)));
    let returned_scopes = payload.get("scope").and_then(Value::as_str).map(|scopes| {
        scopes
            .split_ascii_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    if let Some(scopes) = returned_scopes {
        credential.scopes = scopes;
    }
    Ok(credential)
}

pub fn credential_metadata_json(credential: &ConnectorCredential) -> Value {
    json!({
        "scopes": credential.scopes,
        "expires_at": credential.expires_at,
        "has_refresh_token": credential.refresh_token.is_some(),
    })
}

fn validate_reference(reference: &str) -> Result<()> {
    if reference.is_empty()
        || reference.len() > 128
        || !reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(SecretError::Storage(
            "credential reference must be 1-128 ASCII alphanumeric, '-' or '_'".into(),
        ));
    }
    Ok(())
}

fn map_file_read_error(reference: &str, error: std::io::Error) -> SecretError {
    if error.kind() == std::io::ErrorKind::NotFound {
        SecretError::NotFound(reference.into())
    } else {
        SecretError::Io(error)
    }
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    restrict_file(path)
}

fn form_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn credential(expires_at: Option<DateTime<Utc>>) -> ConnectorCredential {
        ConnectorCredential {
            access_token: "very-secret-token".into(),
            refresh_token: Some("refresh-secret".into()),
            token_type: "bearer".into(),
            scopes: vec!["issues:read".into()],
            expires_at,
            refresh: None,
        }
    }

    #[test]
    fn private_file_store_roundtrips_and_restricts_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            CredentialStore::with_backend(dir.path(), CredentialBackend::PrivateFile).unwrap();
        store.put("credential-test", &credential(None)).unwrap();
        let path = dir.path().join("credentials/credential-test.json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            store.get("credential-test").unwrap().access_token,
            "very-secret-token"
        );
        store.delete("credential-test").unwrap();
        assert!(matches!(
            store.get("credential-test"),
            Err(SecretError::NotFound(_))
        ));
    }

    #[test]
    fn scope_validation_fails_closed() {
        let credential = credential(None);
        assert!(credential.validate_scopes(&["issues:read".into()]).is_ok());
        assert!(credential
            .validate_scopes(&["issues:write".into()])
            .is_err());
    }

    #[test]
    fn debug_output_never_contains_token_material() {
        let debug = format!("{:?}", credential(None));
        assert!(!debug.contains("very-secret-token"));
        assert!(!debug.contains("refresh-secret"));
    }

    #[test]
    fn expired_oauth_credential_refreshes_and_persists_new_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("POST /token HTTP/1.1"));
            assert!(request.contains("grant_type=refresh_token"));
            assert!(request.contains("refresh_token=refresh-secret"));
            let body = r#"{"access_token":"fresh-token","refresh_token":"fresh-refresh","token_type":"bearer","expires_in":3600,"scope":"issues:read"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let dir = tempfile::tempdir().unwrap();
        let store =
            CredentialStore::with_backend(dir.path(), CredentialBackend::PrivateFile).unwrap();
        let mut expiring = credential(Some(Utc::now() - chrono::Duration::seconds(1)));
        expiring.refresh = Some(else_protocol::connector::OAuthRefreshConfig {
            token_url: format!("http://{address}/token"),
            client_id: "else-test".into(),
            client_secret: None,
        });
        store.put("oauth-test", &expiring).unwrap();

        let resolved = store
            .resolve("oauth-test", &["issues:read".into()])
            .unwrap();
        assert_eq!(resolved.access_token, "fresh-token");
        assert_eq!(resolved.refresh_token.as_deref(), Some("fresh-refresh"));
        assert!(resolved.expires_at.unwrap() > Utc::now());
        assert_eq!(store.get("oauth-test").unwrap().access_token, "fresh-token");
        server.join().unwrap();
    }

    #[test]
    fn refresh_rejects_plaintext_non_loopback_token_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            CredentialStore::with_backend(dir.path(), CredentialBackend::PrivateFile).unwrap();
        let mut expiring = credential(Some(Utc::now() - chrono::Duration::seconds(1)));
        expiring.refresh = Some(else_protocol::connector::OAuthRefreshConfig {
            token_url: "http://example.com/token".into(),
            client_id: "else-test".into(),
            client_secret: None,
        });
        store.put("insecure-refresh", &expiring).unwrap();
        assert!(matches!(
            store.resolve("insecure-refresh", &["issues:read".into()]),
            Err(SecretError::Refresh(_))
        ));
    }
}
