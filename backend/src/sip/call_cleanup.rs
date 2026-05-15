//! Cleanup of stale `sip_calls` rows whose dialog never received a final
//! BYE/CANCEL — typically caused by client crashes, network failures, or
//! a backend restart that wiped the in-memory `ActiveDialogs` map.
//!
//! These rows would otherwise linger as "active" forever and inflate the
//! Dashboard "active calls" KPI.
//!
//! Marks matching rows as `status = 'ended'` with `ended_at = NOW()`.
//!
//! - `older_than_hours = None` → close every row that still has `ended_at IS NULL`
//!   (intended for use at process startup, where in-memory dialog state is
//!   guaranteed to be empty).
//! - `older_than_hours = Some(h)` → only close rows whose `started_at` is
//!   older than `h` hours (used by the periodic background task and the
//!   admin API to avoid racing in-flight calls).

use sqlx::MySqlPool;

/// SQL template for closing stale active calls.
/// Uses a NULL-tolerant predicate so a single statement covers both the
/// "close everything" and "close only rows older than N hours" cases.
pub const STALE_CALL_CLEANUP_SQL: &str = "UPDATE sip_calls \
     SET status = 'ended', ended_at = NOW() \
     WHERE ended_at IS NULL \
       AND status IN ('trying', 'answered') \
       AND (? IS NULL OR started_at < DATE_SUB(NOW(), INTERVAL ? HOUR))";

/// Mark stale active-call rows as ended. Returns the number of rows updated.
pub async fn mark_stale_calls_ended(
    pool: &MySqlPool,
    older_than_hours: Option<i64>,
) -> sqlx::Result<u64> {
    let result = sqlx::query(STALE_CALL_CLEANUP_SQL)
        .bind(older_than_hours)
        .bind(older_than_hours.unwrap_or(0))
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
