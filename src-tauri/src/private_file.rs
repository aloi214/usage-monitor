use std::path::Path;

fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| "OS RNG failed".to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// A private same-directory stage. All bytes are written through the original
/// exclusive-create handle; a replaced pathname is never reopened for writing.
/// Identity checks narrow pathname races but cannot make an uncooperative
/// external writer participate in an atomic compare-and-replace transaction.
pub(crate) struct PrivateStage {
    path: std::path::PathBuf,
    file: std::fs::File,
    committed: bool,
}

impl PrivateStage {
    pub(crate) fn new(destination: &Path) -> Result<Self, String> {
        let dir = destination
            .parent()
            .ok_or("file path is missing a directory")?;
        let path = dir.join(format!("pane.{}.tmp", random_id()?));
        #[cfg(windows)]
        let file = create_windows_stage(&path)?;
        #[cfg(not(windows))]
        let file = {
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options
                .open(&path)
                .map_err(|_| "create private stage failed")?
        };
        let stage = Self {
            path,
            file,
            committed: false,
        };
        stage.restrict_handle()?;
        stage.verify_owned()?;
        Ok(stage)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn writer(&mut self) -> &mut std::fs::File {
        &mut self.file
    }
    pub(crate) fn write(&mut self, contents: &[u8]) -> Result<(), String> {
        use std::io::{Seek, Write};
        self.verify_owned()?;
        self.file
            .set_len(0)
            .map_err(|_| "truncate private stage failed")?;
        self.file
            .rewind()
            .map_err(|_| "seek private stage failed")?;
        self.file
            .write_all(contents)
            .map_err(|_| "write private stage failed")?;
        self.file
            .sync_all()
            .map_err(|_| "sync private stage failed")?;
        self.verify_owned()
    }
    pub(crate) fn verify_owned(&self) -> Result<(), String> {
        let meta = std::fs::symlink_metadata(&self.path)
            .map_err(|_| "private stage changed externally")?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("private stage changed externally".into());
        }
        let other =
            std::fs::File::open(&self.path).map_err(|_| "private stage changed externally")?;
        if !same_file(&self.file, &other)? {
            return Err("private stage changed externally".into());
        }
        Ok(())
    }
    fn restrict_handle(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            self.file
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "restrict private stage failed".into())
        }
        #[cfg(windows)]
        {
            set_handle_dacl(
                &self.file,
                &format!("D:P(A;;FA;;;SY)(A;;FA;;;{})", current_user_sid()?),
            )
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err("private file permissions unsupported".into())
        }
    }
    pub(crate) fn commit(self, destination: &Path) -> Result<(), String> {
        self.commit_checked(destination, || Ok(()))
    }
    /// The writer is authorization-neutral. Credential callers can supply a
    /// final checkpoint after blocking sync, immediately before replacement.
    pub(crate) fn commit_checked(
        mut self,
        destination: &Path,
        checkpoint: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        self.verify_owned()?;
        self.file
            .sync_all()
            .map_err(|_| "sync private stage failed")?;
        // The restricted stage carries its permissions through replacement.
        // No post-rename chmod on a pathname which another writer can replace.
        checkpoint()?;
        replace_existing(&self.path, destination).map_err(|_| "replace private file failed")?;
        self.committed = true;
        Ok(())
    }
}
impl Drop for PrivateStage {
    fn drop(&mut self) {
        if !self.committed && self.verify_owned().is_ok() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
/// Windows needs the protected DACL at creation, not only a later permission
/// update: an inherited public ACL could let another process retain a handle
/// before the first restriction. CREATE_NEW and delete sharing preserve unique
/// staging and allow same-directory replacement with this handle still open.
#[cfg(windows)]
fn create_windows_stage(path: &Path) -> Result<std::fs::File, String> {
    use std::os::windows::io::FromRawHandle;
    use windows::core::PCWSTR;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };
    let sddl = format!("D:P(A;;FA;;;SY)(A;;FA;;;{})", current_user_sid()?);
    let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|_| "create private stage security descriptor failed")?;
    let _free = LocalAlloc(descriptor.0);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let path = path_wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            0xC0040000,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            Some(&attributes),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|_| "create private stage failed")?;
    Ok(unsafe { std::fs::File::from_raw_handle(handle.0) })
}

fn same_file(a: &std::fs::File, b: &std::fs::File) -> Result<bool, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let a = a.metadata().map_err(|_| "file identity unavailable")?;
        let b = b.metadata().map_err(|_| "file identity unavailable")?;
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let info = |file: &std::fs::File| -> Result<BY_HANDLE_FILE_INFORMATION, String> {
            let mut i = BY_HANDLE_FILE_INFORMATION::default();
            unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut i) }
                .map_err(|_| "file identity unavailable")?;
            Ok(i)
        };
        let a = info(a)?;
        let b = info(b)?;
        Ok(a.dwVolumeSerialNumber == b.dwVolumeSerialNumber
            && a.nFileIndexHigh == b.nFileIndexHigh
            && a.nFileIndexLow == b.nFileIndexLow)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (a, b);
        Err("file identity unsupported".into())
    }
}

/// Generic private file writes deliberately have no account authorization
/// dependency. Credential callers must apply their own permit checkpoints.
/// All fallible validation, permissions and sync steps precede replacement.
/// Once replacement succeeds this function returns Ok with no post-commit IO.
pub(crate) fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("file path is missing a directory")?;
    std::fs::create_dir_all(dir).map_err(|_| "create config directory failed")?;
    let mut stage = PrivateStage::new(path)?;
    stage.write(contents.as_bytes())?;
    stage.commit(path)
}

pub(crate) fn restrict_owner_only(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let sid = current_user_sid()?;
        set_protected_dacl(path, &format!("D:P(A;;FA;;;SY)(A;;FA;;;{sid})"))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("restrict credential file: {e}"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(windows)]
fn current_user_sid() -> Result<String, String> {
    static SID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(s) = SID.get() {
        return Ok(s.clone());
    }
    let s = query_current_user_sid()?;
    Ok(SID.get_or_init(|| s).clone())
}

#[cfg(windows)]
struct LocalAlloc(*mut core::ffi::c_void);

#[cfg(windows)]
impl Drop for LocalAlloc {
    fn drop(&mut self) {
        if !self.0.is_null() {
            use windows::Win32::Foundation::{LocalFree, HLOCAL};
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }
}

#[cfg(windows)]
fn query_current_user_sid() -> Result<String, String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| format!("restrict credential file: {e}"))?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        let mut buf = vec![0u8; needed as usize];
        let info = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        info.map_err(|e| format!("restrict credential file: {e}"))?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut sid)
            .map_err(|e| format!("restrict credential file: {e}"))?;
        let _free = LocalAlloc(sid.0.cast());
        sid.to_string()
            .map_err(|e| format!("restrict credential file: {e}"))
    }
}

#[cfg(windows)]
fn set_protected_dacl(path: &Path, sddl: &str) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    let path_w = path_wide(path);
    let sddl_w: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_w.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
        .map_err(|e| format!("restrict credential file: {e}"))?;
        let _free = LocalAlloc(sd.0);
        let mut present = windows::core::BOOL::default();
        let mut defaulted = windows::core::BOOL::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted)
            .map_err(|e| format!("restrict credential file: {e}"))?;
        if !present.as_bool() || dacl.is_null() {
            return Err("restrict credential file: missing DACL".into());
        }
        let err = SetNamedSecurityInfoW(
            PCWSTR(path_w.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        );
        if err != ERROR_SUCCESS {
            return Err(format!("restrict credential file: {err:?}"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn set_handle_dacl(file: &std::fs::File, sddl: &str) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetSecurityInfo, SDDL_REVISION_1,
        SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    let sddl_w: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_w.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
        .map_err(|e| format!("restrict credential file: {e}"))?;
        let _free = LocalAlloc(sd.0);
        let mut present = windows::core::BOOL::default();
        let mut defaulted = windows::core::BOOL::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted)
            .map_err(|e| format!("restrict credential file: {e}"))?;
        if !present.as_bool() || dacl.is_null() {
            return Err("restrict credential file: missing DACL".into());
        }
        let err = SetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        );
        if err != ERROR_SUCCESS {
            return Err(format!("restrict credential file: {err:?}"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn path_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(all(windows, test))]
fn dacl_sddl(path: &Path) -> Result<String, String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

    let path_w = path_wide(path);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        let err = GetNamedSecurityInfoW(
            PCWSTR(path_w.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut sd,
        );
        if err != ERROR_SUCCESS {
            return Err(format!("read DACL: {err:?}"));
        }
        let _free_sd = LocalAlloc(sd.0);
        let mut sddl = PWSTR::null();
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut sddl,
            None,
        )
        .map_err(|e| format!("read DACL: {e}"))?;
        let _free_sddl = LocalAlloc(sddl.0.cast());
        sddl.to_string().map_err(|e| format!("read DACL: {e}"))
    }
}

#[cfg(windows)]
fn replace_existing(replacement: &Path, destination: &Path) -> std::io::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let destination = path_wide(destination);
    let replacement = path_wide(replacement);
    unsafe {
        MoveFileExW(
            PCWSTR(replacement.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(std::io::Error::other)
}

#[cfg(not(windows))]
fn replace_existing(replacement: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(replacement, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir()
                .join(format!("pane-private-file-test-{}", random_id().unwrap()));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn owner_only(path: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(path).unwrap().permissions().mode() & 0o077 == 0
        }
        #[cfg(windows)]
        {
            let dacl = dacl_sddl(path).unwrap();
            dacl.contains("D:P")
                && !dacl.contains(";;;WD)")
                && !dacl.contains(";;;BU)")
                && !dacl.contains(";;;AU)")
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            false
        }
    }
    #[test]
    fn atomic_write_replaces_existing_without_temp_residue() {
        let dir = TempDir::new();
        let path = dir.0.join("fake.json");
        atomic_write(&path, "first fake value").unwrap();
        atomic_write(&path, "second fake value").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second fake value");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
        assert!(owner_only(&path));
    }
    #[cfg(windows)]
    #[test]
    fn native_replacement_sharing_violation_preserves_old_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = TempDir::new();
        let path = dir.0.join("fake.json");
        atomic_write(&path, "old fake pair").unwrap();
        let blocker = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(&path)
            .unwrap();
        let mut stage = PrivateStage::new(&path).unwrap();
        let staged = stage.path().to_path_buf();
        stage.write(b"new fake pair").unwrap();
        assert!(stage.commit(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old fake pair");
        assert!(!staged.exists());
        drop(blocker);
    }
    #[test]
    fn atomic_write_restricts_permissive_destination() {
        let dir = TempDir::new();
        let path = dir.0.join("fake.json");
        std::fs::write(&path, "old fake value").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o777)).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        }
        #[cfg(windows)]
        {
            set_protected_dacl(&dir.0, "D:P(A;OICI;FA;;;WD)").unwrap();
            set_protected_dacl(&path, "D:P(A;;FA;;;WD)").unwrap();
        }
        assert!(!owner_only(&path));
        atomic_write(&path, "new fake value").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new fake value");
        assert!(owner_only(&path));
    }
}
