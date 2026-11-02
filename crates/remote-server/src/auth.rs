use std::{io, path::Path};

use axum::http::HeaderValue;
use fetchx_storage::{Database, SqliteConfigRepository, StorageError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

use crate::authorization::{AuthenticatedPrincipal, Role};

const AUTH_CONFIG_KEY: &str = "remote.auth.v1";
const AUTHORIZATION_CONFIG_KEY: &str = "remote.authz.v1";
const TOKEN_PREFIX: &str = "fx_";
const TOKEN_BYTES: usize = 32;
const TOKEN_HEX_BYTES: usize = TOKEN_BYTES * 2;
const VERIFIER_BYTES: usize = 32;

pub struct AuthenticationService {
    config: SqliteConfigRepository,
}

impl AuthenticationService {
    pub async fn from_config(config: SqliteConfigRepository) -> Result<Self, AuthError> {
        let service = Self { config };
        service.read_verifier().await?;
        service.read_role().await?;
        Ok(service)
    }

    pub async fn authenticate_bearer(
        &self,
        authorization: Option<&HeaderValue>,
    ) -> Result<AuthenticatedPrincipal, AuthError> {
        self.authenticate_bearer_with_verifier(authorization)
            .await
            .map(|(principal, _)| principal)
    }

    pub(crate) async fn authenticate_bearer_with_verifier(
        &self,
        authorization: Option<&HeaderValue>,
    ) -> Result<(AuthenticatedPrincipal, [u8; VERIFIER_BYTES]), AuthError> {
        let token = authorization
            .and_then(|value| value.to_str().ok())
            .and_then(parse_bearer_token)
            .ok_or(AuthError::Unauthorized)?;
        let verifier = self.read_verifier().await?;
        let candidate: [u8; VERIFIER_BYTES] = Sha256::digest(token.as_bytes()).into();
        if bool::from(candidate.as_slice().ct_eq(verifier.as_slice())) {
            Ok((
                AuthenticatedPrincipal::primary(self.read_role().await?),
                candidate,
            ))
        } else {
            Err(AuthError::Unauthorized)
        }
    }

    pub(crate) async fn authenticate_verifier(
        &self,
        candidate: [u8; VERIFIER_BYTES],
    ) -> Result<AuthenticatedPrincipal, AuthError> {
        let verifier = self.read_verifier().await?;
        if bool::from(candidate.as_slice().ct_eq(verifier.as_slice())) {
            Ok(AuthenticatedPrincipal::primary(self.read_role().await?))
        } else {
            Err(AuthError::Unauthorized)
        }
    }

    pub async fn verify_bearer(
        &self,
        authorization: Option<&HeaderValue>,
    ) -> Result<(), AuthError> {
        self.authenticate_bearer(authorization).await.map(drop)
    }

    async fn read_role(&self) -> Result<Role, AuthError> {
        let Some(value) = self.config.get(AUTHORIZATION_CONFIG_KEY).await? else {
            return Ok(Role::Admin);
        };
        let stored: StoredRole =
            serde_json::from_str(&value).map_err(|_| AuthError::InvalidConfiguration)?;
        if stored.version != 1 {
            return Err(AuthError::InvalidConfiguration);
        }
        Ok(stored.role)
    }

    async fn read_verifier(&self) -> Result<[u8; VERIFIER_BYTES], AuthError> {
        let value = self
            .config
            .get(AUTH_CONFIG_KEY)
            .await?
            .ok_or(AuthError::NotInitialized)?;
        let stored: StoredVerifier =
            serde_json::from_str(&value).map_err(|_| AuthError::InvalidConfiguration)?;
        if stored.version != 1 {
            return Err(AuthError::InvalidConfiguration);
        }
        decode_hex(&stored.verifier).ok_or(AuthError::InvalidConfiguration)
    }
}

pub async fn initialize_auth(database_path: impl AsRef<Path>) -> Result<String, AuthError> {
    let config = open_auth_config(database_path).await?;
    let token = generate_token()?;
    let verifier = encode_config(&token);
    if !config.set_if_absent(AUTH_CONFIG_KEY, &verifier).await? {
        return Err(AuthError::AlreadyInitialized);
    }
    Ok(token)
}

pub async fn rotate_auth(database_path: impl AsRef<Path>) -> Result<String, AuthError> {
    let config = open_auth_config(database_path).await?;
    let token = generate_token()?;
    if !config
        .replace_if_present(AUTH_CONFIG_KEY, &encode_config(&token))
        .await?
    {
        return Err(AuthError::NotInitialized);
    }
    Ok(token)
}

pub async fn current_auth_role(database_path: impl AsRef<Path>) -> Result<Role, AuthError> {
    let config = open_auth_config(database_path).await?;
    AuthenticationService::from_config(config.clone())
        .await?
        .read_role()
        .await
}

pub async fn set_auth_role(database_path: impl AsRef<Path>, role: Role) -> Result<(), AuthError> {
    let config = open_auth_config(database_path).await?;
    AuthenticationService::from_config(config.clone()).await?;
    let value = serde_json::to_string(&StoredRole { version: 1, role })
        .map_err(|_| AuthError::InvalidConfiguration)?;
    config.set(AUTHORIZATION_CONFIG_KEY, &value).await?;
    Ok(())
}

async fn open_auth_config(
    database_path: impl AsRef<Path>,
) -> Result<SqliteConfigRepository, AuthError> {
    if let Some(parent) = database_path
        .as_ref()
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(AuthError::Io)?;
    }
    let database = Database::connect_path(database_path).await?;
    Ok(database.config())
}

fn generate_token() -> Result<String, AuthError> {
    let mut random = [0_u8; TOKEN_BYTES];
    getrandom::fill(&mut random).map_err(AuthError::Random)?;
    let mut token = String::with_capacity(TOKEN_PREFIX.len() + TOKEN_HEX_BYTES);
    token.push_str(TOKEN_PREFIX);
    for byte in random {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(token)
}

fn parse_bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") || token.len() != TOKEN_PREFIX.len() + TOKEN_HEX_BYTES
    {
        return None;
    }
    let (prefix, entropy) = token.split_at(TOKEN_PREFIX.len());
    if prefix != TOKEN_PREFIX || !entropy.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(token)
}

fn encode_config(token: &str) -> String {
    let verifier = Sha256::digest(token.as_bytes());
    format!(
        "{{\"version\":1,\"verifier\":\"{}\"}}",
        encode_hex(&verifier)
    )
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_hex(value: &str) -> Option<[u8; VERIFIER_BYTES]> {
    if value.len() != VERIFIER_BYTES * 2 {
        return None;
    }
    let mut decoded = [0_u8; VERIFIER_BYTES];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(decoded)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredVerifier {
    version: u8,
    verifier: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRole {
    version: u8,
    role: Role,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("authentication is not initialized; run `fetchx server init-auth`")]
    NotInitialized,
    #[error("authentication is already initialized; use `fetchx server rotate-auth`")]
    AlreadyInitialized,
    #[error("authentication configuration is invalid")]
    InvalidConfiguration,
    #[error("authentication failed")]
    Unauthorized,
    #[error("secure token generation failed")]
    Random(#[source] getrandom::Error),
    #[error("could not prepare authentication storage: {0}")]
    Io(#[source] io::Error),
    #[error("authentication storage failed: {0}")]
    Storage(#[from] StorageError),
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use tempfile::tempdir;

    use super::{
        AUTH_CONFIG_KEY, AuthError, AuthenticationService, current_auth_role, initialize_auth,
        rotate_auth, set_auth_role,
    };
    use crate::authorization::Role;
    use fetchx_storage::Database;

    #[tokio::test]
    async fn generated_token_is_well_formed_unique_and_verifier_only_is_stored()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let database_path = temp.path().join("fetchx.db");
        let token = initialize_auth(&database_path).await?;
        let next = rotate_auth(&database_path).await?;
        assert!(token.starts_with("fx_"));
        assert_eq!(token.len(), 67);
        assert_ne!(token, next);

        let database = Database::connect_path(&database_path).await?;
        let config_value = database
            .config()
            .get(AUTH_CONFIG_KEY)
            .await?
            .ok_or("missing auth verifier")?;
        assert!(!config_value.contains(&token));
        assert!(!config_value.contains(&next));
        assert!(config_value.contains("verifier"));
        assert!(matches!(
            initialize_auth(&database_path).await,
            Err(AuthError::AlreadyInitialized)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn bearer_validation_and_rotation_fail_closed() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempdir()?;
        let database_path = temp.path().join("fetchx.db");
        let token = initialize_auth(&database_path).await?;
        let database = Database::connect_path(&database_path).await?;
        let auth = AuthenticationService::from_config(database.config()).await?;

        assert!(matches!(
            auth.verify_bearer(None).await,
            Err(AuthError::Unauthorized)
        ));
        assert!(matches!(
            auth.verify_bearer(Some(&HeaderValue::from_static("Bearer invalid")))
                .await,
            Err(AuthError::Unauthorized)
        ));
        let header = HeaderValue::from_str(&format!("Bearer {token}"))?;
        auth.verify_bearer(Some(&header)).await?;

        let rotated = rotate_auth(&database_path).await?;
        assert!(matches!(
            auth.verify_bearer(Some(&header)).await,
            Err(AuthError::Unauthorized)
        ));
        let rotated_header = HeaderValue::from_str(&format!("Bearer {rotated}"))?;
        auth.verify_bearer(Some(&rotated_header)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn phase_21c_credentials_default_to_admin_and_roles_persist()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let database_path = temp.path().join("fetchx.db");
        let token = initialize_auth(&database_path).await?;
        assert_eq!(current_auth_role(&database_path).await?, Role::Admin);

        set_auth_role(&database_path, Role::ReadOnly).await?;
        assert_eq!(current_auth_role(&database_path).await?, Role::ReadOnly);

        let database = Database::connect_path(&database_path).await?;
        let auth = AuthenticationService::from_config(database.config()).await?;
        let header = HeaderValue::from_str(&format!("Bearer {token}"))?;
        let principal = auth.authenticate_bearer(Some(&header)).await?;
        assert_eq!(principal.role(), Role::ReadOnly);
        Ok(())
    }
}
