use std::io;

const PRODUCT_NAME_QUERY: &str = r"\StringFileInfo\000004b0\ProductName";

#[derive(Debug, PartialEq)]
enum Cleanup {
    Absent,
    Removed,
    Preserved,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum ExecutableIdentity {
    Missing,
    Private,
    Other,
}
trait StartupRegistry {
    fn executable_identity(&self, target: &str) -> io::Result<ExecutableIdentity>;
    fn read_legacy_run(&self) -> io::Result<Option<String>>;
    fn delete_legacy_run_if_unchanged(&mut self, expected: &str) -> io::Result<bool>;
}
// This migration never enables startup or enumerates/removes other entries.
// auto-launch 0.5.0 used productName = "Pane Private" and no arguments.
fn cleanup_legacy_run(
    registry: &mut impl StartupRegistry,
    targets: &[String],
) -> io::Result<Cleanup> {
    let Some(command) = registry.read_legacy_run()? else {
        return Ok(Cleanup::Absent);
    };
    let Some(target) = command_target(&command) else {
        return Ok(Cleanup::Preserved);
    };
    let owned = match registry.executable_identity(&target)? {
        ExecutableIdentity::Private => true,
        ExecutableIdentity::Other => false,
        // Uninstallation may already have removed the executable. Only exact
        // app-owned paths may establish ownership of a stale command.
        ExecutableIdentity::Missing => targets
            .iter()
            .any(|path| path.eq_ignore_ascii_case(&target)),
    };
    if !owned {
        return Ok(Cleanup::Preserved);
    }
    Ok(if registry.delete_legacy_run_if_unchanged(&command)? {
        Cleanup::Removed
    } else {
        Cleanup::Preserved
    })
}

fn command_target(command: &str) -> Option<String> {
    if command.chars().any(|c| c.is_control() && c != '\t') {
        return None;
    }
    let command = command.trim_matches([' ', '\t']);
    let path = if command.starts_with('"') {
        command.strip_prefix('"')?.strip_suffix('"')?
    } else {
        command
    };
    let path = normalized_absolute_path(path)?;
    path.to_ascii_lowercase().ends_with(".exe").then_some(path)
}

// Lexical Windows paths work even after uninstall. Never canonicalize through
// symlinks/junctions, expand environment variables, or accept appended arguments.
fn normalized_absolute_path(path: &str) -> Option<String> {
    let path = path.replace('/', "\\");
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    let (prefix, rest) = if path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().get(2) == Some(&b'\\')
        && path.as_bytes()[0].is_ascii_alphabetic()
    {
        (&path[..3], &path[3..])
    } else {
        return None;
    };
    if rest.is_empty()
        || rest.split('\\').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with([' ', '.'])
                || part.chars().any(|c| {
                    c.is_control() || matches!(c, '"' | ':' | '%' | '<' | '>' | '|' | '?' | '*')
                })
        })
    {
        return None;
    }
    Some(format!("{prefix}{rest}"))
}

fn known_targets(
    current_exe: &str,
    local_app_data: Option<&str>,
    legacy_install_dir: Option<&str>,
) -> Vec<String> {
    let mut targets = Vec::new();
    if let Some(exe) = normalized_absolute_path(current_exe) {
        if let Some((dir, _)) = exe.rsplit_once('\\') {
            targets.push(format!(r"{dir}\pane.exe"));
        }
        targets.push(exe);
    }
    if let Some(dir) = local_app_data.and_then(normalized_absolute_path) {
        targets.push(format!(r"{dir}\Pane Private\pane.exe"));
    }
    if let Some(dir) = legacy_install_dir.and_then(normalized_absolute_path) {
        targets.push(format!(r"{dir}\pane.exe"));
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Registry {
        values: HashMap<String, String>,
        identities: HashMap<String, ExecutableIdentity>,
        probes: std::cell::RefCell<Vec<String>>,
        fail_read: bool,
        fail_delete: bool,
        replacement: Option<String>,
    }
    impl StartupRegistry for Registry {
        fn executable_identity(&self, target: &str) -> io::Result<ExecutableIdentity> {
            self.probes.borrow_mut().push(target.into());
            Ok(self
                .identities
                .get(target)
                .copied()
                .unwrap_or(ExecutableIdentity::Missing))
        }
        fn read_legacy_run(&self) -> io::Result<Option<String>> {
            if self.fail_read {
                return Err(io::Error::other("synthetic read failure"));
            }
            Ok(self.values.get("Pane Private").cloned())
        }
        fn delete_legacy_run_if_unchanged(&mut self, expected: &str) -> io::Result<bool> {
            if self.fail_delete {
                return Err(io::Error::other("synthetic delete failure"));
            }
            if let Some(value) = self.replacement.take() {
                self.values.insert("Pane Private".into(), value);
            }
            if self.values.get("Pane Private").map(String::as_str) != Some(expected) {
                return Ok(false);
            }
            self.values.remove("Pane Private");
            Ok(true)
        }
    }
    fn registry(command: &str) -> Registry {
        Registry {
            values: HashMap::from([
                ("Pane Private".into(), command.into()),
                ("Pane".into(), r#""C:\Tools\Pane\pane.exe" "#.into()),
                ("Unrelated".into(), r#""C:\Other\other.exe""#.into()),
            ]),
            ..Default::default()
        }
    }
    fn targets() -> Vec<String> {
        known_targets(
            r"C:\Tools\rice monitor\rice-monitor.exe",
            Some(r"C:\Users\Tester\AppData\Local"),
            Some(r"D:\Custom private install"),
        )
    }

    #[test]
    fn removes_only_own_legacy_value_for_each_known_exact_target() {
        for target in [
            r"C:\Tools\rice monitor\rice-monitor.exe",
            r"C:\Tools\rice monitor\pane.exe",
            r"C:\Users\Tester\AppData\Local\Pane Private\pane.exe",
            r"D:\Custom private install\pane.exe",
        ] {
            for command in [format!("\"{target}\" "), format!("{target} ")] {
                let mut registry = registry(&command);
                assert_eq!(
                    cleanup_legacy_run(&mut registry, &targets()).unwrap(),
                    Cleanup::Removed,
                    "{command}"
                );
                assert_eq!(registry.values.len(), 2);
                assert!(registry.values.contains_key("Pane"));
                assert!(registry.values.contains_key("Unrelated"));
                assert_eq!(
                    cleanup_legacy_run(&mut registry, &targets()).unwrap(),
                    Cleanup::Absent
                );
            }
        }
    }
    #[test]
    fn preserves_unrelated_targets_arguments_and_malformed_commands() {
        for command in [
            r#""C:\Tools\Pane\pane.exe" "#,
            r#""C:\Other\pane.exe""#,
            r#""C:\Tools\rice monitor\other.exe""#,
            r#""C:\Tools\rice monitor\pane.exe" --extra"#,
            r"C:\Tools\rice monitor\pane.exe --extra",
            r#""C:\Tools\rice monitor\pane.exe" && calc.exe"#,
            r"pane.exe",
            r"C:pane.exe",
            r"%LOCALAPPDATA%\Pane Private\pane.exe",
            r"C:\Tools\rice monitor\..\Pane\pane.exe",
            r#""C:\Tools\rice monitor\pane.exe"#,
            "C:\\Tools\\rice monitor\\pane.exe\0other",
            "C:\\Tools\\rice monitor\\pane.exe\r\n",
        ] {
            let mut registry = registry(command);
            let before = registry.values.clone();
            assert_eq!(
                cleanup_legacy_run(&mut registry, &targets()).unwrap(),
                Cleanup::Preserved,
                "{command}"
            );
            assert_eq!(registry.values, before);
        }
    }
    #[test]
    fn accepts_case_slashes_and_verbatim_drive_paths() {
        for command in [
            r#""c:\TOOLS\Rice Monitor\PANE.EXE" "#,
            r"C:/Tools/rice monitor/pane.exe",
            r#""\\?\C:\Tools\rice monitor\pane.exe" "#,
        ] {
            assert_eq!(
                cleanup_legacy_run(&mut registry(command), &targets()).unwrap(),
                Cleanup::Removed
            );
        }
    }
    #[test]
    fn does_not_promote_untrusted_or_relative_install_directories() {
        assert!(known_targets(
            "rice-monitor.exe",
            Some("relative"),
            Some(r"C:\Private\..\Pane")
        )
        .is_empty());
    }
    #[test]
    fn missing_values_are_an_idempotent_noop() {
        assert_eq!(
            cleanup_legacy_run(&mut Registry::default(), &targets()).unwrap(),
            Cleanup::Absent
        );
    }
    #[test]
    fn read_and_delete_errors_propagate_without_removing_other_values() {
        for read in [true, false] {
            let mut registry = registry(r#""C:\Tools\rice monitor\pane.exe" "#);
            registry.fail_read = read;
            registry.fail_delete = !read;
            let before = registry.values.clone();
            assert!(cleanup_legacy_run(&mut registry, &targets()).is_err());
            assert_eq!(registry.values, before);
        }
    }
    #[test]
    fn version_resource_query_matches_tauri_winres_neutral_language_default() {
        // tauri-winres 0.3.6 writes "{:04x}04b0" with language = 0;
        // tauri-build does not call set_language. English-only lookup fails.
        assert_eq!(PRODUCT_NAME_QUERY, r"\StringFileInfo\000004b0\ProductName");
    }
    #[test]
    fn remote_and_device_commands_are_preserved_without_probing_files() {
        for command in [
            r"\\server\share\pane.exe",
            r"\\?\UNC\server\share\pane.exe",
            r"\\.\C:\pane.exe",
        ] {
            let mut registry = registry(command);
            assert_eq!(
                cleanup_legacy_run(&mut registry, &targets()).unwrap(),
                Cleanup::Preserved
            );
            assert!(registry.probes.borrow().is_empty());
        }
    }
    #[test]
    fn unrelated_existing_binary_overrides_known_path_assumptions() {
        let target = r"C:\Tools\rice monitor\pane.exe";
        let mut registry = registry(target);
        registry
            .identities
            .insert(target.into(), ExecutableIdentity::Other);
        assert_eq!(
            cleanup_legacy_run(&mut registry, &targets()).unwrap(),
            Cleanup::Preserved
        );
        assert_eq!(registry.values.len(), 3);
    }
    #[test]
    fn verified_private_portable_binary_is_removed_outside_known_paths() {
        let target = r"D:\Portable apps\custom-name.exe";
        let mut registry = registry(target);
        registry
            .identities
            .insert(target.into(), ExecutableIdentity::Private);
        assert_eq!(
            cleanup_legacy_run(&mut registry, &targets()).unwrap(),
            Cleanup::Removed
        );
        assert_eq!(registry.values.len(), 2);
    }
    #[test]
    fn changed_value_is_preserved_before_deletion() {
        let mut registry = registry(r#""C:\Tools\rice monitor\pane.exe" "#);
        registry.replacement = Some(r#""C:\Other\app.exe""#.into());
        assert_eq!(
            cleanup_legacy_run(&mut registry, &targets()).unwrap(),
            Cleanup::Preserved
        );
        assert_eq!(registry.values["Pane Private"], r#""C:\Other\app.exe""#);
    }
}

#[cfg(windows)]
mod windows_registry {
    use super::*;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
        FILE_ATTRIBUTE_REPARSE_POINT,
    };
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
    };
    use windows_core::PCWSTR;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const LEGACY_INSTALL_KEY: &str = r"Software\Pane\Pane Private";
    const LEGACY_VALUE: &str = "Pane Private";
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
    impl Key {
        fn open(path: &str, access: REG_SAM_FLAGS) -> io::Result<Option<Self>> {
            let path = wide(path);
            let mut key = HKEY::default();
            let result = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(path.as_ptr()),
                    None,
                    access,
                    &mut key,
                )
            };
            if result == ERROR_FILE_NOT_FOUND || result == ERROR_PATH_NOT_FOUND {
                return Ok(None);
            }
            if result != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(result.0 as i32));
            }
            Ok(Some(Self(key)))
        }
        fn read_string(&self, name: &str) -> io::Result<Option<String>> {
            let name = wide(name);
            let mut kind = REG_VALUE_TYPE::default();
            let mut length = 0;
            let result = unsafe {
                RegQueryValueExW(
                    self.0,
                    PCWSTR(name.as_ptr()),
                    None,
                    Some(&mut kind),
                    None,
                    Some(&mut length),
                )
            };
            if result == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if result != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(result.0 as i32));
            }
            if kind != REG_SZ || length < 2 || length % 2 != 0 || length > 131_072 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "legacy startup value is not a bounded REG_SZ",
                ));
            }
            let mut value = vec![0u16; length as usize / 2];
            let capacity = length;
            let result = unsafe {
                RegQueryValueExW(
                    self.0,
                    PCWSTR(name.as_ptr()),
                    None,
                    Some(&mut kind),
                    Some(value.as_mut_ptr().cast()),
                    Some(&mut length),
                )
            };
            if result != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(result.0 as i32));
            }
            if kind != REG_SZ || length < 2 || length % 2 != 0 || length > capacity {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "legacy startup value changed during reading",
                ));
            }
            value.truncate(length as usize / 2);
            if value.pop() != Some(0) || value.contains(&0) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid registry string termination",
                ));
            }
            String::from_utf16(&value)
                .map(Some)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid registry UTF-16"))
        }
    }

    struct Registry;
    impl StartupRegistry for Registry {
        fn read_legacy_run(&self) -> io::Result<Option<String>> {
            match Key::open(RUN_KEY, KEY_QUERY_VALUE)? {
                Some(key) => key.read_string(LEGACY_VALUE),
                None => Ok(None),
            }
        }
        fn delete_legacy_run_if_unchanged(&mut self, expected: &str) -> io::Result<bool> {
            let Some(key) = Key::open(RUN_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)? else {
                return Ok(false);
            };
            // Re-read from the same handle immediately before deleting. A value
            // changed since ownership verification must be left alone.
            if key.read_string(LEGACY_VALUE)?.as_deref() != Some(expected) {
                return Ok(false);
            }
            let name = wide(LEGACY_VALUE);
            let result = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
            if result == ERROR_FILE_NOT_FOUND {
                return Ok(false);
            }
            if result != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(result.0 as i32));
            }
            Ok(true)
        }
        fn executable_identity(&self, target: &str) -> io::Result<ExecutableIdentity> {
            use std::os::windows::fs::MetadataExt;
            let root = wide(&target[..3]);
            // Fixed/removable/RAM disks only. Never probe an SMB/remote drive.
            if !matches!(unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) }, 2 | 3 | 6) {
                return Ok(ExecutableIdentity::Other);
            }
            let mut path = std::path::PathBuf::from(&target[..3]);
            for part in target[3..].split('\\') {
                path.push(part);
                match std::fs::symlink_metadata(&path) {
                    // Check each ancestor before proceeding through it so a
                    // junction or symlink cannot redirect a local-looking path.
                    Ok(metadata)
                        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 =>
                    {
                        return Ok(ExecutableIdentity::Other)
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        return Ok(ExecutableIdentity::Missing)
                    }
                    Err(error) => return Err(error),
                }
            }
            // Tauri's Windows resource generator uses the 0000/04b0 string
            // table. Do not execute the target or treat its basename as proof.
            let path = wide(target);
            let length = unsafe { GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None) };
            if length == 0 || length > 1_048_576 {
                return Ok(ExecutableIdentity::Other);
            }
            let mut data = vec![0u32; (length as usize + 3) / 4];
            if unsafe {
                GetFileVersionInfoW(
                    PCWSTR(path.as_ptr()),
                    None,
                    length,
                    data.as_mut_ptr().cast(),
                )
            }
            .is_err()
            {
                return Ok(ExecutableIdentity::Other);
            }
            let query = wide(PRODUCT_NAME_QUERY);
            let mut product = std::ptr::null_mut();
            let mut units = 0;
            let found = unsafe {
                VerQueryValueW(
                    data.as_ptr().cast(),
                    PCWSTR(query.as_ptr()),
                    &mut product,
                    &mut units,
                )
            }
            .as_bool();
            let begin = data.as_ptr() as usize;
            let pointer = product as usize;
            let bytes = units as usize * 2;
            if !found
                || units == 0
                || pointer < begin
                || pointer % 2 != 0
                || pointer
                    .checked_add(bytes)
                    .is_none_or(|end| end > begin + length as usize)
            {
                return Ok(ExecutableIdentity::Other);
            }
            let value =
                unsafe { std::slice::from_raw_parts(product.cast::<u16>(), units as usize) };
            let value = value.strip_suffix(&[0]).unwrap_or(value);
            let product = String::from_utf16(value).unwrap_or_default();
            Ok(if product == "Pane Private" || product == "rice monitor" {
                ExecutableIdentity::Private
            } else {
                ExecutableIdentity::Other
            })
        }
    }

    pub(super) fn run() -> io::Result<Cleanup> {
        let current_exe = std::env::current_exe()?;
        let local = std::env::var("LOCALAPPDATA").ok();
        // NSIS leaves the default install-directory value after uninstall;
        // only Installer Language is removed from this manufacturer key.
        let legacy = match Key::open(LEGACY_INSTALL_KEY, KEY_QUERY_VALUE)? {
            Some(key) => key.read_string("")?,
            None => None,
        };
        let targets = known_targets(
            &current_exe.to_string_lossy(),
            local.as_deref(),
            legacy.as_deref(),
        );
        cleanup_legacy_run(&mut Registry, &targets)
    }
}

/// Best-effort migration: cleanup failures cannot prevent opening the app.
/// Tests use the in-memory adapter above and never touch the real registry.
#[cfg(windows)]
pub(crate) fn remove_legacy_startup() {
    if cfg!(test) || cfg!(debug_assertions) {
        return;
    }
    match windows_registry::run() {
        Ok(Cleanup::Absent | Cleanup::Removed) => {},
        Ok(Cleanup::Preserved) => eprintln!("[rice monitor] Legacy startup entry was preserved because ownership could not be verified; review Windows Startup apps if needed."),
        Err(error) => eprintln!("[rice monitor] Legacy startup cleanup failed: {error}"),
    }
}
#[cfg(not(windows))]
pub(crate) fn remove_legacy_startup() {}
