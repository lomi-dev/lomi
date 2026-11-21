use std::{os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SetNamedSecurityInfoW, SE_FILE_OBJECT,
        },
        GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub fn private(path: &Path, directory: bool) -> Result<(), String> {
    let result = unsafe { restrict(path, directory) };
    result.map_err(|_| "Cannot restrict storage to the current Windows user.".into())
}
unsafe fn restrict(path: &Path, directory: bool) -> Result<(), ()> {
    let mut token = ptr::null_mut();
    if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
        return Err(());
    }
    let mut length = 0;
    GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut length);
    if length == 0 || length > 65536 {
        CloseHandle(token);
        return Err(());
    }
    // TOKEN_USER contains pointers; usize provides the required alignment.
    let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    let read = GetTokenInformation(
        token,
        TokenUser,
        buffer.as_mut_ptr().cast(),
        length,
        &mut length,
    );
    CloseHandle(token);
    if read == 0 {
        return Err(());
    }
    let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
    let mut sid = ptr::null_mut();
    if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
        return Err(());
    }
    let mut size = 0;
    while *sid.add(size) != 0 {
        size += 1;
    }
    let identity = String::from_utf16(std::slice::from_raw_parts(sid, size));
    LocalFree(sid.cast());
    let identity = identity.map_err(|_| ())?;
    let inherit = if directory { "OICI" } else { "" };
    let descriptor = format!("D:P(A;{inherit};FA;;;{identity})(A;{inherit};FA;;;SY)")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut security = ptr::null_mut();
    if ConvertStringSecurityDescriptorToSecurityDescriptorW(
        descriptor.as_ptr(),
        1,
        &mut security,
        ptr::null_mut(),
    ) == 0
    {
        return Err(());
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    if GetSecurityDescriptorDacl(security, &mut present, &mut acl, &mut defaulted) == 0
        || present == 0
        || acl.is_null()
    {
        LocalFree(security);
        return Err(());
    }
    let name = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let code = SetNamedSecurityInfoW(
        name.as_ptr(),
        SE_FILE_OBJECT,
        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
        ptr::null_mut(),
        ptr::null_mut(),
        acl,
        ptr::null(),
    );
    LocalFree(security);
    if code == 0 {
        Ok(())
    } else {
        Err(())
    }
}
