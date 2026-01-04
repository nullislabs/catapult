use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::central::db;
use crate::central::server::AppState;

/// Request to create/update an authorized org
#[derive(Debug, Deserialize)]
pub struct UpsertAuthRequest {
    pub github_org: String,
    pub zones: Vec<String>,
    pub domain_patterns: Vec<String>,
}

/// Request to delete an authorized org
#[derive(Debug, Deserialize)]
pub struct DeleteAuthRequest {
    pub github_org: String,
}

/// Response for authorized org
#[derive(Debug, Serialize)]
pub struct AuthorizedOrgResponse {
    pub id: i32,
    pub github_org: String,
    pub zones: Vec<String>,
    pub domain_patterns: Vec<String>,
    pub enabled: bool,
}

impl From<db::AuthorizedOrg> for AuthorizedOrgResponse {
    fn from(org: db::AuthorizedOrg) -> Self {
        Self {
            id: org.id,
            github_org: org.github_org,
            zones: org.zones,
            domain_patterns: org.domain_patterns,
            enabled: org.enabled,
        }
    }
}

/// Verify admin API key from Authorization header
fn verify_admin_key(headers: &HeaderMap, expected_key: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            // Support both "Bearer <key>" and raw "<key>"
            let key = v.strip_prefix("Bearer ").unwrap_or(v);
            key == expected_key
        })
        .unwrap_or(false)
}

/// List all authorized organizations
pub async fn list_authorized_orgs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !verify_admin_key(&headers, &state.config.admin_api_key) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "Invalid or missing API key"}))).into_response();
    }

    match db::list_authorized_orgs(&state.db).await {
        Ok(orgs) => {
            let response: Vec<AuthorizedOrgResponse> = orgs.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to list authorized orgs");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Database error"}))).into_response()
        }
    }
}

/// Create or update an authorized organization
pub async fn upsert_authorized_org(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpsertAuthRequest>,
) -> impl IntoResponse {
    if !verify_admin_key(&headers, &state.config.admin_api_key) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "Invalid or missing API key"}))).into_response();
    }

    // Validate request
    if request.github_org.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "github_org is required"}))).into_response();
    }
    if request.zones.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "At least one zone is required"}))).into_response();
    }
    if request.domain_patterns.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "At least one domain pattern is required"}))).into_response();
    }

    match db::upsert_authorized_org(&state.db, &request.github_org, &request.zones, &request.domain_patterns).await {
        Ok(org) => {
            tracing::info!(
                github_org = %org.github_org,
                zones = ?org.zones,
                domain_patterns = ?org.domain_patterns,
                "Authorized org created/updated"
            );
            let response: AuthorizedOrgResponse = org.into();
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to upsert authorized org");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Database error"}))).into_response()
        }
    }
}

/// Delete (disable) an authorized organization
pub async fn delete_authorized_org(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<DeleteAuthRequest>,
) -> impl IntoResponse {
    if !verify_admin_key(&headers, &state.config.admin_api_key) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "Invalid or missing API key"}))).into_response();
    }

    match db::delete_authorized_org(&state.db, &request.github_org).await {
        Ok(deleted) => {
            if deleted {
                tracing::info!(github_org = %request.github_org, "Authorized org deleted");
                (StatusCode::OK, Json(serde_json::json!({"deleted": true}))).into_response()
            } else {
                (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Organization not found"}))).into_response()
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to delete authorized org");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Database error"}))).into_response()
        }
    }
}
