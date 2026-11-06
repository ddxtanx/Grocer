use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, SaltString},
    Argon2, PasswordHasher, PasswordVerifier,
};
use axum::{
    extract::{FromRequestParts, State},
    http::{header, request::Parts, HeaderMap, HeaderValue, StatusCode},
    Json,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::{ApiResult, AppError};
use crate::AppState;

pub fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Deserialize)]
pub struct Cred {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct Authed {
    pub user_id: i64,
    pub username: String,
}

pub async fn register(
    State(st): State<AppState>,
    Json(c): Json<Cred>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    if c.username.len() < 3 || !c.username.chars().all(|ch| ch.is_alphanumeric()) {
        return Err(AppError::BadRequest(
            "username must be 3+ alphanumeric chars".into(),
        ));
    }
    if c.password.len() < 8 {
        return Err(AppError::BadRequest("password must be 8+ chars".into()));
    }
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(c.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .to_string();
    let row = sqlx::query_as::<_, (i64,)>(
        "INSERT INTO users (username, pass_hash) VALUES ($1, $2) ON CONFLICT (username) DO NOTHING RETURNING id",
    )
    .bind(&c.username)
    .bind(&hash)
    .fetch_optional(&st.pool)
    .await?;
    let id = row.ok_or_else(|| AppError::BadRequest("username taken".into()))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id.0 }))))
}

fn session_cookie(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "session={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000"
    ))
    .unwrap()
}

pub async fn login(
    State(st): State<AppState>,
    Json(c): Json<Cred>,
) -> ApiResult<(StatusCode, HeaderMap, Json<serde_json::Value>)> {
    let row = sqlx::query_as::<_, (i64, String)>("SELECT id, pass_hash FROM users WHERE username=$1")
        .bind(&c.username)
        .fetch_optional(&st.pool)
        .await?;
    let (uid, hash) = row.ok_or_else(|| AppError::Unauthorized("bad credentials".into()))?;
    let parsed = PasswordHash::new(&hash)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Argon2::default()
        .verify_password(c.password.as_bytes(), &parsed)
        .map_err(|_| AppError::Unauthorized("bad credentials".into()))?;

    // random 32-byte token; only its SHA-256 is stored
    let mut bytes = [0u8; 32];
    argon2::password_hash::rand_core::RngCore::fill_bytes(&mut OsRng, &mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let expires = Utc::now() + Duration::days(30);
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1,$2,$3)")
        .bind(uid)
        .bind(sha256_hex(&token))
        .bind(expires)
        .execute(&st.pool)
        .await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, session_cookie(&token));
    Ok((StatusCode::OK, headers, Json(json!({ "id": uid }))))
}

pub async fn logout(
    State(st): State<AppState>,
    parts: Parts,
) -> ApiResult<(HeaderMap, Json<serde_json::Value>)> {
    if let Some(token) = session_token(&parts.headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
            .bind(sha256_hex(token))
            .execute(&st.pool)
            .await?;
    }
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, session_cookie(""));
    Ok((headers, Json(json!({ "ok": true }))))
}

pub async fn me(authed: Authed) -> Json<serde_json::Value> {
    Json(json!({ "id": authed.user_id, "username": authed.username }))
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie
        .split(';')
        .map(|kv| kv.trim())
        .find_map(|kv| kv.strip_prefix("session="))
        .filter(|t| !t.is_empty())
}

impl FromRequestParts<AppState> for Authed {
    type Rejection = AppError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = session_token(&parts.headers)
            .ok_or_else(|| AppError::Unauthorized("no session cookie".into()))?;
        let row = sqlx::query_as::<_, (i64, String)>(
            "SELECT u.id, u.username FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash=$1 AND s.expires_at > now()",
        )
        .bind(sha256_hex(token))
        .fetch_optional(&state.pool)
        .await?;
        let (user_id, username) =
            row.ok_or_else(|| AppError::Unauthorized("session expired".into()))?;
        Ok(Authed { user_id, username })
    }
}
