//! Handler for inbound Fireblocks `transaction-status` webhooks.
//!
//! Phase 1 (this module) only verifies the request signature and logs the
//! `(transaction id, status)` it carries; it does not yet resolve any waiting
//! caller. The listener registry and the `poll_fireblocks_transaction`
//! migration land in later phases (see the
//! `2026-05-28-fireblocks-tx-status-webhooks` ticket).
//!
//! Signature scheme (legacy `Fireblocks-Signature` header):
//! `Base64( RSA-PKCS1v1.5-Sign( privkey, SHA512(raw_body) ) )`. We verify with
//! Fireblocks' published RSA public key. The signature is computed over the
//! raw request bytes, so the route must capture the body with
//! `warp::body::bytes()` — any JSON re-serialization changes whitespace/key
//! order and invalidates the signature.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bytes::Bytes;
use fireblocks_sdk::models::TransactionResponse;
use rsa::{pkcs8::DecodePublicKey, Pkcs1v15Sign, RsaPublicKey};
use sha2::{Digest, Sha512};

use crate::custody_client::tx_webhook::global_tx_listeners;
use crate::error::ApiError;
use crate::log_task;
use crate::logger::{Outcome, Task};

/// Fireblocks' production webhook signing public key (covers both US mainnet
/// and testnet workspaces). The renegade funds-manager talks to the Fireblocks
/// production API in every environment, so this is the only key we verify
/// against. Published at
/// <https://developers.fireblocks.com/reference/validating-webhooks>.
const FIREBLOCKS_PROD_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----
MIICIjANBgkqhkiG9w0BAQEFAAOCAg8AMIICCgKCAgEA0+6wd9OJQpK60ZI7qnZG
jjQ0wNFUHfRv85Tdyek8+ahlg1Ph8uhwl4N6DZw5LwLXhNjzAbQ8LGPxt36RUZl5
YlxTru0jZNKx5lslR+H4i936A4pKBjgiMmSkVwXD9HcfKHTp70GQ812+J0Fvti/v
4nrrUpc011Wo4F6omt1QcYsi4GTI5OsEbeKQ24BtUd6Z1Nm/EP7PfPxeb4CP8KOH
clM8K7OwBUfWrip8Ptljjz9BNOZUF94iyjJ/BIzGJjyCntho64ehpUYP8UJykLVd
CGcu7sVYWnknf1ZGLuqqZQt4qt7cUUhFGielssZP9N9x7wzaAIFcT3yQ+ELDu1SZ
dE4lZsf2uMyfj58V8GDOLLE233+LRsRbJ083x+e2mW5BdAGtGgQBusFfnmv5Bxqd
HgS55hsna5725/44tvxll261TgQvjGrTxwe7e5Ia3d2Syc+e89mXQaI/+cZnylNP
SwCCvx8mOM847T0XkVRX3ZrwXtHIA25uKsPJzUtksDnAowB91j7RJkjXxJcz3Vh1
4k182UFOTPRW9jzdWNSyWQGl/vpe9oQ4c2Ly15+/toBo4YXJeDdDnZ5c/O+KKadc
IMPBpnPrH/0O97uMPuED+nI6ISGOTMLZo35xJ96gPBwyG5s2QxIkKPXIrhgcgUnk
tSM7QYNhlftT4/yVvYnk0YcCAwEAAQ==
-----END PUBLIC KEY-----";

/// Verify a Fireblocks webhook signature.
///
/// `signature_b64` is the base64 value of the `Fireblocks-Signature` header.
/// `body` is the raw request body bytes (no re-serialization). Returns `Ok(())`
/// when the signature is valid for the pinned production key, otherwise an
/// `ApiError::Unauthenticated` describing why verification failed.
fn verify_fireblocks_signature(body: &[u8], signature_b64: &str) -> Result<(), ApiError> {
    let public_key = RsaPublicKey::from_public_key_pem(FIREBLOCKS_PROD_PUBLIC_KEY)
        .map_err(|e| ApiError::Unauthenticated(format!("pinned Fireblocks key invalid: {e}")))?;
    let signature = BASE64_STANDARD
        .decode(signature_b64)
        .map_err(|e| ApiError::Unauthenticated(format!("signature not valid base64: {e}")))?;
    let hashed = Sha512::digest(body);
    public_key
        .verify(Pkcs1v15Sign::new::<Sha512>(), &hashed, &signature)
        .map_err(|e| ApiError::Unauthenticated(format!("bad webhook signature: {e}")))
}

/// Pull `(transaction id, status)` from a Fireblocks webhook payload for
/// logging. Fireblocks nests the transaction under `data`; we fall back to the
/// top level for resilience to payload-shape changes. Missing fields render as
/// `"<none>"` rather than failing — Phase 1 is observe-only.
fn extract_id_and_status(payload: &serde_json::Value) -> (&str, &str) {
    let pick = |field: &str| -> &str {
        payload
            .get("data")
            .and_then(|d| d.get(field))
            .or_else(|| payload.get(field))
            .and_then(|v| v.as_str())
            .unwrap_or("<none>")
    };
    (pick("id"), pick("status"))
}

/// Handler for `POST /webhooks/fireblocks/transaction-status`.
///
/// Verifies the signature, deserializes the transaction payload, and dispatches
/// it to any waiter in the process-wide [`global_tx_listeners`] registry (so a
/// blocked `poll_fireblocks_transaction` resolves immediately). Always acks with
/// 200; non-transaction events or payloads no one is awaiting are logged and
/// dropped. Unsigned or bad-signature requests are rejected with 401 before the
/// body is parsed. No DB writes, no chain reads.
pub(crate) async fn fireblocks_tx_status_webhook_handler(
    signature: Option<String>,
    body: Bytes,
) -> Result<impl warp::Reply, warp::Rejection> {
    let signature = signature.ok_or_else(|| {
        log_task!(
            Task::FireblocksWebhook,
            Outcome::Failed,
            "rejecting webhook: missing Fireblocks-Signature header"
        );
        warp::reject::custom(ApiError::Unauthenticated(
            "missing Fireblocks-Signature header".to_string(),
        ))
    })?;

    if let Err(e) = verify_fireblocks_signature(&body, &signature) {
        log_task!(
            Task::FireblocksWebhook,
            Outcome::Failed,
            error = %e,
            "rejecting webhook: signature verification failed (check for Fireblocks public key rotation)"
        );
        return Err(warp::reject::custom(e));
    }

    let envelope: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
        warp::reject::custom(ApiError::BadRequest(format!("invalid webhook JSON: {e}")))
    })?;
    let event_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("<none>");

    // The transaction sits under `data`. Deserialize it into the SDK type and
    // hand it to any waiter in the listener registry. A payload whose `data`
    // isn't a transaction (other Fireblocks event types) or that no one is
    // awaiting is acked and dropped — never rejected, so Fireblocks won't retry.
    match envelope.get("data").cloned().map(serde_json::from_value::<TransactionResponse>) {
        Some(Ok(tx)) => {
            let tx_id = tx.id.clone();
            let tx_status = format!("{:?}", tx.status);
            let delivered = global_tx_listeners().dispatch(tx);
            log_task!(
                Task::FireblocksWebhook,
                Outcome::Ok,
                event_type = event_type,
                tx_id = %tx_id,
                tx_status = %tx_status,
                delivered = delivered,
                "received fireblocks tx webhook"
            );
        },
        _ => {
            let (tx_id, tx_status) = extract_id_and_status(&envelope);
            log_task!(
                Task::FireblocksWebhook,
                Outcome::Skipped,
                event_type = event_type,
                tx_id = tx_id,
                tx_status = tx_status,
                "fireblocks webhook ignored (non-transaction or unparseable data)"
            );
        },
    }

    Ok(warp::reply::with_status("ok", warp::http::StatusCode::OK))
}

#[cfg(test)]
#[allow(clippy::missing_docs_in_private_items)]
mod tests {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use rsa::pkcs8::{DecodePublicKey, EncodePublicKey, LineEnding};
    use rsa::{Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
    use sha2::{Digest, Sha512};

    use super::{verify_fireblocks_signature, FIREBLOCKS_PROD_PUBLIC_KEY};

    /// Sign a body the way Fireblocks does: SHA-512 then RSA PKCS1v1.5, base64.
    fn sign(key: &RsaPrivateKey, body: &[u8]) -> String {
        let hashed = Sha512::digest(body);
        let sig = key.sign(Pkcs1v15Sign::new::<Sha512>(), &hashed).expect("sign");
        BASE64_STANDARD.encode(sig)
    }

    /// Same verification as the handler, but against a caller-provided key so
    /// tests can use a locally-generated keypair instead of the pinned one.
    fn verify_with(pem: &str, body: &[u8], sig_b64: &str) -> bool {
        let key = RsaPublicKey::from_public_key_pem(pem).expect("parse pem");
        let Ok(sig) = BASE64_STANDARD.decode(sig_b64) else { return false };
        key.verify(Pkcs1v15Sign::new::<Sha512>(), &Sha512::digest(body), &sig).is_ok()
    }

    #[test]
    fn pinned_prod_key_parses() {
        RsaPublicKey::from_public_key_pem(FIREBLOCKS_PROD_PUBLIC_KEY)
            .expect("pinned Fireblocks production key must parse as an RSA public key");
    }

    #[test]
    fn missing_signature_is_rejected() {
        // An empty/garbage base64 signature must not verify.
        assert!(verify_fireblocks_signature(b"{}", "not-base64!!").is_err());
    }

    #[test]
    fn good_signature_verifies_and_tampering_fails() {
        let mut rng = rand::thread_rng();
        let key = RsaPrivateKey::new(&mut rng, 2048).expect("keygen");
        let pem = key.to_public_key().to_public_key_pem(LineEnding::LF).expect("pem");

        let body =
            br#"{"type":"TRANSACTION_STATUS_UPDATED","data":{"id":"abc","status":"COMPLETED"}}"#;
        let sig = sign(&key, body);

        // Valid signature over the exact bytes verifies.
        assert!(verify_with(&pem, body, &sig));

        // Tampered body fails.
        let tampered =
            br#"{"type":"TRANSACTION_STATUS_UPDATED","data":{"id":"abc","status":"FAILED"}}"#;
        assert!(!verify_with(&pem, tampered, &sig));

        // Tampered signature fails.
        let mut bad = BASE64_STANDARD.decode(&sig).unwrap();
        bad[0] ^= 0xff;
        assert!(!verify_with(&pem, body, &BASE64_STANDARD.encode(bad)));

        // A signature from a different key fails against the pinned prod key.
        assert!(verify_fireblocks_signature(body, &sig).is_err());
    }

    #[test]
    fn extract_id_and_status_reads_nested_and_root() {
        let nested = serde_json::json!({"data":{"id":"i1","status":"COMPLETED"}});
        assert_eq!(super::extract_id_and_status(&nested), ("i1", "COMPLETED"));

        let root = serde_json::json!({"id":"i2","status":"PENDING"});
        assert_eq!(super::extract_id_and_status(&root), ("i2", "PENDING"));

        let empty = serde_json::json!({});
        assert_eq!(super::extract_id_and_status(&empty), ("<none>", "<none>"));
    }
}
