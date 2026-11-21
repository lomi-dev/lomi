use super::{
    ledger::{self, Channel, Intent},
    Push,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

// Store reads and CAS always go through the existing owner. None of these
// operations runs under emission/auth/preferences locks.
#[derive(Clone, Copy)]
enum Source {
    Terminal,
}
fn rows(app: &AppHandle, source: Source, channel: Channel) -> Result<Vec<Intent>, String> {
    match source {
        Source::Terminal => {
            let push = app.state::<Push>();
            let owner = push
                .terminal
                .lock()
                .map_err(|_| "Terminal notification ledger unavailable.")?;
            owner
                .as_ref()
                .map(|s| s.ready(channel))
                .unwrap_or_else(|| Ok(Vec::new()))
        }
    }
}
fn cas(
    app: &AppHandle,
    source: Source,
    channel: Channel,
    intent: &Intent,
    expected: &str,
    state: &str,
    next: u64,
) -> Result<bool, String> {
    match source {
        Source::Terminal => {
            let push = app.state::<Push>();
            let owner = push
                .terminal
                .lock()
                .map_err(|_| "Terminal notification ledger unavailable.")?;
            owner
                .as_ref()
                .ok_or("Terminal notification ledger unavailable.")?
                .transition(channel, intent, expected, state, next)
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
enum Eligibility {
    Allow,
    Wait,
    Suppress(&'static str),
}
fn checking_may_wait(elapsed: Duration) -> bool {
    elapsed < Duration::from_secs(10)
}
fn cloud_eligibility(app: &AppHandle, intent: &Intent) -> Eligibility {
    let Some(stamp) = intent.admission.cloud.as_ref() else {
        return Eligibility::Suppress("NO_ADMISSION");
    };
    if ledger::now() >= intent.expires_at {
        return Eligibility::Suppress("EXPIRED");
    }
    let auth = app.state::<crate::auth::AuthController>();
    if auth.snapshot().status == crate::auth::AuthStatus::Checking {
        return Eligibility::Wait;
    }
    let Ok(b) = auth.remote_binding() else {
        // A verified retained offline session may wait for revalidation, but
        // cannot use HTTP. It never obtains a new stamp from another owner.
        if let Ok(b) = auth.local_notification_binding() {
            let push = app.state::<Push>();
            let c = push.inner.lock().unwrap();
            if stamp.owner.matches(&b, &c.process_id)
                && !c.file.retired_sessions.contains(&b.session_id)
            {
                return Eligibility::Wait;
            }
        }
        return Eligibility::Suppress("SESSION_REQUIRED");
    };
    let push = app.state::<Push>();
    let c = push.inner.lock().unwrap();
    if c.path.is_none()
        || c.file.retired_sessions.contains(&b.session_id)
        || !stamp.owner.matches(&b, &c.process_id)
    {
        return Eligibility::Suppress("OWNER_CHANGED");
    }
    let Some(cache) = c
        .file
        .cache
        .as_ref()
        .filter(|c| c.user_id == b.user_id && c.session_id == b.session_id)
    else {
        return Eligibility::Wait;
    };
    cloud_policy(cache, stamp, &c.file.installation_id)
}
fn cloud_policy(
    cache: &super::Cache,
    stamp: &ledger::CloudStamp,
    installation: &str,
) -> Eligibility {
    if !cache.preferences.available
        || !cache.preferences.enabled
        || cache.preferences.consent_epoch != stamp.consent_epoch
    {
        return Eligibility::Suppress("CONSENT_CHANGED");
    }
    if !cache.sources.iter().any(|s| {
        s.id == stamp.source_id
            && s.enabled
            && !s.revoked
            && s.source_epoch == stamp.source_epoch
            && s.current
            && s.desktop_installation_id == installation
    }) {
        return Eligibility::Suppress("SOURCE_CHANGED");
    }
    Eligibility::Allow
}

fn retry_delay(count: u32, key: &str, retry_after: Option<u64>) -> u64 {
    let base = match count {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        5 => 30,
        _ => 60,
    };
    let jitter = u64::from(key.bytes().next().unwrap_or(0) % 3);
    retry_after
        .map(|seconds| seconds.clamp(1, 600))
        .unwrap_or((base + jitter).min(60))
}
fn retryable(status: Option<u16>, code: &str) -> bool {
    matches!(status, Some(429 | 500..=599))
        || (status.is_none() && matches!(code, "TRANSPORT_UNKNOWN" | "SERVICE_UNAVAILABLE"))
}
async fn cloud(
    app: &AppHandle,
    source: Source,
    mut intent: Intent,
    checking: &mut HashMap<String, Instant>,
) {
    match cloud_eligibility(app, &intent) {
        Eligibility::Wait => {
            if app.state::<crate::auth::AuthController>().snapshot().status
                == crate::auth::AuthStatus::Checking
                && !checking_may_wait(
                    checking
                        .entry(intent.key.clone())
                        .or_insert_with(Instant::now)
                        .elapsed(),
                )
            {
                intent.reason = Some("CHECKING_TIMEOUT".into());
                let _ = cas(
                    app,
                    source,
                    Channel::Cloud,
                    &intent,
                    "pending",
                    "suppressed",
                    0,
                );
                checking.remove(&intent.key);
            }
            return;
        }
        Eligibility::Suppress(reason) => {
            intent.reason = Some(reason.into());
            let state = if ledger::now() >= intent.expires_at {
                "expired"
            } else {
                "suppressed"
            };
            let _ = cas(app, source, Channel::Cloud, &intent, "pending", state, 0);
            return;
        }
        Eligibility::Allow => {}
    }
    checking.remove(&intent.key);
    let auth = app.state::<crate::auth::AuthController>();
    let Ok(binding) = auth.remote_binding() else {
        return;
    };
    let stamp = intent.admission.cloud.as_ref().unwrap();
    let process = app.state::<Push>().inner.lock().unwrap().process_id.clone();
    if !stamp.owner.matches(&binding, &process) {
        return;
    }
    let timestamp = chrono::DateTime::from_timestamp(intent.completed_at as i64, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default();
    let payload = serde_json::json!({"schemaVersion":1,"eventKey":intent.key,"sourceId":stamp.source_id,"type":event_type(&intent),"sourceKind":intent.source_kind,"logicalTaskId":intent.logical_task_id,"turnId":intent.turn_id,"completedAt":timestamp,"consentEpoch":stamp.consent_epoch,"sourceEpoch":stamp.source_epoch,"target":null});
    let result = auth
        .push_request_bound(
            reqwest::Method::POST,
            "/v1/remote/native/push/events",
            Some(&payload),
            &binding,
        )
        .await;
    let (state, next) = match result {
        Ok((_, value)) => match serde_json::from_value::<super::EventReceipt>(value) {
            Ok(receipt) if receipt.event_id.len() <= 64 => {
                intent.server_event_id = Some(receipt.event_id);
                (
                    if receipt.status == super::EventStatus::Queued {
                        "accepted"
                    } else {
                        "suppressed"
                    },
                    0,
                )
            }
            _ => {
                intent.reason = Some("INVALID_RESPONSE".into());
                ("suppressed", 0)
            }
        },
        Err(error) => {
            intent.reason = Some(error.code.clone());
            if retryable(error.status, &error.code) && ledger::now() < intent.expires_at {
                let next = ledger::now()
                    + retry_delay(intent.retry_count, &intent.key, error.retry_after_seconds);
                intent.retry_count = intent.retry_count.saturating_add(1);
                ("pending", next.min(intent.expires_at))
            } else {
                // Conflict/update-required cannot overwrite stale consent. A
                // later refresh reconciles the UI, never retries this event.
                ("suppressed", 0)
            }
        }
    };
    let _ = cas(app, source, Channel::Cloud, &intent, "pending", state, next);
}
fn event_type(intent: &Intent) -> &'static str {
    if intent.source_kind == "terminal-notification" {
        "agent.terminal.notification.v1"
    } else {
        "agent.task.completed.v1"
    }
}
pub(super) fn start(app: AppHandle) {
    // There is exactly one cloud dispatcher.
    if !crate::desktop_runtime::shared_host_enabled() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut checking = HashMap::new();
        let mut last_refresh = Instant::now() - Duration::from_secs(60);
        let mut authority = None;
        loop {
            let auth = app.state::<crate::auth::AuthController>();
            let current = auth
                .remote_binding()
                .ok()
                .map(|b| (b.user_id, b.session_id, b.generation));
            if current.is_some()
                && (current != authority || last_refresh.elapsed() >= Duration::from_secs(30))
            {
                let _ = app.state::<Push>().refresh(&app).await;
                last_refresh = Instant::now();
            }
            if current != authority {
                app.state::<Push>().publish(&app);
                authority = current;
            }
            for source in [Source::Terminal] {
                if let Ok(intents) = rows(&app, source, Channel::Cloud) {
                    for intent in intents.into_iter().take(2) {
                        cloud(&app, source, intent, &mut checking).await;
                    }
                }
            }
            checking.retain(|_, t| t.elapsed() < Duration::from_secs(60));
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}
