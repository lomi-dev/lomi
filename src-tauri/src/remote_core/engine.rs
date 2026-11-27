//! Sans-I/O glue between the shared Remote core (`lomi-remote-core`) and the desktop.
//! The engine owns the link state machine and the Host directory and answers every
//! input with a list of [`Out`] actions; sockets, timers, HTTP and the keychain stay in
//! the driver (`mod.rs`). It mirrors the web client (`remote-web/src/core/host.ts` and
//! `client.ts`): list from device memory first, then the session response, then the link;
//! the list is "fresh" only while the link is ready and recently heard from, and a link
//! that stays unconfirmed for [`DEGRADED_AFTER_MS`] re-enables the old inventory path.

use lomi_remote_core::directory::{Directory, Effect, Key, NONCE_LEN};
use lomi_remote_core::link::{Link, LinkConfig, LinkEvent, Now, Output, StopReason};
use lomi_remote_core::session::{
    self, Credential, DeviceState, RelayToken, SessionError, SessionOutcome,
};
use serde::Serialize;

/// The core closes a link that is silent this long; freshness uses the same bound.
pub const SILENCE_MS: u64 = 8_000;
/// Not fresh for 2 x silence: the UI falls back to the inventory reads until the link recovers.
pub const DEGRADED_AFTER_MS: u64 = 2 * SILENCE_MS;
/// Link token renewal: ask this long before expiry (never before half its life) and never
/// twice within the minimum interval. Both are enforced by the core.
pub const REFRESH_LEAD_MS: u64 = 60_000;
pub const MIN_REFRESH_INTERVAL_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Idle,
    Cached,
    Session,
    LinkReady,
    LinkDown,
    UpdateRequired,
    SignedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceView {
    pub id: String,
    pub name: Option<String>,
    pub epoch: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub id: String,
    pub name: Option<String>,
    pub online: bool,
    pub access_epoch: String,
    pub runtime_generation: String,
    pub workspaces: Vec<WorkspaceView>,
}

/// What the UI renders; the same fields as the web `CoreState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreState {
    pub status: Status,
    pub hosts: Vec<HostView>,
    pub from_memory: bool,
    pub refreshing: bool,
    /// Presence is vouched for by a live link; false while the link is down or stale.
    pub fresh: bool,
    pub degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

impl CoreState {
    fn idle() -> Self {
        Self {
            status: Status::Idle,
            hosts: Vec::new(),
            from_memory: false,
            refreshing: false,
            fresh: false,
            degraded: false,
            account_id: None,
        }
    }
}

/// Whose directory this is: Remote account, device and the local desktop user it is bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ids {
    pub account: String,
    pub device: String,
    pub user: String,
}

/// An action for the driver.
#[derive(Debug, PartialEq)]
pub enum Out {
    Connect,
    Send(Vec<u8>),
    Close,
    /// Seal the directory and store it.
    Persist,
    /// Destroy the key and the stored blob.
    WipeVault,
    /// Fetch a new link token now (single flight; the core already spaced the request).
    NeedToken,
    /// The link stopped without an automatic retry: fetch a new token with backoff.
    NeedTokenLater,
    State(CoreState),
    Mark(&'static str, Option<(&'static str, u64)>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Exchange {
    Done,
    Retry,
    Final,
}

pub struct Engine {
    link: Link,
    dir: Option<Directory>,
    ids: Option<Ids>,
    expected_account: Option<String>,
    state: CoreState,
    emitted: Option<CoreState>,
    unfresh_since: Option<u64>,
    touched: bool,
    contract_hash: String,
}

impl Engine {
    pub fn new(contract_hash: &str, seed: u64) -> Self {
        let mut cfg = LinkConfig::new(contract_hash);
        cfg.silence_ms = SILENCE_MS;
        cfg.refresh_lead_ms = REFRESH_LEAD_MS;
        cfg.min_refresh_interval_ms = MIN_REFRESH_INTERVAL_MS;
        Self {
            link: Link::new(cfg, seed),
            dir: None,
            ids: None,
            expected_account: None,
            state: CoreState::idle(),
            emitted: None,
            unfresh_since: None,
            touched: false,
            contract_hash: contract_hash.to_owned(),
        }
    }

    pub fn state(&self) -> &CoreState {
        &self.state
    }

    pub fn ids(&self) -> Option<&Ids> {
        self.ids.as_ref()
    }

    pub fn set_expected_account(&mut self, account: Option<String>) {
        self.expected_account = account;
    }

    /// Body of `POST /remote/session`; the desktop bearer goes in the header, never here.
    pub fn session_body(&self) -> Vec<u8> {
        session::session_request(&self.contract_hash, &Credential::Cookie).body
    }

    pub fn seal(&self, key: &Key, nonce: [u8; NONCE_LEN]) -> Option<Vec<u8>> {
        self.dir.as_ref()?.seal(key, nonce).ok()
    }

    /// Shows the directory read from device memory before any network.
    pub fn load_cache(&mut self, dir: Directory, ids: Ids, now: Now) -> Vec<Out> {
        let mut out = Vec::new();
        self.state.account_id = Some(ids.account.clone());
        self.dir = Some(dir);
        self.ids = Some(ids);
        self.sync_hosts();
        self.state.status = Status::Cached;
        self.state.from_memory = true;
        self.touched = true;
        out.push(Out::Mark(
            "core-list-cached",
            Some(("hosts", self.state.hosts.len() as u64)),
        ));
        self.finish(now, &mut out);
        out
    }

    pub fn begin_exchange(&mut self, now: Now) -> Vec<Out> {
        let mut out = Vec::new();
        self.touched = true;
        self.state.refreshing = true;
        self.finish(now, &mut out);
        out
    }

    pub fn end_exchange(&mut self, now: Now) -> Vec<Out> {
        let mut out = Vec::new();
        self.state.refreshing = false;
        self.finish(now, &mut out);
        out
    }

    /// One answer of `POST /remote/session` (any status). `user` is the local desktop user
    /// that sent the request.
    pub fn on_session(
        &mut self,
        status: u16,
        body: &[u8],
        user: &str,
        now: Now,
    ) -> (Exchange, Vec<Out>) {
        let mut out = vec![Out::Mark(
            "session-response",
            Some(("status", status.into())),
        )];
        let result = match session::parse_session_response(status, body) {
            Err(SessionError::Unauthorized) => {
                self.wipe_into(Status::SignedOut, now, &mut out);
                Exchange::Final
            }
            Err(SessionError::UpdateRequired) => {
                self.state.status = Status::UpdateRequired;
                Exchange::Final
            }
            Err(SessionError::OutcomeUnknown) => Exchange::Retry,
            Err(e) if e.is_retryable() => Exchange::Retry,
            Err(_) => Exchange::Final,
            // Desktop sends no refresh credential, so a degraded answer is not expected; retry.
            Ok(SessionOutcome::Degraded { .. }) => Exchange::Retry,
            Ok(SessionOutcome::Full(r)) => self.apply_full(*r, user, now, &mut out),
        };
        self.finish(now, &mut out);
        (result, out)
    }

    fn apply_full(
        &mut self,
        r: session::SessionResponse,
        user: &str,
        now: Now,
        out: &mut Vec<Out>,
    ) -> Exchange {
        let device = match &r.device {
            DeviceState::Revoked => {
                self.wipe_into(Status::SignedOut, now, out);
                return Exchange::Final;
            }
            DeviceState::Registered { device_id } => device_id.clone(),
            DeviceState::Unregistered => "unregistered".to_owned(),
        };
        let account = r.account.id.clone();
        if self
            .expected_account
            .as_deref()
            .is_some_and(|e| !e.eq_ignore_ascii_case(&account))
        {
            // The bearer belongs to another account than the page believes: keep nothing.
            self.wipe_into(Status::SignedOut, now, out);
            return Exchange::Final;
        }
        let same = self
            .ids
            .as_ref()
            .is_some_and(|i| i.account == account && i.device == device && i.user == user);
        if self.dir.is_some() && !same {
            // Another account, device or desktop user: old link, directory and key go first.
            self.wipe_into(Status::Idle, now, out);
        }
        if self.dir.is_none() {
            self.dir = Some(Directory::new(account.clone(), device.clone()));
            self.ids = Some(Ids {
                account: account.clone(),
                device,
                user: user.to_owned(),
            });
        }
        let effect = self.dir.as_mut().map(|d| d.apply_session(&r));
        match effect {
            Some(Ok(Effect::Unchanged)) => {}
            Some(Ok(Effect::Changed)) => out.push(Out::Persist),
            _ => {
                // Revoked or a response that does not fit this directory: forget it.
                self.wipe_into(Status::SignedOut, now, out);
                return Exchange::Final;
            }
        }
        self.sync_hosts();
        self.state.account_id = Some(account);
        self.state.from_memory = false;
        if self.state.status != Status::LinkReady {
            self.state.status = Status::Session;
        }
        self.supply_token(r.relay, now, out);
        Exchange::Done
    }

    fn supply_token(&mut self, token: RelayToken, now: Now, out: &mut Vec<Out>) {
        let outputs = if self.link.is_ready() {
            match self.link.refresh_token(token.clone(), now) {
                Ok(outputs) => outputs,
                Err(_) => self.link.start(token, now),
            }
        } else {
            self.link.start(token, now)
        };
        self.link_outputs(outputs, now, out);
    }

    // ---- link ----

    pub fn on_open(&mut self, now: Now) -> Vec<Out> {
        let outputs = self.link.on_open(now);
        self.run(outputs, now)
    }

    pub fn on_close(&mut self, now: Now) -> Vec<Out> {
        let outputs = self.link.on_close(now);
        self.run(outputs, now)
    }

    pub fn on_message(&mut self, bytes: &[u8], now: Now) -> Vec<Out> {
        let outputs = self.link.on_message(bytes, now);
        self.run(outputs, now)
    }

    pub fn tick(&mut self, now: Now) -> Vec<Out> {
        let outputs = self.link.tick(now);
        self.run(outputs, now)
    }

    /// Earliest monotonic time at which `tick` has work.
    pub fn next_deadline(&self) -> Option<u64> {
        let degrade = self
            .unfresh_since
            .filter(|_| !self.state.degraded)
            .map(|since| since + DEGRADED_AFTER_MS);
        [self.link.next_deadline(), degrade]
            .into_iter()
            .flatten()
            .min()
    }

    fn run(&mut self, outputs: Vec<Output>, now: Now) -> Vec<Out> {
        let mut out = Vec::new();
        self.link_outputs(outputs, now, &mut out);
        self.finish(now, &mut out);
        out
    }

    fn link_outputs(&mut self, outputs: Vec<Output>, now: Now, out: &mut Vec<Out>) {
        for output in outputs {
            match output {
                Output::Connect => out.push(Out::Connect),
                Output::Send(bytes) => out.push(Out::Send(bytes)),
                Output::Close => out.push(Out::Close),
                Output::Event(event) => self.link_event(event, now, out),
            }
        }
    }

    fn link_event(&mut self, event: LinkEvent, now: Now, out: &mut Vec<Out>) {
        match event {
            LinkEvent::Ready { state, .. } => {
                let effect = self.dir.as_mut().map(|d| d.apply_ready_state(&state));
                if self.merge(effect, now, out) {
                    self.state.status = Status::LinkReady;
                    out.push(Out::Mark("link-ready", None));
                }
            }
            LinkEvent::Server { name, data } => {
                let effect = self.dir.as_mut().map(|d| d.apply_event(&name, &data));
                // A malformed event never takes the link down.
                self.merge(effect, now, out);
            }
            LinkEvent::TokenRefreshNeeded
            | LinkEvent::TokenNeeded
            | LinkEvent::TokenRefreshFailed { .. } => out.push(Out::NeedToken),
            LinkEvent::Stopped(StopReason::UpdateRequired { .. }) => {
                self.state.status = Status::UpdateRequired;
            }
            LinkEvent::Stopped(StopReason::Requested) => {}
            LinkEvent::Stopped(_) => {
                self.link_down();
                out.push(Out::NeedTokenLater);
            }
            LinkEvent::Disconnected { .. } => self.link_down(),
            LinkEvent::Stale { .. }
            | LinkEvent::CallDone { .. }
            | LinkEvent::TokenRefreshed { .. } => {}
        }
    }

    /// Applies a directory merge. Returns false when the directory was revoked and wiped.
    fn merge(
        &mut self,
        effect: Option<Result<Effect, lomi_remote_core::directory::Error>>,
        now: Now,
        out: &mut Vec<Out>,
    ) -> bool {
        match effect {
            Some(Ok(Effect::Revoked)) => {
                self.wipe_into(Status::SignedOut, now, out);
                false
            }
            Some(Ok(Effect::Changed)) => {
                self.sync_hosts();
                self.state.from_memory = false;
                out.push(Out::Persist);
                true
            }
            _ => true,
        }
    }

    fn link_down(&mut self) {
        if !matches!(
            self.state.status,
            Status::SignedOut | Status::UpdateRequired
        ) {
            self.state.status = Status::LinkDown;
        }
    }

    // ---- names, wipe ----

    /// Names learned from the inventory path (the server has none).
    pub fn set_names(&mut self, hosts: &[NameHint], now: Now) -> Vec<Out> {
        let mut out = Vec::new();
        let Some(dir) = self.dir.as_mut() else {
            return out;
        };
        let mut changed = false;
        for host in hosts {
            if let Some(name) = &host.name {
                changed |= dir.set_host_name(&host.host_id, name);
            }
            for w in &host.workspaces {
                if let Some(name) = &w.name {
                    changed |= dir.set_workspace_name(&host.host_id, &w.workspace_id, name);
                }
            }
        }
        if changed {
            self.sync_hosts();
            out.push(Out::Persist);
        }
        self.finish(now, &mut out);
        out
    }

    /// Logout, revoke, account change or lock: link closed, directory wiped, key destroyed.
    pub fn wipe(&mut self, status: Status, now: Now) -> Vec<Out> {
        let mut out = Vec::new();
        self.wipe_into(status, now, &mut out);
        self.finish(now, &mut out);
        out
    }

    fn wipe_into(&mut self, status: Status, now: Now, out: &mut Vec<Out>) {
        let outputs = self.link.stop();
        self.link_outputs(outputs, now, out);
        if let Some(dir) = self.dir.as_mut() {
            dir.wipe();
        }
        self.dir = None;
        self.ids = None;
        self.state = CoreState::idle();
        self.state.status = status;
        self.unfresh_since = None;
        self.touched = false;
        out.push(Out::WipeVault);
    }

    // ---- state ----

    fn sync_hosts(&mut self) {
        self.state.hosts = self
            .dir
            .as_ref()
            .map(|d| {
                d.hosts()
                    .map(|h| HostView {
                        id: h.id.clone(),
                        name: h.name.clone(),
                        online: h.online,
                        access_epoch: h.access_epoch.to_string(),
                        runtime_generation: h.runtime_generation.to_string(),
                        workspaces: h
                            .workspaces
                            .iter()
                            .map(|w| WorkspaceView {
                                id: w.id.clone(),
                                name: w.name.clone(),
                                epoch: w.epoch.clone(),
                            })
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    /// Recomputes freshness and the fallback flag, and reports a changed state.
    fn finish(&mut self, now: Now, out: &mut Vec<Out>) {
        self.state.fresh = self.link.is_ready()
            && self
                .link
                .last_server_frame_mono_ms()
                .is_some_and(|m| now.mono_ms.saturating_sub(m) <= SILENCE_MS);
        let needs_fallback = !self.state.fresh
            && !matches!(
                self.state.status,
                Status::SignedOut | Status::UpdateRequired
            );
        if needs_fallback && self.touched {
            let since = *self.unfresh_since.get_or_insert(now.mono_ms);
            self.state.degraded = now.mono_ms.saturating_sub(since) >= DEGRADED_AFTER_MS;
        } else {
            self.unfresh_since = None;
            self.state.degraded = false;
        }
        if self.emitted.as_ref() != Some(&self.state) {
            self.emitted = Some(self.state.clone());
            out.push(Out::State(self.state.clone()));
        }
    }
}

/// Names carried by an inventory read, for the core to remember.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameHint {
    pub host_id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceNameHint>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNameHint {
    pub workspace_id: String,
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use lomi_remote_core::frame::{self, Frame, Role};
    use serde_json::{json, Value};

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaa";
    const HOST: &str = "11111111-1111-4111-8111-111111111111";
    const WS: &str = "22222222-2222-4222-8222-222222222222";
    const EPOCH: &str = "33333333-3333-4333-8333-333333333333";

    fn at(ms: u64) -> Now {
        Now {
            mono_ms: ms,
            wall_ms: 1_000_000 + ms,
        }
    }

    fn session_body(account: &str, device: &str, online: bool) -> Vec<u8> {
        let workspaces = if online {
            json!([{ "workspaceId": WS, "workspaceEpoch": EPOCH }])
        } else {
            json!([])
        };
        serde_json::to_vec(&json!({
            "account": { "id": account },
            "sessionExpiresAt": 9_000_000_000_000u64,
            "relay": { "token": TOKEN, "expiresAt": 1_000_000 + 300_000 },
            "hosts": [{ "hostId": HOST, "online": online, "accessEpoch": "1",
                        "runtimeGeneration": "1", "workspaces": workspaces }],
            "device": { "state": "registered", "deviceId": device },
        }))
        .unwrap()
    }

    fn ready_frame(expires_at: u64) -> Vec<u8> {
        let state = json!({ "hosts": [{ "hostId": HOST, "online": true, "accessEpoch": "1",
            "runtimeGeneration": "1" }] });
        frame::encode_frame(
            &Frame::ReadyOk {
                role: Role::Device,
                expires_at,
                state: state.as_object().unwrap().clone(),
            },
            frame::MAX_FRAME_BYTES,
        )
        .unwrap()
    }

    fn event_frame(name: &str, data: Value) -> Vec<u8> {
        frame::encode_frame(
            &Frame::Event {
                name: name.into(),
                data: data.as_object().unwrap().clone(),
            },
            frame::MAX_FRAME_BYTES,
        )
        .unwrap()
    }

    fn has(out: &[Out], wanted: &Out) -> bool {
        out.iter().any(|o| o == wanted)
    }

    fn last_state(out: &[Out]) -> Option<&CoreState> {
        out.iter().rev().find_map(|o| match o {
            Out::State(s) => Some(s),
            _ => None,
        })
    }

    /// Session answered, socket opened, READY received at `t`.
    fn connected(user: &str) -> Engine {
        let mut e = Engine::new(HASH, 7);
        e.begin_exchange(at(0));
        let (exchange, out) =
            e.on_session(200, &session_body("acc-1", "dev-1", true), user, at(10));
        assert_eq!(exchange, Exchange::Done);
        assert!(has(&out, &Out::Connect));
        assert!(has(&out, &Out::Persist));
        e.on_open(at(20));
        let out = e.on_message(&ready_frame(1_000_000 + 300_000), at(30));
        assert!(has(&out, &Out::Mark("link-ready", None)));
        e
    }

    #[test]
    fn list_comes_from_memory_then_session_then_link_with_timing_marks() {
        let mut cached = Directory::new("acc-1", "dev-1");
        let seeded = session::parse_session_response(200, &session_body("acc-1", "dev-1", true));
        let Ok(SessionOutcome::Full(r)) = seeded else {
            panic!("fixture")
        };
        cached.apply_session(&r).unwrap();

        let mut e = Engine::new(HASH, 1);
        let ids = Ids {
            account: "acc-1".into(),
            device: "dev-1".into(),
            user: "u1".into(),
        };
        let out = e.load_cache(cached, ids, at(0));
        assert_eq!(out[0], Out::Mark("core-list-cached", Some(("hosts", 1))));
        let s = last_state(&out).unwrap();
        assert_eq!(
            (s.status, s.from_memory, s.fresh),
            (Status::Cached, true, false)
        );
        assert_eq!(s.hosts.len(), 1);

        e.begin_exchange(at(5));
        let (_, out) = e.on_session(200, &session_body("acc-1", "dev-1", true), "u1", at(10));
        assert_eq!(out[0], Out::Mark("session-response", Some(("status", 200))));
        let s = last_state(&out).unwrap();
        assert_eq!(
            (s.status, s.from_memory, s.fresh),
            (Status::Session, false, false)
        );

        e.on_open(at(20));
        let out = e.on_message(&ready_frame(1_300_000), at(30));
        let s = last_state(&out).unwrap();
        assert_eq!(
            (s.status, s.fresh, s.degraded),
            (Status::LinkReady, true, false)
        );
    }

    #[test]
    fn unfresh_link_falls_back_after_sixteen_seconds_and_recovers() {
        let mut e = connected("u1");
        // No frame for 8 s: the core closes the stale link; the list is no longer fresh.
        let out = e.tick(at(30 + SILENCE_MS + 1));
        assert!(has(&out, &Out::Close));
        let s = last_state(&out).unwrap();
        assert!(!s.fresh && !s.degraded && s.status == Status::LinkDown);
        // Still not confirmed 16 s after the first unfresh moment: fall back to polling.
        let deadline = e.next_deadline().unwrap();
        let out = e.tick(at(deadline.max(30 + SILENCE_MS + 1 + DEGRADED_AFTER_MS)));
        assert!(last_state(&out).unwrap().degraded);
        // A reconnect that reaches READY is fresh again and clears the fallback.
        let mut now = 100_000;
        let mut out = e.tick(at(now));
        while !has(&out, &Out::Connect) {
            now += 1_000;
            out = e.tick(at(now));
        }
        e.on_open(at(now));
        let out = e.on_message(&ready_frame(1_300_000), at(now + 1));
        let s = last_state(&out).unwrap();
        assert!(s.fresh && !s.degraded);
    }

    #[test]
    fn token_is_renewed_before_expiry_without_dropping_the_link() {
        let mut e = connected("u1");
        // Expiry at wall 1_300_000 (mono 300_000): lead 60 s -> refresh asked at mono 240_000+.
        let at_lead = 300_000 - REFRESH_LEAD_MS;
        assert_eq!(e.next_deadline().is_some(), true);
        let mut now = 30;
        let mut asked = false;
        while now <= at_lead + 1_000 && !asked {
            // keep the link alive like authority.fresh does
            e.on_message(
                &event_frame(
                    "presence",
                    json!({"hostId": HOST, "online": true,
                "accessEpoch": "1", "runtimeGeneration": "1"}),
                ),
                at(now),
            );
            now += 2_000;
            asked = has(&e.tick(at(now)), &Out::NeedToken);
        }
        assert!(asked, "TokenRefreshNeeded must surface as NeedToken");
        // The renewal answer reaches the ready link as token.update, not as a reconnect.
        let (exchange, out) =
            e.on_session(200, &session_body("acc-1", "dev-1", true), "u1", at(now));
        assert_eq!(exchange, Exchange::Done);
        assert!(out.iter().any(|o| matches!(o, Out::Send(_))));
        assert!(!has(&out, &Out::Connect) && !has(&out, &Out::Close));
    }

    #[test]
    fn revoked_event_and_revoked_device_wipe_everything() {
        let mut e = connected("u1");
        let out = e.on_message(&event_frame("revoked", json!({})), at(40));
        assert!(has(&out, &Out::WipeVault) && has(&out, &Out::Close));
        let s = last_state(&out).unwrap();
        assert_eq!(
            (s.status, s.hosts.len(), s.account_id.clone()),
            (Status::SignedOut, 0, None)
        );
        assert!(e.ids().is_none() && e.seal(&Key::new(1, [1; 32]), [0; NONCE_LEN]).is_none());

        let mut e = Engine::new(HASH, 2);
        let mut body: Value =
            serde_json::from_slice(&session_body("acc-1", "dev-1", true)).unwrap();
        body["device"] = json!({ "state": "revoked" });
        let (exchange, out) = e.on_session(200, &serde_json::to_vec(&body).unwrap(), "u1", at(1));
        assert_eq!(exchange, Exchange::Final);
        assert!(has(&out, &Out::WipeVault));
    }

    #[test]
    fn unauthorized_wipes_and_update_required_keeps_the_directory() {
        let mut e = connected("u1");
        let (exchange, out) = e.on_session(401, b"", "u1", at(50));
        assert_eq!(exchange, Exchange::Final);
        assert!(has(&out, &Out::WipeVault));

        let mut e = connected("u1");
        let body = br#"{"code":"UPDATE_REQUIRED"}"#;
        let (exchange, out) = e.on_session(409, body, "u1", at(50));
        assert_eq!(exchange, Exchange::Final);
        assert!(!has(&out, &Out::WipeVault));
        assert_eq!(last_state(&out).unwrap().status, Status::UpdateRequired);
    }

    #[test]
    fn server_trouble_is_retried_and_other_refusals_are_final() {
        let mut e = Engine::new(HASH, 3);
        assert_eq!(e.on_session(503, b"", "u1", at(1)).0, Exchange::Retry);
        assert_eq!(e.on_session(429, b"", "u1", at(1)).0, Exchange::Retry);
        assert_eq!(e.on_session(404, b"", "u1", at(1)).0, Exchange::Final);
        assert_eq!(e.on_session(200, b"{}", "u1", at(1)).0, Exchange::Final);
    }

    #[test]
    fn another_account_device_or_user_replaces_the_directory() {
        let mut e = connected("u1");
        let (_, out) = e.on_session(200, &session_body("acc-2", "dev-1", true), "u1", at(60));
        assert!(has(&out, &Out::WipeVault), "old key and blob must go first");
        assert_eq!(e.ids().unwrap().account, "acc-2");

        let mut e = connected("u1");
        let (_, out) = e.on_session(200, &session_body("acc-1", "dev-1", true), "u2", at(60));
        assert!(has(&out, &Out::WipeVault));
        assert_eq!(e.ids().unwrap().user, "u2");
    }

    #[test]
    fn expected_account_mismatch_keeps_nothing() {
        let mut e = Engine::new(HASH, 4);
        e.set_expected_account(Some("acc-9".into()));
        let (exchange, out) = e.on_session(200, &session_body("acc-1", "dev-1", true), "u1", at(1));
        assert_eq!(exchange, Exchange::Final);
        assert!(has(&out, &Out::WipeVault) && e.ids().is_none());
        assert!(last_state(&out).unwrap().hosts.is_empty());
    }

    #[test]
    fn names_from_the_inventory_path_survive_server_snapshots_and_persist() {
        let mut e = connected("u1");
        let hints: Vec<NameHint> = serde_json::from_value(json!([{
            "hostId": HOST, "name": "Studio", "workspaces": [{ "workspaceId": WS, "name": "app" }]
        }]))
        .unwrap();
        let out = e.set_names(&hints, at(70));
        assert!(has(&out, &Out::Persist));
        assert_eq!(e.state().hosts[0].name.as_deref(), Some("Studio"));
        assert!(e
            .set_names(&hints, at(71))
            .iter()
            .all(|o| *o != Out::Persist));
        e.on_session(200, &session_body("acc-1", "dev-1", true), "u1", at(80));
        assert_eq!(e.state().hosts[0].name.as_deref(), Some("Studio"));
    }

    #[test]
    fn sealed_directory_opens_only_with_its_key_and_ids() {
        let e = connected("u1");
        let blob = e.seal(&Key::new(1, [9; 32]), [3; NONCE_LEN]).unwrap();
        assert!(Directory::open(&blob, "acc-1", "dev-1", &[Key::new(1, [9; 32])], 1).is_ok());
        assert!(Directory::open(&blob, "acc-1", "dev-1", &[Key::new(1, [8; 32])], 1).is_err());
        assert!(Directory::open(&blob, "acc-2", "dev-1", &[Key::new(1, [9; 32])], 1).is_err());
    }

    #[test]
    fn session_body_carries_the_contract_hash_and_no_credential() {
        let e = Engine::new(HASH, 5);
        let body: Value = serde_json::from_slice(&e.session_body()).unwrap();
        assert_eq!(body, json!({ "contractHash": HASH }));
    }
}
