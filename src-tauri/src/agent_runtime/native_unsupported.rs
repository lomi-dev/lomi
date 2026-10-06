use super::native_wire::NativeKind;
use crate::{cli_catalog::TitleCli, terminal::Shells};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};
use tauri::AppHandle;
pub(crate) struct Prepared;
impl Prepared {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        _: &AppHandle,
        _: &Shells,
        _: &str,
        _: &str,
        _: TitleCli,
        _: &Path,
        _: Arc<AtomicBool>,
        _: Option<()>,
    ) -> Result<Self, String> {
        Err("Native account execution and terminals are unqualified on this platform.".into())
    }
    pub(crate) fn version(&self) -> &str {
        ""
    }
    pub(crate) fn kind(&self) -> NativeKind {
        NativeKind::Agy
    }
    pub(crate) fn collect_command(
        self,
        _: &[String],
        _: impl FnOnce() -> Result<(), String>,
    ) -> Result<Vec<u8>, String> {
        Err("Native account verification is unqualified on this platform.".into())
    }
    pub(crate) fn terminal_command(self) -> Result<crate::terminal::NativeTerminalLaunch, String> {
        Err("Native account terminals are unqualified on this platform.".into())
    }
}
