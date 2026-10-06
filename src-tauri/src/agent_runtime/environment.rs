use crate::cli_catalog::TitleCli;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
pub(crate) struct Launch {
    pub(crate) arguments: Vec<OsString>,
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) private_home: PathBuf,
}

pub(crate) fn check_private_directory(path: &Path) -> Result<(), String> {
    super::native_accounts::check_private_directory(path)
}
pub(crate) fn admitted_version(cli: TitleCli, output: &[u8]) -> Option<String> {
    if cli == TitleCli::Grok && !super::grok_artifact::version_matches(output) {
        return None;
    }
    let text = std::str::from_utf8(output).ok()?;
    let kind = super::native_wire::NativeKind::from_version(cli, text).ok()?;
    text.split_whitespace()
        .map(|v| {
            v.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';'))
                .trim_start_matches('v')
        })
        .find(|v| kind.versions().contains(v))
        .map(str::to_owned)
}
