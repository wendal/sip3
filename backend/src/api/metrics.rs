//! Prometheus `/api/metrics` endpoint and global metric registry.
//!
//! Design: a single process-wide [`Metrics`] struct is lazily initialized and
//! stored in a [`OnceLock`]. Modules throughout the backend call free functions
//! like [`inc_registration`] without needing to thread an `Arc<Metrics>`
//! through every constructor.
//!
//! The `AppState` shares the same `Registry` (see [`register_with_registry`])
//! so the HTTP handler in `api/mod.rs` can render Prometheus text format.

use prometheus::{
    Encoder, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::sync::OnceLock;

pub struct Metrics {
    pub registry: Registry,

    // Registrations / calls
    pub registrations_total: IntCounter,
    pub active_registrations: IntGauge,
    pub active_calls: IntGauge,

    // Conferences
    pub conference_participants: IntGaugeVec,

    // Security
    pub auth_failures_total: IntCounterVec,
    pub auto_bans_active: IntGauge,

    // Rate limiting
    pub rate_limit_hits_total: IntCounter,

    // Background workers
    pub email_outbox_pending: IntGauge,
    pub email_outbox_sent: IntCounter,
    pub webhook_deliveries_total: IntCounterVec,
    pub rtp_relay_sessions_active: IntGauge,

    // Config hot-reload
    pub config_reload_count: IntCounter,
}

static GLOBAL: OnceLock<&'static Metrics> = OnceLock::new();

/// Build the metrics struct. Pure function with no global state mutation;
/// called from both [`init`] and the lazy accessor [`global`].
fn build() -> &'static Metrics {
    let registry = Registry::new();

    let registrations_total = IntCounter::with_opts(Opts::new(
        "sip3_registrations_total",
        "Total REGISTER successes",
    ))
    .expect("counter");
    let active_registrations = IntGauge::with_opts(Opts::new(
        "sip3_active_registrations",
        "Currently registered SIP users",
    ))
    .expect("gauge");
    let active_calls = IntGauge::with_opts(Opts::new(
        "sip3_active_calls",
        "Calls currently in trying/answered state",
    ))
    .expect("gauge");

    let conference_participants = IntGaugeVec::new(
        Opts::new(
            "sip3_conference_participants",
            "Active conference participants per room",
        ),
        &["room"],
    )
    .expect("gauge vec");

    let auth_failures_total = IntCounterVec::new(
        Opts::new(
            "sip3_auth_failures_total",
            "Auth failures and rejections by surface and event type",
        ),
        &["surface", "event_type"],
    )
    .expect("counter vec");

    let auto_bans_active = IntGauge::with_opts(Opts::new(
        "sip3_auto_bans_active",
        "Number of IPs currently in active auto-ban state",
    ))
    .expect("gauge");

    let rate_limit_hits_total = IntCounter::with_opts(Opts::new(
        "sip3_rate_limit_hits_total",
        "Number of requests rejected by API rate limiter",
    ))
    .expect("counter");

    let email_outbox_pending = IntGauge::with_opts(Opts::new(
        "sip3_email_outbox_pending",
        "Pending entries in the voicemail email outbox",
    ))
    .expect("gauge");

    let email_outbox_sent = IntCounter::with_opts(Opts::new(
        "sip3_email_outbox_sent_total",
        "Total emails successfully sent by the outbox worker",
    ))
    .expect("counter");

    let webhook_deliveries_total = IntCounterVec::new(
        Opts::new(
            "sip3_webhook_deliveries_total",
            "Webhook delivery outcomes (delivered / failed / dead)",
        ),
        &["status"],
    )
    .expect("counter vec");

    let rtp_relay_sessions_active = IntGauge::with_opts(Opts::new(
        "sip3_rtp_relay_sessions_active",
        "RTP/SRTP relay sessions currently allocated",
    ))
    .expect("gauge");

    let config_reload_count = IntCounter::with_opts(Opts::new(
        "sip3_config_reload_count",
        "Number of times the runtime config has been reloaded",
    ))
    .expect("counter");

    let collectors: Vec<Box<dyn prometheus::core::Collector>> = vec![
        Box::new(registrations_total.clone()),
        Box::new(active_registrations.clone()),
        Box::new(active_calls.clone()),
        Box::new(conference_participants.clone()),
        Box::new(auth_failures_total.clone()),
        Box::new(auto_bans_active.clone()),
        Box::new(rate_limit_hits_total.clone()),
        Box::new(email_outbox_pending.clone()),
        Box::new(email_outbox_sent.clone()),
        Box::new(webhook_deliveries_total.clone()),
        Box::new(rtp_relay_sessions_active.clone()),
        Box::new(config_reload_count.clone()),
    ];
    for c in collectors {
        registry.register(c).expect("register metric");
    }

    Box::leak(Box::new(Metrics {
        registry,
        registrations_total,
        active_registrations,
        active_calls,
        conference_participants,
        auth_failures_total,
        auto_bans_active,
        rate_limit_hits_total,
        email_outbox_pending,
        email_outbox_sent,
        webhook_deliveries_total,
        rtp_relay_sessions_active,
        config_reload_count,
    }))
}

/// Initialize the global metrics registry. Idempotent; returns the live
/// `&'static Metrics` reference on every call after the first.
pub fn init() -> &'static Metrics {
    GLOBAL.get_or_init(build)
}

/// Public accessor. Lazily initializes the registry on first call so that
/// unit tests that construct `SecurityGuard` or `RateLimiter` directly
/// (without going through `main`) do not panic. The production binary
/// always calls [`init`] at startup.
pub fn global() -> &'static Metrics {
    GLOBAL.get_or_init(build)
}

pub fn inc_registration() {
    global().registrations_total.inc();
}

pub fn inc_auth_failure(surface: &str, event_type: &str) {
    global()
        .auth_failures_total
        .with_label_values(&[surface, event_type])
        .inc();
}

pub fn inc_rate_limit_hit() {
    global().rate_limit_hits_total.inc();
}

pub fn inc_webhook_delivery(status: &str) {
    global()
        .webhook_deliveries_total
        .with_label_values(&[status])
        .inc();
}

pub fn inc_config_reload() {
    global().config_reload_count.inc();
}

pub fn set_conference_participants(room: &str, count: i64) {
    global()
        .conference_participants
        .with_label_values(&[room])
        .set(count);
}

pub fn set_auto_bans_active(count: i64) {
    global().auto_bans_active.set(count);
}

pub fn set_active_registrations(count: i64) {
    global().active_registrations.set(count);
}

pub fn set_active_calls(count: i64) {
    global().active_calls.set(count);
}

pub fn set_email_outbox_pending(count: i64) {
    global().email_outbox_pending.set(count);
}

pub fn inc_email_outbox_sent() {
    global().email_outbox_sent.inc();
}

pub fn set_rtp_relay_sessions_active(count: i64) {
    global().rtp_relay_sessions_active.set(count);
}

/// Render the registry in Prometheus text format.
pub fn render() -> Vec<u8> {
    let encoder = TextEncoder::new();
    let mut buf = Vec::new();
    let metric_families = global().registry.gather();
    encoder.encode(&metric_families, &mut buf).ok();
    buf
}
