use super::{auth, storage::valid_id};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Mutex};
use tokio::sync::oneshot;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub request_id: String,
    pub workspace_id: String,
    pub panel_id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Intent {
    pub id: String,
    pub context: Option<Context>,
    pub device_id: String,
    pub cold_boot: bool,
    pub deadline_ms: u64,
}
#[derive(Default)]
pub struct Requests {
    inner: Mutex<Inner>,
}
#[derive(Default)]
struct Inner {
    context: Option<Context>,
    pending: BTreeMap<String, oneshot::Sender<Result<(), String>>>,
}
impl Requests {
    pub fn prepare(&self, workspace: String, panel: String) -> Result<Context, String> {
        if !valid_id(&workspace) || !valid_id(&panel) {
            return Err("Invalid Android setup target".into());
        }
        let context = Context {
            request_id: auth::new_id()?,
            workspace_id: workspace,
            panel_id: panel,
        };
        self.inner
            .lock()
            .map_err(|_| "Android open requests failed")?
            .context = Some(context.clone());
        Ok(context)
    }
    pub fn context(&self) -> Result<Option<Context>, String> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| "Android open requests failed")?
            .context
            .clone())
    }
    pub fn request(
        &self,
        context: Option<String>,
        device_id: String,
        cold_boot: bool,
    ) -> Result<(Intent, oneshot::Receiver<Result<(), String>>), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Android open requests failed")?;
        if !valid_id(&device_id) || inner.pending.len() >= 8 {
            return Err("Invalid or excessive Android open requests".into());
        }
        let context = context.map(|id| inner.context.clone().filter(|context| context.request_id == id).ok_or("This setup request has expired. Open a new Android panel from the workspace.")).transpose()?;
        let (reply, receive) = oneshot::channel();
        let deadline_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis(),
        )
        .map_err(|e| e.to_string())?
            + 15000;
        let intent = Intent {
            id: auth::new_id()?,
            context,
            device_id,
            cold_boot,
            deadline_ms,
        };
        inner.pending.insert(intent.id.clone(), reply);
        Ok((intent, receive))
    }
    pub fn complete(&self, id: &str, result: Result<(), String>) -> Result<(), String> {
        let reply = self
            .inner
            .lock()
            .map_err(|_| "Android open requests failed")?
            .pending
            .remove(id)
            .ok_or("Android open request has expired")?;
        let _ = reply.send(result);
        Ok(())
    }
    pub fn expired(&self, id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.pending.remove(id);
        }
    }
}
