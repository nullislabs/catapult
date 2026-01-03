use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::central::db;
use crate::central::server::AppState;
use crate::shared::auth::verify_signature;

/// Heartbeat request from worker
#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    /// The zone/environment this worker serves
    pub zone: String,
}

/// Heartbeat response to worker
#[derive(Debug, Serialize)]
pub struct HeartbeatResponse {
    /// Whether the heartbeat was acknowledged
    pub ok: bool,
    /// Human-readable message
    pub message: String,
}

/// Handle heartbeat from workers
///
/// Updates the `last_seen` timestamp for the worker in the database.
/// Workers can call this periodically to indicate they're alive.
pub async fn handle_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Extract signature and timestamp headers
    let signature = match headers.get("x-worker-signature") {
        Some(sig) => sig.to_str().unwrap_or_default(),
        None => {
            tracing::warn!("Missing X-Worker-Signature header");
            return (
                StatusCode::UNAUTHORIZED,
                Json(HeartbeatResponse {
                    ok: false,
                    message: "Missing signature".to_string(),
                }),
            );
        }
    };

    let timestamp: u64 = match headers.get("x-request-timestamp") {
        Some(ts) => ts.to_str().unwrap_or("0").parse().unwrap_or(0),
        None => {
            tracing::warn!("Missing X-Request-Timestamp header");
            return (
                StatusCode::UNAUTHORIZED,
                Json(HeartbeatResponse {
                    ok: false,
                    message: "Missing timestamp".to_string(),
                }),
            );
        }
    };

    // Verify signature
    if !verify_signature(
        state.config.worker_shared_secret.as_bytes(),
        &body,
        signature,
        timestamp,
    ) {
        tracing::warn!("Invalid worker signature for heartbeat");
        return (
            StatusCode::UNAUTHORIZED,
            Json(HeartbeatResponse {
                ok: false,
                message: "Invalid signature".to_string(),
            }),
        );
    }

    // Parse heartbeat request
    let request: HeartbeatRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => {
            tracing::error!(error = %e, "Failed to parse heartbeat request");
            return (
                StatusCode::BAD_REQUEST,
                Json(HeartbeatResponse {
                    ok: false,
                    message: format!("Invalid request: {}", e),
                }),
            );
        }
    };

    // Update worker last_seen
    match db::update_worker_heartbeat(&state.db, &request.zone).await {
        Ok(updated) => {
            if updated {
                tracing::debug!(zone = %request.zone, "Worker heartbeat received");
                (
                    StatusCode::OK,
                    Json(HeartbeatResponse {
                        ok: true,
                        message: "Heartbeat acknowledged".to_string(),
                    }),
                )
            } else {
                tracing::warn!(zone = %request.zone, "Unknown or disabled worker");
                (
                    StatusCode::NOT_FOUND,
                    Json(HeartbeatResponse {
                        ok: false,
                        message: format!("Unknown zone: {}", request.zone),
                    }),
                )
            }
        }
        Err(e) => {
            tracing::error!(error = %e, zone = %request.zone, "Failed to update heartbeat");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(HeartbeatResponse {
                    ok: false,
                    message: "Internal error".to_string(),
                }),
            )
        }
    }
}
