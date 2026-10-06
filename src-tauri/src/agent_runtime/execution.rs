use super::{
    native_process::{Prepared, Process},
    native_wire::*,
    service::{append, handoff, history_digest, target, Active, AgentRuntime, Permission},
    store::workspace,
    types::*,
};

use crate::{cli_catalog::TitleCli, terminal::Shells};

use serde_json::{json, Value};

use std::{
    path::Path,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use super::owned_operation::OwnedOperation;
use tauri::AppHandle;

pub(crate) fn model_parts(cli: TitleCli, model: &str) -> Result<(Option<&str>, &str), String> {
    if matches!(cli, TitleCli::Pi | TitleCli::Kilo | TitleCli::Opencode) {
        let (p, m) = model
            .split_once('/')
            .filter(|(p, m)| !p.is_empty() && !m.is_empty())
            .ok_or("Choose an exact provider/model identifier.")?;

        Ok((Some(p), m))
    } else {
        Ok((None, model))
    }
}

struct Binding<'a> {
    state: &'a AgentRuntime,
    app: &'a AppHandle,
    task: Task,
    attempt: Attempt,
    active: Active,
    identity: (u64, u64),
}

impl Binding<'_> {
    fn fence(&self) -> Result<(), String> {
        if self.active.abort.load(Ordering::SeqCst) {
            return Err("The native process is aborting; settlement is unproved.".into());
        }

        if workspace(Path::new(&self.task.cwd))?.1 != self.identity {
            return Err("The task project directory was replaced.".into());
        }

        self.state.with(self.app, |i| {
            let s = i.store.as_ref().unwrap();

            let t = s.task(&self.task.task_id)?;

            if t.generation != self.attempt.generation
                || t.active_attempt_id.as_deref() != Some(&self.attempt.attempt_id)
            {
                return Err("Late native events belong to a fenced attempt.".into());
            }

            target(
                s,
                &self.attempt.account_id,
                self.attempt.auth_revision,
                Some(&t),
            )?;

            Ok(())
        })
    }

    fn observe(&self, event: NativeEvent, startup: bool) -> Result<(), String> {
        self.observe_projected(event, startup, &mut false)
    }

    fn observe_projected(
        &self,
        event: NativeEvent,
        startup: bool,
        retained_projection: &mut bool,
    ) -> Result<(), String> {
        self.fence()?;

        self.state.with(self.app,|i|{

  let s=i.store.as_mut().unwrap();

let mut t=s.task(&self.task.task_id)?;


  if t.generation!=self.attempt.generation||t.active_attempt_id.as_deref()!=Some(&self.attempt.attempt_id){
return Err("Late native events were fenced.".into());

}

  let terminal_seen=t.history.iter().any(|h|h.attempt_id==self.attempt.attempt_id&&h.kind=="terminal");


  let (kind,state,content)=match event {

   NativeEvent::Accepted{
request_id,raw}
=>("accepted","observed",json!({
"requestId":request_id,"raw":raw}
)),
   NativeEvent::Session{
session_id,raw}
=>{
t.attempts.iter_mut().find(|a|a.attempt_id==self.attempt.attempt_id).unwrap().native_ref=Some(session_id.clone());

("session","observed",json!({
"sessionId":session_id,"raw":raw}
))}
,
   NativeEvent::TurnStarted{
session_id,turn_id,raw}
=>("turn_started","observed",json!({
"sessionId":session_id,"turnId":turn_id,"raw":raw}
)),
   NativeEvent::Text{
session_id,turn_id,text}
=>{
let a=t.attempts.iter_mut().find(|a|a.attempt_id==self.attempt.attempt_id).unwrap();

if a.output.len().saturating_add(text.len())>16*1024*1024{
return Err("Native output exceeds the retained journal bound; recovery is required.".into());

}
a.output.push_str(&text);

("assistant","partial",json!({
"sessionId":session_id,"turnId":turn_id,"text":text}
))}
,
   NativeEvent::Tool(tool)=>{
let denied=i.permissions.values().any(|p|p.preview.attempt_id==self.attempt.attempt_id&&p.preview.permission.tool_id.as_deref()==Some(tool.tool_id.as_str())&&p.consumed&&p.decision.as_ref().is_some_and(|(_,allow,_)| !allow));
if denied&&matches!(tool.state,ToolState::Completed|ToolState::Failed) {s.process_tool_denied(&self.attempt.attempt_id,&tool.tool_id)?;}
else if tool_ownership_unqualified(NativeKind::from_cli(self.task.cli.ok_or("Native family lost.")?).ok_or("Native family unsupported.")?,&tool) {s.process_tool_untracked(&self.attempt.attempt_id,&tool.tool_id)?;}
("tool",match tool.state{
ToolState::Started=>"started",ToolState::Pending=>"pending",ToolState::Completed=>"completed",ToolState::Failed=>"failed"}
,serde_json::to_value(tool).map_err(|_|"Cannot retain native tool observation.")?)},
   NativeEvent::Permission(permission)=>{

    if startup{
return Err("Native initialization requested a permission before dispatch; review the retained account session.".into());

}

    let token=super::new_id()?;

let preview=PendingPermission{
task_id:t.task_id.clone(),attempt_id:self.attempt.attempt_id.clone(),generation:self.attempt.generation,approval_token:token.clone(),permission:permission.clone(),project_root:t.cwd.clone()}
;


    if i.permissions.len()>=128{
return Err("Native permission capacity exceeded.".into());

}

    i.permissions.insert(token,Permission{
preview,decision:None,consumed:false}
);

("permission","pending",serde_json::to_value(permission).map_err(|_|"Cannot retain permission.")?)
   }
,
   NativeEvent::PermissionWithdrawn{
request_id}
=>{
i.permissions.retain(|_,p|!(p.preview.attempt_id==self.attempt.attempt_id&&p.preview.permission.request_id==request_id));

("permission_withdrawn","settled",json!({
"requestId":request_id}
))}
,
   NativeEvent::Terminal(terminal)=>{
if startup{
return Err("Native initialization ended before dispatch.".into());

}
 ("terminal","observed",serde_json::to_value(terminal).map_err(|_|"Cannot retain native terminal.")?)}
,
   NativeEvent::Error{
class,raw}
=>("error","observed",json!({
"class":class,"raw":raw}
)),
   NativeEvent::Observed(raw)=>("observation","observed",raw),
  }
;


  if terminal_seen&&!matches!(kind,"accepted"|"observation"){
return Err("Native activity continued after terminal; tool effects require recovery.".into());

}

  if kind=="observation"&&(content["type"]=="resync_required"||content["payload"]["type"]=="resync_required"){
return Err("Native journal continuity was lost. Recovery is required.".into());

}

  append(&mut t,&self.attempt,kind,state,content);

t.revision+=1;

if let Err(error)=s.update_task(&t,None,None) {i.pending_observations.insert(t.task_id.clone(),t);*retained_projection=true;return Err(error);}

Ok(())
 }
)?;

        self.state.changed(self.app, Some(&self.task.task_id));

        Ok(())
    }

    fn observe_frames(
        &self,
        wire: &mut NativeWire,
        frames: Vec<Value>,
        startup: bool,
    ) -> Result<(), String> {
        let mut frames = frames.into_iter();
        while let Some(raw) = frames.next() {
            if let Err(error) = self.observe_frame(wire, raw, startup) {
                if let Ok(mut inner) = self.state.inner.lock() {
                    let batch = inner
                        .pending_event_batches
                        .entry(self.task.task_id.clone())
                        .or_insert_with(|| (self.attempt.clone(), vec![]));
                    batch.1.extend(
                        frames.map(|raw| NativeEvent::Observed(json!({"retainedRawFrame":raw}))),
                    );
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn observe_frame(
        &self,
        wire: &mut NativeWire,
        raw: Value,
        startup: bool,
    ) -> Result<(), String> {
        match wire.observe(raw.clone()) {
            Ok(events) => self.observe_all(events, startup),
            Err(error) => {
                self.observe_all(
                    vec![NativeEvent::Observed(json!({"raw":raw,"wireError":error}))],
                    startup,
                )?;
                Err(error)
            }
        }
    }

    fn observe_all(&self, events: Vec<NativeEvent>, startup: bool) -> Result<(), String> {
        observe_batch(
            events,
            |event, projected| self.observe_projected(event, startup, projected),
            |retained| {
                if let Ok(mut inner) = self.state.inner.lock() {
                    retain_event_batch(&mut inner, &self.task.task_id, &self.attempt, retained);
                }
            },
        )
    }
}

pub(super) fn observe_batch(
    events: Vec<NativeEvent>,
    mut observe: impl FnMut(NativeEvent, &mut bool) -> Result<(), String>,
    mut retain: impl FnMut(Vec<NativeEvent>),
) -> Result<(), String> {
    let mut events = events.into_iter();
    while let Some(event) = events.next() {
        let mut projected = false;
        if let Err(error) = observe(event.clone(), &mut projected) {
            let mut retained = Vec::new();
            if !projected {
                retained.push(event);
            }
            retained.extend(events);
            retain(retained);
            return Err(error);
        }
    }
    Ok(())
}

pub(super) fn retain_event_batch(
    inner: &mut super::service::Inner,
    task_id: &str,
    attempt: &Attempt,
    events: Vec<NativeEvent>,
) {
    inner
        .pending_event_batches
        .entry(task_id.to_owned())
        .or_insert_with(|| (attempt.clone(), Vec::new()))
        .1
        .extend(events);
}

fn send(
    b: &Binding<'_>,
    process: &mut Process,
    wire: &mut NativeWire,
    request: WireRequest,
    id: &str,
    startup: bool,
) -> Result<(), String> {
    b.fence()?;

    match request {
        WireRequest::Stdio(v) => process.send(&v)?,
        WireRequest::Http { method, path, body } => {
            if path == "/event" {
                process.open_events(&path)?;
            } else {
                let (status, result) = process.request_with_status(method, &path, body.as_ref())?;

                b.observe_all(vec![wire.accepted_http(id, status, result)?], startup)?;
            }
        }

        WireRequest::Websocket { path } => process.open_websocket(&path)?,
    };

    Ok(())
}

fn startup(b: &Binding<'_>, process: &mut Process, wire: &mut NativeWire) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);

    while !wire.pending_requests().is_empty() {
        b.fence()?;

        if b.active.stop.load(Ordering::SeqCst) {
            return Err("Stopped before user prompt dispatch.".into());
        }

        if Instant::now() >= deadline {
            return Err(
                "Native initialization did not acknowledge account, session and model.".into(),
            );
        }

        if let Some(raw) = process.poll()? {
            b.observe_frame(wire, raw, true)?;
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }

    Ok(())
}

fn permission_decisions(
    b: &Binding<'_>,
    process: &mut Process,
    wire: &mut NativeWire,
) -> Result<(), String> {
    permission_dispatch(&b.active, || {
        let replies = b.state.with(b.app, |i| {
            let mut replies = vec![];

            for p in i
                .permissions
                .values_mut()
                .filter(|p| p.preview.attempt_id == b.attempt.attempt_id)
            {
                if !wire.permission_pending(&p.preview.permission.request_id) {
                    p.consumed = true;

                    continue;
                }

                if !p.consumed {
                    if let Some((value, allow, freeze)) = &p.decision {
                        if *allow {
                            let lease = i
                                .freezes
                                .get(freeze)
                                .ok_or("The retained editor freeze lease is missing.")?;

                            if lease.0 != b.task.task_id
                                || lease.1 != b.attempt.attempt_id
                                || lease.2 != b.attempt.generation
                                || lease.3
                            {
                                return Err(
                                    "Editor freeze lease is fenced or already released.".into()
                                );
                            }
                        }

                        p.consumed = true;

                        replies.push((p.preview.permission.request_id.clone(), value.clone()));
                    }
                }
            }

            Ok(replies)
        })?;

        for (id, value) in replies {
            b.fence()?;

            let request = wire.permission_reply(&id, value)?;

            match request {
                WireRequest::Stdio(v) => process.send(&v)?,
                WireRequest::Http { method, path, body } => {
                    process.request(method, &path, body.as_ref())?;
                }

                WireRequest::Websocket { .. } => {
                    return Err("Unqualified permission transport.".into())
                }
            }

            b.observe(
                NativeEvent::Observed(json!({
                "type":"permission_reply","requestId":id}
                )),
                false,
            )?;
        }

        Ok(())
    })
}

fn interrupt(b: &Binding<'_>, p: &mut Process, w: &mut NativeWire) -> Result<(), String> {
    b.fence()?;

    let request = match w.kind() {
        NativeKind::Codex => WireRequest::Stdio(json!({
        "id":"stop","method":"turn/interrupt","params":{
        "threadId":w.session_id(),"turnId":w.turn_id()}
        }
        )),
        NativeKind::Claude => WireRequest::Stdio(json!({
        "type":"control_request","request_id":"stop","request":{
        "subtype":"interrupt"}
        }
        )),
        NativeKind::Pi => WireRequest::Stdio(json!({
        "id":"stop","type":"abort"}
        )),
        NativeKind::Grok => WireRequest::Stdio(json!({
        "jsonrpc":"2.0","method":"session/cancel","params":{
        "sessionId":w.session_id()}
        }
        )),
        NativeKind::Kimi => WireRequest::Http {
            method: "POST",
            path: format!("/api/v1/sessions/{}:abort", w.session_id()),
            body: Some(json!({})),
        },
        NativeKind::Kilo | NativeKind::OpenCode => WireRequest::Http {
            method: "POST",
            path: format!("/session/{}/abort", w.session_id()),
            body: Some(json!({})),
        },
        NativeKind::Agy => {
            return Err("Native PTY stop cannot establish managed tool settlement.".into())
        }
    };

    match request {
        WireRequest::Stdio(v) => p.send(&v)?,
        WireRequest::Http { method, path, body } => {
            p.request(method, &path, body.as_ref())?;
        }

        WireRequest::Websocket { .. } => unreachable!(),
    };

    Ok(())
}

pub(crate) fn execute(state: AgentRuntime, app: AppHandle, shells: Shells, id: String) {
    let active = state
        .inner
        .lock()
        .ok()
        .and_then(|i| i.active.get(&id).cloned());
    let Some(active) = active else {
        state.closing.store(true, Ordering::SeqCst);
        return;
    };

    let result = run(&state, &app, &shells, &id, &active);

    if let Ok(mut inner) = state.inner.lock() {
        inner
            .pending_finishes
            .insert(id.clone(), (active.clone(), result));
        let _ = retry_finishes(&mut inner);
    } else {
        state.closing.store(true, Ordering::SeqCst);
    }

    state.changed(&app, Some(&id));
}

fn run(
    state: &AgentRuntime,
    app: &AppHandle,
    shells: &Shells,
    id: &str,
    active: &Active,
) -> Result<(), String> {
    let (task, attempt, root, binding, shell, identity, checkpoint) = state.with(app, |i| {
        let s = i.store.as_mut().unwrap();

        let t = s.task(id)?;

        let a = t
            .attempts
            .iter()
            .find(|a| a.attempt_id == active.attempt)
            .cloned()
            .ok_or("Attempt missing.")?;

        target(s, &a.account_id, a.auth_revision, Some(&t))?;

        Ok((
            t.clone(),
            a,
            s.task_root(id)?,
            s.binding(&active.account)?,
            s.shell(id)?,
            s.directory_identity(id)?,
            s.checkpoint(id)?,
        ))
    })?;

    let cli = task
        .cli
        .ok_or("The archived native CLI family is unknown; execution is unavailable.")?;

    let b = Binding {
        state,
        app,
        task: task.clone(),
        attempt: attempt.clone(),
        active: active.clone(),
        identity,
    };

    b.fence()?;

    let lease = crate::project_write_guard::activate(
        Path::new(&task.cwd),
        &task.task_id,
        attempt.generation,
    )?;

    let mut project = RetainedProject {
        state,
        task: id.into(),
        attempt: attempt.attempt_id.clone(),
        lease: Some(lease),
        settled: false,
    };

    let admitted_intent = state.with(app, |i| {
        if state.closing.load(Ordering::SeqCst)
            || i.closing_tasks.contains(id)
            || active.stop.load(Ordering::SeqCst)
        {
            return Err("Stopped before native helper admission.".into());
        }
        let s = i.store.as_mut().unwrap();
        #[cfg(target_os = "macos")]
        {
            let mut current = s.task(id)?;
            let a = current
                .attempts
                .iter_mut()
                .find(|a| a.attempt_id == attempt.attempt_id)
                .ok_or("Attempt missing.")?;
            a.state = "spawn_intent".into();
            current.revision += 1;
            s.owned_process_intent_context(&current, &attempt)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let account = s.account(&attempt.account_id)?;
            s.helper_intent("attempt_prepare", &account, &task.cwd, Some((id, &attempt)))?;
            Ok(String::new())
        }
    })?;
    #[cfg(target_os = "macos")]
    let (ownership_marker, context) = admitted_intent;
    #[cfg(not(target_os = "macos"))]
    let ownership_marker = admitted_intent;
    #[cfg(target_os = "macos")]
    let mut operation_guard = OwnedOperation::new(state, app, context.clone());
    let prepared = Prepared::prepare(
        app,
        shells,
        &shell,
        &task.cwd,
        cli,
        Path::new(&binding.physical_root),
        active.abort.clone(),
        #[cfg(target_os = "macos")]
        Some(context),
        #[cfg(not(target_os = "macos"))]
        None,
    )?;

    let version = prepared.version().to_owned();

    let kind = prepared.kind();
    let boundary_qualified = prepared.boundary_qualified();

    let (provider, model) = model_parts(cli, &task.model)?;

    let mut arguments = kind.launch_for_model(model, provider)?.arguments;

    let transfer = attempt.continuation_method == "reviewed_transfer";

    let resume = attempt.continuation_method == "native_resume" || transfer;

    let mut session = if resume {
        checkpoint
            .as_ref()
            .ok_or("Native checkpoint missing.")?
            .native_ref
            .clone()
    } else {
        String::new()
    };

    let mut transfer_receipt = None;

    if transfer {
        let mut prior = task.clone();

        prior.history.retain(|h| h.attempt_id != attempt.attempt_id);

        prior.history_revision = prior.history.last().map(|h| h.sequence).unwrap_or(0);

        prior.generation -= 1;

        let req = state.with(app, |i| {
            super::transfer::request(
                i.store.as_ref().unwrap(),
                &prior,
                &attempt.account_id,
                attempt.auth_revision,
            )
        })?;

        let receipt = super::native_transfer::read(&root, attempt.generation)?;

        super::native_transfer::fence(&root, &req, &receipt)?;

        session = receipt.session_id.clone();

        transfer_receipt = Some((req, receipt));
    }

    let mut history_fence = None;

    if resume && !transfer {
        let c = checkpoint.as_ref().unwrap();

        if c.version != version
            || c.cwd_identity != identity
            || c.account_id != attempt.account_id
            || c.auth_revision != attempt.auth_revision
        {
            return Err(
                "Native checkpoint qualification changed; explicit handoff is required.".into(),
            );
        }
    }

    if kind == NativeKind::Claude {
        if session.is_empty() {
            let nonce = super::new_id()?;
            let variant = char::from_digit(
                (nonce[16..17].chars().next().unwrap().to_digit(16).unwrap() & 3) | 8,
                16,
            )
            .unwrap();
            session = format!(
                "{}-{}-4{}-{}{}-{}",
                &nonce[..8],
                &nonce[8..12],
                &nonce[13..16],
                variant,
                &nonce[17..20],
                &nonce[20..]
            );

            arguments.extend(["--session-id".into(), session.clone()]);
        } else {
            arguments.extend(["--resume".into(), session.clone()]);
        }
    }

    let session_file = if kind == NativeKind::Pi {
        let name = if transfer {
            transfer_receipt
                .as_ref()
                .unwrap()
                .1
                .destination_file
                .clone()
        } else if resume {
            checkpoint
                .as_ref()
                .and_then(|c| c.session_file.clone())
                .ok_or("Pi session file binding missing.")?
        } else {
            format!("pi-session-{}.jsonl", attempt.generation)
        };

        if !name
            .strip_prefix("pi-session-")
            .and_then(|v| v.strip_suffix(".jsonl"))
            .is_some_and(|v| v.parse::<u64>().is_ok())
        {
            return Err("Invalid native Pi file binding.".into());
        }

        let path = root.join(&name);

        if resume {
            history_fence = Some(super::native_history::pi_file(
                &root, &path, &session, &task.cwd,
            )?);
        }

        arguments.extend(["--session".into(), path.to_string_lossy().into_owned()]);

        Some(name)
    } else {
        None
    };

    if resume && kind != NativeKind::Pi {
        history_fence = Some(super::native_history::select_account(
            Path::new(&binding.physical_root),
            cli,
            &version,
            &task.cwd,
            &session,
        )?);
    }

    let mut wire = NativeWire::new(kind, &session, &attempt.attempt_id)?;

    #[cfg(not(target_os = "macos"))]
    let ownership_marker = state.with(app, |i| {
        let s = i.store.as_mut().unwrap();

        let mut t = s.task(id)?;

        let a = t
            .attempts
            .iter_mut()
            .find(|a| a.attempt_id == attempt.attempt_id)
            .unwrap();

        a.state = "spawn_intent".into();

        t.revision += 1;

        s.process_intent(&t, &attempt)
    })?;

    let mut created_group = None;

    let spawned = prepared.spawn(
        &arguments,
        &ownership_marker,
        || {
            b.fence()?;

            if active.stop.load(Ordering::SeqCst) {
                return Err("Stopped before native launch.".into());
            }

            if let Some(fence) = &history_fence {
                fence.fence()?;
            }

            if let Some((req, receipt)) = &transfer_receipt {
                super::native_transfer::fence(&root, req, receipt)?;
            }

            Ok(())
        },
        |group| {
            created_group = Some(group);

            state.with(app, |i| {
                i.store
                    .as_mut()
                    .unwrap()
                    .process_spawned(&attempt.attempt_id, group)
            })
        },
    );

    let mut process = match spawned {
        Ok(process) => process,
        Err(error) => {
            if created_group.is_none() {
                #[cfg(target_os = "macos")]
                let no_native_attempt = {
                    operation_guard.finish()?;
                    operation_guard.no_attempt_created()?
                };
                #[cfg(not(target_os = "macos"))]
                let no_native_attempt = true;
                state.with(app, |i| {
                    let s = i.store.as_mut().unwrap();

                    let mut t = s.task(id)?;

                    let a = t
                        .attempts
                        .iter_mut()
                        .find(|a| a.attempt_id == attempt.attempt_id)
                        .unwrap();

                    if no_native_attempt {
                        a.state = "rejected".into();
                        a.effects_state = "settled".into();
                        s.process_not_spawned(&attempt.attempt_id)?;
                    } else {
                        // The adapter may fail after creating/releasing its
                        // journal and before reporting a process identity.
                        a.state = "spawn_intent".into();
                        a.effects_state = "uncertain".into();
                    }

                    s.update_task(&t, None, None)
                })?;
            }

            return Err(error);
        }
    };

    for request in wire.initialize("initialize")? {
        send(&b, &mut process, &mut wire, request, "initialize", true)?;
    }

    startup(&b, &mut process, &mut wire)?;

    if let Some(request) = wire.authentication_request("authenticate")? {
        send(&b, &mut process, &mut wire, request, "authenticate", true)?;

        startup(&b, &mut process, &mut wire)?;
    }

    if let Some(request) = wire.initialized() {
        send(&b, &mut process, &mut wire, request, "initialized", true)?;
    }

    process.validate_configuration(
        |frames| b.observe_frames(&mut wire, frames, true),
        || active.stop.load(Ordering::SeqCst),
    )?;

    if !matches!(kind, NativeKind::Claude | NativeKind::Pi) {
        let r = wire.session_request("session", &task.cwd, resume)?;

        send(&b, &mut process, &mut wire, r, "session", true)?;

        startup(&b, &mut process, &mut wire)?;
    }

    for request in
        wire.configure_model("model", model, provider, task.reasoning_effort.as_deref())?
    {
        send(&b, &mut process, &mut wire, request, "model", true)?;
    }

    startup(&b, &mut process, &mut wire)?;

    if matches!(
        kind,
        NativeKind::Kimi | NativeKind::Kilo | NativeKind::OpenCode
    ) {
        let request = wire.event_stream()?;

        send(&b, &mut process, &mut wire, request, "events", true)?;

        if kind == NativeKind::Kimi {
            process.send(&wire.websocket_hello("hello")?)?;

            let deadline = Instant::now() + Duration::from_secs(30);

            while !wire.subscription_ready() {
                b.fence()?;

                if Instant::now() > deadline {
                    return Err("Native journal subscription was not acknowledged. No prompt was dispatched.".into());
                }

                if let Some(raw) = process.poll()? {
                    b.observe_frame(&mut wire, raw, true)?;
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    if !wire.ready_for_prompt() {
        return Err("Native model/account initialization was not confirmed.".into());
    }

    let prompt = if attempt.continuation_method == "handoff" {
        let mut prior = task.clone();

        prior.history.retain(|h| h.attempt_id != attempt.attempt_id);

        prior.history_revision = prior.history.last().map(|h| h.sequence).unwrap_or(0);

        format!(
            "{}\nCurrent user input: {}",
            handoff(&prior)?,
            attempt.input
        )
    } else {
        attempt.input.clone()
    };

    if prompt.len() > super::service::CONTEXT_BUDGET {
        return Err("The complete context plus input exceeds the qualified handoff budget; no truncation occurred.".into());
    }

    let request = wire.prompt(&uuid(&attempt.attempt_id)?, &prompt)?;

    state.with(app, |i| {
        let s = i.store.as_mut().unwrap();

        let mut t = s.task(id)?;

        target(s, &attempt.account_id, attempt.auth_revision, Some(&t))?;

        if active.stop.load(Ordering::SeqCst) {
            return Err("Stopped before user prompt dispatch.".into());
        }

        t.state = TaskState::Running;

        t.status_message = "Native account is running with its own tools and permissions.".into();

        let a = t
            .attempts
            .iter_mut()
            .find(|a| a.attempt_id == attempt.attempt_id)
            .unwrap();

        a.state = "dispatched".into();

        a.version = Some(version.clone());

        a.native_ref = Some(wire.session_id().into());

        t.revision += 1;

        for sw in &mut t.switches {
            if sw.phase == "dispatch_pending" {
                sw.phase = "target_running".into();
            }
        }

        s.update_task(&t, None, Some((&attempt.operation_id, "claimed")))
    })?;

    send(
        &b,
        &mut process,
        &mut wire,
        request,
        &uuid(&attempt.attempt_id)?,
        false,
    )?;

    let deadline = Instant::now() + Duration::from_secs(3600);

    let mut stop_deadline = None;

    while wire.terminal().is_none() {
        b.fence()?;

        project
            .lease
            .as_ref()
            .unwrap()
            .check(id, attempt.generation)?;

        if active.stop.load(Ordering::SeqCst) && stop_deadline.is_none() {
            interrupt(&b, &mut process, &mut wire)?;

            stop_deadline = Some(Instant::now() + Duration::from_secs(10));
        }

        if Instant::now() >= deadline || stop_deadline.is_some_and(|d| Instant::now() >= d) {
            process.stop()?;

            return Err("Stopped native process without a settled tool terminal. Review retained history and project effects before continuing.".into());
        }

        if let Some(raw) = process.poll()? {
            b.observe_frame(&mut wire, raw, false)?;
        } else if process.exited() {
            return Err("Native client ended without a correlated terminal; delivery and effects require recovery.".into());
        } else {
            thread::sleep(Duration::from_millis(10));
        }

        permission_decisions(&b, &mut process, &mut wire)?;
    }

    let terminal = wire.terminal().cloned().ok_or("Native terminal missing.")?;

    if matches!(
        kind,
        NativeKind::Kimi | NativeKind::Kilo | NativeKind::OpenCode
    ) {
        let deadline = Instant::now() + Duration::from_secs(30);

        loop {
            if Instant::now() > deadline {
                return Err(
                    "Native terminal journal could not reconcile its snapshot watermark.".into(),
                );
            }

            let mut views = vec![];

            for request in wire.idle_snapshot_requests()? {
                let WireRequest::Http { method, path, body } = request else {
                    return Err("Unqualified snapshot transport.".into());
                };

                b.fence()?;

                views.push(process.request(method, &path, body.as_ref())?);
            }

            if let Some((target, epoch)) = wire.idle_snapshot_watermark(&views)? {
                loop {
                    let (observed, observed_epoch) = wire
                        .observed_journal_cursor()
                        .ok_or("Native journal cursor missing.")?;

                    if epoch != observed_epoch {
                        return Err("Native journal epoch changed.".into());
                    }

                    if observed >= target {
                        break;
                    }

                    if Instant::now() > deadline {
                        return Err("Native journal failed to reach idle watermark.".into());
                    }

                    if let Some(raw) = process.poll()? {
                        b.observe_frame(&mut wire, raw, false)?;
                    } else {
                        thread::sleep(Duration::from_millis(10));
                    }
                }

                if wire.observed_journal_cursor() != Some((target, epoch)) {
                    continue;
                }
            }

            wire.validate_idle_snapshots(&task.cwd, &views)?;

            break;
        }
    }

    b.observe_frames(&mut wire, process.drain()?, false)?;

    state.with(app,|i| {
       let t=i.store.as_ref().unwrap().task(id)?;
       if unqualified_observed_tools(&t,&attempt.attempt_id,&i.permissions) {
          i.store.as_mut().unwrap().process_untracked(&attempt.attempt_id)?;
          return Err("Native external, asynchronous or scrubbed-environment tool effects lack qualified process ownership; recovery remains fenced.".into());
       }
        if !boundary_qualified {
            return Err("This CLI's background or lifecycle work cannot yet be verified as stopped. Ownership remains protected; effects acknowledgement cannot release it before a verified OS restart.".into());
        }
        Ok(())
    })?;
    #[cfg(not(target_os = "macos"))]
    super::process_supervision::stop(&ownership_marker)?;
    if !process.drained()
        || wire
            .tools()
            .iter()
            .any(|t| matches!(t.state, ToolState::Started | ToolState::Pending))
    {
        return Err("Native descendants, tools or observations remain unsettled.".into());
    }

    state.with(app, |inner| {
        if inner.permissions.values().any(|permission| {
            permission.preview.attempt_id == attempt.attempt_id
                && wire.permission_pending(&permission.preview.permission.request_id)
        }) {
            return Err("Pending native permissions remain unresolved.".into());
        }
        // Local coalition retirement alone cannot discharge unknown effects.
        // This point requires qualified configuration, all correlated native
        // observations committed, a complete transport drain and idle tools.
        inner
            .store
            .as_mut()
            .unwrap()
            .clear_lifecycle_boundary(&attempt.attempt_id)
    })?;
    #[cfg(target_os = "macos")]
    operation_guard.finish()?;

    state.with(app,|i|{
let pending=i.permissions.values().any(|p|p.preview.attempt_id==attempt.attempt_id&&wire.permission_pending(&p.preview.permission.request_id));

if pending{
return Err("Pending native permissions remain unresolved.".into());

}
let s=i.store.as_mut().unwrap();
s.reconcile_processes()?;
if !s.processes_settled(id)? || !s.helpers_settled(Some(id))? {return Err("Native tool process inheritance or descendant ownership remains unqualified; checkpoint and project release are fenced.".into());}

let mut t=s.task(id)?;

target(s,&attempt.account_id,attempt.auth_revision,Some(&t))?;

let a=t.attempts.iter_mut().find(|a|a.attempt_id==attempt.attempt_id).unwrap();

a.native_ref=Some(wire.session_id().into());

a.effects_state="settled".into();

a.state=match terminal.outcome{
NativeOutcome::Completed=>"completed",NativeOutcome::Cancelled=>"stopped",_=>"recovery_required"}
.into();

t.state=match terminal.outcome{
NativeOutcome::Completed=>TaskState::Completed,NativeOutcome::Cancelled=>TaskState::Stopped,_=>TaskState::RecoveryRequired}
;

t.revision+=1;

t.status_message="Native terminal, tool records, event journal and descendants drained and checkpointed.".into();

let c=HistoryCheckpoint{
task_id:id.into(),attempt_id:attempt.attempt_id.clone(),account_id:attempt.account_id.clone(),auth_revision:attempt.auth_revision,generation:attempt.generation,history_revision:t.history_revision,digest:history_digest(&t)?,version:version.clone(),model:task.model.clone(),cwd_identity:identity,native_ref:wire.session_id().into(),session_file,settled:true}
;

s.update_task(&t,Some(&c),Some((&attempt.operation_id,"settled")))}
)?;

    project.settled = true;

    drop(project);

    Ok(())
}

fn uuid(id: &str) -> Result<String, String> {
    if id.len() != 32 || !id.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err("Invalid native prompt ID.".into());
    }

    let mut bytes = id.as_bytes().to_vec();

    bytes[12] = b'4';

    bytes[16] = b'8';

    let s = std::str::from_utf8(&bytes).map_err(|_| "Invalid prompt ID.")?;

    Ok(format!(
        "{}-{}-{}-{}-{}",
        &s[..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..]
    ))
}

struct RetainedProject<'a> {
    state: &'a AgentRuntime,
    task: String,
    attempt: String,
    lease: Option<crate::project_write_guard::Lease>,
    settled: bool,
}

impl Drop for RetainedProject<'_> {
    fn drop(&mut self) {
        if self.settled {
            return;
        }

        let Some(lease) = self.lease.take() else {
            return;
        };

        match self.state.inner.lock() {
            Ok(mut i) => {
                let helper_unknown = i
                    .store
                    .as_ref()
                    .and_then(|s| s.helpers_settled(Some(&self.task)).ok())
                    .is_none_or(|settled| !settled);
                let retain = helper_unknown
                    || i.store
                        .as_ref()
                        .and_then(|s| s.task(&self.task).ok())
                        .map(|t| {
                            t.attempts.iter().any(|a| {
                                a.attempt_id == self.attempt
                                    && matches!(
                                        a.state.as_str(),
                                        "spawn_intent" | "dispatched" | "running" | "stopping"
                                    )
                            })
                        })
                        .unwrap_or(true);
                if retain {
                    i.recovery_projects.insert(self.task.clone(), lease);
                }
            }
            Err(_) => std::mem::forget(lease),
        }
    }
}

pub(super) fn untracked_effect(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            (matches!(
                key.as_str(),
                "background"
                    | "run_in_background"
                    | "runInBackground"
                    | "detached"
                    | "async"
                    | "clean_env"
                    | "cleanEnv"
                    | "scrubEnvironment"
            ) && value == &Value::Bool(true))
                || untracked_effect(value)
        }),
        Value::Array(items) => items.iter().any(untracked_effect),
        _ => false,
    }
}

pub(super) fn permission_dispatch(
    active: &Active,
    dispatch: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let _control = active
        .control
        .lock()
        .map_err(|_| "Task control unavailable.")?;
    if active.stop.load(Ordering::SeqCst) {
        return Ok(());
    }
    dispatch()
}

pub(super) fn tool_ownership_unqualified(kind: NativeKind, tool: &NativeTool) -> bool {
    if untracked_effect(&tool.raw) {
        return true;
    }
    let name = tool.raw["name"]
        .as_str()
        .or_else(|| tool.raw["toolName"].as_str())
        .or_else(|| tool.raw["tool_name"].as_str())
        .or_else(|| tool.raw.pointer("/part/tool").and_then(Value::as_str))
        .or_else(|| {
            tool.raw
                .pointer("/content_block/name")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            tool.raw
                .pointer("/request/tool_name")
                .and_then(Value::as_str)
        });
    if kind == NativeKind::Codex
        && tool
            .raw
            .pointer("/params/item/type")
            .and_then(Value::as_str)
            == Some("fileChange")
    {
        return false;
    }
    // Plugin registries can shadow built-in names (including Pi read/write).
    // A name alone never attests that extensible drivers ran an in-process tool.
    if kind != NativeKind::Claude && kind != NativeKind::Pi {
        return true;
    }
    !matches!(
        name,
        Some(
            "Read"
                | "Edit"
                | "Write"
                | "read"
                | "edit"
                | "write"
                | "TodoRead"
                | "TodoWrite"
                | "AskUserQuestion"
                | "EnterPlanMode"
                | "ExitPlanMode"
        )
    )
}

pub(super) fn retry_finishes(inner: &mut super::service::Inner) -> Result<(), String> {
    retry_observations(inner)?;
    let work = inner
        .pending_finishes
        .iter()
        .map(|(id, (a, r))| (id.clone(), a.clone(), r.clone()))
        .collect::<Vec<_>>();
    for (id, active, result) in work {
        finish_record(inner, &id, &active, &result)?;
    }
    Ok(())
}
fn finish_record(
    i: &mut super::service::Inner,
    id: &str,
    active: &Active,
    result: &Result<(), String>,
) -> Result<(), String> {
    let s = i.store.as_mut().unwrap();

    let mut t = s.task(id)?;
    if unqualified_observed_tools(&t, &active.attempt, &i.permissions) {
        s.process_untracked(&active.attempt)?;
    }

    if t.generation == active.generation && t.active_attempt_id.as_deref() == Some(&active.attempt)
    {
        if let Err(message) = &result {
            let dispatched = t
                .attempts
                .iter()
                .find(|a| a.attempt_id == active.attempt)
                .is_some_and(|a| {
                    matches!(a.state.as_str(), "spawn_intent" | "running" | "dispatched")
                });

            t.state = if dispatched {
                TaskState::RecoveryRequired
            } else {
                TaskState::Stopped
            };

            t.status_message = message.clone();

            if let Some(a) = t
                .attempts
                .iter_mut()
                .find(|a| a.attempt_id == active.attempt)
            {
                a.state = if dispatched {
                    "recovery_required"
                } else {
                    "rejected"
                }
                .into();

                a.effects_state = if dispatched { "uncertain" } else { "settled" }.into();
            }

            t.revision += 1;
        }

        t.active_attempt_id = None;

        t.active_account_id = None;

        super::service::settle_switches(s, &mut t)?;

        for sw in &mut t.switches {
            if sw.phase == "target_running" || sw.phase == "dispatch_pending" {
                sw.phase = if result.is_ok() {
                    "completed"
                } else {
                    "recovery_required"
                }
                .into();
            }
        }

        let operation = t
            .attempts
            .iter()
            .find(|a| a.attempt_id == active.attempt)
            .ok_or("Attempt lost.")?
            .operation_id
            .clone();
        let phase = if result.is_ok() {
            "settled"
        } else {
            "recovery"
        };
        s.update_task(&t, None, Some((&operation, phase)))?;
    }

    active.worker_finished.store(true, Ordering::SeqCst);
    i.pending_finishes.remove(id);
    i.permissions
        .retain(|_, p| p.preview.attempt_id != active.attempt);

    for freeze in i.freezes.values_mut() {
        if freeze.1 == active.attempt && result.is_ok() {
            freeze.3 = true;
        }
    }

    s.reconcile_processes()?;

    if !s.processes_settled(id)? || !s.helpers_settled(Some(id))? {
        return Err("Native process group ownership remains unresolved; shutdown and binding changes are fenced.".into());
    }

    i.active.remove(id);

    i.account_leases.remove(&active.account);

    active.done.store(true, Ordering::SeqCst);
    Ok(())
}

pub(super) fn unqualified_observed_tools(
    task: &Task,
    attempt: &str,
    permissions: &std::collections::HashMap<String, Permission>,
) -> bool {
    let Some(kind) = task.cli.and_then(NativeKind::from_cli) else {
        return false;
    };
    task.history
        .iter()
        .filter(|h| h.attempt_id == attempt && h.kind == "tool")
        .filter_map(|h| serde_json::from_value::<NativeTool>(h.content.clone()).ok())
        .any(|tool| {
            let explicitly_denied = permissions.values().any(|p| {
                p.preview.attempt_id == attempt
                    && p.preview.permission.tool_id.as_deref() == Some(tool.tool_id.as_str())
                    && p.consumed
                    && p.decision.as_ref().is_some_and(|(_, allow, _)| !allow)
            });
            !explicitly_denied && tool_ownership_unqualified(kind, &tool)
        })
}

pub(super) fn retry_observations(inner: &mut super::service::Inner) -> Result<(), String> {
    let projections = inner
        .pending_observations
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for task in projections {
        inner
            .store
            .as_mut()
            .ok_or("Runtime storage unavailable.")?
            .update_task(&task, None, None)?;
        inner.pending_observations.remove(&task.task_id);
    }
    let batches = inner
        .pending_event_batches
        .iter()
        .map(|(id, (a, e))| (id.clone(), a.clone(), e.clone()))
        .collect::<Vec<_>>();
    for (id, attempt, events) in batches {
        let s = inner.store.as_mut().ok_or("Runtime storage unavailable.")?;
        let mut task = s.task(&id)?;
        if task.generation != attempt.generation {
            return Err("Retained native observation belongs to a different generation; recovery remains fenced.".into());
        }
        for event in events {
            if let NativeEvent::Text { text, .. } = &event {
                if let Some(a) = task
                    .attempts
                    .iter_mut()
                    .find(|a| a.attempt_id == attempt.attempt_id)
                {
                    a.output.push_str(text);
                }
            }
            if let NativeEvent::Tool(tool) = &event {
                if task
                    .cli
                    .and_then(NativeKind::from_cli)
                    .is_some_and(|kind| tool_ownership_unqualified(kind, tool))
                {
                    s.process_tool_untracked(&attempt.attempt_id, &tool.tool_id)?;
                }
            }
            append(
                &mut task,
                &attempt,
                "retained_native_observation",
                "observed",
                serde_json::to_value(event).map_err(|_| "Cannot retain native observation.")?,
            );
        }
        task.revision += 1;
        s.update_task(&task, None, None)?;
        inner.pending_event_batches.remove(&id);
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn native_boundary_qualified(_kind: NativeKind) -> bool {
    // No admitted native artifact/configuration yet proves all lifecycle and
    // dynamic configuration helpers, including standard Pi's auth/models and
    // package manager callbacks. Tool-name classification cannot clear this.
    false
}
