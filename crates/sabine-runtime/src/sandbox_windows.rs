use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::{
            ACCESS_ALLOWED_ACE, ACE_FLAGS, ACE_HEADER, ACL, Authorization::*,
            DACL_SECURITY_INFORMATION, EqualSid, GetAce, INHERIT_ONLY_ACE, PSECURITY_DESCRIPTOR,
            PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        },
        Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ},
    },
    core::{PCWSTR, PWSTR, w},
};

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe { LocalFree(Some(HLOCAL(self.0))) };
    }
}

pub fn prepare_sandbox_access(path: &Path, inherit: bool) -> Result<(), String> {
    grant_access(path, inherit).map_err(|error| {
        format!(
            "could not prepare Chromium sandbox access to {}: {error}",
            path.display()
        )
    })
}

fn grant_access(path: &Path, inherit: bool) -> windows::core::Result<()> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let name = PCWSTR(name.as_ptr());
    let mut sid = PSID::default();
    unsafe { ConvertStringSidToSidW(w!("S-1-15-2-2"), &mut sid)? };
    let _sid = LocalAllocation(sid.0);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let mut acl = ptr::null_mut::<ACL>();
    unsafe {
        GetNamedSecurityInfoW(
            name,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut acl),
            None,
            &mut descriptor,
        )
        .ok()?;
    }
    let _descriptor = LocalAllocation(descriptor.0);
    let permissions = FILE_GENERIC_READ.0 | FILE_GENERIC_EXECUTE.0;
    let inheritance = if inherit {
        SUB_CONTAINERS_AND_OBJECTS_INHERIT
    } else {
        ACE_FLAGS(0)
    };
    if acl.is_null() || has_access(acl, sid, permissions, inheritance)? {
        return Ok(());
    }
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: permissions,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: inheritance,
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: PWSTR(sid.0.cast()),
            ..Default::default()
        },
    };
    let mut updated = ptr::null_mut::<ACL>();
    unsafe { SetEntriesInAclW(Some(&[entry]), Some(acl), &mut updated).ok()? };
    let _updated = LocalAllocation(updated.cast());
    unsafe {
        SetNamedSecurityInfoW(
            name,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(updated),
            None,
        )
        .ok()
    }
}

fn has_access(
    acl: *const ACL,
    sid: PSID,
    permissions: u32,
    inheritance: ACE_FLAGS,
) -> windows::core::Result<bool> {
    for index in 0..unsafe { (*acl).AceCount } {
        let mut entry = ptr::null_mut::<c_void>();
        unsafe { GetAce(acl, u32::from(index), &mut entry)? };
        if unsafe { (*entry.cast::<ACE_HEADER>()).AceType } != 0 {
            continue;
        }
        let entry = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
        if u32::from(entry.Header.AceFlags) & INHERIT_ONLY_ACE.0 == 0
            && u32::from(entry.Header.AceFlags) & inheritance.0 == inheritance.0
            && entry.Mask & permissions == permissions
            && unsafe { EqualSid(PSID(ptr::from_ref(&entry.SidStart).cast_mut().cast()), sid) }
                .is_ok()
        {
            return Ok(true);
        }
    }
    Ok(false)
}
