//! Public CommonCrypto SHA-256 ABI from the macOS SDK CommonDigest.h.
//! Native hashing avoids debug-build software hashing throttled by launchd.
#[repr(C)]
#[derive(Default)]
struct Context {
    count: [u32; 2],
    hash: [u32; 8],
    wbuf: [u32; 16],
}
#[link(name = "System")]
unsafe extern "C" {
    fn CC_SHA256_Init(context: *mut Context) -> i32;
    fn CC_SHA256_Update(context: *mut Context, data: *const std::ffi::c_void, length: u32) -> i32;
    fn CC_SHA256_Final(output: *mut u8, context: *mut Context) -> i32;
}
pub(super) struct Hasher(Context);
impl Hasher {
    pub(super) fn new() -> Result<Self, String> {
        let mut value = Self(Context::default());
        if unsafe { CC_SHA256_Init(&mut value.0) } != 1 {
            return Err(super::failure());
        }
        Ok(value)
    }
    pub(super) fn update(&mut self, bytes: &[u8]) -> Result<(), String> {
        let length = u32::try_from(bytes.len()).map_err(|_| super::failure())?;
        if unsafe { CC_SHA256_Update(&mut self.0, bytes.as_ptr().cast(), length) } != 1 {
            return Err(super::failure());
        }
        Ok(())
    }
    pub(super) fn finish(mut self) -> Result<String, String> {
        let mut output = [0u8; 32];
        if unsafe { CC_SHA256_Final(output.as_mut_ptr(), &mut self.0) } != 1 {
            return Err(super::failure());
        }
        Ok(output.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn native_digest_matches_the_existing_sha256_contract() {
        let bytes: Vec<u8> = (0..70000).map(|i| (i % 251) as u8).collect();
        let mut hasher = super::Hasher::new().unwrap();
        for chunk in bytes.chunks(4096) {
            hasher.update(chunk).unwrap();
        }
        assert_eq!(
            hasher.finish().unwrap(),
            crate::agent_runtime::host_boundary::digest(&bytes)
        );
    }
}
