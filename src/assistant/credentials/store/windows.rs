//! Windows' Credential Manager: each key a generic credential named
//! "Hex Assistant/<account>", kept for this user on this computer
//! (`CRED_PERSIST_LOCAL_MACHINE`), its blob the key's UTF-8.

use color_eyre::Result;
use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};

use super::{SERVICE, refused, target};
use crate::assistant::backends::os::Os;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Saves `key` (already checked to be plausible) for `account`.
pub fn save(account: &str, key: &str) -> Result<()> {
    let mut name = wide(&target(account));
    let mut user = wide(SERVICE);
    let mut blob = key.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: name.as_mut_ptr(),
        UserName: user.as_mut_ptr(),
        CredentialBlobSize: u32::try_from(blob.len()).unwrap_or(u32::MAX),
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..CREDENTIALW::default()
    };
    // SAFETY: every pointer in `credential` points into a buffer that lives
    // through the call; CredWriteW copies what it keeps.
    let written = unsafe { CredWriteW(&credential, 0) };
    if written == 0 {
        return Err(refused(
            Os::Windows,
            "save",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

pub fn load(account: &str) -> Option<String> {
    let name = wide(&target(account));
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated; on success `found` points to a
    // credential Windows allocated, freed below with CredFree.
    if unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut found) } == 0 || found.is_null()
    {
        return None;
    }
    // SAFETY: `found` is valid until CredFree; its blob has the size given.
    let key = unsafe {
        let credential = &*found;
        let size = credential.CredentialBlobSize as usize;
        let bytes = if credential.CredentialBlob.is_null() || size == 0 {
            &[][..]
        } else {
            std::slice::from_raw_parts(credential.CredentialBlob, size)
        };
        String::from_utf8(bytes.to_vec()).ok()
    };
    // SAFETY: `found` came from CredReadW and is freed once.
    unsafe { CredFree(found.cast()) };
    key
}

pub fn delete(account: &str) -> Result<()> {
    let name = wide(&target(account));
    // SAFETY: `name` is NUL-terminated and lives through the call.
    if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
        // Nothing was saved.
        return Ok(());
    }
    Err(refused(Os::Windows, "delete", error))
}
