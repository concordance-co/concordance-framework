use chrono::{Duration, Utc};
use jwt::RegisteredClaims;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use jwt::{Header, SignWithKey, Token, VerifyWithKey};
use sha2::Sha256;

/// Manages JWT token creation and validation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JwtManager {
    /// The secret key used for signing and verifying JWTs
    secret_key: String,
}

impl JwtManager {
    /// Creates a new JWT manager with the given secret key
    pub fn new(secret_key: String) -> Self {
        Self { secret_key }
    }

    /// Sign a claims object and return a JWT token
    pub fn create_token(&self, claims: ConcordanceClaims) -> Result<String, jwt::Error> {
        let key: Hmac<Sha256> = Hmac::new_from_slice(self.secret_key.as_bytes())
            .map_err(|_| jwt::Error::InvalidSignature)?;

        claims.sign_with_key(&key)
    }

    /// Verify a JWT token and return the claims if valid
    pub fn verify_token(&self, token: &str) -> Result<ConcordanceClaims, jwt::Error> {
        let key: Hmac<Sha256> = Hmac::new_from_slice(self.secret_key.as_bytes())
            .map_err(|_| jwt::Error::InvalidSignature)?;

        token.verify_with_key(&key)
    }

    /// Create a token with header and claims
    pub fn create_token_with_header(
        &self,
        header: Header,
        claims: ConcordanceClaims,
    ) -> Result<String, jwt::Error> {
        let key: Hmac<Sha256> = Hmac::new_from_slice(self.secret_key.as_bytes())
            .map_err(|_| jwt::Error::InvalidSignature)?;

        let token = Token::new(header, claims).sign_with_key(&key)?;
        Ok(token.as_str().to_string())
    }

    /// Verify a token and return both header and claims
    pub fn verify_token_with_header(
        &self,
        token: &str,
    ) -> Result<Token<Header, ConcordanceClaims, jwt::Verified>, jwt::Error> {
        let key: Hmac<Sha256> = Hmac::new_from_slice(self.secret_key.as_bytes())
            .map_err(|_| jwt::Error::InvalidSignature)?;

        token.verify_with_key(&key)
    }
}

/// Custom claims for our JWT tokens
#[derive(Debug, Serialize, Deserialize)]
pub struct ConcordanceClaims {
    /// Standard registered claims (exp, iat, etc.)
    #[serde(flatten)]
    pub registered: RegisteredClaims,
    /// User ID
    pub user_id: String,
    /// Additional custom claims
    #[serde(flatten)]
    pub custom: BTreeMap<String, serde_json::Value>,
}

impl ConcordanceClaims {
    /// Create a new claims object for a user
    pub fn new(user_id: &str, expiration_hours: i64) -> Self {
        let now = Utc::now();
        let expiration = now + Duration::hours(expiration_hours);

        let registered = RegisteredClaims {
            issuer: Some("concordance".to_string()),
            subject: Some(user_id.to_string()),
            audience: Some("concordance-api".to_string()),
            expiration: Some(expiration.timestamp() as u64),
            not_before: Some(now.timestamp() as u64),
            issued_at: Some(now.timestamp() as u64),
            json_web_token_id: Some(uuid::Uuid::new_v4().to_string()),
        };

        Self {
            registered,
            user_id: user_id.to_string(),
            custom: BTreeMap::new(),
        }
    }

    /// Add a custom claim
    pub fn with_custom_claim<T: Serialize>(
        mut self,
        key: &str,
        value: T,
    ) -> Result<Self, serde_json::Error> {
        let value = serde_json::to_value(value)?;
        self.custom.insert(key.to_string(), value);
        Ok(self)
    }
}
