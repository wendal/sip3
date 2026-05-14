use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use serde_json::{json, Value};

use super::AppState;

#[derive(Debug, Serialize, sqlx::FromRow)]
struct UserCount {
    pub user: String,
    pub count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct PeriodStats {
    pub total: i64,
    pub answered: i64,
    pub duration_secs: i64,
}

pub async fn get_stats(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let err = |e: sqlx::Error| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());

    // Today's aggregate (use range instead of DATE() so an index on started_at can be used)
    let today: PeriodStats = sqlx::query_as(
        "SELECT COUNT(*) AS total,
                COALESCE(SUM(CASE WHEN answered_at IS NOT NULL THEN 1 ELSE 0 END), 0) AS answered,
                COALESCE(SUM(TIMESTAMPDIFF(SECOND, answered_at, ended_at)), 0) AS duration_secs
         FROM sip_calls
         WHERE started_at >= CURDATE() AND started_at < CURDATE() + INTERVAL 1 DAY",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(err)?;

    // Last 7 days aggregate
    let week: PeriodStats = sqlx::query_as(
        "SELECT COUNT(*) AS total,
                COALESCE(SUM(CASE WHEN answered_at IS NOT NULL THEN 1 ELSE 0 END), 0) AS answered,
                COALESCE(SUM(TIMESTAMPDIFF(SECOND, answered_at, ended_at)), 0) AS duration_secs
         FROM sip_calls WHERE started_at >= DATE_SUB(NOW(), INTERVAL 7 DAY)",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(err)?;

    // Average duration for answered calls in the last 30 days (seconds, may be NULL)
    let avg_duration: Option<f64> = sqlx::query_scalar(
        "SELECT AVG(TIMESTAMPDIFF(SECOND, answered_at, ended_at))
         FROM sip_calls
         WHERE answered_at IS NOT NULL AND ended_at IS NOT NULL
           AND started_at >= DATE_SUB(NOW(), INTERVAL 30 DAY)",
    )
    .fetch_optional(&state.pool)
    .await
    .map_err(err)?
    .flatten();

    // Top 5 callers in last 7 days
    let top_callers: Vec<UserCount> = sqlx::query_as(
        "SELECT caller AS user, COUNT(*) AS count
         FROM sip_calls WHERE started_at >= DATE_SUB(NOW(), INTERVAL 7 DAY)
         GROUP BY caller ORDER BY count DESC LIMIT 5",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(err)?;

    // Top 5 callees in last 7 days
    let top_callees: Vec<UserCount> = sqlx::query_as(
        "SELECT callee AS user, COUNT(*) AS count
         FROM sip_calls WHERE started_at >= DATE_SUB(NOW(), INTERVAL 7 DAY)
         GROUP BY callee ORDER BY count DESC LIMIT 5",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(err)?;

    // Per-hour call counts for the last 24 hours (returns only hours with calls)
    let hourly_rows: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT HOUR(started_at) AS hour, COUNT(*) AS count
         FROM sip_calls WHERE started_at >= DATE_SUB(NOW(), INTERVAL 24 HOUR)
         GROUP BY HOUR(started_at) ORDER BY hour",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(err)?;

    // Fill gaps: all 24 hours present, missing hours get 0
    let mut hourly_calls = vec![0i64; 24];
    for (hour, count) in hourly_rows {
        if (0..24).contains(&hour) {
            hourly_calls[hour as usize] = count;
        }
    }

    Ok(Json(json!({
        "today": {
            "calls": today.total,
            "answered": today.answered,
            "duration_secs": today.duration_secs,
        },
        "week": {
            "calls": week.total,
            "answered": week.answered,
            "duration_secs": week.duration_secs,
        },
        "avg_duration_secs": avg_duration.unwrap_or(0.0),
        "top_callers": top_callers,
        "top_callees": top_callees,
        "hourly_calls": hourly_calls,
    })))
}
