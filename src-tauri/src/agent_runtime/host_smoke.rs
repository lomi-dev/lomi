//! Account-free native desktop fixture. The launch and settlement paths below
//! are the same production modules; only the synthetic data is smoke-only.
use super::{
    native_launch::{self, Purpose},
    owned_operation::OwnedOperation,
    service::{history_digest, AgentRuntime, Inner},
    store::{workspace, Store},
    types::*,
};
use crate::cli_catalog::TitleCli;
use serde_json::{json, Value};
use std::{
    io::Read,
    os::unix::fs::DirBuilderExt,
    path::Path,
    process::Command,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) fn run(app: &tauri::AppHandle, fixture: &Path) -> Result<Value, String> {
    let root = fixture.join("owned-production-entry");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .map_err(|e| e.to_string())?;
    let project = root.join("project");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&project)
        .map_err(|e| e.to_string())?;
    let (cwd, identity) = workspace(&project)?;
    let mut store = Store::open(root.join("owner"))?;
    let account_root = store.root.join("accounts").join("offline-host-fixture");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&account_root)
        .map_err(|e| e.to_string())?;
    let account = AccountInstance {
        account_id: "offline-host-fixture".into(),
        cli: TitleCli::Codex,
        label: "Synthetic local lifecycle fixture".into(),
        enabled: true,
        revision: 1,
        auth_revision: 1,
        auth_state: "unverified".into(),
        availability_reason: None,
        accepted_version: Some("0.160.0".into()),
        recovery: None,
    };
    let binding = CredentialBinding {
        account_id: account.account_id.clone(),
        auth_revision: 1,
        physical_root: account_root.to_string_lossy().into_owned(),
        namespace: "codex".into(),
        credential_reference: "native-managed".into(),
    };
    store.save_account(
        &json!({}),
        "offline-create-account",
        &account,
        &binding,
        &json!({}),
    )?;
    let mut task: Task = serde_json::from_value(json!({
        "taskId":"offline-production-task", "cwd":cwd,
        "title":"Local production entry fixture", "cli":"codex",
        "availabilityReason":null, "model":"gpt-6.1-sol", "reasoningEffort":null,
        "revision":1, "historyRevision":0, "generation":1, "state":"starting",
        "nextAccountId":account.account_id, "activeAccountId":account.account_id,
        "activeAttemptId":"offline-production-attempt", "statusMessage":"",
        "attempts":[{
            "attemptId":"offline-production-attempt", "operationId":"offline-production-operation",
            "accountId":account.account_id, "authRevision":1, "generation":1,
            "input":"", "continuationMethod":"local-fixture", "state":"spawn_intent",
            "output":"", "nativeRef":null, "version":"0.160.0", "effectsState":"unsettled"
        }], "history":[], "grants":[], "switches":[]
    }))
    .map_err(|e| e.to_string())?;
    store.create_task(
        &json!({}),
        "offline-create-task",
        &task,
        "local:sh",
        identity,
    )?;
    store.owned_process_intent(&task, &task.attempts[0])?;
    let context = store.host_context("offline-production-operation")?;
    let state = AgentRuntime {
        inner: Arc::new(Mutex::new(Inner {
            store: Some(store),
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut operation = OwnedOperation::new(&state, app, context.clone());
    let mut command = Command::new("/usr/bin/printf");
    command
        .arg("production-worker-and-runner\\n")
        .current_dir(&project)
        .env_clear()
        .env("HOME", &account_root)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "en_US.UTF-8");
    let mut child = native_launch::spawn(
        command,
        Some(&context),
        TitleCli::Codex,
        Purpose::Attempt,
        None,
        &[],
        &[],
        None,
        &AtomicBool::new(false),
        || {
            state.with(app, |inner| {
                inner
                    .store
                    .as_ref()
                    .unwrap()
                    .host_context("offline-production-operation")?;
                Ok(())
            })
        },
    )?;
    state.with(app, |inner| {
        inner
            .store
            .as_mut()
            .unwrap()
            .process_spawned("offline-production-attempt", child.id())
    })?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !child.exit_pending()? {
        if Instant::now() > deadline {
            return Err("Owned native fixture did not exit.".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = child
        .stop_and_wait()?
        .ok_or("Native fixture has no confirmed exit result.")?;
    if !status.success() {
        let mut errors = String::new();
        if let Some(stderr) = child.stderr.take() {
            stderr
                .take(65536)
                .read_to_string(&mut errors)
                .map_err(|e| e.to_string())?;
        }
        return Err(format!(
            "Inert native fixture exit {:?}: {errors}",
            status.code()
        ));
    }
    let mut output = String::new();
    child
        .stdout
        .take()
        .ok_or("Native fixture stdout is absent.")?
        .read_to_string(&mut output)
        .map_err(|e| e.to_string())?;
    if output != "production-worker-and-runner\n" {
        return Err("Native fixture output differs.".into());
    }
    // Keep Child alive here: its boundary lock is still held. A positive sealed
    // receipt must complete the durable operation without reopening that lock.
    state.with(app, |inner| {
        inner
            .store
            .as_mut()
            .unwrap()
            .clear_lifecycle_boundary("offline-production-attempt")
    })?;
    operation.finish()?;
    state.with(app, |inner| {
        let store = inner.store.as_mut().unwrap();
        if !store.processes_settled(&task.task_id)?
            || !store.helpers_settled(Some(&task.task_id))?
        {
            return Err("Positive native retirement left an ownership fence.".into());
        }
        task.state = TaskState::Completed;
        task.active_account_id = None;
        task.active_attempt_id = None;
        task.attempts[0].state = "completed".into();
        task.attempts[0].effects_state = "settled".into();
        task.attempts[0].output = output.clone();
        task.attempts[0].native_ref = Some("local-production-fixture".into());
        let checkpoint = HistoryCheckpoint {
            task_id: task.task_id.clone(),
            attempt_id: task.attempts[0].attempt_id.clone(),
            account_id: account.account_id.clone(),
            auth_revision: 1,
            generation: 1,
            history_revision: 0,
            digest: history_digest(&task)?,
            version: "0.160.0".into(),
            model: task.model.clone(),
            cwd_identity: identity,
            native_ref: "local-production-fixture".into(),
            session_file: None,
            settled: true,
        };
        store.update_task(&task, Some(&checkpoint), None)?;
        if !store.checkpoint(&task.task_id)?.is_some_and(|c| c.settled) {
            return Err("Native fixture checkpoint was not durable.".into());
        }
        Ok(())
    })?;
    drop(child);
    let owned_pty = owned_pty_drain(app, &state, &project, &account)?;
    // A reopen fixture must release the first Store's exclusive owner lock.
    drop(operation);
    drop(state);
    let reopened = Store::open(root.join("owner"))?;
    if reopened.task(&task.task_id)?.state != TaskState::Completed
        || !reopened
            .checkpoint(&task.task_id)?
            .is_some_and(|c| c.settled)
    {
        return Err("Native fixture checkpoint did not survive reopening.".into());
    }
    let claude_file_store = claude_file_store(app, fixture)?;
    Ok(
        json!({"passed":true, "output":output, "root":root, "claudeFileStore":claude_file_store, "ownedPtyDrain":owned_pty,
        "checks":["normal production worker and runner execute",
            "shared native launch has a durable intent before release",
            "sealed kernel retirement finalizes while Child retains its lock",
            "owned helper and process fences settle",
            "completed checkpoint survives store reopening"],
        "inference":false, "credentials":false}),
    )
}

fn claude_file_store(app: &tauri::AppHandle, fixture: &Path) -> Result<Value, String> {
    use super::{managed_config::ManagedConfiguration, native_artifact, native_policy::Mode};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let binary = std::path::PathBuf::from(
        std::env::var("LOMI_AGENT_RUNTIME_PUBLIC_CLAUDE")
            .map_err(|_| "Public pinned Claude fixture path is missing.")?,
    );
    let proof = native_artifact::admit(TitleCli::Claude, &binary)?
        .ok_or("Public Claude fixture was not admitted.")?;
    let root = fixture.join("claude-native-file-store");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .map_err(|e| e.to_string())?;
    let project = root.join("project");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&project)
        .map_err(|e| e.to_string())?;
    let mut store = Store::open(root.join("owner"))?;
    let home = store.root.join("accounts").join("offline-claude");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&home)
        .map_err(|e| e.to_string())?;
    let mut account: AccountInstance = serde_json::from_value(json!({
        "accountId":"offline-claude", "cli":"claude", "label":"Public no-account auth fixture",
        "enabled":true, "revision":1, "authRevision":1, "authState":"unverified",
        "availabilityReason":null, "acceptedVersion":"2.1.287"
    }))
    .map_err(|e| e.to_string())?;
    let binding = CredentialBinding {
        account_id: account.account_id.clone(),
        auth_revision: 1,
        physical_root: home.to_string_lossy().into_owned(),
        namespace: "claude".into(),
        credential_reference: "native-managed".into(),
    };
    store.save_account(
        &json!({}),
        "offline-claude-create",
        &account,
        &binding,
        &json!({}),
    )?;
    let state = AgentRuntime {
        inner: Arc::new(Mutex::new(Inner {
            store: Some(store),
            ..Default::default()
        })),
        ..Default::default()
    };
    let config = ManagedConfiguration::prepare(TitleCli::Claude, "2.1.287", &home, &project, None)?;
    let mut results = Vec::new();
    for has_synthetic_file in [false, true, false] {
        let credentials = home.join(".credentials.json");
        if has_synthetic_file {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&credentials)
                .map_err(|e| e.to_string())?;
            use std::io::Write;
            file.write_all(
                &serde_json::to_vec(&json!({"claudeAiOauth":{
                    "accessToken":"owned-offline-placeholder-not-a-credential",
                    "refreshToken":"owned-offline-placeholder-refresh",
                    "expiresAt":4102444800000u64,
                    "scopes":["user:inference","user:profile"],
                    "subscriptionType":"max", "rateLimitTier":"default_claude_max_5x"
                }}))
                .unwrap(),
            )
            .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            if std::fs::metadata(&credentials)
                .map_err(|e| e.to_string())?
                .permissions()
                .mode()
                & 0o777
                != 0o600
            {
                return Err("Synthetic auth fixture was not private.".into());
            }
        } else if credentials.exists() {
            std::fs::remove_file(&credentials).map_err(|e| e.to_string())?;
        }
        let context = state.with(app, |inner| {
            let store = inner.store.as_mut().unwrap();
            let id =
                store.owned_helper_intent("verify", &account, project.to_str().unwrap(), None)?;
            store.host_context(&id)
        })?;
        let mut operation = OwnedOperation::new(&state, app, context.clone());
        let mut command = Command::new(proof.path());
        command
            .args(config.arguments(Mode::Collector)?)
            .args(["auth", "status"])
            .env_clear()
            .env("HOME", &home)
            .env("CLAUDE_CONFIG_DIR", &home)
            .env("CFFIXED_USER_HOME", &home)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("DISABLE_AUTOUPDATER", "1")
            .current_dir(&project);
        let mut child = native_launch::spawn(
            command,
            Some(&context),
            TitleCli::Claude,
            Purpose::Collector,
            Some(&config),
            &[],
            &[],
            None,
            &AtomicBool::new(false),
            || native_artifact::recheck(&proof),
        )?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while !child.exit_pending()? {
            if Instant::now() > deadline {
                return Err("Public Claude auth fixture did not exit.".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = child
            .stop_and_wait()?
            .ok_or("Public Claude fixture exit is unknown.")?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or("Public Claude stdout is absent.")?
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let value: Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                let mut errors = Vec::new();
                if let Some(stderr) = child.stderr.take() {
                    stderr
                        .take(65536)
                        .read_to_end(&mut errors)
                        .map_err(|e| e.to_string())?;
                }
                return Err(format!(
                    "Public Claude offline auth output is not JSON: {error}; exit={:?}; stdout={}; stderr={}",
                    status.code(),
                    String::from_utf8_lossy(&bytes[..bytes.len().min(65536)]),
                    String::from_utf8_lossy(&errors)
                ));
            }
        };
        if value["loggedIn"].as_bool() != Some(has_synthetic_file) {
            return Err(format!(
                "Public Claude file-store selection differs: {value}"
            ));
        }
        operation.finish()?;
        results.push(json!({"syntheticFile":has_synthetic_file,"loggedIn":value["loggedIn"],"exit":status.code()}));
        account = state.with(app, |inner| {
            inner.store.as_ref().unwrap().account("offline-claude")
        })?;
    }
    Ok(json!({"passed":true,"root":root,"results":results,
        "scope":"Exact public native auth status: absent / synthetic noncredential OAuth file / file removed; keychain denied; no auth login or network",
        "remaining":"Authenticated OAuth save, refresh and logout acceptance still requires authorized test accounts."}))
}

fn owned_pty_drain(
    app: &tauri::AppHandle,
    state: &AgentRuntime,
    project: &Path,
    account: &AccountInstance,
) -> Result<Value, String> {
    use std::sync::atomic::Ordering;
    use tauri::Manager;
    let context = state.with(app, |inner| {
        let store = inner.store.as_mut().unwrap();
        let id = store.owned_helper_intent(
            "account_terminal",
            account,
            project.to_str().unwrap(),
            None,
        )?;
        store.host_context(&id)
    })?;
    let mut operation = OwnedOperation::new(state, app, context.clone());
    let transport_done = Arc::new(AtomicBool::new(false));
    let durable_done = Arc::new(AtomicBool::new(false));
    state
        .inner
        .lock()
        .map_err(|_| "Missing smoke owner.")?
        .account_terminal_finishes
        .insert(context.parent_operation_id.clone(), durable_done.clone());
    let mut builder = portable_pty::CommandBuilder::new("/bin/cat");
    builder.cwd(project);
    let physical_home = context.physical_account_root.clone();
    let physical_project = project.to_owned();
    let launch = crate::terminal::NativeTerminalLaunch {
        command: (builder, project.to_string_lossy().into_owned()),
        admission: Box::new(|| Ok(())),
        owned_boundary: Some(Box::new(move |size| {
            let mut command = Command::new("/bin/cat");
            command
                .env_clear()
                .env("HOME", &physical_home)
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .current_dir(&physical_project);
            native_launch::spawn_pty(
                command,
                &context,
                TitleCli::Codex,
                None,
                &[],
                &[],
                &AtomicBool::new(false),
                size,
                || Ok(()),
            )
        })),
    };
    let terminals = app.state::<crate::terminal::Terminals>();
    let id = terminals.start_owned_smoke(app, launch, transport_done.clone())?;
    let done = durable_done.clone();
    let thread = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(35);
        while !transport_done.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                return Err("Owned PTY fixture transport did not retire.".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Deterministically expose the old transport-before-durable race.
        std::thread::sleep(Duration::from_millis(250));
        let result = operation.finish();
        done.store(true, Ordering::SeqCst);
        result
    });
    let result = state.drain_all(app);
    let finish = thread
        .join()
        .map_err(|_| "Owned PTY fixture finalizer panicked.")?;
    result?;
    finish?;
    if !durable_done.load(Ordering::SeqCst) {
        return Err("PTY drain skipped durable finalization.".into());
    }
    terminals.close(&id)?;
    state.with(app, |inner| {
        if !inner.store.as_ref().unwrap().helpers_settled(None)? {
            return Err("Owned PTY helper remained unresolved after application drain.".into());
        }
        Ok(())
    })?;
    Ok(
        json!({"passed":true,"checks":["owned interactive PTY stopped by application drain",
        "transport completion precedes delayed durable helper finalization",
        "application drain waits finalization token and positive helper ledger"]}),
    )
}
