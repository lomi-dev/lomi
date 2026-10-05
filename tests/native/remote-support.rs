//! Debug-only real application qualification. Credentials never enter renderer commands.
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::Manager;
static STARTED: AtomicBool = AtomicBool::new(false);
fn directory() -> Result<PathBuf, String> {
    let path = PathBuf::from(
        std::env::var_os("LOMI_REMOTE_PROBE_DIRECTORY").ok_or("Remote probe not configured.")?,
    );
    if !path.is_absolute() || !path.is_dir() {
        return Err("Invalid Remote probe directory.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&path)
            .map_err(|_| "Probe directory unavailable.")?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err("Remote probe requires a private directory.".into());
        }
    }
    Ok(path)
}
fn write(path: &std::path::Path, value: &Value) -> Result<(), String> {
    let next = path.with_extension("next");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut f = options
        .open(&next)
        .map_err(|_| "Probe receipt unavailable.")?;
    f.write_all(&serde_json::to_vec(value).map_err(|_| "Probe receipt invalid.")?)
        .map_err(|_| "Probe receipt unavailable.")?;
    std::fs::rename(next, path).map_err(|_| "Probe receipt unavailable.".into())
}
pub fn start(app: tauri::AppHandle) {
    let Ok(root) = directory() else { return };
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = run(&app, &root).await;
        if let Err(message) = result {
            let _ = write(
                &root.join("native-result.json"),
                &json!({"passed":false,"error":message}),
            );
        }
    });
}
async fn run(app: &tauri::AppHandle, root: &std::path::Path) -> Result<(), String> {
    use crate::{
        auth::AuthController,
        remote::{self, Remote},
        terminal::{self, Terminals},
    };
    let file = root.join("fixture.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&file)
            .map_err(|_| "Fixture missing.")?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err("Fixture must be private.".into());
        }
    }
    let bytes = zeroize::Zeroizing::new(std::fs::read(file).map_err(|_| "Fixture missing.")?);
    if bytes.len() > 16384 {
        return Err("Fixture too large.".into());
    }
    let fixture: Value = serde_json::from_slice(&bytes).map_err(|_| "Fixture malformed.")?;
    let field = |name: &str| {
        fixture
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "Fixture field missing.".to_string())
    };
    app.state::<AuthController>().probe_session(
        field("desktopToken")?,
        field("userId")?,
        field("desktopSessionId")?,
        field("desktopExpiresAt")?,
    )?;
    // The real retained renderer creates the shell and owns its ordinary PTY ACK/reply path.
    let mut terminal = None;
    for _ in 0..600 {
        let s = app.state::<Remote>().state();
        if let Some(session) = s
            .sessions
            .first()
            .filter(|_| s.domain_epoch.is_some() && !s.workspaces.is_empty())
        {
            terminal = Some(session.id.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let id = terminal.ok_or("Real workbench PTY did not start.")?;
    app.state::<Remote>().probe_enable(app).await?;
    let mut settings = None;
    for _ in 0..100 {
        if let Some(w) = app.get_window("settings") {
            settings = Some(w);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let settings = settings.ok_or("Settings did not initialize.")?;

    let initial = if std::env::var("LOMI_REMOTE_PROBE_MANUAL").as_deref() == Ok("1") {
        "unset HISTFILE; printf 'LOMI_NATIVE_REMOTE_READY\\n'\n"
    } else {
        "unset HISTFILE; stty -echo; PS1=''; printf 'LOMI_NATIVE_REMOTE_READY\\n'\n"
    };
    terminal::write_terminal(
        app.get_window("main").ok_or("Main window missing.")?,
        app.state::<Terminals>(),
        id.clone(),
        initial.into(),
    )
    .await?;
    let _ = write(
        &root.join("native-state.json"),
        &json!({"ready":true,"sessionId":id,"remote":app.state::<Remote>().state()}),
    );
    let mut last = 0u64;
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let command = std::fs::read(root.join("command.json"))
            .ok()
            .filter(|b| b.len() <= 65536)
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        let Some(command) = command else { continue };
        let Some(seq) = command
            .get("seq")
            .and_then(Value::as_u64)
            .filter(|n| *n > last)
        else {
            continue;
        };
        last = seq;
        let text = |name: &str| {
            command
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| "Probe command field missing.".to_string())
        };
        let result:Result<Value,String>=async{
 match command.get("op").and_then(Value::as_str){
 Some("ui-terminal-action")=>{
 let action=text("action")?;
 if !matches!(action.as_str(), "new-tab" | "split") { return Err("Invalid UI terminal action.".into()); }
 let (key,code,shift)=if action=="new-tab" {("T","KeyT",true)} else {("d","KeyD",false)};
 let script=format!(r#"(() => {{
 const deadline = performance.now() + 5000;
 let startupChoiceRequested = false;
 const tick = () => {{
  const startup = document.querySelector('dialog.agent-control-startup-dialog[open]');
  if (startup) {{
   if (startup.querySelector('h2')?.textContent !== 'Start the MCP server automatically?') throw new Error('Unexpected MCP startup dialog');
   const keepDisabled = Array.from(startup.querySelectorAll('button')).find(button => button.textContent.trim() === 'Keep disabled');
   if (!startupChoiceRequested && keepDisabled && !keepDisabled.disabled) {{
    startupChoiceRequested = true;
    keepDisabled.click();
   }}
   if (performance.now() >= deadline) throw new Error('MCP startup choice did not finish');
   setTimeout(tick, 50);
   return;
  }}
  if (document.querySelector('dialog[open]')) throw new Error('Unexpected open dialog blocks terminal action');
  const event = new KeyboardEvent('keydown', {{key:'{}',code:'{}',ctrlKey:!navigator.platform.includes('Mac'),metaKey:navigator.platform.includes('Mac'),shiftKey:{},bubbles:true,cancelable:true}});
  document.body.dispatchEvent(event);
  console.warn('LOMI_REMOTE_UI_ACTION ' + JSON.stringify({{action:'{}',startupChoiceRequested,handled:event.defaultPrevented}}));
  if (!event.defaultPrevented) throw new Error('Terminal action shortcut was not handled');
 }};
 tick();
 }})()"#,key,code,shift,action);
 app.get_webview_window("main").ok_or("Main missing.")?.eval(&script).map_err(|_|"UI action failed.")?;Ok(json!({"requested":true}))
 },
 Some("ui-workspace-action")=>{
 let workspace_id=text("workspaceId")?;remote::uuid_bytes(&workspace_id)?;
 let action=text("action")?;let label=match action.as_str(){"share"=>"Share remotely","stop"=>"Stop sharing remotely",_=>return Err("Invalid UI workspace action.".into())};
 let window=app.get_window("main").ok_or("Main missing.")?;
 window.unminimize().map_err(|_|"Could not restore the main window.")?;
 window.show().map_err(|_|"Could not show the main window.")?;
 window.set_focus().map_err(|_|"Could not focus the main window.")?;
 let script=format!(r#"(() => {{
   const workspaceId = {};
   const label = {};
   const deadline = performance.now() + 5000;
   let opened = false;
   let confirming = false;
   let finished = false;
   let frame = 0;
   let lastReason = 'Main window did not receive focus';
   const blurred = () => {{ opened = false; }};
   const cleanup = () => {{
     finished = true;
     cancelAnimationFrame(frame);
     clearTimeout(timeout);
     window.removeEventListener('blur', blurred);
   }};
   const timeout = setTimeout(() => {{
     if (finished) return;
     cleanup();
     throw new Error('Workspace UI action timed out: ' + label + ': ' + lastReason);
   }}, 5000);
   window.addEventListener('blur', blurred);
   const tick = () => {{
     if (finished) return;
     if (performance.now() >= deadline) return;
     if (document.hasFocus()) {{
       if (confirming) {{
         const dialog = document.querySelector('.remote-sharing-dialog');
         const confirm = dialog && Array.from(dialog.querySelectorAll('button')).find(button => button.textContent.trim() === 'Share remotely');
         if (confirm && !confirm.disabled) {{ cleanup(); confirm.click(); return; }}
         lastReason = 'Workspace sharing confirmation has not mounted';
         frame = requestAnimationFrame(tick);
         return;
       }}
       const row = document.querySelector('.workspace-list-entry[data-workspace-id="' + workspaceId + '"] .workspace-list-item');
       if (!row) {{ lastReason = 'Workspace row missing'; }}
       else {{
         if (!opened) {{
           const rect = row.getBoundingClientRect();
           if (rect.width > 0 && rect.height > 0) {{
             opened = true;
             row.dispatchEvent(new MouseEvent('contextmenu', {{bubbles:true, cancelable:true, clientX:rect.x+10, clientY:rect.y+10}}));
           }} else {{ lastReason = 'Workspace row is not visible'; }}
         }}
         const menu = document.querySelector('[role="menu"][aria-label="Workspace actions"]');
         const item = menu && Array.from(menu.querySelectorAll('[role="menuitem"]')).find(element => element.getAttribute('aria-label') === label);
         if (item && !item.disabled && item.getAttribute('aria-disabled') !== 'true') {{
           item.click();
           if (label === 'Share remotely') {{ confirming = true; }}
           else {{ cleanup(); return; }}
         }}
         lastReason = item ? 'Workspace action is disabled' : 'Workspace action menu has not mounted';
       }}
     }} else {{ lastReason = 'Main window lost focus'; }}
     frame = requestAnimationFrame(tick);
   }};
   frame = requestAnimationFrame(tick);
 }})()"#,serde_json::to_string(&workspace_id).map_err(|_|"Invalid workspace id.")?,serde_json::to_string(label).map_err(|_|"Invalid action.")?);
 app.get_webview_window("main").ok_or("Main missing.")?.eval(&script).map_err(|_|"UI action failed.")?;Ok(json!({"requested":true}))
 },
 Some("begin-domain")=>{let s=remote::workspace::remote_begin_workspace_sync(app.get_window("main").ok_or("Main missing.")?,app.state::<Remote>())?;Ok(s)},
 Some("sync-domain")=>{let workspaces=serde_json::from_value(command.get("workspaces").cloned().ok_or("Missing workspace inventory.")?).map_err(|_|"Invalid workspace inventory.")?;let s=remote::workspace::remote_sync_workspaces(app.get_window("main").ok_or("Main missing.")?,app.state::<Remote>(),text("epoch")?,command.get("revision").and_then(Value::as_u64).ok_or("Missing revision.")?,workspaces)?;Ok(json!({"remote":s}))},
 Some("share-workspace-background" | "resume-background")=>{
   let operation=command.get("op").and_then(Value::as_str).ok_or("Missing operation.")?.to_string();
   let workspace_id=if operation=="share-workspace-background" {Some(text("workspaceId")?)} else {None};
   let shared=command.get("shared").and_then(Value::as_bool).unwrap_or(false);
   let owned_app=app.clone();let owned_root=root.to_path_buf();
   tauri::async_runtime::spawn(async move {
     let result=match owned_app.get_window("main") {
       Some(window)=>if let Some(workspace_id)=workspace_id {
         remote::workspace::remote_share_workspace(window,owned_app.state::<Remote>(),workspace_id,shared).await
       } else { remote::remote_resume(window,owned_app.state::<Remote>()).await },
       None=>Err("Main missing.".into()),
     };
     let receipt=match result {Ok(state)=>json!({"ok":true,"remote":state}),Err(error)=>json!({"ok":false,"error":error})};
     let name=if operation=="share-workspace-background" {"native-overlap-stop.json"}else{"native-overlap-resume.json"};
     let _=write(&owned_root.join(name),&receipt);
   });
   Ok(json!({"requested":true}))
 },
 Some("share-workspace")=>{let s=remote::workspace::remote_share_workspace(app.get_window("main").ok_or("Main missing.")?,app.state::<Remote>(),text("workspaceId")?,command.get("shared").and_then(Value::as_bool).ok_or("Missing shared intent.")?).await?;Ok(json!({"remote":s}))},
 Some("inspect")=>Ok(json!({"remote":app.state::<Remote>().state(),"windowVisible":app.get_window("main").and_then(|w|w.is_visible().ok())})),
 Some("idle-hour" | "helper-fault" | "snapshot" | "terminal-unavailable")=>app.state::<Remote>().probe_lifecycle(app,command.get("op").and_then(Value::as_str).ok_or("Missing operation.")?,&id),
 Some("resume")=>{let s=remote::remote_resume(app.get_window("main").ok_or("Main missing.")?,app.state::<Remote>()).await?;Ok(json!({"remote":s}))},
 Some("approve")=>{let permission: lomi_remote_crypto::Permissions=serde_json::from_value(command.get("permissions").cloned().ok_or("Missing permission.")?).map_err(|_|"Invalid permission.")?;let s=remote::remote_approve_pairing(settings.clone(),app.state::<Remote>(),text("pairingId")?,text("fingerprint")?,vec![id.clone()],permission).await?;Ok(json!({"remote":s}))},
 Some("revoke")=>{let s=remote::remote_revoke_grant(settings.clone(),app.state::<Remote>(),text("grantId")?).await?;Ok(json!({"remote":s}))},
 Some("local-input")=>{terminal::write_terminal(app.get_window("main").ok_or("Main missing.")?,app.state::<Terminals>(),command.get("sessionId").and_then(Value::as_str).unwrap_or(&id).to_string(),text("data")?).await?;Ok(json!({"sent":true}))},
 Some("terminal-response")=>{terminal::write_terminal_response(app.get_window("main").ok_or("Main missing.")?,app.state::<Terminals>(),command.get("sessionId").and_then(Value::as_str).unwrap_or(&id).to_string(),text("data")?).await?;Ok(json!({"sent":true}))},
 Some("close-window")=>{app.get_window("main").ok_or("Main missing.")?.close().map_err(|_|"Close failed.")?;Ok(json!({"requested":true}))},
 Some("reopen-window")=>{let window=app.get_window("main").ok_or("Main missing.")?;window.show().map_err(|_|"Show failed.")?;Ok(json!({"requested":true}))},
 Some("quit")=>Ok(json!({"requested":true})),
 _=>Err("Invalid probe operation.".into())
 }
 }.await;
        let receipt = match result {
            Ok(value) => json!({"seq":seq,"ok":true,"result":value}),
            Err(error) => json!({"seq":seq,"ok":false,"error":error}),
        };
        write(&root.join("native-reply.json"), &receipt)?;
        if command.get("op").and_then(Value::as_str) == Some("quit") {
            write(
                &root.join("native-result.json"),
                &json!({"quitRequested":true}),
            )?;
            app.exit(0);
            return Ok(());
        }
    }
}
