//! # Data access — user SQL, and the Postgres session store
//!
//! Two things live here because both are "talk to Postgres": the three queries
//! the `users` table needs, and the four methods `tower-sessions` calls to keep
//! its own records.
//!
//! ## Why the session store is hand-written
//!
//! The official `tower-sessions-sqlx-store` crate exists, but it depends on
//! sqlx 0.8 while this app is on sqlx 0.9. Two sqlx versions in one binary
//! means two different, incompatible `PgPool` types, and our pool cannot be
//! handed to it. Four short methods are cheaper than that.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;

use axum_login::tower_sessions::{
    ExpiredDeletion, SessionStore,
    session::{Id, Record},
    session_store::{Error as SessionError, Result as SessionResult},
};

use super::{AUTH_DATA_KEY, User};

/// Postgres's `unique_violation`. Thrown when the `users_email_key` index stops
/// a second account appearing on an address that already has one.
const UNIQUE_VIOLATION: &str = "23505";

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some(UNIQUE_VIOLATION))
}

/// What a one-time token is for.
///
/// Stored as a column so that a verification link cannot be pasted into the
/// reset form. The database checks the value against a CHECK constraint, so a
/// typo here fails loudly rather than storing a token nothing can ever consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Verify,
    Reset,
}

impl Purpose {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
        }
    }
}

/// The account a session belongs to, read out of the session payload.
///
/// `tower-sessions` hands us an opaque `Record`; the owner is inside the JSON
/// the auth layer wrote, under `AUTH_DATA_KEY`. Copying it into a real column
/// on the way in is what makes "log out everywhere" a single indexed DELETE
/// rather than a JSON expression buried in a WHERE clause.
///
/// Returns `None` for a session with nobody logged in, which is normal.
fn owner_of(record: &Record) -> Option<i64> {
    let data = record.data.get(AUTH_DATA_KEY)?.as_object()?;
    data.get("user_id")?.as_i64()
}

/// The row as it comes back from Postgres, still carrying the Argon2 hash.
///
/// [`User`] deliberately does not keep the hash, so the login path works with
/// this instead and only converts once the password has been verified.
#[derive(sqlx::FromRow)]
pub(super) struct UserRow {
    pub(super) id: i64,
    pub(super) email: String,
    pub(super) password_hash: String,
    pub(super) email_verified_at: Option<OffsetDateTime>,
}

impl UserRow {
    pub(super) fn into_user(self) -> User {
        User::new(
            self.id,
            self.email,
            self.email_verified_at,
            &self.password_hash,
        )
    }
}

/// Creates an account. Returns `None` when the email is already registered —
/// which is a form error the visitor can fix, not a server fault, so it never
/// becomes a 500.
pub async fn create_user(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
) -> Result<Option<User>, sqlx::Error> {
    let inserted = sqlx::query_as::<_, UserRow>(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id, email, \
         password_hash, email_verified_at",
    )
    .bind(email)
    .bind(password_hash)
    .fetch_optional(pool)
    .await;

    match inserted {
        Ok(row) => Ok(row.map(UserRow::into_user)),
        Err(err) if is_unique_violation(&err) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Finds an account by email, case-insensitively, matching the unique index.
pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<UserRow>, sqlx::Error> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, email, password_hash, email_verified_at FROM users
         WHERE lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Finds an account by id. This is the lookup behind every authenticated
/// request, so it goes by primary key.
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<UserRow>, sqlx::Error> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, email, password_hash, email_verified_at FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Finds an account by id and returns the `User`.
///
/// `find_by_id` is the raw row, because login needs the password hash before it
/// can decide anything. This is the version for everything that has already
/// established *who* is asking and only wants the profile.
pub async fn find_user(pool: &PgPool, id: i64) -> Result<Option<User>, sqlx::Error> {
    Ok(find_by_id(pool, id).await?.map(UserRow::into_user))
}

/// The id of an account, if one exists on that email.
///
/// Deliberately returns only the id: it exists so tests and the password-reset
/// flow can ask "is there an account here?" without carrying a password hash
/// around.
pub async fn user_id_by_email(pool: &PgPool, email: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
        .bind(email)
        .fetch_optional(pool)
        .await
}

/// Replaces an account's password hash.
///
/// The hash is what `session_auth_hash` is derived from, so changing it here
/// invalidates every session that was created from the old one — which is how
/// "log every other device out" works without touching the sessions table.
///
/// Returns false when no account has that id.
pub async fn set_password(pool: &PgPool, user_id: i64, hash: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(hash)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Records that the address was confirmed.
pub async fn mark_email_verified(pool: &PgPool, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET email_verified_at = now() WHERE id = $1 AND email_verified_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// When this address was last sent a token, for the resend cooldown. `None`
/// when it has never been sent one.
///
/// Returns the newest of any purpose, so the cooldown applies to the whole
/// mailbox rather than per flow — otherwise a visitor could ask for a reset and
/// then immediately a verification to bypass the wait.
pub async fn last_token_sent_at(
    pool: &PgPool,
    user_id: i64,
) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    sqlx::query_scalar("SELECT max(created_at) FROM email_tokens WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

// --- one-time tokens ---------------------------------------------------------

/// The SHA-256 of a raw token, stored instead of the token.
///
/// A raw token is a password: anyone holding it can take over the account. A
/// hash of it is useless to them, and verifying it is a single comparison.
pub fn hash_token(raw: &str) -> String {
    hex(&Sha256::digest(raw.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Stores a token and returns nothing — the caller already has the raw value
/// and must hand it to the user, so there is nothing useful to return.
///
/// Any earlier unused token for the same purpose is cleared first, so only one
/// link per flow is ever live.
pub async fn put_token(
    pool: &PgPool,
    user_id: i64,
    purpose: Purpose,
    token_hash: &str,
    expires_at: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM email_tokens WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL")
        .bind(user_id)
        .bind(purpose.as_str())
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO email_tokens (user_id, purpose, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(purpose.as_str())
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// Consumes a token, returning the account it belonged to.
///
/// The `used_at = now()` happens **inside the same UPDATE** that matches the
/// row, so two requests arriving together cannot both succeed: the second finds
/// nothing because the row is no longer unused. Doing the check and the write as
/// two separate queries would leave a window between them.
///
/// `None` covers every rejection — unknown, expired, already used, or issued for
/// the other purpose. The caller tells the visitor the link is not valid, which
/// does not tell them which of those it was.
pub async fn take_token(
    pool: &PgPool,
    raw: &str,
    purpose: Purpose,
) -> Result<Option<i64>, sqlx::Error> {
    let claimed = sqlx::query_scalar::<_, i64>(
        "UPDATE email_tokens SET used_at = now()
         WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(hash_token(raw))
    .bind(purpose.as_str())
    .fetch_optional(pool)
    .await?;

    Ok(claimed)
}

/// Deletes every token belonging to an account. Used after a successful reset,
/// so an old link cannot be tried afterwards.
pub async fn delete_tokens(pool: &PgPool, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM email_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

// --- the tower-sessions store ------------------------------------------------

/// Keeps session records in the `sessions` table.
#[derive(Clone, Debug)]
pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// A plain INSERT, used by `create` so that a colliding id fails loudly
    /// instead of overwriting somebody else's session.
    async fn insert_new(&self, record: &Record) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO sessions (id, data, expiry_date, user_id) VALUES ($1, $2::jsonb, $3, $4)",
        )
        .bind(record.id.0.to_string())
        // Serialising is not a database problem, so it cannot be a
        // `sqlx::Error` — but the caller only speaks sqlx, so it is folded
        // into a protocol error rather than widening every signature.
        .bind(encode(record).map_err(|err| sqlx::Error::Protocol(err.to_string()))?)
        .bind(record.expiry_date)
        .bind(owner_of(record))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for PostgresStore {
    async fn create(&self, record: &mut Record) -> SessionResult<()> {
        // Session ids are 128 bits of randomness, so a collision should not
        // happen. If one does, ask for a fresh id and try again — never let a
        // second visitor inherit the first visitor's session.
        for _ in 0..3 {
            match self.insert_new(record).await {
                Ok(()) => return Ok(()),
                Err(err) if is_unique_violation(&err) => record.id = Id::default(),
                Err(err) => return Err(backend(err)),
            }
        }
        Err(SessionError::Backend(
            "no free session id after 3 attempts".into(),
        ))
    }

    async fn save(&self, record: &Record) -> SessionResult<()> {
        sqlx::query(
            "INSERT INTO sessions (id, data, expiry_date, user_id) VALUES ($1, $2::jsonb, $3, $4)
             ON CONFLICT (id) DO UPDATE
                 SET data = excluded.data,
                     expiry_date = excluded.expiry_date,
                     user_id = excluded.user_id",
        )
        .bind(record.id.0.to_string())
        .bind(encode(record)?)
        .bind(record.expiry_date)
        .bind(owner_of(record))
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> SessionResult<Option<Record>> {
        // Expiry is decided by Postgres, so an expired row is never handed back
        // and never has to be parsed.
        let row: Option<(String,)> =
            sqlx::query_as("SELECT data::text FROM sessions WHERE id = $1 AND expiry_date > now()")
                .bind(session_id.0.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(backend)?;

        // Asked for `::text` explicitly: jsonb's own decoding rules are not
        // what serde_json needs, and this column only ever holds our own JSON.
        row.map(|(json,)| decode(&json)).transpose()
    }

    async fn delete(&self, session_id: &Id) -> SessionResult<()> {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(session_id.0.to_string())
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        Ok(())
    }
}

#[async_trait]
impl ExpiredDeletion for PostgresStore {
    async fn delete_expired(&self) -> SessionResult<()> {
        sqlx::query("DELETE FROM sessions WHERE expiry_date <= now()")
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        Ok(())
    }
}

/// Throws away every session belonging to an account, returning how many went.
///
/// This is the "log out everywhere" and "my password changed" primitive. It
/// cannot reach an account it is not given, because `user_id` comes from the
/// caller's own session rather than from anything in the request.
///
/// The `ON DELETE CASCADE` on `sessions.user_id` means a deleted account takes
/// its sessions with it, so this is only needed for a live account.
pub async fn delete_sessions_for_user(pool: &PgPool, user_id: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// How many live sessions an account has. Shown on the account page so the
/// visitor can see that "log out everywhere" did something.
pub async fn count_sessions_for_user(pool: &PgPool, user_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = $1 AND expiry_date > now()")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

/// The session record is a library type, so it has to be turned into text on
/// the way in and back into the type on the way out.
fn encode(record: &Record) -> Result<String, SessionError> {
    serde_json::to_string(record).map_err(|err| SessionError::Encode(err.to_string()))
}

fn decode(json: &str) -> Result<Record, SessionError> {
    serde_json::from_str(json).map_err(|err| SessionError::Decode(err.to_string()))
}

/// sqlx's error type does not implement the session store's error type, so the
/// detail is carried across as text. It is still logged by whatever surfaces
/// the failure.
fn backend(err: sqlx::Error) -> SessionError {
    SessionError::Backend(err.to_string())
}
