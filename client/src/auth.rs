use hex::FromHexError;
use jsonwebtoken::{encode, get_current_timestamp, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

/// Default algorithm used for JWT token signing.
const DEFAULT_ALGORITHM: Algorithm = Algorithm::HS256;

/// JWT secret length in bytes.
pub const JWT_SECRET_LENGTH: usize = 32;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to encode JWT: {0}")]
    JwtEncode(jsonwebtoken::errors::Error),
    #[error("Invalid hex string: {0}")]
    InvalidKey(FromHexError),
    #[error("Invalid JWT: {0}")]
    InvalidJwt(String),
}

/// Provides wrapper around `[u8; JWT_SECRET_LENGTH]` that implements `Zeroize`.
#[derive(Zeroize, Clone)]
#[zeroize(drop)]
pub struct JwtKey([u8; JWT_SECRET_LENGTH]);

impl JwtKey {
    /// Wrap given slice in `Self`. Returns an error if slice.len() != `JWT_SECRET_LENGTH`.
    pub fn from_slice(key: &[u8]) -> Result<Self, String> {
        if key.len() != JWT_SECRET_LENGTH {
            return Err(format!(
                "Invalid key length. Expected {} got {}",
                JWT_SECRET_LENGTH,
                key.len()
            ));
        }
        let mut res = [0; JWT_SECRET_LENGTH];
        res.copy_from_slice(key);
        Ok(Self(res))
    }

    /// Returns a reference to the underlying byte array.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Contains the JWT secret and claims parameters.
#[derive(Clone)]
pub struct Auth {
    key: EncodingKey,
    id: Option<String>,
    clv: Option<String>,
}

impl Auth {
    pub fn new(secret: JwtKey, id: Option<String>, clv: Option<String>) -> Self {
        Self {
            key: EncodingKey::from_secret(secret.as_bytes()),
            id,
            clv,
        }
    }

    /// Generate a JWT token with `claims.iat` set to current time.
    pub fn generate_token(&self) -> Result<String, Error> {
        let claims = self.generate_claims_at_timestamp();
        self.generate_token_with_claims(&claims)
    }

    /// Generate a JWT token with the given claims.
    fn generate_token_with_claims(&self, claims: &Claims) -> Result<String, Error> {
        let header = Header::new(DEFAULT_ALGORITHM);
        encode(&header, claims, &self.key).map_err(Error::JwtEncode)
    }

    /// Generate a `Claims` struct with `iat` set to current time
    fn generate_claims_at_timestamp(&self) -> Claims {
        Claims {
            iat: get_current_timestamp(),
            id: self.id.clone(),
            clv: self.clv.clone(),
        }
    }
}

/// Claims struct as defined in https://github.com/ethereum/execution-apis/blob/main/src/engine/authentication.md#jwt-claims
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Claims {
    /// issued-at claim. Represented as seconds passed since UNIX_EPOCH.
    iat: u64,
    /// Optional unique identifier for the CL node.
    id: Option<String>,
    /// Optional client version for the CL node.
    clv: Option<String>,
}
