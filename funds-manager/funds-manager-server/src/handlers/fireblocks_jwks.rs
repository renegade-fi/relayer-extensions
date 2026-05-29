//! Verification for Fireblocks Webhooks v2 signatures.
//!
//! v2 signs each webhook with a **detached JWS** (RS512 = RSA PKCS#1 v1.5 over
//! SHA-512) in the `Fireblocks-Webhook-Signature` header. The compact form is
//! `<protected>..<signature>` — the payload segment is empty because the body
//! is sent separately. The signing key is selected by the `kid` in the JWS
//! protected header and fetched from Fireblocks' JWKS endpoint.
//!
//! (Legacy `Fireblocks-Signature` webhooks — verified in `fireblocks_webhook`
//! — are deprecated 2026-06-15; v2 is the path forward.)

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
use serde::Deserialize;
use sha2::{Digest, Sha512};
use tokio::sync::Mutex;

use crate::error::ApiError;

/// US-production JWKS endpoint for Fireblocks webhook signing keys. (EU:
/// `eu-keys` / `eu2-keys`; sandbox: `sandbox-keys`. renegade uses US prod.)
const FIREBLOCKS_JWKS_URL: &str = "https://keys.fireblocks.io/.well-known/jwks.json";

/// JWS `alg` Fireblocks v2 uses. Pinned to reject alg-confusion attempts.
const EXPECTED_ALG: &str = "RS512";

/// Re-fetch the JWKS at most this often (Fireblocks sets Cache-Control
/// max-age=3600). An unknown `kid` forces an immediate refetch regardless, to
/// pick up a freshly-rotated key.
const JWKS_TTL: Duration = Duration::from_secs(3600);

/// A single RSA key from the Fireblocks JWKS.
#[derive(Deserialize)]
struct Jwk {
    /// Key ID, matched against the JWS protected-header `kid`.
    kid: String,
    /// RSA modulus, base64url (no pad), big-endian.
    n: String,
    /// RSA public exponent, base64url (no pad), big-endian.
    e: String,
}

/// JWKS document shape.
#[derive(Deserialize)]
struct Jwks {
    /// The set of keys.
    keys: Vec<Jwk>,
}

/// Cached `kid` → RSA public key map with a fetch timestamp.
struct JwksCache {
    /// Parsed keys by `kid`, plus when they were fetched. `None` until first
    /// fetch.
    inner: Mutex<Option<(Instant, HashMap<String, RsaPublicKey>)>>,
}

impl JwksCache {
    /// Resolve a `kid` to its RSA public key, fetching/refreshing the JWKS as
    /// needed: served from cache while fresh, refetched on TTL expiry or on a
    /// cache miss (so a rotated key is picked up immediately).
    async fn key_for(&self, kid: &str) -> Result<RsaPublicKey, ApiError> {
        {
            let guard = self.inner.lock().await;
            if let Some((fetched, keys)) = guard.as_ref() {
                if fetched.elapsed() < JWKS_TTL {
                    if let Some(key) = keys.get(kid) {
                        return Ok(key.clone());
                    }
                }
            }
        }

        let keys = fetch_jwks().await?;
        let resolved = keys.get(kid).cloned();
        *self.inner.lock().await = Some((Instant::now(), keys));
        resolved.ok_or_else(|| {
            ApiError::Unauthenticated(format!("no Fireblocks JWKS key for kid {kid}"))
        })
    }
}

/// Fetch and parse the Fireblocks JWKS into a `kid` → RSA key map.
/// Network/parse failures are `InternalError` (→ 500) so Fireblocks retries the
/// delivery rather than treating it as permanently rejected.
async fn fetch_jwks() -> Result<HashMap<String, RsaPublicKey>, ApiError> {
    let response = reqwest::get(FIREBLOCKS_JWKS_URL)
        .await
        .map_err(|e| ApiError::InternalError(format!("JWKS fetch failed: {e}")))?;
    let jwks: Jwks = response
        .json()
        .await
        .map_err(|e| ApiError::InternalError(format!("JWKS parse failed: {e}")))?;

    let mut keys = HashMap::new();
    for jwk in jwks.keys {
        let n = decode_b64url_uint(&jwk.n)?;
        let e = decode_b64url_uint(&jwk.e)?;
        let key = RsaPublicKey::new(n, e)
            .map_err(|err| ApiError::InternalError(format!("invalid JWKS RSA key: {err}")))?;
        keys.insert(jwk.kid, key);
    }
    Ok(keys)
}

/// Decode a base64url (no-pad) big-endian integer into a `BigUint`.
fn decode_b64url_uint(s: &str) -> Result<BigUint, ApiError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| ApiError::InternalError(format!("invalid JWK base64url: {e}")))?;
    Ok(BigUint::from_bytes_be(&bytes))
}

/// Process-wide JWKS cache.
static JWKS_CACHE: OnceLock<Arc<JwksCache>> = OnceLock::new();

/// Handle to the process-wide JWKS cache.
fn jwks_cache() -> Arc<JwksCache> {
    JWKS_CACHE.get_or_init(|| Arc::new(JwksCache { inner: Mutex::new(None) })).clone()
}

/// Verify a detached JWS against a known key: reconstruct the signing input
/// `<protected>.<base64url(body)>` and RS512-verify the signature over it.
fn verify_detached_jws(
    key: &RsaPublicKey,
    protected_b64: &str,
    signature_b64: &str,
    body: &[u8],
) -> Result<(), ApiError> {
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|e| ApiError::Unauthenticated(format!("invalid JWS signature b64: {e}")))?;
    let signing_input = format!("{protected_b64}.{}", URL_SAFE_NO_PAD.encode(body));
    key.verify(Pkcs1v15Sign::new::<Sha512>(), &Sha512::digest(signing_input.as_bytes()), &signature)
        .map_err(|e| ApiError::Unauthenticated(format!("bad v2 webhook signature: {e}")))
}

/// Verify a Fireblocks Webhooks v2 `Fireblocks-Webhook-Signature` over the raw
/// body: parse the detached JWS, pin `alg = RS512`, look up the key by `kid` in
/// the cached JWKS, and verify.
pub(crate) async fn verify_fireblocks_v2_signature(
    signature_header: &str,
    body: &[u8],
) -> Result<(), ApiError> {
    // Detached JWS compact form: "<protected>..<signature>".
    let mut parts = signature_header.split('.');
    let protected_b64 = parts.next().unwrap_or_default();
    let payload_segment = parts.next().unwrap_or_default();
    let signature_b64 = parts.next().unwrap_or_default();
    if protected_b64.is_empty()
        || !payload_segment.is_empty()
        || signature_b64.is_empty()
        || parts.next().is_some()
    {
        return Err(ApiError::Unauthenticated("malformed detached JWS".to_string()));
    }

    // Parse the protected header for `alg` and `kid`.
    let header_bytes = URL_SAFE_NO_PAD
        .decode(protected_b64)
        .map_err(|e| ApiError::Unauthenticated(format!("invalid JWS header b64: {e}")))?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes)
        .map_err(|e| ApiError::Unauthenticated(format!("invalid JWS header json: {e}")))?;
    if header.get("alg").and_then(|v| v.as_str()) != Some(EXPECTED_ALG) {
        return Err(ApiError::Unauthenticated(format!("JWS alg is not {EXPECTED_ALG}")));
    }
    let kid = header
        .get("kid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::Unauthenticated("JWS header missing kid".to_string()))?;

    let key = jwks_cache().key_for(kid).await?;
    verify_detached_jws(&key, protected_b64, signature_b64, body)
}

#[cfg(test)]
#[allow(clippy::missing_docs_in_private_items)]
mod tests {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use rsa::{Pkcs1v15Sign, RsaPrivateKey};
    use sha2::{Digest, Sha512};

    use super::verify_detached_jws;

    /// Build a detached-JWS signature for `body` with `key`, returning the
    /// (protected_b64, signature_b64) pair the verifier consumes.
    fn sign_detached(key: &RsaPrivateKey, body: &[u8]) -> (String, String) {
        let protected_b64 = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS512","kid":"test"}"#);
        let signing_input = format!("{protected_b64}.{}", URL_SAFE_NO_PAD.encode(body));
        let sig = key
            .sign(Pkcs1v15Sign::new::<Sha512>(), &Sha512::digest(signing_input.as_bytes()))
            .expect("sign");
        (protected_b64, URL_SAFE_NO_PAD.encode(sig))
    }

    #[test]
    fn detached_jws_roundtrip_and_tamper() {
        let mut rng = rand::thread_rng();
        let key = RsaPrivateKey::new(&mut rng, 2048).expect("keygen");
        let pubkey = key.to_public_key();

        let body =
            br#"{"eventType":"transaction.status.updated","data":{"id":"x","status":"COMPLETED"}}"#;
        let (protected_b64, sig_b64) = sign_detached(&key, body);

        // Valid signature over the exact body verifies.
        assert!(verify_detached_jws(&pubkey, &protected_b64, &sig_b64, body).is_ok());
        // Tampered body fails.
        let tampered =
            br#"{"eventType":"transaction.status.updated","data":{"id":"x","status":"FAILED"}}"#;
        assert!(verify_detached_jws(&pubkey, &protected_b64, &sig_b64, tampered).is_err());
        // Garbage signature fails.
        assert!(verify_detached_jws(&pubkey, &protected_b64, "not-b64!!", body).is_err());
    }
}
