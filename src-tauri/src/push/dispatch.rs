use super::{
    ledger::{self, Channel, Intent},
    Push,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

// Store reads and CAS always go through the existing owner. None of these
// operations runs under emission/auth/preferences locks.
#[derive(Clone, Copy)]
enum Source {
    Runtime,
    Chat,
}
fn rows(app: &AppHandle, source: Source, channel: Channel) -> Result<Vec<Intent>, String> {
    match source {
        Source::Runtime => {
            let runtime = app.state::<crate::agent_runtime::AgentRuntime>();
            let owner = runtime
                .inner
                .lock()
                .map_err(|_| "Runtime owner unavailable.")?;
            owner
                .store
                .as_ref()
                .map(|s| s.notification_ready(channel))
                .unwrap_or_else(|| Ok(Vec::new()))
        }
        Source::Chat => {
            let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
            let owner = backend
                .services
                .lock()
                .map_err(|_| "Chat owner unavailable.")?;
            owner
                .store
                .as_ref()
                .map_err(|e| e.clone())?
                .notification_ready(channel)
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
        Source::Runtime => {
            let runtime = app.state::<crate::agent_runtime::AgentRuntime>();
            let owner = runtime
                .inner
                .lock()
                .map_err(|_| "Runtime owner unavailable.")?;
            owner
                .store
                .as_ref()
                .ok_or("Runtime unavailable.")?
                .notification_transition(channel, intent, expected, state, next)
        }
        Source::Chat => {
            let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
            let owner = backend
                .services
                .lock()
                .map_err(|_| "Chat owner unavailable.")?;
            owner
                .store
                .as_ref()
                .map_err(|e| e.clone())?
                .notification_transition(channel, intent, expected, state, next)
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
enum Eligibility {
    Allow,
    Wait,
    Suppress(&'static str),
}
fn local_eligibility(app: &AppHandle, intent: &Intent) -> Eligibility {
    let Some(stamp) = intent.admission.desktop.as_ref() else {
        return Eligibility::Suppress("NO_ADMISSION");
    };
    if ledger::now() >= intent.expires_at {
        return Eligibility::Suppress("EXPIRED");
    }
    let auth = app.state::<crate::auth::AuthController>();
    if auth.snapshot().status == crate::auth::AuthStatus::Checking {
        return Eligibility::Wait;
    }
    let Ok(b) = auth.local_notification_binding() else {
        return Eligibility::Suppress("SESSION_REQUIRED");
    };
    let push = app.state::<Push>();
    let c = push.inner.lock().unwrap();
    if c.path.is_none()
        || c.file.retired_sessions.contains(&b.session_id)
        || stamp.installation_id != c.file.installation_id
        || !stamp.owner.matches(&b, &c.process_id)
    {
        return Eligibility::Suppress("OWNER_CHANGED");
    }
    let Some(p) = c.file.accounts.get(&b.user_id) else {
        return Eligibility::Suppress("CONSENT_CHANGED");
    };
    local_policy(
        p,
        stamp.consent_epoch,
        c.denied,
        app.get_window("main").and_then(|w| w.is_focused().ok()),
    )
}
fn local_policy(
    p: &super::DesktopPreferences,
    epoch: u64,
    denied: bool,
    focused: Option<bool>,
) -> Eligibility {
    if !p.enabled || p.consent_epoch != epoch {
        return Eligibility::Suppress("CONSENT_CHANGED");
    }
    if denied {
        return Eligibility::Suppress("PERMISSION_DENIED");
    }
    if p.background_only && focused != Some(false) {
        return Eligibility::Suppress("MAIN_FOCUSED_OR_UNKNOWN");
    }
    Eligibility::Allow
}

fn local(
    app: &AppHandle,
    source: Source,
    mut intent: Intent,
    checking: &mut HashMap<String, Instant>,
) {
    let eligibility = local_eligibility(app, &intent);
    if eligibility == Eligibility::Wait
        && checking_may_wait(
            checking
                .entry(intent.key.clone())
                .or_insert_with(Instant::now)
                .elapsed(),
        )
    {
        return;
    }
    checking.remove(&intent.key);
    if eligibility != Eligibility::Allow {
        intent.reason = Some(
            match eligibility {
                Eligibility::Suppress(reason) => reason,
                _ => "CHECKING_TIMEOUT",
            }
            .into(),
        );
        let state = if ledger::now() >= intent.expires_at {
            "expired"
        } else {
            "suppressed"
        };
        let _ = cas(app, source, Channel::Desktop, &intent, "ready", state, 0);
        return;
    }
    let Some(result) = claim_and_enqueue(
        || {
            matches!(
                cas(
                    app,
                    source,
                    Channel::Desktop,
                    &intent,
                    "ready",
                    "claimed",
                    0
                ),
                Ok(true)
            )
        },
        || match local_eligibility(app, &intent) {
            Eligibility::Allow => app
                .notification()
                .builder()
                .title("Lomi")
                .body("Agent zakończył zadanie.")
                .show()
                .map(|_| "plugin_accepted")
                .unwrap_or("failed"),
            _ => "suppressed",
        },
    ) else {
        return;
    };
    if result == "failed" {
        intent.reason = Some("PLUGIN_ERROR".into());
    }
    if result == "suppressed" {
        intent.reason = Some("AUTHORITY_CHANGED".into());
    }
    let _ = cas(app, source, Channel::Desktop, &intent, "claimed", result, 0);
}
// This is the production claim/enqueue boundary. Injected closures let tests
// drive SQLite CAS and authority changes without showing an actual OS alert.
fn claim_and_enqueue(
    claim: impl FnOnce() -> bool,
    enqueue: impl FnOnce() -> &'static str,
) -> Option<&'static str> {
    if !claim() {
        return None;
    }
    let _guard = crate::auth::controller::LOCAL_NOTIFICATION_EMISSION_GUARD
        .lock()
        .ok()?;
    Some(enqueue())
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
    let payload = serde_json::json!({"schemaVersion":1,"eventKey":intent.key,"sourceId":stamp.source_id,"type":"agent.task.completed.v1","sourceKind":intent.source_kind,"logicalTaskId":intent.logical_task_id,"turnId":intent.turn_id,"completedAt":timestamp,"consentEpoch":stamp.consent_epoch,"sourceEpoch":stamp.source_epoch,"target":null});
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
pub(super) fn start(app: AppHandle) {
    // Local alerts have their own consumer and never wait for HTTP/cache
    // refresh or a cloud queue. There is exactly one cloud dispatcher.
    let local_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut checking = HashMap::new();
        loop {
            for source in [Source::Runtime, Source::Chat] {
                if let Ok(intents) = rows(&local_app, source, Channel::Desktop) {
                    for intent in intents {
                        local(&local_app, source, intent, &mut checking);
                    }
                }
            }
            checking.retain(|_, t| t.elapsed() < Duration::from_secs(60));
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
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
            for source in [Source::Runtime, Source::Chat] {
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
#[cfg(test)]
mod tests {
    use super::*;
    fn intent_fixture() -> (rusqlite::Connection, Intent) {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(ledger::SCHEMA).unwrap();
        let owner = ledger::Owner {
            user_id: "u".into(),
            session_id: "s".into(),
            process_id: "p".into(),
            generation: 1,
        };
        ledger::admit(
            &c,
            "turn",
            &ledger::Admission {
                desktop: Some(ledger::DesktopStamp {
                    owner,
                    installation_id: "i".into(),
                    consent_epoch: 1,
                }),
                cloud: None,
            },
        )
        .unwrap();
        ledger::complete(&c, "chat", "conversation", "turn").unwrap();
        let intent = ledger::ready(&c, Channel::Desktop).unwrap().pop().unwrap();
        (c, intent)
    }
    #[test]
    fn off_or_logout_after_claim_before_enqueue_wins_shared_guard() {
        for reason in ["off", "logout"] {
            let (c, intent) = intent_fixture();
            let active = std::sync::atomic::AtomicBool::new(true);
            let enqueues = std::cell::Cell::new(0);
            let result = claim_and_enqueue(
                || {
                    assert!(ledger::transition(
                        &c,
                        Channel::Desktop,
                        &intent,
                        "ready",
                        "claimed",
                        0
                    )
                    .unwrap());
                    let _guard = crate::auth::controller::LOCAL_NOTIFICATION_EMISSION_GUARD
                        .lock()
                        .unwrap();
                    active.store(false, std::sync::atomic::Ordering::SeqCst);
                    true
                },
                || {
                    if !active.load(std::sync::atomic::Ordering::SeqCst) {
                        return "suppressed";
                    }
                    enqueues.set(enqueues.get() + 1);
                    "plugin_accepted"
                },
            )
            .unwrap();
            assert_eq!(result, "suppressed", "{reason}");
            assert_eq!(enqueues.get(), 0);
            assert!(
                ledger::transition(&c, Channel::Desktop, &intent, "claimed", result, 0).unwrap()
            );
            assert!(!ledger::transition(
                &c,
                Channel::Desktop,
                &intent,
                "claimed",
                "plugin_accepted",
                0
            )
            .unwrap());
        }
    }
    #[test]
    fn enqueue_before_logout_finishes_once_and_delayed_ack_is_cas_guarded() {
        let (c, intent) = intent_fixture();
        let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let other = active.clone();
        let logout = std::thread::spawn(move || {
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let _guard = crate::auth::controller::LOCAL_NOTIFICATION_EMISSION_GUARD
                .lock()
                .unwrap();
            other.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        let enqueues = std::cell::Cell::new(0);
        let result = claim_and_enqueue(
            || ledger::transition(&c, Channel::Desktop, &intent, "ready", "claimed", 0).unwrap(),
            || {
                assert!(active.load(std::sync::atomic::Ordering::SeqCst));
                enqueues.set(enqueues.get() + 1);
                started_tx.send(()).unwrap();
                "plugin_accepted"
            },
        )
        .unwrap();
        logout.join().unwrap();
        assert!(!active.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(enqueues.get(), 1);
        assert!(
            ledger::transition(&c, Channel::Desktop, &intent, "claimed", "suppressed", 0).unwrap()
        );
        assert!(!ledger::transition(&c, Channel::Desktop, &intent, "claimed", result, 0).unwrap());
        assert_eq!(
            claim_and_enqueue(
                || ledger::transition(&c, Channel::Desktop, &intent, "ready", "claimed", 0)
                    .unwrap(),
                || panic!("replayed plugin")
            ),
            None
        );
    }
    #[tokio::test]
    async fn hanging_cloud_cannot_delay_local_claim_and_plugin_enqueue() {
        let cloud = tokio::spawn(std::future::pending::<()>());
        let (c, intent) = intent_fixture();
        let enqueued = std::cell::Cell::new(false);
        let result = claim_and_enqueue(
            || ledger::transition(&c, Channel::Desktop, &intent, "ready", "claimed", 0).unwrap(),
            || {
                enqueued.set(true);
                "plugin_accepted"
            },
        );
        assert_eq!(result, Some("plugin_accepted"));
        assert!(enqueued.get());
        assert!(!cloud.is_finished());
        cloud.abort();
    }
    #[test]
    fn checking_has_an_exact_ten_second_bound_without_claim() {
        assert!(checking_may_wait(Duration::from_millis(9999)));
        assert!(!checking_may_wait(Duration::from_secs(10)));
        let (c, _) = intent_fixture();
        assert_eq!(ledger::ready(&c, Channel::Desktop).unwrap().len(), 1);
    }
    #[test]
    fn local_consent_epoch_permission_and_main_focus_are_final_gates() {
        let mut p = super::super::DesktopPreferences::default();
        assert_eq!(
            local_policy(&p, 1, false, Some(false)),
            Eligibility::Suppress("CONSENT_CHANGED")
        );
        p.enabled = true;
        assert_eq!(
            local_policy(&p, 2, false, Some(false)),
            Eligibility::Suppress("CONSENT_CHANGED")
        );
        assert_eq!(
            local_policy(&p, 1, true, Some(false)),
            Eligibility::Suppress("PERMISSION_DENIED")
        );
        assert_eq!(
            local_policy(&p, 1, false, Some(true)),
            Eligibility::Suppress("MAIN_FOCUSED_OR_UNKNOWN")
        );
        assert_eq!(
            local_policy(&p, 1, false, None),
            Eligibility::Suppress("MAIN_FOCUSED_OR_UNKNOWN")
        );
        assert_eq!(local_policy(&p, 1, false, Some(false)), Eligibility::Allow);
        p.background_only = false;
        assert_eq!(local_policy(&p, 1, false, Some(true)), Eligibility::Allow);
        assert_eq!(local_policy(&p, 1, false, None), Eligibility::Allow);
    }
    #[test]
    fn errors_retry_only_transport_rate_limit_and_server_errors() {
        for s in [None, Some(429), Some(500), Some(503)] {
            assert!(retryable(s, "TRANSPORT_UNKNOWN"));
        }
        for s in [
            Some(400),
            Some(401),
            Some(403),
            Some(404),
            Some(409),
            Some(426),
        ] {
            assert!(!retryable(s, "SERVICE_UNAVAILABLE"));
        }
    }
    #[test]
    fn local_authority_and_invalid_response_errors_are_not_transport_retries() {
        for code in [
            "SESSION_REQUIRED",
            "AUTHORITY_CHANGED",
            "INVALID_REQUEST",
            "INVALID_RESPONSE",
        ] {
            assert!(!retryable(None, code));
        }
        assert!(retryable(None, "TRANSPORT_UNKNOWN"));
    }
    #[test]
    fn retries_are_bounded_and_honor_bounded_retry_after() {
        assert_eq!(retry_delay(100, "a", Some(999)), 600);
        assert_eq!(retry_delay(0, "a", Some(0)), 1);
        assert!(retry_delay(0, "a", None) <= 3);
        assert!(retry_delay(100, "a", None) <= 60);
    }
}
