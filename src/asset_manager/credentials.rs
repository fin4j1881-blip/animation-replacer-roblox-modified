/// Scans Windows Credential Manager for a logged-in Roblox Studio `.ROBLOSECURITY` cookie.
///
/// Returns `Some(cookie)` if a valid cookie starting with `_|WARNING:-DO-NOT-SHARE-THIS` is found,
/// or `None` on non-Windows platforms or if no Studio session is stored.
#[cfg(target_os = "windows")]
pub fn scan_windows_credential_for_cookie() -> Option<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    #[allow(clippy::upper_case_acronyms)]
    struct CREDENTIALW {
        flags: u32,
        cred_type: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: [u32; 2],
        credential_blob_size: u32,
        credential_blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut std::ffi::c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn CredEnumerateW(
            filter: *const u16,
            flags: u32,
            count: *mut u32,
            credentials: *mut *mut *mut CREDENTIALW,
        ) -> i32;
        fn CredReadW(
            target_name: *const u16,
            cred_type: u32,
            flags: u32,
            credential: *mut *mut CREDENTIALW,
        ) -> i32;
        fn CredFree(buffer: *mut std::ffi::c_void);
    }

    unsafe {
        // First try to enumerate targets matching RobloxStudioAuth*
        let filter: Vec<u16> = OsStr::new("https://www.roblox.com:RobloxStudioAuth*")
            .encode_wide()
            .chain(Some(0))
            .collect();

        let mut count: u32 = 0;
        let mut creds_ptr: *mut *mut CREDENTIALW = std::ptr::null_mut();

        if CredEnumerateW(filter.as_ptr(), 0, &mut count, &mut creds_ptr) != 0
            && !creds_ptr.is_null()
        {
            let slice = std::slice::from_raw_parts(creds_ptr, count as usize);
            for &c_ptr in slice {
                if c_ptr.is_null() {
                    continue;
                }
                let c = &*c_ptr;
                if c.credential_blob.is_null() || c.credential_blob_size == 0 {
                    continue;
                }
                let blob_len = c.credential_blob_size as usize;
                let blob = std::slice::from_raw_parts(c.credential_blob, blob_len);

                let parsed = String::from_utf8(blob.to_vec()).ok().or_else(|| {
                    if blob_len.is_multiple_of(2) {
                        let u16_slice = std::slice::from_raw_parts(
                            c.credential_blob as *const u16,
                            blob_len / 2,
                        );
                        String::from_utf16(u16_slice).ok()
                    } else {
                        None
                    }
                });

                if let Some(s) = parsed {
                    let trimmed = s.trim().to_string();
                    if trimmed.starts_with("_|WARNING:-DO-NOT-SHARE-THIS") {
                        CredFree(creds_ptr as *mut std::ffi::c_void);
                        return Some(trimmed);
                    }
                }
            }
            CredFree(creds_ptr as *mut std::ffi::c_void);
        }

        // Fallback: try direct targets
        let targets = [
            "https://www.roblox.com:RobloxStudioAuthCookies",
            "https://www.roblox.com:RobloxStudioAuth.ROBLOSECURITY",
        ];

        for t in targets {
            let wide: Vec<u16> = OsStr::new(t).encode_wide().chain(Some(0)).collect();
            let mut ptr: *mut CREDENTIALW = std::ptr::null_mut();
            if CredReadW(wide.as_ptr(), 1, 0, &mut ptr) != 0 && !ptr.is_null() {
                let c = &*ptr;
                let parsed = if !c.credential_blob.is_null() && c.credential_blob_size > 0 {
                    let blob_len = c.credential_blob_size as usize;
                    let blob = std::slice::from_raw_parts(c.credential_blob, blob_len);
                    String::from_utf8(blob.to_vec()).ok().or_else(|| {
                        if blob_len.is_multiple_of(2) {
                            let u16_slice = std::slice::from_raw_parts(
                                c.credential_blob as *const u16,
                                blob_len / 2,
                            );
                            String::from_utf16(u16_slice).ok()
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };
                CredFree(ptr as *mut std::ffi::c_void);

                if let Some(s) = parsed {
                    let trimmed = s.trim().to_string();
                    if trimmed.starts_with("_|WARNING:-DO-NOT-SHARE-THIS") {
                        return Some(trimmed);
                    }
                }
            }
        }
    }

    None
}

#[cfg(not(target_os = "windows"))]
pub fn scan_windows_credential_for_cookie() -> Option<String> {
    None
}
