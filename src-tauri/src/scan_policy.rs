//! Explicit local log grants. Account credentials never participate in this policy.
//! Canonical containment is rechecked before I/O and after opening a log. This
//! reduces ordinary symlink/path replacement races; it is not an OS no-follow
//! guarantee against every adversarial filesystem race.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File, Metadata, ReadDir};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const SOURCES: &[&str] = &[
    "claude", "codex", "opencode", "pi", "stepcode", "grok", "devin", "minimax", "hermes", "kimi",
    "qwen",
];

/// Apply one backend-owned source change, returning whether the grant changed.
pub fn update_source_grant(
    roots: &mut BTreeMap<String, Vec<PathBuf>>,
    epochs: &mut BTreeMap<String, String>,
    source: &str,
    directories: Vec<PathBuf>,
) -> Result<bool, String> {
    if !SOURCES.contains(&source) {
        return Err("Unknown log source".into());
    }
    if directories.is_empty() {
        let removed_roots = roots.remove(source).is_some();
        let removed_epoch = epochs.remove(source).is_some();
        return Ok(removed_roots || removed_epoch);
    }
    let previous = roots.get(source).cloned().unwrap_or_default();
    let same_spelling = |left: &PathBuf, right: &PathBuf| left.as_os_str() == right.as_os_str();
    let exact_subset = directories
        .iter()
        .all(|requested| previous.iter().any(|saved| same_spelling(requested, saved)));
    let validated = if exact_subset {
        // These are exact, already-persisted canonical grants. Only discard
        // entries; never resolve an arbitrary submitted spelling in this path.
        // Neither retained nor unrelated offline roots can block revocation.
        let retained: Vec<PathBuf> = previous
            .iter()
            .filter(|saved| {
                directories
                    .iter()
                    .any(|requested| same_spelling(requested, saved))
            })
            .cloned()
            .collect();
        if retained == previous {
            return Ok(false);
        }
        let mut reduced = roots.clone();
        reduced.insert(source.to_string(), retained);
        reduced
    } else {
        // Only genuinely new input may resolve to a new target. Exact saved
        // entries stay fixed, including other roots of this same source.
        if directories.len() > 32 {
            return Err("Too many log directories".into());
        }
        let mut selected = Vec::new();
        for requested in directories {
            let path = if let Some(saved) = previous
                .iter()
                .find(|saved| same_spelling(&requested, saved))
            {
                saved.clone()
            } else {
                let mut explicit =
                    ScanPolicy::new(BTreeMap::from([(source.to_string(), vec![requested])]))?;
                explicit
                    .roots
                    .remove(source)
                    .and_then(|paths| paths.into_iter().next())
                    .ok_or("Log directory is unavailable")?
            };
            if !selected.contains(&path) {
                selected.push(path);
            }
        }
        let mut proposed = roots.clone();
        proposed.insert(source.to_string(), selected);
        // Validate, never retarget, all retained canonical grants. A changed
        // symlink/junction cannot piggyback on a different directory's grant.
        let canonical = ScanPolicy::from_saved(proposed)?.roots;
        let selected = canonical.get(source).cloned().unwrap_or_default();
        if selected.len() == previous.len()
            && selected
                .iter()
                .all(|requested| previous.iter().any(|saved| same_spelling(requested, saved)))
        {
            return Ok(false);
        }
        canonical
    };
    let mut epoch = [0u8; 16];
    getrandom::getrandom(&mut epoch).map_err(|_| "Cannot create a new source-grant generation")?;
    epochs.insert(
        source.to_string(),
        epoch.iter().map(|byte| format!("{byte:02x}")).collect(),
    );
    *roots = validated;
    Ok(true)
}

#[derive(Clone, Default)]
pub struct ScanPolicy {
    pub roots: BTreeMap<String, Vec<PathBuf>>,
    pub revision: u64,
    pub epochs: BTreeMap<String, String>,
    check: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}
impl ScanPolicy {
    /// Canonicalize explicit grant-time input only. Stored grants must instead
    /// use from_saved so a later symlink/junction change cannot expand access.
    pub fn new(roots: BTreeMap<String, Vec<PathBuf>>) -> Result<Self, String> {
        Self::validate_roots(roots, false)
    }
    fn validate_roots(
        roots: BTreeMap<String, Vec<PathBuf>>,
        persisted: bool,
    ) -> Result<Self, String> {
        let mut canonical: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for (source, paths) in roots {
            if !SOURCES.contains(&source.as_str()) || paths.len() > 32 {
                return Err("Unknown log source or too many directories".into());
            }
            let mut dirs = Vec::new();
            for path in paths {
                if !path.is_absolute() {
                    return Err("Choose an absolute log directory".into());
                }
                let resolved = path
                    .canonicalize()
                    .map_err(|_| "Log directory is unavailable")?;
                if persisted && resolved.as_os_str() != path.as_os_str() {
                    return Err("A saved log directory now resolves elsewhere; remove it and explicitly select its new location".into());
                }
                let path = resolved;
                if !path.is_dir() || path.parent().is_none() {
                    return Err("Choose a log directory, not a filesystem root".into());
                }
                if canonical
                    .values()
                    .flatten()
                    .any(|other| path.starts_with(other) || other.starts_with(&path))
                {
                    return Err("Different log sources must use separate directories".into());
                }
                if !dirs.contains(&path) {
                    dirs.push(path);
                }
            }
            if !dirs.is_empty() {
                canonical.insert(source, dirs);
            }
        }
        Ok(Self {
            roots: canonical,
            revision: 0,
            epochs: BTreeMap::new(),
            check: None,
        })
    }
    /// Validate persisted canonical authority without silently adopting a new
    /// target. Unavailable or retargeted stored roots fail closed.
    pub fn from_saved(roots: BTreeMap<String, Vec<PathBuf>>) -> Result<Self, String> {
        Self::validate_roots(roots, true)
    }
    /// The desktop bridge supplies a backend revision check. Tests supply a
    /// synthetic revocation source, never a user's config/home directory.
    pub fn with_check(mut self, check: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.check = Some(Arc::new(check));
        self
    }
    pub fn with_epochs(mut self, epochs: BTreeMap<String, String>) -> Self {
        self.epochs = epochs;
        self
    }
    pub fn with_revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }
    pub fn is_current(&self) -> bool {
        self.check.as_ref().is_none_or(|check| check())
    }
    pub fn allows_path(&self, source: &str, path: &Path) -> bool {
        self.resolve(source, path).is_ok()
    }
    fn resolve(&self, source: &str, path: &Path) -> io::Result<PathBuf> {
        if !self.is_current() {
            return Err(denied());
        }
        let roots = self.roots.get(source).ok_or_else(denied)?;
        // Do not even stat/canonicalize an unselected source/path.
        let root = roots
            .iter()
            .find(|root| path.starts_with(root))
            .ok_or_else(denied)?;
        if root.canonicalize()? != *root {
            return Err(denied());
        }
        let resolved = path.canonicalize()?;
        if !resolved.starts_with(root) || !self.is_current() {
            return Err(denied());
        }
        Ok(resolved)
    }
    pub fn enter(&self, source: &str) -> ScanScope {
        let prior = CURRENT.with(|slot| {
            slot.replace(Some(Context {
                policy: self.clone(),
                source: source.into(),
            }))
        });
        ScanScope(prior, std::marker::PhantomData)
    }
    pub fn scope<T>(&self, source: &str, f: impl FnOnce() -> T) -> T {
        let _scope = self.enter(source);
        f()
    }
}
pub struct ScanScope(Option<Context>, std::marker::PhantomData<std::rc::Rc<()>>);
impl Drop for ScanScope {
    fn drop(&mut self) {
        CURRENT.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}
/// Cache authority changes at every consent revision, including an off/on
/// cycle that restores the same directories. An adapter cannot borrow another
/// tool's file permission merely because its path lies below that tool's root.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceGrant {
    pub revision: u64,
    pub roots: Vec<PathBuf>,
    pub epoch: Option<String>,
}
pub fn grant_for(source: &str) -> Option<SourceGrant> {
    CURRENT.with(|slot| {
        let slot = slot.borrow();
        let c = slot.as_ref()?;
        if c.source != source || !c.policy.is_current() {
            return None;
        }
        let roots = c.policy.roots.get(source)?.clone();
        if roots.is_empty() {
            return None;
        }
        Some(SourceGrant {
            revision: c.policy.revision,
            roots,
            epoch: c.policy.epochs.get(source).cloned(),
        })
    })
}
pub fn current_policy() -> Option<ScanPolicy> {
    CURRENT.with(|slot| slot.borrow().as_ref().map(|c| c.policy.clone()))
}
pub fn current_source() -> Option<String> {
    CURRENT.with(|slot| slot.borrow().as_ref().map(|c| c.source.clone()))
}
#[derive(Clone)]
struct Context {
    policy: ScanPolicy,
    source: String,
}
thread_local! { static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Local log access is off, revoked, or outside the selected directory",
    )
}
pub fn current_is_valid() -> bool {
    CURRENT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|c| c.policy.is_current())
    })
}
pub fn source_roots() -> Vec<PathBuf> {
    CURRENT.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|c| c.policy.is_current())
            .and_then(|c| c.policy.roots.get(&c.source).cloned())
            .unwrap_or_default()
    })
}
pub fn checked_path(path: &Path) -> io::Result<PathBuf> {
    CURRENT.with(|slot| {
        let slot = slot.borrow();
        let c = slot.as_ref().ok_or_else(denied)?;
        c.policy.resolve(&c.source, path)
    })
}
pub fn log_metadata(path: &Path) -> io::Result<Metadata> {
    fs::metadata(checked_path(path)?)
}
pub fn read_dir(path: &Path) -> io::Result<ReadDir> {
    let resolved = checked_path(path)?;
    let entries = fs::read_dir(&resolved)?;
    if checked_path(path)? != resolved {
        return Err(denied());
    }
    Ok(entries)
}
pub fn open_log(path: &Path) -> io::Result<File> {
    let resolved = checked_path(path)?;
    // Resolve before checking the extension: a .jsonl alias to auth.json is
    // never a log, including an alias whose target stays inside the root.
    if resolved.extension().is_none_or(|ext| ext != "jsonl") {
        return Err(denied());
    }
    if !fs::metadata(&resolved)?.is_file() {
        return Err(denied());
    }
    let file = File::open(&resolved)?;
    if checked_path(path)? != resolved {
        return Err(denied());
    }
    revalidate_log(path, &file)?;
    Ok(file)
}
/// Recheck that the open handle still names the authorized file before
/// publishing/cache insertion. Unix and Windows compare file identity rather
/// than timestamps, which can be preserved across equal-size replacements.
pub fn revalidate_log(path: &Path, file: &File) -> io::Result<()> {
    let resolved = checked_path(path)?;
    let opened = file.metadata()?;
    let current = fs::metadata(&resolved)?;
    if !opened.is_file() || !current.is_file() {
        return Err(denied());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != current.dev() || opened.ino() != current.ino() {
            return Err(denied());
        }
    }
    #[cfg(windows)]
    {
        // Keep both handles alive while comparing IDs, so a removed original
        // cannot have its identity recycled before the comparison finishes.
        let current_file = File::open(&resolved)?;
        if !current_file.metadata()?.is_file()
            || windows_file_identity(file)? != windows_file_identity(&current_file)?
            || checked_path(path)? != resolved
        {
            return Err(denied());
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        if opened.created().ok() != current.created().ok() {
            return Err(denied());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn windows_file_identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandleEx, FileIdInfo, FILE_ID_INFO,
    };

    let mut identity = FILE_ID_INFO::default();
    // SAFETY: file owns a live handle for this synchronous call. The writable
    // buffer is correctly aligned and sized for FileIdInfo and outlives it.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileIdInfo,
            std::ptr::from_mut(&mut identity).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(|error| io::Error::other(format!("Cannot verify log file identity: {error}")))?;
    // Failure to obtain identity propagates; never fall back to timestamps.
    Ok((identity.VolumeSerialNumber, identity.FileId.Identifier))
}

/// SQLite may open its WAL, SHM, or rollback journal implicitly. Verify those
/// names too; checking the main database alone would permit sidecar escapes.
pub fn checked_sqlite(path: &Path) -> io::Result<PathBuf> {
    let resolved = checked_path(path)?;
    if resolved.file_name() != path.file_name() {
        return Err(denied());
    }
    if !resolved.is_file()
        || resolved
            .extension()
            .is_none_or(|ext| ext != "db" && ext != "sqlite")
    {
        return Err(denied());
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = resolved.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = PathBuf::from(name);
        match fs::symlink_metadata(&sidecar) {
            Ok(meta) => {
                if !meta.file_type().is_file() {
                    return Err(denied());
                }
                let target = checked_path(&sidecar)?;
                if target.file_name() != sidecar.file_name() {
                    return Err(denied());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                checked_path(sidecar.parent().ok_or_else(denied)?)?;
            }
            Err(error) => return Err(error),
        }
    }
    if checked_path(path)? != resolved {
        return Err(denied());
    }
    Ok(resolved)
}

#[cfg(all(test, windows))]
mod windows_identity_tests {
    use super::*;
    use std::os::windows::fs::FileTimesExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Fixture {
        root: PathBuf,
        _scope: ScanScope,
    }

    impl Fixture {
        fn new(name: &str, current: Arc<AtomicBool>) -> Self {
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "pane-file-identity-{name}-{}", std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            let policy = ScanPolicy::new(BTreeMap::from([("claude".into(), vec![root.clone()])]))
                .unwrap()
                .with_check(move || current.load(Ordering::SeqCst));
            Self { root, _scope: policy.enter("claude") }
        }

        fn log(&self) -> PathBuf {
            let path = self.root.join("session.jsonl");
            fs::write(&path, "{}\n").unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn unchanged_file_identity_is_accepted() {
        let fixture = Fixture::new("unchanged", Arc::new(AtomicBool::new(true)));
        let path = fixture.log();
        let file = open_log(&path).unwrap();
        revalidate_log(&path, &file).expect("unchanged authorized file");
    }

    #[test]
    fn hard_link_to_the_same_file_within_the_grant_is_accepted() {
        let fixture = Fixture::new("hardlink", Arc::new(AtomicBool::new(true)));
        let path = fixture.log();
        let file = open_log(&path).unwrap();
        let alias = fixture.root.join("alias.jsonl");
        fs::hard_link(&path, &alias).unwrap();
        revalidate_log(&alias, &file).expect("same identity under an authorized alias");
    }

    #[test]
    fn equal_size_replacement_with_preserved_creation_time_is_rejected() {
        let fixture = Fixture::new("replacement", Arc::new(AtomicBool::new(true)));
        let path = fixture.log();
        let file = open_log(&path).unwrap();
        let created = file.metadata().unwrap().created().unwrap();
        fs::rename(&path, fixture.root.join("original.jsonl")).unwrap();
        fs::write(&path, "{}\n").unwrap();
        let replacement = fs::OpenOptions::new().write(true).open(&path).unwrap();
        replacement.set_times(fs::FileTimes::new().set_created(created)).unwrap();
        drop(replacement);
        assert_eq!(file.metadata().unwrap().len(), fs::metadata(&path).unwrap().len());
        assert_eq!(file.metadata().unwrap().created().unwrap(), fs::metadata(&path).unwrap().created().unwrap());
        assert_eq!(revalidate_log(&path, &file).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn revoked_grant_still_rejects_an_unchanged_open_file() {
        let current = Arc::new(AtomicBool::new(true));
        let fixture = Fixture::new("revoked", current.clone());
        let path = fixture.log();
        let file = open_log(&path).unwrap();
        current.store(false, Ordering::SeqCst);
        assert_eq!(revalidate_log(&path, &file).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
}
