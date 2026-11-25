use std::{
    ffi::c_void, io, mem::size_of, os::windows::io::AsRawHandle, path::Path, ptr::null_mut,
    sync::OnceLock, time::Duration,
};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_PIPE_BUSY, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
        },
        GetTokenInformation, OWNER_SECURITY_INFORMATION, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct LocalAllocation(*mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn sid_string(sid: *mut c_void) -> io::Result<String> {
    let mut text = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _owned = LocalAllocation(text.cast());
    let mut len = 0;
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
            text, len,
        )))
    }
}

pub fn user_sid() -> io::Result<String> {
    static SID: OnceLock<String> = OnceLock::new();
    if let Some(sid) = SID.get() {
        return Ok(sid.clone());
    }
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut bytes = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut bytes);
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        );
        let error = io::Error::last_os_error();
        CloseHandle(token);
        if ok == 0 {
            return Err(error);
        }
        let sid = sid_string((*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid)?;
        let _ = SID.set(sid.clone());
        Ok(sid)
    }
}

pub(crate) fn local(path: &Path) -> io::Result<()> {
    if !path
        .to_string_lossy()
        .strip_prefix(r"\\.\pipe\")
        .is_some_and(|name| !name.is_empty() && !name.contains(['\\', '/', '\0']))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            r"expected a local \\.\pipe\NAME",
        ));
    }
    Ok(())
}

pub(crate) fn listener(path: &Path, first: bool) -> io::Result<NamedPipeServer> {
    local(path)?;
    let sid = user_sid()?;
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})\0")
        .encode_utf16()
        .collect();
    unsafe {
        let mut descriptor = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let _owned = LocalAllocation(descriptor);
        let mut attrs = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                path,
                (&mut attrs as *mut SECURITY_ATTRIBUTES).cast(),
            )
    }
}

pub async fn connect(path: &Path) -> io::Result<NamedPipeClient> {
    local(path)?;
    let mut attempts = 0;
    let stream = loop {
        match ClientOptions::new().open(path) {
            Ok(stream) => break stream,
            Err(err) if err.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) && attempts < 40 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(err) => return Err(err),
        }
    };
    unsafe {
        let mut owner = null_mut();
        let mut descriptor = null_mut();
        let code = GetSecurityInfo(
            stream.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        );
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        let _owned = LocalAllocation(descriptor);
        if owner.is_null() || sid_string(owner)? != user_sid()? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "named pipe is not owned by the current user",
            ));
        }
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{ACCESS_ALLOWED_ACE, DACL_SECURITY_INFORMATION, GetAce};

    #[tokio::test]
    async fn pipe_only_grants_access_to_current_user() -> io::Result<()> {
        assert!(local(Path::new(r"\\remote\pipe\test")).is_err());
        let path = std::path::PathBuf::from(format!(
            r"\\.\pipe\hostlane-test-{}",
            crate::util::new_id("acl")
        ));
        let server = listener(&path, true)?;
        assert!(listener(&path, true).is_err());
        let client = connect(&path).await?;
        server.connect().await?;
        unsafe {
            let mut descriptor = null_mut();
            let mut acl = null_mut();
            let code = GetSecurityInfo(
                client.as_raw_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            );
            assert_eq!(code, 0);
            let _owned = LocalAllocation(descriptor);
            assert!(!acl.is_null());
            assert_eq!((*acl).AceCount, 1);
            let mut ace = null_mut();
            assert_ne!(GetAce(acl, 0, &mut ace), 0);
            let allowed = ace.cast::<ACCESS_ALLOWED_ACE>();
            assert_eq!((*allowed).Header.AceType, 0);
            assert_eq!(
                sid_string(std::ptr::addr_of_mut!((*allowed).SidStart).cast())?,
                user_sid()?
            );
        }
        Ok(())
    }
}
