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
use crate::handlers::fireblocks_jwks::verify_fireblocks_v2_signature;
use crate::log_task;
use crate::logger::{Outcome, Task};
use crate::metrics::labels::{
    FIREBLOCKS_WEBHOOK_INFLIGHT_METRIC_NAME, FIREBLOCKS_WEBHOOK_PROCESS_LATENCY_MS_METRIC_NAME,
};

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
/// ACKs `200` IMMEDIATELY, then verifies the signature and dispatches the
/// payload to the [`global_tx_listeners`] registry on a spawned task.
///
/// Verifying and dispatching inline would put webhook ACK latency on the shared
/// warp runtime's critical path: under load (a swap/withdraw sweep, or the
/// signing `/rpc` path awaiting `poll_fireblocks_transaction`) ACKs slow down,
/// Fireblocks hits its delivery timeout, and RETRIES pile on — amplifying the
/// very overload that delayed them (the ~2.25x webhook storm observed
/// 2026-05-28 / 2026-06-01). Acking first makes ACK latency independent of that
/// backlog and breaks the retry feedback loop.
///
/// We always ACK `200`, even for an unverified or non-transaction payload: we
/// never act on a payload before verifying it in the task, and any non-2xx
/// would itself trigger a Fireblocks retry. No DB writes, no chain reads.
pub(crate) async fn fireblocks_tx_status_webhook_handler(
    v2_signature: Option<String>,
    legacy_signature: Option<String>,
    body: Bytes,
) -> Result<impl warp::Reply, warp::Rejection> {
    tokio::spawn(verify_and_dispatch_webhook(v2_signature, legacy_signature, body));
    Ok(warp::reply::with_status("ok", warp::http::StatusCode::OK))
}

/// Verify a webhook's signature and, if valid, dispatch its transaction payload
/// to any waiter. Runs off the request path (see the handler doc); errors are
/// logged and dropped, never propagated, since the caller has already ACKed.
async fn verify_and_dispatch_webhook(
    v2_signature: Option<String>,
    legacy_signature: Option<String>,
    body: Bytes,
) {
    // Track in-flight count + total processing latency across every exit path.
    let _inflight = WebhookInflightGuard::new();

    // Webhooks v2 (current; legacy deprecates 2026-06-15) signs with
    // `Fireblocks-Webhook-Signature` (detached JWS, RS512); legacy uses
    // `Fireblocks-Signature` (RSA-SHA512). Prefer v2, fall back to legacy.
    let verification = if let Some(sig) = v2_signature.as_deref() {
        verify_fireblocks_v2_signature(sig, &body).await
    } else if let Some(sig) = legacy_signature.as_deref() {
        verify_fireblocks_signature(&body, sig)
    } else {
        Err(ApiError::Unauthenticated("missing Fireblocks webhook signature header".to_string()))
    };
    if let Err(e) = verification {
        log_task!(
            Task::FireblocksWebhook,
            Outcome::Failed,
            error = %e,
            "dropping webhook: signature verification failed (check Fireblocks key rotation / scheme)"
        );
        return;
    }

    let envelope: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(e) => {
            log_task!(
                Task::FireblocksWebhook,
                Outcome::Failed,
                error = %e,
                "dropping webhook: invalid JSON body"
            );
            return;
        },
    };
    // Legacy uses `type`; v2 uses `eventType`.
    let event_type = envelope
        .get("type")
        .or_else(|| envelope.get("eventType"))
        .and_then(|v| v.as_str())
        .unwrap_or("<none>")
        .to_string();

    // The transaction sits under `data`. A payload whose `data` isn't a
    // transaction (other Fireblocks event types) or that no one is awaiting is
    // logged and dropped.
    match classify_webhook_data(&envelope) {
        WebhookData::Transaction(tx) => {
            let tx_id = tx.id.clone();
            let tx_status = format!("{:?}", tx.status);
            let delivered = global_tx_listeners().dispatch(*tx);
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
        WebhookData::Ignored => {
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
}

/// Result of [`classify_webhook_data`].
enum WebhookData {
    /// A transaction-status event with its parsed transaction.
    Transaction(Box<TransactionResponse>),
    /// Not a dispatchable transaction (other event type, or unparseable data).
    Ignored,
}

/// Classify a webhook envelope's `data` field: a parsed transaction (to
/// dispatch) or ignored. Pure — unit-tested.
fn classify_webhook_data(envelope: &serde_json::Value) -> WebhookData {
    match envelope.get("data").cloned().map(serde_json::from_value::<TransactionResponse>) {
        Some(Ok(tx)) => WebhookData::Transaction(Box::new(tx)),
        _ => WebhookData::Ignored,
    }
}

/// RAII guard for webhook observability: increments the in-flight gauge on
/// creation and, on drop, decrements it and records total processing latency.
/// As a guard it covers every exit path of `verify_and_dispatch_webhook`
/// (verification failure, bad JSON, dispatch) without per-branch bookkeeping.
struct WebhookInflightGuard {
    /// When processing started, used to record latency on drop.
    start: std::time::Instant,
}

impl WebhookInflightGuard {
    /// Mark a webhook as in-flight and start its latency timer.
    fn new() -> Self {
        metrics::gauge!(FIREBLOCKS_WEBHOOK_INFLIGHT_METRIC_NAME).increment(1.0);
        Self { start: std::time::Instant::now() }
    }
}

impl Drop for WebhookInflightGuard {
    fn drop(&mut self) {
        metrics::gauge!(FIREBLOCKS_WEBHOOK_INFLIGHT_METRIC_NAME).decrement(1.0);
        metrics::histogram!(FIREBLOCKS_WEBHOOK_PROCESS_LATENCY_MS_METRIC_NAME)
            .record(self.start.elapsed().as_secs_f64() * 1000.0);
    }
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
    fn classify_ignores_missing_and_non_transaction_data() {
        use super::{classify_webhook_data, WebhookData};

        // No `data` field at all → Ignored.
        let no_data = serde_json::json!({"eventType": "PING"});
        assert!(matches!(classify_webhook_data(&no_data), WebhookData::Ignored));

        // `data` present but not a transaction object → Ignored, not a panic.
        let non_tx = serde_json::json!({"type": "X", "data": "not-a-transaction"});
        assert!(matches!(classify_webhook_data(&non_tx), WebhookData::Ignored));
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
