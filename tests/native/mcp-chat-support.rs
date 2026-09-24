//! Private, provider-free SQLite and native MCP history qualification.
use super::*;

pub(super) async fn prepare(
    app: &tauri::AppHandle,
    main: &Webview,
    settings: &Webview,
    workspace: &Value,
    directory: &Path,
) -> Result<(), String> {
    let origin = javascript(settings, &format!(r#"
        const s=await window.__TAURI_INTERNALS__.invoke('agent_control_state');
        const w=s.broker.workspaces.find(w=>w.id==={workspace});
        return {{projectId:w.projectId,projectName:w.projectName,workspaceId:w.id,workspaceName:w.name}};
    "#)).await?;
    javascript(main, &format!(r#"
        const call=input=>window.__TAURI_INTERNALS__.invoke('chat_main',{{input}});
        const origin={origin};
        for(const id of ['native-shared-a','native-shared-b','native-private','native-foreign']){{
          const c=await call({{action:'create',id,origin:{{...origin,projectId:id==='native-foreign'?'other-project':origin.projectId}}}});
          await call({{action:'rename',id,title:id}});
          if(id==='native-shared-a'){{
            const current=await call({{action:'load',id}});
            await call({{action:'configure',id,expected:current.conversation.revision,config:{{...c.config,system:'PRIVATE_SYSTEM_FIXTURE'}}}});
            await call({{action:'draft',id,text:'A🙂B',expected:0}});
          }}
        }}
        try{{await window.__TAURI_INTERNALS__.invoke('agent_control_chat_catalog',{{projectId:origin.projectId,afterId:null}});return false;}}catch{{return true;}}
    "#)).await?.as_bool().filter(|v| *v).ok_or("Main can discover private history through the Settings picker")?;
    {
        let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
        let services = backend.services.lock().map_err(|_| "Chat storage lock")?;
        let store = services.store.as_ref().map_err(Clone::clone)?;
        store.connection.execute("INSERT INTO messages(id,conversation_id,role,parts,status,metadata) VALUES('native-message','native-shared-a','assistant',?1,'completed',?2)",rusqlite::params![r#"[{"type":"text","text":"Saved answer 🙂"},{"type":"reasoning","text":"PRIVATE_REASONING_FIXTURE"}]"#,r#"{"credentialRevision":"PRIVATE_CREDENTIAL_FIXTURE"}"#]).map_err(|e|e.to_string())?;
        store
            .connection
            .execute(
                "UPDATE conversations SET active_leaf='native-message' WHERE id='native-shared-a'",
                [],
            )
            .map_err(|e| e.to_string())?;
        store.connection.execute("INSERT INTO messages(id,conversation_id,role,parts,status) VALUES('private-message','native-private','user','[]','completed')",[]).map_err(|e|e.to_string())?;
    }
    let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
    let (locked, ready) = tokio::sync::oneshot::channel();
    let contention = tauri::async_runtime::spawn_blocking(move || {
        let _services = backend.services.lock().unwrap();
        let _ = locked.send(());
        std::thread::sleep(Duration::from_millis(200));
    });
    ready.await.map_err(|e| e.to_string())?;
    evaluate(settings,"[...document.querySelectorAll('.agent-control-request label')].find(l=>l.textContent.includes('Allow reading selected Chat AI conversations')).querySelector('input').click();true").await?;
    if let Err(error) = wait_for(settings,"[...document.querySelectorAll('fieldset label')].some(l=>l.textContent.includes('native-shared-a'))").await {
        let ui=javascript(settings,"return document.body.innerText.slice(-16000);").await?;
        let native=crate::chat::agent::catalog(app,origin["projectId"].as_str().ok_or("Missing fixture project")?,None);
        return Err(format!("{error}; Settings={ui}; native catalog={native:?}"));
    }
    contention.await.map_err(|e| e.to_string())?;
    if evaluate(
        settings,
        "document.querySelector('.agent-control-request').textContent.includes('native-foreign')",
    )
    .await?
        != false
    {
        return Err("Settings picker disclosed another project's conversation".into());
    }
    for id in ["native-shared-a", "native-shared-b"] {
        evaluate(settings,&format!("[...document.querySelectorAll('fieldset label')].find(l=>l.querySelector('code')?.textContent==={}).querySelector('input').click();true",json!(id))).await?;
    }
    wait_for(
        settings,
        "document.body.textContent.includes('2 of 64 conversations selected')",
    )
    .await?;
    javascript(settings,"[...document.querySelectorAll('fieldset')].find(f=>f.querySelector('legend')?.textContent==='Conversations to share').scrollIntoView({block:'center'});return true;").await?;
    if std::env::var_os("LOMI_MCP_CHAT_OPEN_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_CHAT_DRAFT_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some()
    {
        for text in [
            "Allow opening selected Chat AI conversations",
            "Allow creating Chat AI conversations",
        ] {
            wait_for(settings,&format!("![...document.querySelectorAll('.agent-control-request label')].find(l=>l.textContent.includes({})).querySelector('input').disabled",json!(text))).await?;
            evaluate(settings,&format!("[...document.querySelectorAll('.agent-control-request label')].find(l=>l.textContent.includes({})).querySelector('input').click();true",json!(text))).await?;
        }
    }
    if std::env::var_os("LOMI_MCP_CHAT_DRAFT_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some()
    {
        evaluate(settings,"[...document.querySelectorAll('.agent-control-request label')].find(l=>l.textContent.includes('Allow editing selected Chat AI drafts')).querySelector('input').click();true").await?;
    }
    if std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some() {
        evaluate(settings,"[...document.querySelectorAll('.agent-control-request label')].find(l=>l.textContent.includes('Allow sending selected Chat AI messages')).querySelector('input').click();true").await?;
        evaluate(settings,"[...document.querySelectorAll('.agent-control-request label')].find(l=>l.textContent.includes('Allow stopping selected Chat AI responses')).querySelector('input').click();true").await?;
        evaluate(settings,"[...document.querySelectorAll('.agent-control-request label')].find(l=>l.textContent.includes('Allow exporting selected Chat AI text')).querySelector('input').click();true").await?;
    }
    screenshot(settings, directory.join("chat-grant.png")).await?;
    Ok(())
}

fn changes(app: &tauri::AppHandle) -> Result<u64, String> {
    let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
    let services = backend.services.lock().map_err(|_| "Chat storage lock")?;
    Ok(services
        .store
        .as_ref()
        .map_err(Clone::clone)?
        .connection
        .total_changes())
}

async fn qualify_open(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    directory: &Path,
) -> Result<Vec<&'static str>, String> {
    let connected = wire
        .tool("lomi_connect", json!({"workspaceId":workspace}))
        .await?;
    let epoch = &connected["structuredContent"]["data"]["retryEpoch"];
    let mut opened: Vec<Value> = Vec::new();
    for (index, target) in [
        json!({"type":"existing","conversationId":"native-shared-a"}),
        json!({"type":"existing","conversationId":"native-shared-a"}),
        json!({"type":"new"}),
    ]
    .into_iter()
    .enumerate()
    {
        let domain = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"workspaceId":workspace,"target":target,"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":epoch,"requestKey":format!("chat-open-{index}")});
        let receipt = wire.tool("lomi_chat_open", args.clone()).await?;
        let id = receipt["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| format!("Chat open receipt: {receipt}"))?;
        let result = wire.settled(id).await?;
        if result["structuredContent"]["data"]["state"] != "succeeded" {
            return Err(format!("Chat open {index}: {result}"));
        }
        let value = result["structuredContent"]["data"]["result"].clone();
        let retry = wire.tool("lomi_chat_open", args).await?;
        if retry["structuredContent"]["data"] != result["structuredContent"]["data"] {
            return Err("Chat open retry changed its receipt".into());
        }
        if index == 0 {
            if value["conversationId"] != "native-shared-a" || value["created"] != false {
                return Err("Opened wrong existing chat".into());
            }
            javascript(main,"const r=(await import('/src/chat/chat-runtime.ts')).getChat('native-shared-a');window.__chatRetained=r;window.__chatSdk=r.chat;r.setText('Human retained draft 🙂');return true;").await?;
        } else {
            let retained=javascript(main,"const r=(await import('/src/chat/chat-runtime.ts')).getChat('native-shared-a');return r===window.__chatRetained && r.chat===window.__chatSdk && r.snapshot.text==='Human retained draft 🙂';").await?;
            if retained != true {
                return Err("Chat open replaced the retained SDK or human draft".into());
            }
            if index == 1 && value["panelId"] != opened[0]["panelId"] {
                return Err("Chat open duplicated an existing view".into());
            }
            if index == 2
                && (value["created"] != true || value["conversationId"] == "native-shared-a")
            {
                return Err("New chat reused another conversation".into());
            }
        }
        opened.push(value);
    }
    let conversations = wire
        .tool("lomi_chat_list", json!({"workspaceId":workspace}))
        .await?;
    let items = conversations["structuredContent"]["data"]["items"]
        .as_array()
        .ok_or("Missing chat list")?;
    if items.len() != 3
        || !items
            .iter()
            .any(|c| c["conversationId"] == opened[2]["conversationId"])
    {
        return Err("Creator did not receive exact new chat access".into());
    }
    let domain = wire.tool("lomi_workspace_list", json!({})).await?;
    let denied=wire.tool("lomi_chat_open",json!({"workspaceId":workspace,"target":{"type":"existing","conversationId":"native-private"},"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":epoch,"requestKey":"private-chat-open"})).await?;
    if denied["structuredContent"]["code"] != "SCOPE_DENIED" {
        return Err("Unshared chat was opened".into());
    }
    {
        let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
        let services = backend.services.lock().map_err(|_| "Chat lock")?;
        let store = services.store.as_ref().map_err(Clone::clone)?;
        let requests: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if requests != 0 {
            return Err("Chat open dispatched a provider request".into());
        }
    }
    screenshot(main, directory.join("chat-open.png")).await?;
    std::fs::write(directory.join("chat-open.json"),serde_json::to_vec_pretty(&json!({"opened":opened,"list":conversations,"denied":denied,"retained":true,"providerRequests":0})).unwrap()).map_err(|e|e.to_string())?;
    Ok(vec![
        "existing chat open",
        "standalone view reuse",
        "retained SDK and human draft",
        "new conversation with creator access",
        "open retry without duplicate",
        "unshared open denied and no provider request",
    ])
}

async fn qualify_draft(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    directory: &Path,
) -> Result<Vec<&'static str>, String> {
    let opened: Value = serde_json::from_slice(
        &std::fs::read(directory.join("chat-open.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let panel = &opened["opened"][0]["panelId"];
    javascript(main, "await window.__chatRetained.flush();return true;").await?;
    let connected = wire
        .tool("lomi_connect", json!({"workspaceId":workspace}))
        .await?;
    let domain = wire.tool("lomi_workspace_list", json!({})).await?;
    let read_args =
        json!({"workspaceId":workspace,"conversationId":"native-shared-a","part":{"type":"draft"}});
    let before = wire.tool("lomi_chat_read", read_args.clone()).await?;
    let mut args = json!({"workspaceId":workspace,"panelId":panel,"conversationId":"native-shared-a","text":"Agent native draft 日本語 🙂","expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"expectedDraftRevision":before["structuredContent"]["data"]["draftRevision"],"expectedConversationRevision":before["structuredContent"]["data"]["conversation"]["conversationRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"native-chat-draft"});
    let queued = wire.tool("lomi_chat_draft", args.clone()).await?;
    let id = queued["structuredContent"]["data"]["operationId"]
        .as_str()
        .ok_or_else(|| format!("Draft receipt: {queued}"))?;
    let result = wire.settled(id).await?;
    if result["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Draft failed: {result}"));
    }
    let after = wire.tool("lomi_chat_read", read_args.clone()).await?;
    if after["structuredContent"]["data"]["content"] != args["text"] {
        return Err("Native draft was not persisted".into());
    }
    if javascript(main,"const r=window.__chatRetained;return r===(await import('/src/chat/chat-runtime.ts')).existing('native-shared-a') && r.chat===window.__chatSdk && r.snapshot.text==='Agent native draft 日本語 🙂' && !r.dirty;").await? != true { return Err("Native draft did not reuse the live Chat runtime".into()); }
    let retry = wire.tool("lomi_chat_draft", args.clone()).await?;
    if retry["structuredContent"]["data"] != result["structuredContent"]["data"] {
        return Err("Draft retry changed its receipt".into());
    }
    let mut stale = args.clone();
    stale["requestKey"] = json!("native-stale-draft");
    let pending = wire.tool("lomi_chat_draft", stale).await?;
    let failure = wire
        .settled(
            pending["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or("Missing stale receipt")?,
        )
        .await?;
    if failure["structuredContent"]["data"]["state"] != "failed"
        || failure["structuredContent"]["data"]["result"]["code"] != "REVISION_CONFLICT"
    {
        return Err(format!("Stale draft was not rejected: {failure}"));
    }
    let mut private = args.clone();
    private["conversationId"] = json!("native-private");
    private["requestKey"] = json!("native-private-draft");
    if wire.tool("lomi_chat_draft", private).await?["structuredContent"]["code"] != "SCOPE_DENIED" {
        return Err("Unshared draft was writable".into());
    }
    args["expectedDraftRevision"] = after["structuredContent"]["data"]["draftRevision"].clone();
    args["text"] = json!("Agent draft before human input");
    args["requestKey"] = json!("native-racing-draft");
    javascript(
        main,
        r#"
        const runtime=window.__chatRetained;
        window.__draftApply=runtime.applyAgentDraft.bind(runtime);
        runtime.applyAgentDraft=(...args)=>{
          const commit=args[3];
          args[3]=async ()=>{
            const value=await commit();
            window.__nativeDraftCommitted=value;
            await new Promise(resolve=>{window.__releaseDraft=resolve;});
            return value;
          };
          return window.__draftApply(...args);
        };
        return true;
    "#,
    )
    .await?;
    let pending = wire.tool("lomi_chat_draft", args.clone()).await?;
    if let Err(error) = wait_for(main, "!!window.__nativeDraftCommitted").await {
        let runtime = javascript(main, "const r=window.__chatRetained;return {text:r.snapshot.text,dirty:r.dirty,loaded:r.snapshot.loaded,error:r.snapshot.error,applyPatched:r.applyAgentDraft!==window.__draftApply};").await?;
        let receipt = if let Some(id) = pending["structuredContent"]["data"]["operationId"].as_str()
        {
            wire.settled(id).await?
        } else {
            pending.clone()
        };
        return Err(format!(
            "{error}; queued={pending}; receipt={receipt}; runtime={runtime}; input={args}"
        ));
    }
    javascript(
        main,
        "window.__chatRetained.setText('Human during native write 日本語 🙂');return true;",
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    javascript(main,"window.__chatRetained.applyAgentDraft=window.__draftApply;window.__releaseDraft();await window.__chatRetained.flush();return true;").await?;
    let race = wire
        .settled(
            pending["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or("Missing race receipt")?,
        )
        .await?;
    if race["structuredContent"]["data"]["state"] != "succeeded" {
        return Err(format!("Race draft failed: {race}"));
    }
    let final_draft = wire.tool("lomi_chat_read", read_args).await?;
    if final_draft["structuredContent"]["data"]["content"] != "Human during native write 日本語 🙂" || javascript(main,"return window.__chatRetained.snapshot.text==='Human during native write 日本語 🙂' && !window.__chatRetained.dirty && !window.__chatRetained.snapshot.storageFailed;").await? != true { return Err("Human draft was lost after native ACK".into()); }
    {
        let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
        let services = backend.services.lock().map_err(|_| "Chat lock")?;
        let requests: i64 = services
            .store
            .as_ref()
            .map_err(Clone::clone)?
            .connection
            .query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if requests != 0 {
            return Err("Draft sent a provider request".into());
        }
    }
    std::fs::write(directory.join("chat-draft.json"),serde_json::to_vec_pretty(&json!({"written":result,"retry":retry,"stale":failure,"race":race,"finalDraft":final_draft,"providerRequests":0})).unwrap()).map_err(|e|e.to_string())?;
    Ok(vec![
        "native draft CAS and retained runtime",
        "draft retry without another write",
        "stale draft refusal",
        "unshared draft refusal",
        "native late ACK preserves and saves human input",
        "draft makes no provider request",
    ])
}

fn request_count(app: &tauri::AppHandle) -> Result<i64, String> {
    let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
    let services = backend.services.lock().map_err(|_| "Chat storage lock")?;
    services
        .store
        .as_ref()
        .map_err(Clone::clone)?
        .connection
        .query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0))
        .map_err(|e| e.to_string())
}

async fn qualify_send(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    directory: &Path,
) -> Result<Vec<&'static str>, String> {
    {
        let backend = app.state::<crate::chat::commands::Chats>().backend(app)?;
        let mut services = backend.services.lock().map_err(|_| "Chat storage lock")?;
        let settings = services.settings.as_mut().map_err(|e| e.clone())?;
        let mut data = settings.data.clone();
        data.connections = vec![crate::chat::preferences::Connection {
            id: "native-fixture".into(),
            name: "Native fixture".into(),
            provider: "openai".into(),
            enabled: true,
            credential_revision: 0,
            secret_mode: "session".into(),
            secret_id: None,
            models: vec![],
            tested_model: None,
            test_status: None,
        }];
        settings.save(
            data.clone(),
            data.revision,
            Some(("native-fixture", "fixture-private-key")),
        )?;
    }
    javascript(main,"const r=window.__chatRetained;await r.configure({...r.snapshot.loaded.conversation.config,connectionId:'native-fixture',model:'fixture-fast',configured:true});await r.flush();return true;").await?;
    let opened: Value = serde_json::from_slice(
        &std::fs::read(directory.join("chat-open.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let panel = &opened["opened"][0]["panelId"];
    let connected = wire
        .tool("lomi_connect", json!({"workspaceId":workspace}))
        .await?;
    let read_args = json!({"workspaceId":workspace,"conversationId":"native-shared-a","part":{"type":"draft"},"includeSendTarget":true});
    tauri::Emitter::emit_to(app, "main", "chat-preferences-changed", ())
        .map_err(|e| e.to_string())?;
    let domain = wire.tool("lomi_workspace_list", json!({})).await?;
    let reveal = wire.tool("lomi_chat_open", json!({"workspaceId":workspace,"target":{"type":"existing","conversationId":"native-shared-a"},"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"show-send-conversation"})).await?;
    let reveal = wire
        .settled(
            reveal["structuredContent"]["data"]["operationId"]
                .as_str()
                .ok_or("Missing reveal operation")?,
        )
        .await?;
    if reveal["structuredContent"]["data"]["state"] != "succeeded" {
        return Err("Could not reveal the sending conversation".into());
    }
    let mut proofs = Vec::new();
    for mode in ["declined", "stale-native", "accepted"] {
        javascript(
            main,
            "await window.__chatRetained.reload();await window.__chatRetained.flush();return true;",
        )
        .await?;
        let before = wire.tool("lomi_chat_read", read_args.clone()).await?;
        let value = &before["structuredContent"]["data"];
        if value["sendTarget"]["connectionId"] != "native-fixture"
            || value["sendTarget"]["model"] != "fixture-fast"
        {
            return Err(format!("Explicit send target unavailable: {before}"));
        }
        let domain = wire.tool("lomi_workspace_list", json!({})).await?;
        let args = json!({"workspaceId":workspace,"panelId":panel,"conversationId":"native-shared-a","connectionId":"native-fixture","model":"fixture-fast","expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"expectedDraftRevision":value["draftRevision"],"expectedConversationRevision":value["conversation"]["conversationRevision"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":format!("native-send-{mode}")});
        let queued = wire.tool("lomi_chat_send", args.clone()).await?;
        let operation = queued["structuredContent"]["data"]["operationId"]
            .as_str()
            .ok_or_else(|| format!("Send receipt: {queued}"))?;
        let reserved = &queued["structuredContent"]["data"]["result"];
        if reserved["requestId"].as_str().is_none() || !reserved["draftRevision"].is_null() {
            return Err(format!("Missing reserved request identity: {queued}"));
        }
        wait_for(main, "!!document.querySelector('.agent-chat-approval')").await?;
        let preview = javascript(main,"const d=document.querySelector('.agent-chat-approval');return {text:d.textContent,draft:d.querySelector('textarea').value,cancel:document.activeElement?.textContent==='Cancel'};").await?;
        if preview["draft"] != value["content"]
            || preview["cancel"] != true
            || !preview["text"].as_str().is_some_and(|s| {
                s.contains("fixture-fast") && s.contains("Native fixture") && s.contains("charges")
            })
            || preview.to_string().contains("fixture-private-key")
        {
            return Err(format!("Native send preview mismatch: {preview}"));
        }
        if mode == "declined" {
            screenshot(main, directory.join("chat-send-approval.png")).await?;
            evaluate(main,"[...document.querySelectorAll('.agent-chat-approval button')].find(b=>b.textContent==='Cancel').click();true").await?;
        } else {
            if mode == "stale-native" {
                // Keep the renderer's cached plan unchanged; only native revalidation can detect this.
                javascript(main,"const invoke=window.__TAURI_INTERNALS__.invoke;const c=await invoke('chat_main',{input:{action:'load',id:'native-shared-a'}});await invoke('chat_main',{input:{action:'configure',id:'native-shared-a',expected:c.conversation.revision,config:{...c.conversation.config,system:'CHANGED_NATIVE_SYSTEM'}}});return true;").await?;
            }
            evaluate(main,"[...document.querySelectorAll('.agent-chat-approval button')].find(b=>b.textContent==='Send message').click();true").await?;
        }
        wait_for(main, "!document.querySelector('.agent-chat-approval')").await?;
        let receipt = wire.settled(operation).await?;
        let data = &receipt["structuredContent"]["data"];
        if mode == "declined" {
            if data["state"] != "cancelled" || data["effectState"] != "none" {
                return Err(format!("Declined send: {receipt}"));
            }
        } else if mode == "stale-native" {
            if data["state"] != "failed"
                || data["effectState"] != "none"
                || data["result"]["rejection"] != "REVISION_CONFLICT"
            {
                return Err(format!("Native stale send: {receipt}"));
            }
        } else if data["state"] != "succeeded"
            || data["result"]["requestId"] != reserved["requestId"]
            || data["result"]["draftRevision"].as_str().is_none()
        {
            return Err(format!("Accepted send mismatch: {receipt}"));
        }
        let retry = wire.tool("lomi_chat_send", args).await?;
        if retry["structuredContent"]["data"] != *data {
            return Err("Send retry changed its durable result".into());
        }
        if request_count(app)? != i64::from(mode == "accepted") {
            return Err("Send dispatched an unexpected number of native requests".into());
        }
        if mode != "accepted" {
            let unchanged = wire.tool("lomi_chat_read", read_args.clone()).await?;
            if unchanged["structuredContent"]["data"]["content"] != value["content"]
                || unchanged["structuredContent"]["data"]["draftRevision"] != value["draftRevision"]
            {
                return Err("Rejected send consumed the draft".into());
            }
        } else {
            wait_for(main,"window.__chatRetained.chat.messages.some(m=>m.role==='assistant'&&m.status!=='completed'&&m.parts.some(p=>p.type==='text'&&p.text.length>10))").await?;
            if javascript(main,"return window.__chatRetained.chat===window.__chatSdk && window.__chatRetained.snapshot.busy;").await? != true { return Err("Send replaced the retained SDK or lost live streaming".into()); }
            javascript(main,"window.__chatRetained.setText('Human next native draft 🙂');await window.__chatRetained.flush();return true;").await?;
            let active = wire.tool("lomi_chat_read", read_args.clone()).await?;
            if active["structuredContent"]["data"]["request"]["requestId"] != reserved["requestId"]
                || active["structuredContent"]["data"]["request"]["status"] != "active"
            {
                return Err("History did not expose the exact active request identity".into());
            }
            qualify_layout(
                wire,
                main,
                workspace,
                panel,
                &connected["structuredContent"]["data"]["retryEpoch"],
                &reserved["requestId"],
                directory,
            )
            .await?;
            let stop_args = json!({"workspaceId":workspace,"conversationId":"native-shared-a","requestId":reserved["requestId"],"retryEpoch":connected["structuredContent"]["data"]["retryEpoch"],"requestKey":"stop-native-send"});
            let mut unknown = stop_args.clone();
            unknown["requestId"] = json!("unknown-native-request");
            unknown["requestKey"] = json!("stop-unknown");
            let denied = wire.tool("lomi_chat_stop", unknown).await?;
            if denied["structuredContent"]["data"]["result"]["code"] != "TARGET_NOT_FOUND"
                || denied["structuredContent"]["data"]["effectState"] != "none"
            {
                return Err(format!("Unknown stop: {denied}"));
            }
            let stopped = wire.tool("lomi_chat_stop", stop_args.clone()).await?;
            if stopped["structuredContent"]["data"]["state"] != "succeeded"
                || stopped["structuredContent"]["data"]["result"]["request"]["status"]
                    != "cancelled"
            {
                return Err(format!("Native stop: {stopped}"));
            }
            wait_for(main, "!window.__chatRetained.snapshot.busy").await?;
            let final_draft = wire.tool("lomi_chat_read", read_args.clone()).await?;
            if final_draft["structuredContent"]["data"]["content"] != "Human next native draft 🙂"
            {
                return Err("Live generation lost the next human draft".into());
            }
            let saved = javascript(main,"const {request}=await window.__TAURI_INTERNALS__.invoke('chat_main',{input:{action:'load',id:'native-shared-a'}});return {request:request&&{id:request.id,status:request.status}};").await?;
            if saved["request"]["id"] != reserved["requestId"]
                || saved["request"]["status"] != "cancelled"
            {
                return Err("Native stream cancellation was not checkpointed".into());
            }
            javascript(main, "await window.__chatRetained.send();return true;").await?;
            wait_for(main,"window.__chatRetained.snapshot.busy && window.__chatRetained.chat.messages.at(-1)?.parts.some(p=>p.type==='text'&&p.text.length>10)").await?;
            let replacement = wire.tool("lomi_chat_read", read_args.clone()).await?;
            let replacement = &replacement["structuredContent"]["data"]["request"];
            if replacement["requestId"] == reserved["requestId"]
                || replacement["status"] != "active"
            {
                return Err("Replacement fixture generation did not start".into());
            }
            let retry_stop = wire.tool("lomi_chat_stop", stop_args.clone()).await?;
            if retry_stop["structuredContent"]["data"] != stopped["structuredContent"]["data"] {
                return Err("Stop retry changed its original receipt".into());
            }
            let live = wire.tool("lomi_chat_read", read_args.clone()).await?;
            if live["structuredContent"]["data"]["request"] != *replacement {
                return Err("Old stop retry cancelled the replacement request".into());
            }
            let mut stop_next = stop_args;
            stop_next["requestId"] = replacement["requestId"].clone();
            stop_next["requestKey"] = json!("stop-native-replacement");
            let final_stop = wire.tool("lomi_chat_stop", stop_next).await?;
            if final_stop["structuredContent"]["data"]["state"] != "succeeded" {
                return Err(format!("Replacement stop: {final_stop}"));
            }
            wait_for(main, "!window.__chatRetained.snapshot.busy").await?;
            std::fs::write(directory.join("chat-stop.json"), serde_json::to_vec_pretty(&json!({"active":active,"unknown":denied,"stopped":stopped,"retry":retry_stop,"replacement":replacement,"final":final_stop})).unwrap()).map_err(|e| e.to_string())?;
        }
        proofs.push(json!({"mode":mode,"queued":queued,"receipt":receipt,"retry":retry}));
    }
    std::fs::write(directory.join("chat-send.json"), serde_json::to_vec_pretty(&json!({"checks":proofs,"nativeRequests":request_count(app)?,"provider":"local fixture; no external request"})).unwrap()).map_err(|e| e.to_string())?;
    Ok(vec![
        "explicit send target",
        "exact native approval with cancel focus",
        "declined send preserves draft",
        "native context change invalidates approval",
        "durable pre-dispatch send identity",
        "send retry dispatches exactly once",
        "live stream retains SDK and next human draft",
        "native stream cancellation checkpoint",
        "read exposes exact active request identity",
        "unknown stop cannot register a cancellation",
        "native MCP stop preserves checkpoint and next draft",
        "stop retry leaves replacement generation active",
        "docking retains chat SDK, draft and active request",
        "mixed pane move and tab reorder retain live PTY",
        "chat open reveals the existing mixed pane",
        "panel focus reveals the existing mixed chat",
        "workspace transfer retains chat and terminal identities",
        "return transfer preserves exact chat request and draft",
    ])
}

async fn qualify_layout(
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    panel: &Value,
    epoch: &Value,
    request: &Value,
    directory: &Path,
) -> Result<(), String> {
    let (_, terminal) = layout_call(wire, "lomi_terminal_create", json!({"workspaceId":workspace,"cwdRelative":".","title":"Chat layout fixture","retryEpoch":epoch,"requestKey":"chat-layout-terminal"})).await?;
    let terminal = &terminal["structuredContent"]["data"]["result"];
    let list = wire
        .tool("lomi_panel_list", json!({"workspaceId":workspace}))
        .await?;
    let tab = list["structuredContent"]["data"]["items"]
        .as_array()
        .and_then(|items| items.iter().find(|p| p["id"] == terminal["panelId"]))
        .ok_or_else(|| format!("Missing terminal layout: {list}"))?["tabId"]
        .clone();
    let mut proofs = Vec::new();
    for (key, movement) in [
        (
            "chat-dock",
            json!({"type":"dock_tab","tabId":panel,"targetTabId":tab,"side":"right"}),
        ),
        (
            "chat-reorder",
            json!({"type":"reorder_tab","tabId":tab,"beforeTabId":null}),
        ),
        (
            "chat-pane-move",
            json!({"type":"move_pane","panelId":panel,"targetPanelId":terminal["panelId"],"side":"left"}),
        ),
    ] {
        let (args, result) = layout_call(wire, "lomi_panel_move", json!({"workspaceId":workspace,"movement":movement,"retryEpoch":epoch,"requestKey":key})).await?;
        let retry = wire.tool("lomi_panel_move", args).await?;
        if retry["structuredContent"]["data"] != result["structuredContent"]["data"] {
            return Err("Chat layout retry changed its receipt".into());
        }
        proofs.push(result);
    }
    // Hide the mixed tab before each reveal; the retained SDK remains active.
    let other: Value = serde_json::from_slice(
        &std::fs::read(directory.join("chat-open.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    for tool in ["lomi_chat_open", "lomi_panel_focus"] {
        layout_call(wire, "lomi_panel_focus", json!({"workspaceId":workspace,"panelId":other["opened"][2]["panelId"],"retryEpoch":epoch,"requestKey":format!("hide-before-{tool}")})).await?;
        let args = if tool == "lomi_chat_open" {
            json!({"workspaceId":workspace,"target":{"type":"existing","conversationId":"native-shared-a"},"retryEpoch":epoch,"requestKey":"reveal-mixed-chat"})
        } else {
            json!({"workspaceId":workspace,"panelId":panel,"retryEpoch":epoch,"requestKey":"focus-mixed-chat"})
        };
        let (_, result) = layout_call(wire, tool, args).await?;
        if result["structuredContent"]["data"]["result"]["panelId"] != *panel {
            return Err("Mixed reveal duplicated the chat view".into());
        }
        wait_for(
            main,
            &format!(
                "!!document.querySelector('[data-chat-pane-id=\"{}\"] textarea')",
                panel.as_str().ok_or("Missing chat panel ID")?
            ),
        )
        .await?;
        proofs.push(result);
    }
    let (_, created) = layout_call(wire, "lomi_workspace_create", json!({"workspaceId":workspace,"name":"Chat transfer fixture","retryEpoch":epoch,"requestKey":"chat-workspace"})).await?;
    let destination = &created["structuredContent"]["data"]["result"]["workspaceId"];
    for (index, (source, target)) in [(workspace, destination), (destination, workspace)]
        .into_iter()
        .enumerate()
    {
        let (_, moved) = layout_call(wire, "lomi_panel_move", json!({"workspaceId":source,"movement":{"type":"transfer_tab","tabId":tab,"targetWorkspaceId":target,"beforeTabId":null},"retryEpoch":epoch,"requestKey":format!("chat-transfer-{index}")})).await?;
        layout_call(wire, "lomi_panel_focus", json!({"workspaceId":target,"panelId":panel,"retryEpoch":epoch,"requestKey":format!("chat-transfer-focus-{index}")})).await?;
        let live = wire.tool("lomi_chat_read", json!({"workspaceId":target,"conversationId":"native-shared-a","part":{"type":"draft"}})).await?;
        if live["structuredContent"]["data"]["request"]["requestId"] != *request
            || live["structuredContent"]["data"]["request"]["status"] != "active"
            || live["structuredContent"]["data"]["content"] != "Human next native draft 🙂"
        {
            return Err(format!(
                "Chat transfer lost draft or active generation: {live}"
            ));
        }
        let pty = wire.tool("lomi_terminal_read", json!({"workspaceId":target,"panelId":terminal["panelId"],"terminalSessionId":terminal["terminalSessionId"]})).await?;
        if pty["structuredContent"]["data"]["terminalSessionId"] != terminal["terminalSessionId"] {
            return Err(format!("Chat transfer lost sibling PTY: {pty}"));
        }
        if javascript(main,"const r=(await import('/src/chat/chat-runtime.ts')).existing('native-shared-a');return r===window.__chatRetained && r.chat===window.__chatSdk && r.snapshot.busy && r.snapshot.text==='Human next native draft 🙂';").await? != true { return Err("Chat layout replaced runtime or human draft".into()); }
        proofs.push(moved);
    }
    screenshot(main, directory.join("chat-layout.png")).await?;
    std::fs::write(directory.join("chat-layout.json"), serde_json::to_vec_pretty(&json!({"operations":proofs,"retainedSdk":true,"retainedDraft":true,"retainedActiveRequest":true,"terminal":terminal})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}

async fn qualify_close(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    directory: &Path,
) -> Result<Vec<&'static str>, String> {
    let connected = wire
        .tool("lomi_connect", json!({"workspaceId":workspace}))
        .await?;
    let epoch = &connected["structuredContent"]["data"]["retryEpoch"];
    let initial_pty = javascript(
        main,
        "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
    )
    .await?;
    let mut proofs = Vec::new();
    for kind in ["panel", "workspace", "project"] {
        let (_, opened) = layout_call(wire, "lomi_chat_open", json!({"workspaceId":workspace,"target":{"type":"existing","conversationId":"native-shared-a"},"retryEpoch":epoch,"requestKey":format!("close-open-{kind}")})).await?;
        let panel = opened["structuredContent"]["data"]["result"]["panelId"].clone();
        javascript(main, "const r=(await import('/src/chat/chat-runtime.ts')).getChat('native-shared-a');await r.ready;window.__closingChat=r;r.setText('Close active response');await r.flush();await r.send();r.setText('Draft preserved through close 日本語 🙂');return true;").await?;
        wait_for(
            main,
            "window.__closingChat.snapshot.busy && window.__closingChat.chat.status==='streaming'",
        )
        .await?;
        let before = wire.tool("lomi_chat_read", json!({"workspaceId":workspace,"conversationId":"native-shared-a","part":{"type":"draft"}})).await?;
        let request = before["structuredContent"]["data"]["request"]["requestId"].clone();
        if kind == "panel" {
            javascript(main,"const r=window.__closingChat;window.__closeFlush=r.flush.bind(r);r.flush=async()=>{throw Error('Fixture draft save failure')};return true;").await?;
            let domain = wire.tool("lomi_workspace_list", json!({})).await?;
            let args = json!({"workspaceId":workspace,"panelId":panel,"expectedRevision":domain["structuredContent"]["data"]["domainRevision"],"retryEpoch":epoch,"requestKey":"chat-close-save-failure"});
            let queued = wire.tool("lomi_panel_close", args.clone()).await?;
            let failed = wire
                .settled(
                    queued["structuredContent"]["data"]["operationId"]
                        .as_str()
                        .ok_or_else(|| queued.to_string())?,
                )
                .await?;
            if failed["structuredContent"]["data"]["state"] != "outcome_unknown"
                || failed["structuredContent"]["data"]["effectState"] != "unknown"
            {
                return Err(format!("Draft close guard did not fail safely: {failed}"));
            }
            if wire.tool("lomi_panel_close", args).await?["structuredContent"]["data"]
                != failed["structuredContent"]["data"]
            {
                return Err("Failed close replay changed its receipt".into());
            }
            let live = wire.tool("lomi_chat_read",json!({"workspaceId":workspace,"conversationId":"native-shared-a","part":{"type":"draft"}})).await?;
            if live["structuredContent"]["data"]["request"]["status"] != "active"
                || live["structuredContent"]["data"]["request"]["requestId"] != request
            {
                return Err("Failed draft guard cancelled the response".into());
            }
            javascript(
                main,
                "window.__closingChat.flush=window.__closeFlush;return true;",
            )
            .await?;
            proofs.push(failed);
        }
        let (tool, mut args) = if kind == "workspace" {
            let (_, created) = layout_call(wire,"lomi_workspace_create",json!({"workspaceId":workspace,"name":"Chat close fixture","retryEpoch":epoch,"requestKey":"chat-close-workspace"})).await?;
            let destination = created["structuredContent"]["data"]["result"]["workspaceId"].clone();
            layout_call(wire,"lomi_panel_move",json!({"workspaceId":workspace,"movement":{"type":"transfer_tab","tabId":panel,"targetWorkspaceId":destination,"beforeTabId":null},"retryEpoch":epoch,"requestKey":"close-transfer-chat"})).await?;
            (
                "lomi_workspace_update",
                json!({"action":"close","workspaceId":destination}),
            )
        } else if kind == "project" {
            let domain = wire.tool("lomi_workspace_list", json!({})).await?;
            let project = domain["structuredContent"]["data"]["items"]
                .as_array()
                .and_then(|items| items.iter().find(|w| w["id"] == *workspace))
                .ok_or("Missing close project")?["projectId"]
                .clone();
            (
                "lomi_project_close",
                json!({"workspaceId":workspace,"projectId":project}),
            )
        } else {
            (
                "lomi_panel_close",
                json!({"workspaceId":workspace,"panelId":panel}),
            )
        };
        args["retryEpoch"] = epoch.clone();
        args["requestKey"] = json!(format!("chat-close-{kind}"));
        let (args, result) = layout_call(wire, tool, args).await?;
        if wire.tool(tool, args).await?["structuredContent"]["data"]
            != result["structuredContent"]["data"]
        {
            return Err(format!("Close {kind} replay changed its receipt"));
        }
        let saved = javascript(main,"const c=await window.__TAURI_INTERNALS__.invoke('chat_main',{input:{action:'load',id:'native-shared-a'}});return {request:c.request,draft:c.draft.text};").await?;
        if saved["request"]["id"] != request
            || saved["request"]["status"] != "cancelled"
            || saved["draft"] != "Draft preserved through close 日本語 🙂"
        {
            return Err(format!(
                "Close {kind} lost the checkpoint or draft: {saved}"
            ));
        }
        let pty = javascript(
            main,
            "return await window.__TAURI_INTERNALS__.invoke('terminal_contexts');",
        )
        .await?;
        if (kind != "project" && pty != initial_pty)
            || (kind == "project" && pty.as_object().is_none_or(|p| !p.is_empty()))
        {
            return Err(format!("Close {kind} changed unexpected PTYs: {pty}"));
        }
        proofs.push(json!({"kind":kind,"receipt":result,"saved":saved,"pty":pty}));
    }
    if request_count(app)? != 5 {
        return Err("Close fixture duplicated a generation".into());
    }
    screenshot(main, directory.join("chat-project-closed.png")).await?;
    std::fs::write(
        directory.join("chat-close.json"),
        serde_json::to_vec_pretty(&proofs).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(vec![
        "draft close failure preserves active response",
        "failed close replay does not cancel",
        "panel close checkpoints exact active response",
        "workspace close checkpoints active chat",
        "project close checkpoints active chat",
        "all close levels preserve next human draft",
        "panel and workspace close preserve unrelated PTY",
    ])
}

async fn qualify_export(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    workspace: &Value,
    directory: &Path,
) -> Result<Vec<&'static str>, String> {
    use sha2::Digest;
    javascript(main,"window.__chatRetained.setText('Export saved draft 日本語 🙂');await window.__chatRetained.flush();return true;").await?;
    let before = changes(app)?;
    let base = json!({"workspaceId":workspace,"conversationId":"native-shared-a","format":"json","maxChars":1024});
    let first = wire.tool("lomi_chat_export", base.clone()).await?;
    let header = &first["structuredContent"]["data"];
    if header["kind"] != "chat_export" {
        return Err(format!("Export unavailable: {first}"));
    }
    let mut document = String::new();
    let mut page = first.clone();
    let mut pages = 0;
    loop {
        let data = &page["structuredContent"]["data"];
        document.push_str(data["content"].as_str().ok_or("Missing export content")?);
        pages += 1;
        let Some(next) = data["nextUtf16"].as_u64() else {
            break;
        };
        let mut input = base.clone();
        input["startUtf16"] = json!(next);
        input["expectedRevision"] = header["revision"].clone();
        page = wire.tool("lomi_chat_export", input).await?;
        if pages > 4096 {
            return Err("Unbounded export pagination".into());
        }
    }
    let decoded: Value = serde_json::from_str(&document).map_err(|e| e.to_string())?;
    if format!("{:x}", sha2::Sha256::digest(document.as_bytes()))
        != header["revision"].as_str().ok_or("Missing export hash")?
        || document.len() as u64 != header["totalBytes"].as_u64().unwrap_or(0)
        || document.contains("PRIVATE_")
        || document.contains("CHANGED_NATIVE_SYSTEM")
        || document.contains("fixture-private-key")
        || decoded["draftText"] != "Export saved draft 日本語 🙂"
        || decoded["messages"].as_array().map(Vec::len) != Some(5)
        || header["nonTextPartsOmitted"] != true
    {
        return Err(
            "Export pages lost text, disclosed private fields or failed the document hash".into(),
        );
    }
    let mut markdown_args = base.clone();
    markdown_args["format"] = json!("markdown");
    let markdown = wire.tool("lomi_chat_export", markdown_args).await?;
    if !markdown["structuredContent"]["data"]["content"]
        .as_str()
        .is_some_and(|s| s.starts_with("# native-shared-a") && s.contains("Saved answer"))
    {
        return Err("Markdown export missing saved text".into());
    }
    if changes(app)? != before {
        return Err("Export modified the native history store".into());
    }
    let mut private = base.clone();
    private["conversationId"] = json!("native-private");
    if wire.tool("lomi_chat_export", private).await?["structuredContent"]["code"] != "SCOPE_DENIED"
    {
        return Err("Export disclosed an unshared conversation".into());
    }
    let emoji = document.find('🙂').ok_or("No scalar fixture")?;
    let mut split = base.clone();
    split["startUtf16"] = json!(document[..emoji].encode_utf16().count() + 1);
    split["expectedRevision"] = header["revision"].clone();
    if wire.tool("lomi_chat_export", split).await?["structuredContent"]["code"]
        != "RESOURCE_EXHAUSTED"
    {
        return Err("Export accepted a split surrogate".into());
    }
    javascript(main,"window.__chatRetained.setText('Changed export checkpoint');await window.__chatRetained.flush();return true;").await?;
    let mut stale = base;
    stale["startUtf16"] = json!(1);
    stale["expectedRevision"] = header["revision"].clone();
    if wire.tool("lomi_chat_export", stale).await?["structuredContent"]["code"]
        != "REVISION_CONFLICT"
    {
        return Err("Export concatenated different history revisions".into());
    }
    std::fs::write(directory.join("chat-export.json"), serde_json::to_vec_pretty(&json!({"first":first,"pages":pages,"sha256":header["revision"],"messageCount":5,"readOnly":true,"privateFieldsOmitted":true,"markdown":markdown})).unwrap()).map_err(|e| e.to_string())?;
    Ok(vec![
        "JSON export pages reconstruct an exact document hash",
        "export includes all saved messages and draft",
        "export omits private fields and makes no writes",
        "Markdown export",
        "unshared export denied",
        "export rejects split scalars and changed history",
    ])
}

pub(super) async fn qualify(
    app: &tauri::AppHandle,
    wire: &mut Wire,
    main: &Webview,
    settings: &Webview,
    workspace: &Value,
    directory: &Path,
) -> Result<(), String> {
    let before = changes(app)?;
    let mut checks = vec!["Settings-only picker", "project-filtered exact selection"];
    let first = wire
        .tool("lomi_chat_list", json!({"workspaceId":workspace,"limit":1}))
        .await?;
    let data = &first["structuredContent"]["data"];
    if data["total"] != 2
        || data["items"][0]["conversationId"] != "native-shared-a"
        || data["nextOffset"] != 1
    {
        return Err(format!("Chat list mismatch: {first}"));
    }
    let next=wire.tool("lomi_chat_list",json!({"workspaceId":workspace,"limit":1,"offset":1,"expectedRevision":data["revision"]})).await?;
    if next["structuredContent"]["data"]["items"][0]["conversationId"] != "native-shared-b" {
        return Err(format!("Chat list second page: {next}"));
    }
    checks.push("exact metadata pages");
    let base = json!({"workspaceId":workspace,"conversationId":"native-shared-a","part":{"type":"draft"},"maxChars":3});
    let draft = wire.tool("lomi_chat_read", base.clone()).await?;
    let value = &draft["structuredContent"]["data"];
    if value["content"] != "A🙂"
        || value["nextUtf16"] != 3
        || value["totalUtf16"] != 4
        || value["source"] != "persisted_checkpoint"
    {
        return Err(format!("Chat Unicode checkpoint: {draft}"));
    }
    let mut tail = base.clone();
    tail["startUtf16"] = json!(3);
    tail["expectedRevision"] = value["revision"].clone();
    let end = wire.tool("lomi_chat_read", tail.clone()).await?;
    if end["structuredContent"]["data"]["content"] != "B" {
        return Err(format!("Chat Unicode tail: {end}"));
    }
    checks.push("UTF-16 checkpoint pages");
    tail["startUtf16"] = json!(2);
    if wire.tool("lomi_chat_read", tail.clone()).await?["structuredContent"]["code"]
        != "RESOURCE_EXHAUSTED"
    {
        return Err("Chat accepted a split surrogate".into());
    }
    checks.push("surrogate boundary rejection");
    for id in ["native-private", "native-foreign"] {
        let mut denied = base.clone();
        denied["conversationId"] = json!(id);
        if wire.tool("lomi_chat_read", denied).await?["structuredContent"]["code"] != "SCOPE_DENIED"
        {
            return Err("Chat disclosed an unselected conversation".into());
        }
    }
    checks.push("unselected history denied");
    let mut message = base.clone();
    message["part"] = json!({"type":"message","messageId":"private-message"});
    if wire.tool("lomi_chat_read", message.clone()).await?["structuredContent"]["code"]
        != "TARGET_NOT_FOUND"
    {
        return Err("Foreign message ID escaped its conversation".into());
    }
    checks.push("foreign message denied");
    message["part"] = json!({"type":"message","messageId":null});
    message["maxChars"] = json!(8192);
    let answer = wire.tool("lomi_chat_read", message).await?;
    let content = &answer["structuredContent"]["data"];
    if content["content"] != "Saved answer 🙂"
        || content["nonTextPartsOmitted"] != true
        || content["part"]["messageId"] != "native-message"
        || answer.to_string().contains("PRIVATE_")
        || draft.to_string().contains("PRIVATE_")
    {
        return Err(format!("Chat private-field filtering: {answer}"));
    }
    checks.push("text-only message with private fields omitted");
    let read_only = changes(app)? == before;
    if !read_only {
        return Err("History read wrote to the native store".into());
    }
    checks.push("read-only native store");
    javascript(main,"await window.__TAURI_INTERNALS__.invoke('chat_main',{input:{action:'draft',id:'native-shared-a',text:'Changed checkpoint',expected:1}});return true;").await?;
    tail["startUtf16"] = json!(3);
    if wire.tool("lomi_chat_read", tail).await?["structuredContent"]["code"] != "REVISION_CONFLICT"
    {
        return Err("Stale Chat checkpoint was paginated".into());
    }
    checks.push("concurrent checkpoint change rejected");
    if std::env::var_os("LOMI_MCP_CHAT_OPEN_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_CHAT_DRAFT_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some()
    {
        checks.extend(qualify_open(app, wire, main, workspace, directory).await?);
    }
    if std::env::var_os("LOMI_MCP_CHAT_DRAFT_ONLY").is_some()
        || std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some()
    {
        checks.extend(qualify_draft(app, wire, main, workspace, directory).await?);
    }

    if std::env::var_os("LOMI_MCP_CHAT_SEND_ONLY").is_some() {
        checks.extend(qualify_send(app, wire, main, workspace, directory).await?);
        checks.extend(qualify_export(app, wire, main, workspace, directory).await?);
        checks.extend(qualify_close(app, wire, main, workspace, directory).await?);
    }
    javascript(
        settings,
        "await window.__TAURI_INTERNALS__.invoke('agent_control_revoke');return true;",
    )
    .await?;
    let revoked = wire
        .tool("lomi_chat_read", base)
        .await?
        .get("isError")
        .and_then(Value::as_bool)
        == Some(true);
    if !revoked {
        return Err("Revoked history remained readable".into());
    }
    checks.push("native revocation denies subsequent history");
    std::fs::write(directory.join("chat-read.json"),serde_json::to_vec_pretty(&json!({"checks":checks,"readOnly":read_only,"revoked":revoked,"list":first,"draft":draft,"answer":answer})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}
