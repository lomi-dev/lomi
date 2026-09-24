use super::*;

pub type ChatCloseDispatch =
    Arc<dyn Fn(&str, &[String], Option<NativePermit>) -> Result<(), ErrorCode> + Send + Sync>;

impl Broker {
    pub fn set_chat_close_dispatch(&self, dispatch: ChatCloseDispatch) -> io::Result<()> {
        *self.chat_close_dispatch.lock().map_err(|_| failure())? = Some(dispatch);
        Ok(())
    }

    pub(super) fn closing_chats(
        state: &State,
        owner: &str,
        panels: &[String],
    ) -> Result<Vec<String>, ErrorCode> {
        let mut conversations = Vec::new();
        for panel in state
            .projection
            .panels
            .iter()
            .filter(|p| p.kind == "chat" && panels.contains(&p.id))
        {
            Self::chat_panel_scope(state, owner, &panel.id, "chat.read")?;
            let id = panel
                .chat_conversation_id
                .as_ref()
                .ok_or(ErrorCode::TargetNotFound)?;
            if !state.projection.panels.iter().any(|p| {
                p.kind == "chat"
                    && p.chat_conversation_id.as_ref() == Some(id)
                    && !panels.contains(&p.id)
            }) {
                Self::chat_panel_scope(state, owner, &panel.id, "chat.stop")?;
                conversations.push(id.clone());
            }
        }
        conversations.sort();
        conversations.dedup();
        Ok(conversations)
    }
}
