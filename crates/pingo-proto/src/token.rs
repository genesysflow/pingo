//! Media tokens: `voice.join` (an RxFerrix mutation) mints one, the SFU verifies it on `/rtc`.
//!
//! Format: `base64url(json claims) + "." + base64url(HMAC-SHA256(secret, first part))`. The secret is the
//! `PINGO_MEDIA_SECRET` environment variable, shared by the server and every SFU node.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaClaims {
    /// RxFerrix `Id<Room>` as a string.
    pub room: String,
    /// RxFerrix `Id<User>` as a string.
    pub user: String,
    /// Display name and avatar color, so the SFU can describe participants without a DB round-trip.
    pub name: String,
    pub color: String,
    pub can_speak: bool,
    pub can_video: bool,
    pub can_share: bool,
    /// Expiry, Unix milliseconds. Tokens are short-lived (60 s to connect).
    pub exp: f64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TokenError {
    #[error("malformed token")]
    Malformed,
    #[error("bad signature")]
    BadSignature,
    #[error("token expired")]
    Expired,
}

fn mac(secret: &[u8]) -> Hmac<Sha256> {
    Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length")
}

pub fn sign(claims: &MediaClaims, secret: &[u8]) -> String {
    let body = B64.encode(serde_json::to_vec(claims).expect("claims serialize"));
    let mut m = mac(secret);
    m.update(body.as_bytes());
    let sig = B64.encode(m.finalize().into_bytes());
    format!("{body}.{sig}")
}

/// Verifies the signature and expiry. `now_ms` is Unix milliseconds.
pub fn verify(token: &str, secret: &[u8], now_ms: f64) -> Result<MediaClaims, TokenError> {
    let (body, sig) = token.split_once('.').ok_or(TokenError::Malformed)?;
    let sig = B64.decode(sig).map_err(|_| TokenError::Malformed)?;
    let mut m = mac(secret);
    m.update(body.as_bytes());
    m.verify_slice(&sig).map_err(|_| TokenError::BadSignature)?;
    let json = B64.decode(body).map_err(|_| TokenError::Malformed)?;
    let claims: MediaClaims = serde_json::from_slice(&json).map_err(|_| TokenError::Malformed)?;
    if claims.exp < now_ms {
        return Err(TokenError::Expired);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(exp: f64) -> MediaClaims {
        MediaClaims {
            room: "r1".into(),
            user: "u1".into(),
            name: "Ann".into(),
            color: "#f97316".into(),
            can_speak: true,
            can_video: true,
            can_share: false,
            exp,
        }
    }

    #[test]
    fn round_trip() {
        let t = sign(&claims(2000.0), b"secret");
        assert_eq!(verify(&t, b"secret", 1000.0), Ok(claims(2000.0)));
    }

    #[test]
    fn rejects_wrong_secret_expiry_and_garbage() {
        let t = sign(&claims(2000.0), b"secret");
        assert_eq!(verify(&t, b"other", 1000.0), Err(TokenError::BadSignature));
        assert_eq!(verify(&t, b"secret", 3000.0), Err(TokenError::Expired));
        assert_eq!(verify("nope", b"secret", 0.0), Err(TokenError::Malformed));
    }
}
