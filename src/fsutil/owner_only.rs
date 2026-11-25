//! Windows has no file mode, so the 0700 and 0600 of Unix become an access
//! list that names only the signed-in user. It's protected, so nothing the
//! folders above allow carries over, and files created inside inherit it.

use anyhow::{Context, Result};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
    SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub fn restrict(path: &Path) -> Result<()> {
    let descriptor = format!("D:P(A;OICI;FA;;;{})", current_user()?);
    let mut security: PSECURITY_DESCRIPTOR = null_mut();

    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(wide(descriptor.as_ref()).as_ptr(), SDDL_REVISION_1, &mut security, null_mut()) == 0 {
            return Err(std::io::Error::last_os_error()).context("could not build an access list for the current user");
        }

        let mut present = 0;
        let mut defaulted = 0;
        let mut acl: *mut ACL = null_mut();
        let result = if GetSecurityDescriptorDacl(security, &mut present, &mut acl, &mut defaulted) == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            match SetNamedSecurityInfoW(
                wide(path.as_os_str()).as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null(),
            ) {
                ERROR_SUCCESS => Ok(()),
                code => Err(std::io::Error::from_raw_os_error(code as i32)),
            }
        };

        LocalFree(security);
        result.with_context(|| format!("could not restrict {} to the current user", path.display()))
    }
}

/// The signed-in user's SID, as in S-1-5-21-….
fn current_user() -> Result<String> {
    unsafe {
        let mut token: HANDLE = null_mut();

        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error()).context("could not look up the current user");
        }

        let user = user_of(token);
        CloseHandle(token);
        user
    }
}

unsafe fn user_of(token: HANDLE) -> Result<String> {
    unsafe {
        let mut size = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);

        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        if GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), size, &mut size) == 0 {
            return Err(std::io::Error::last_os_error()).context("could not look up the current user");
        }

        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(std::io::Error::last_os_error()).context("could not look up the current user");
        }

        let length = (0..).take_while(|&it| *text.add(it) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        Ok(sid)
    }
}

fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}
