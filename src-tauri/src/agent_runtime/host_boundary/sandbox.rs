//! Slice-one profile has no Mach services or network delegation exceptions.
//! Provider/TLS/MCP compatibility must be qualified separately before admission.
use super::failure;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Policy {
    pub project_root: PathBuf,
    pub account_root: PathBuf,
    pub temp_root: PathBuf,
    pub runtime_reads: Vec<PathBuf>,
    #[serde(default)]
    pub protected_reads: Vec<PathBuf>,
    #[serde(default)]
    // Configuration denial covers every project ancestor, including deep macOS
    // temporary roots. These entries only remove read authority.
    pub blocked_reads: Vec<PathBuf>,
    #[serde(default)]
    pub runtime_read_roots: Vec<PathBuf>,
    #[serde(default)]
    pub codex_preferences: bool,
    #[serde(default)]
    pub loopback_tcp_ports: Vec<u16>,
    #[serde(default)]
    pub loopback_listener: bool,
    #[serde(default)]
    pub unix_sockets: Vec<PathBuf>,
    #[serde(default)]
    pub allow_native_tools: bool,
}
fn quote(path: &Path) -> Result<String, String> {
    let p = path.to_str().ok_or_else(failure)?;
    if p.len() > 16384 || p.chars().any(char::is_control) || !path.is_absolute() {
        return Err(failure());
    }
    serde_json::to_string(p).map_err(|_| failure())
}
fn canonical_or_future(path: &Path) -> Result<(), String> {
    use std::path::Component;
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::CurDir | Component::ParentDir | Component::Prefix(_)
            )
        })
    {
        return Err(failure());
    }
    let mut ancestor = path;
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                if ancestor.canonicalize().map_err(|_| failure())? != ancestor {
                    return Err(failure());
                }
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor.parent().ok_or_else(failure)?
            }
            Err(_) => return Err(failure()),
        }
    }
}
impl Policy {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.runtime_reads.len() > 32
            || self.protected_reads.len() > 32
            || self.blocked_reads.len() > 256
            || self.runtime_read_roots.len() > 32
            || self.loopback_tcp_ports.len() > 32
            || self.loopback_tcp_ports.contains(&0)
            || self.unix_sockets.len() > 8
        {
            return Err(failure());
        }
        for path in [&self.project_root, &self.account_root, &self.temp_root]
            .into_iter()
            .chain(self.runtime_reads.iter())
            .chain(self.runtime_read_roots.iter())
            .enumerate()
        {
            let (index, path) = path;
            if path
                .canonicalize()
                .map_err(|_| format!("Native runtime policy path {index} is unavailable."))?
                != *path
            {
                return Err(format!(
                    "Native runtime policy path {index} is not canonical."
                ));
            }
            quote(path)?;
        }
        for path in self
            .protected_reads
            .iter()
            .chain(self.blocked_reads.iter())
            .chain(self.unix_sockets.iter())
            .enumerate()
        {
            let (index, path) = path;
            canonical_or_future(path)
                .map_err(|_| format!("Native protected policy path {index} is not canonical."))?;
            quote(path)?;
        }
        Ok(())
    }
    pub(super) fn profile(&self, control: &Path) -> Result<String, String> {
        self.validate()?;
        for p in [&self.project_root, &self.account_root, &self.temp_root] {
            if control.starts_with(p) || p.starts_with(control) {
                return Err("Control and native writable domains must not overlap.".into());
            }
        }
        for runtime in self
            .runtime_reads
            .iter()
            .chain(self.runtime_read_roots.iter())
        {
            for protected in [
                &self.project_root,
                &self.account_root,
                &self.temp_root,
                control,
            ] {
                if runtime.starts_with(protected) || protected.starts_with(runtime) {
                    return Err(
                        "Executable runtime reads and writable/control domains must not overlap."
                            .into(),
                    );
                }
            }
        }
        for socket in &self.unix_sockets {
            for protected in [
                &self.project_root,
                &self.account_root,
                &self.temp_root,
                control,
            ] {
                if socket.starts_with(protected) || protected.starts_with(socket) {
                    return Err(failure());
                }
            }
            if let Ok(metadata) = std::fs::symlink_metadata(socket) {
                use std::os::unix::fs::FileTypeExt;
                if !metadata.file_type().is_socket() {
                    return Err(failure());
                }
            }
        }
        for read in &self.protected_reads {
            if read.starts_with(control) || control.starts_with(read) {
                return Err(failure());
            }
        }
        let mut p="(version 1)\n(deny default)\n(allow process-exec process-fork)\n(allow signal (target self))\n(allow sysctl-read)\n(allow file-read-metadata)\n".to_owned();
        // Darwin 27 libignition opens the root directory for openat bootstrap.
        // This literal grants directory data only, not descendant contents.
        p.push_str("(allow file-read-data (literal \"/\"))\n");
        // Executable mapping is a separate operation from file-read*. System
        // libraries and explicitly admitted runtime files may be mapped.
        // Project mappings require the separately selected native-tools policy;
        // writable account, temporary and control roots receive no mapping grant.
        if self.allow_native_tools {
            for path in [
                Path::new("/bin"),
                Path::new("/usr/bin"),
                Path::new("/usr/sbin"),
                Path::new("/sbin"),
                self.project_root.as_path(),
            ] {
                p.push_str(&format!(
                    "(allow file-read* file-map-executable (literal {}) (subpath {}))\n",
                    quote(path)?,
                    quote(path)?
                ));
            }
        }
        for path in [Path::new("/System/Library"), Path::new("/usr/lib")] {
            p.push_str(&format!(
                "(allow file-map-executable (subpath {}))\n",
                quote(path)?
            ));
        }
        for path in &self.runtime_reads {
            p.push_str(&format!(
                "(allow file-map-executable (literal {}))\n",
                quote(path)?
            ));
        }
        for path in [
            Path::new("/System/Library"),
            Path::new("/usr/lib"),
            Path::new("/usr/share"),
        ]
        .into_iter()
        .chain(self.runtime_reads.iter().map(PathBuf::as_path))
        .chain(self.protected_reads.iter().map(PathBuf::as_path))
        .chain(self.runtime_read_roots.iter().map(PathBuf::as_path))
        .chain([
            self.project_root.as_path(),
            self.account_root.as_path(),
            self.temp_root.as_path(),
        ]) {
            p.push_str(&format!(
                "(allow file-read* (literal {}) (subpath {}))\n",
                quote(path)?,
                quote(path)?
            ));
        }
        for path in [&self.project_root, &self.account_root, &self.temp_root] {
            p.push_str(&format!("(allow file-write* (subpath {}))\n", quote(path)?));
        }
        for path in self
            .protected_reads
            .iter()
            .chain(self.runtime_reads.iter())
            .chain(self.runtime_read_roots.iter())
        {
            p.push_str(&format!(
                "(deny file-write* (literal {}) (subpath {}))\n",
                quote(path)?,
                quote(path)?
            ));
        }
        for path in &self.blocked_reads {
            p.push_str(&format!(
                "(deny file-read* file-write* (literal {}) (subpath {}))\n",
                quote(path)?,
                quote(path)?
            ));
        }
        p.push_str("(allow file-read* file-write* (literal \"/dev/null\"))\n(allow file-read* (literal \"/dev/urandom\") (literal \"/dev/random\"))\n");
        p.push_str(&format!(
            "(deny file-read* file-write* (literal {}) (subpath {}))\n",
            quote(control)?,
            quote(control)?
        ));
        // Native stdio uses anonymous pipes or the worker-owned terminal. Only
        // the unsandboxed supervisor opens these private transport FIFOs.
        for ancestor in control.ancestors() {
            p.push_str(&format!(
                "(deny file-write-unlink (literal {}))\n",
                quote(ancestor)?
            ));
        }
        if self.codex_preferences {
            p.push_str("(allow mach-lookup (global-name \"com.apple.cfprefsd.agent\") (local-name \"com.apple.cfprefsd.agent\") (global-name \"com.apple.cfprefsd.daemon\"))\n");
            p.push_str("(allow user-preference-read (preference-domain \"com.openai.codex\"))\n(deny user-preference-write)\n");
            p.push_str(&format!("(allow ipc-posix-shm-read-data (ipc-posix-name \"apple.cfprefs.{}v1\") (ipc-posix-name \"apple.cfprefs.daemonv1\"))\n", unsafe { libc::geteuid() }));
        } else {
            p.push_str("(deny mach-lookup)\n");
        }
        p.push_str("(deny mach-register)\n");
        if self.loopback_tcp_ports.is_empty()
            && !self.loopback_listener
            && self.unix_sockets.is_empty()
        {
            p.push_str("(deny network-outbound)\n(deny network-inbound)\n(deny network-bind)\n");
        } else {
            for port in &self.loopback_tcp_ports {
                p.push_str(&format!(
                    "(allow network-outbound (remote tcp \"localhost:{}\"))\n",
                    port
                ));
            }
            if self.loopback_listener {
                p.push_str("(allow network-bind network-inbound (local tcp \"localhost:*\"))\n");
            }
            for path in &self.unix_sockets {
                let parent = path.parent().ok_or_else(failure)?;
                p.push_str(&format!(
                    "(allow file-read-metadata (literal {}))\n",
                    quote(parent)?
                ));
                p.push_str(&format!(
                    "(allow network-outbound (remote unix-socket (path-literal {})))\n",
                    quote(path)?
                ));
                p.push_str(&format!(
                    "(allow file-read-metadata file-read-data file-write-data (literal {}))\n",
                    quote(path)?
                ));
            }
        }
        Ok(p)
    }
    pub(super) fn apply_terminal(
        &self,
        control: &Path,
        terminal: Option<&Path>,
    ) -> Result<(), String> {
        unsafe extern "C" {
            fn sandbox_init(
                profile: *const std::ffi::c_char,
                flags: u64,
                error: *mut *mut std::ffi::c_char,
            ) -> i32;
            fn sandbox_free_error(error: *mut std::ffi::c_char);
        }
        let mut profile = self.profile(control)?;
        if let Some(path) = terminal {
            if path.parent() != Some(Path::new("/dev"))
                || !path.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
                    s.starts_with("ttys") && s[4..].bytes().all(|b| b.is_ascii_digit())
                })
            {
                return Err(failure());
            }
            profile.push_str(&format!(
                "(allow file-read* file-write* (literal {}))\n",
                quote(path)?
            ));
            profile.push_str(&format!("(allow file-ioctl (literal {}))\n", quote(path)?));
        }
        let p = std::ffi::CString::new(profile).map_err(|_| failure())?;
        let mut error = std::ptr::null_mut();
        let result = unsafe { sandbox_init(p.as_ptr(), 0, &mut error) };
        if !error.is_null() {
            unsafe { sandbox_free_error(error) };
        }
        if result != 0 {
            return Err("The restrictive host sandbox profile was not admitted.".into());
        }
        Ok(())
    }
}
