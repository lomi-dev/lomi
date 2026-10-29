use lomi_control_protocol::ErrorCode;
use lomi_remote_protocol::{
    ipc::{LocalNativeGitResult, NativeIntentId},
    WorkspaceId,
};
use std::path::PathBuf;

pub struct GitMutationHostPrepare {
    pub operation_id: NativeIntentId,
    pub workspace_id: WorkspaceId,
    pub workspace_root: PathBuf,
    pub request_json: String,
    pub timeout_ms: u64,
}

pub struct GitMutationHostPrepared {
    pub request_hash: String,
    pub response: Result<LocalNativeGitResult, ErrorCode>,
}

pub trait GitMutationHost {
    fn prepare_guarded(
        &mut self,
        request: GitMutationHostPrepare,
        check: &mut dyn FnMut() -> bool,
    ) -> Result<GitMutationHostPrepared, ErrorCode>;

    fn execute_guarded(
        &mut self,
        operation_id: NativeIntentId,
        request_hash: &str,
        plan_hash: &str,
        timeout_ms: u64,
        check: &mut dyn FnMut() -> bool,
    ) -> Result<LocalNativeGitResult, ErrorCode>;

    fn status(&mut self, operation_id: NativeIntentId) -> Result<LocalNativeGitResult, ErrorCode>;
    fn cancel_prepared(
        &mut self,
        operation_id: NativeIntentId,
    ) -> Result<LocalNativeGitResult, ErrorCode>;
    fn acknowledge(
        &mut self,
        operation_id: NativeIntentId,
    ) -> Result<LocalNativeGitResult, ErrorCode>;

    fn retain_prepared_for_approval(&mut self, _operation_id: NativeIntentId) {}
}
