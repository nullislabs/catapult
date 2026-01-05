use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Maximum age of a request signature in seconds (5 minutes)
const MAX_SIGNATURE_AGE_SECS: u64 = 300;

/// Sign a request body with the shared secret and timestamp
///
/// Returns (signature, timestamp) tuple
pub fn sign_request(secret: &[u8], body: &[u8]) -> (String, u64) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();

    let signature = compute_signature(secret, body, timestamp);
    (signature, timestamp)
}

/// Verify a request signature with replay protection
///
/// Returns `true` if the signature is valid and not expired
pub fn verify_signature(secret: &[u8], body: &[u8], signature: &str, timestamp: u64) -> bool {
    // Check timestamp is not too old (replay protection)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();

    if now.saturating_sub(timestamp) > MAX_SIGNATURE_AGE_SECS {
        tracing::warn!(
            timestamp,
            now,
            max_age = MAX_SIGNATURE_AGE_SECS,
            "Request signature expired"
        );
        return false;
    }

    // Also reject timestamps in the future (with some tolerance)
    if timestamp > now + 60 {
        tracing::warn!(timestamp, now, "Request timestamp is in the future");
        return false;
    }

    let expected = compute_signature(secret, body, timestamp);
    constant_time_eq(signature.as_bytes(), expected.as_bytes())
}

/// Compute HMAC-SHA256 signature
fn compute_signature(secret: &[u8], body: &[u8], timestamp: u64) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size");

    // Include timestamp in the signed data
    mac.update(&timestamp.to_be_bytes());
    mac.update(body);

    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Constant-time comparison to prevent timing attacks
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// Verify a GitHub webhook signature (X-Hub-Signature-256)
///
/// GitHub signatures do not include timestamps, so no replay protection
pub fn verify_github_signature(secret: &[u8], payload: &[u8], signature: &str) -> bool {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size");
    mac.update(payload);

    let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    constant_time_eq(signature.as_bytes(), expected.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let secret = b"test-secret";
        let body = b"test-body";

        let (signature, timestamp) = sign_request(secret, body);
        assert!(verify_signature(secret, body, &signature, timestamp));
    }

    #[test]
    fn test_invalid_signature() {
        let secret = b"test-secret";
        let body = b"test-body";

        let (_, timestamp) = sign_request(secret, body);
        assert!(!verify_signature(secret, body, "sha256=invalid", timestamp));
    }

    #[test]
    fn test_wrong_secret() {
        let secret = b"test-secret";
        let wrong_secret = b"wrong-secret";
        let body = b"test-body";

        let (signature, timestamp) = sign_request(secret, body);
        assert!(!verify_signature(wrong_secret, body, &signature, timestamp));
    }

    #[test]
    fn test_expired_signature() {
        let secret = b"test-secret";
        let body = b"test-body";

        let (signature, _) = sign_request(secret, body);
        // Use a timestamp from 10 minutes ago
        let old_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 600;

        // Recompute signature with old timestamp
        let old_signature = compute_signature(secret, body, old_timestamp);
        assert!(!verify_signature(
            secret,
            body,
            &old_signature,
            old_timestamp
        ));
    }

    #[test]
    fn test_github_signature() {
        let secret = b"webhook-secret";
        let payload = b"{\"action\":\"opened\"}";

        // Compute expected signature
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(payload);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        assert!(verify_github_signature(secret, payload, &signature));
        assert!(!verify_github_signature(secret, payload, "sha256=wrong"));
    }
}
