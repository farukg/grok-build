//! Git branch/worktree info: cached queries shared across views.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use xai_grok_telemetry::region;
use xai_grok_telemetry::region::Parent;

use crate::host::HostOs;
use crate::terminal::{TerminalName, terminal_context};

/// Per-cwd git cache: the single source of truth for every git display in the pager. Keyed per directory so one
/// directory's branch never leaks onto another's. A branch switch inside an agent thus reflects immediately instead
/// of waiting out [`CWD_GIT_REFRESH_TTL`].
type CwdCacheEntry = (Option<CwdGitInfo>, Instant);
#[derive(Default)]
struct CwdGitCache {
    entries: HashMap<PathBuf, CwdCacheEntry>,
    active_scope: HashSet<PathBuf>,
    in_flight: HashSet<PathBuf>,
}
static CWD_GIT_CACHE: LazyLock<Mutex<CwdGitCache>> =
    LazyLock::new(|| Mutex::new(CwdGitCache::default()));

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepositoryFamily {
    Pending,
    NonGit,
    Failed,
    Resolved(PathBuf),
}

/// Minimum interval between off-thread refreshes for the same cwd, so a per-frame caller can't spawn a storm of git lookups.
const CWD_GIT_REFRESH_TTL: Duration = Duration::from_secs(5);

/// Inactive display entries are bounded; active scope members and running resolutions are retained.
const CWD_GIT_CACHE_CAP: usize = 64;

/// A branch switch inside an agent's session thus reflects in every view immediately instead of waiting out
/// [`CWD_GIT_REFRESH_TTL`]. The worktree label and repo root aren't carried by the notification (and are immutable
/// for a path), so any previously-resolved values are preserved.
pub fn update_from_notification(
    dir: &Path,
    branch: Option<&str>,
    main_repo: Option<String>,
    is_worktree: bool,
) {
    if let Ok(mut cache) = CWD_GIT_CACHE.lock() {
        let prior = cache.entries.get(dir).and_then(|(info, _)| info.clone());
        let worktree_label = prior.as_ref().and_then(|i| i.worktree_label.clone());
        let repo_root = prior.as_ref().and_then(|i| i.repo_root.clone());
        let family = prior.as_ref().map_or(RepositoryFamily::Pending, |info| match &info.family {
            RepositoryFamily::Resolved(_) => info.family.clone(),
            RepositoryFamily::Pending | RepositoryFamily::NonGit | RepositoryFamily::Failed => RepositoryFamily::Pending,
        });
        let is_worktree =
            is_worktree || is_cwd_worktree(main_repo.as_deref(), worktree_label.as_deref());
        let info = CwdGitInfo {
            is_worktree,
            branch: branch.map(str::to_string),
            main_repo,
            repo_root,
            family,
            worktree_label,
        };
        cwd_cache_insert(&mut cache, dir.to_path_buf(), (Some(info), Instant::now()));
    }
}

/// Eagerly warm [`CWD_GIT_CACHE`] for `cwd` off-thread, e.g. at pager startup and after a dashboard location change.
/// The header / top bar then show the branch and worktree on the next frame instead of waiting for the first lazy refresh.
/// No subprocess (libgit2 is filesystem-based) and a no-op when there is no tokio runtime, so callers stay infallible.
pub fn populate_from_cwd_async(cwd: PathBuf) {
    let _ = cwd_git_info_lazy(&cwd);
}

struct GitSnapshot {
    family: RepositoryFamily,
    /// Tilde-shortened path to the repo workdir. `None` when not in a repo.
    repo_root_display: Option<String>,
    /// `None` when not in a repo. `Some("")` for detached HEAD, otherwise the branch shorthand.
    branch: Option<String>,
    /// Tilde-shortened path to the main repo when in a worktree, else `None`.
    main_repo_display: Option<String>,
    /// Human-readable label from the worktree metadata DB, if this cwd is a managed worktree with a label set.
    worktree_label: Option<String>,
}

/// Freshly-computed git context for a directory. Returned by [`compute_cwd_git_info`].
#[derive(Debug, Clone)]
pub struct CwdGitInfo {
    /// Branch shorthand. `Some("")` for detached HEAD.
    pub branch: Option<String>,
    /// Whether `cwd` is a worktree rather than the primary checkout (linked `git worktree`, grok standalone clone, or worktree DB hit).
    pub is_worktree: bool,
    /// Tilde-shortened path to the main repo when in a worktree.
    pub main_repo: Option<String>,
    /// Tilde-shortened checkout root for display only.
    pub repo_root: Option<String>,
    pub(crate) family: RepositoryFamily,
    /// Human-readable worktree label from the metadata DB, if any.
    pub worktree_label: Option<String>,
}

impl CwdGitInfo {
    pub fn family_root(&self) -> Option<&Path> {
        match &self.family {
            RepositoryFamily::Resolved(path) => Some(path),
            RepositoryFamily::Pending | RepositoryFamily::NonGit | RepositoryFamily::Failed => None,
        }
    }
}

/// Synchronously compute fresh git context (branch and worktree info) for `cwd`. Call it off the render path, never
/// per frame. Callers can then leave existing cached values untouched rather than clobbering them with empties.
pub fn compute_cwd_git_info(cwd: &Path) -> Option<CwdGitInfo> {
    let info = compute_cwd_context(cwd);
    info.repo_root.as_ref()?;
    Some(info)
}

fn compute_cwd_context(cwd: &Path) -> CwdGitInfo {
    let snap = compute_snapshot(cwd);
    CwdGitInfo {
        is_worktree: is_cwd_worktree(
            snap.main_repo_display.as_deref(),
            snap.worktree_label.as_deref(),
        ),
        branch: snap.branch,
        main_repo: snap.main_repo_display,
        repo_root: snap.repo_root_display,
        family: snap.family,
        worktree_label: snap.worktree_label,
    }
}

fn is_cwd_worktree(main_repo: Option<&str>, worktree_label: Option<&str>) -> bool {
    main_repo.is_some() || worktree_label.is_some_and(|s| !s.is_empty())
}

/// Per-cwd git info for render paths that display many directories (the dashboard agent list, each agent's status
/// bar). Never blocks and never spawns `git` subprocesses (uses `git2`). Keyed per directory, so each agent shows
/// the branch/worktree of its own location rather than the process cwd's.
pub fn cwd_git_info_lazy(cwd: &Path) -> Option<CwdGitInfo> {
    let mut cache = CWD_GIT_CACHE.lock().ok()?;
    reserve_cwd_git_refresh(&mut cache, cwd, GitRead::Display);
    cache.entries.get(cwd).and_then(|(info, _)| info.clone())
}

enum GitRead {
    Display,
    Scope,
}

fn reserve_cwd_git_refresh(cache: &mut CwdGitCache, cwd: &Path, read: GitRead) {
    let needs_refresh = match cache.entries.get(cwd) {
        Some((Some(info), ts)) => match read {
            GitRead::Display => info.family == RepositoryFamily::Pending
                || ts.elapsed() >= CWD_GIT_REFRESH_TTL,
            GitRead::Scope => match &info.family {
                RepositoryFamily::Resolved(_) | RepositoryFamily::NonGit => false,
                RepositoryFamily::Pending => true,
                RepositoryFamily::Failed => ts.elapsed() >= CWD_GIT_REFRESH_TTL,
            },
        },
        Some((None, ts)) => ts.elapsed() >= CWD_GIT_REFRESH_TTL,
        None => true,
    };
    if needs_refresh && !cache.in_flight.contains(cwd)
        && let Ok(handle) = tokio::runtime::Handle::try_current()
    {
        let prior = cache.entries.get(cwd).and_then(|(info, _)| info.clone());
        cache.in_flight.insert(cwd.to_path_buf());
        cwd_cache_insert(cache, cwd.to_path_buf(), (prior, Instant::now()));
        spawn_cwd_git_refresh(handle, cwd.to_path_buf());
    }
}

pub(crate) struct ScopeFamilyIndex {
    cache: MutexGuard<'static, CwdGitCache>,
}

impl ScopeFamilyIndex {
    pub(crate) fn family_root(&self, cwd: &Path) -> Option<&Path> {
        self.cache.entries.get(cwd)?.0.as_ref()?.family_root()
    }
}

pub(crate) fn scope_family_index<'a>(
    cwds: impl IntoIterator<Item = &'a Path>,
) -> Option<ScopeFamilyIndex> {
    let active: HashSet<&Path> = cwds.into_iter().collect();
    let mut cache = CWD_GIT_CACHE.lock().ok()?;
    if cache.active_scope.len() != active.len()
        || !active.iter().all(|cwd| cache.active_scope.contains(*cwd))
    {
        cache.active_scope = active.iter().map(|cwd| cwd.to_path_buf()).collect();
        trim_cwd_cache(&mut cache);
    }
    for cwd in active {
        reserve_cwd_git_refresh(&mut cache, cwd, GitRead::Scope);
    }
    Some(ScopeFamilyIndex { cache })
}

/// Store `info` for `dir` in [`CWD_GIT_CACHE`], the synchronous stand-in for the async refresh
/// that never runs without a tokio runtime. Lets cross-module tests exercise cache-backed
/// consumers (e.g. the dashboard's worktree scope) with exactly the metadata they assert over.
#[cfg(test)]
pub(crate) fn seed_cwd_cache_for_tests(dir: &Path, info: Option<CwdGitInfo>) {
    if let Ok(mut cache) = CWD_GIT_CACHE.lock() {
        cwd_cache_insert(&mut cache, dir.to_path_buf(), (info, Instant::now()));
    }
}

/// Off-thread refresh of a reserved cwd.
fn spawn_cwd_git_refresh(handle: tokio::runtime::Handle, cwd: PathBuf) {
    let refresh_span = region!(debug, "git_info.refresh", Parent::Root);
    handle.spawn_blocking(move || {
        let _region = refresh_span;
        // Guard against panics from the vendored libgit2 C bindings.
        let info =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute_cwd_context(&cwd)))
                .ok();
        if let Ok(mut cache) = CWD_GIT_CACHE.lock() {
            cache.in_flight.remove(&cwd);
            apply_cwd_git_refresh(&mut cache, cwd, info);
        }
    });
}

/// A failed refresh preserves display metadata but cannot establish repository membership.
fn apply_cwd_git_refresh(
    cache: &mut CwdGitCache,
    cwd: PathBuf,
    info: Option<CwdGitInfo>,
) {
    let value = match info {
        Some(info) if info.family != RepositoryFamily::Failed => info,
        Some(_) | None => {
            let mut info = cache.entries.get(&cwd).and_then(|(prev, _)| prev.clone())
                .unwrap_or_else(|| CwdGitInfo {
                    branch: None,
                    is_worktree: false,
                    main_repo: None,
                    repo_root: None,
                    family: RepositoryFamily::Failed,
                    worktree_label: None,
                });
            info.family = RepositoryFamily::Failed;
            info
        }
    };
    cwd_cache_insert(cache, cwd, (Some(value), Instant::now()));
}

fn cwd_cache_insert(cache: &mut CwdGitCache, key: PathBuf, entry: CwdCacheEntry) {
    let protected = cache.active_scope.contains(&key) || cache.in_flight.contains(&key);
    cache.entries.insert(key, entry);
    if !protected {
        trim_cwd_cache(cache);
    }
}

fn trim_cwd_cache(cache: &mut CwdGitCache) {
    let excess = cache.entries.len().saturating_sub(CWD_GIT_CACHE_CAP);
    if excess == 0 {
        return;
    }
    if excess == 1 {
        let oldest = cache.entries.iter()
            .filter(|(key, _)| !cache.active_scope.contains(*key) && !cache.in_flight.contains(*key))
            .min_by_key(|(_, (_, ts))| *ts)
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            cache.entries.remove(&oldest);
        }
        return;
    }
    let mut candidates: Vec<_> = cache.entries.iter()
        .filter(|(key, _)| !cache.active_scope.contains(*key) && !cache.in_flight.contains(*key))
        .map(|(key, (_, ts))| (*ts, key.clone()))
        .collect();
    candidates.sort_unstable();
    for (_, key) in candidates.into_iter().take(excess) {
        cache.entries.remove(&key);
    }
}

pub(crate) fn clear_scope_families() {
    if let Ok(mut cache) = CWD_GIT_CACHE.lock() {
        if !cache.active_scope.is_empty() {
            cache.active_scope.clear();
            trim_cwd_cache(&mut cache);
        }
    }
}

fn compute_snapshot(cwd: &Path) -> GitSnapshot {
    let repo = match git2::Repository::discover(cwd) {
        Ok(repo) => repo,
        Err(error) => return GitSnapshot {
            family: if error.code() == git2::ErrorCode::NotFound && cwd.is_dir() {
                RepositoryFamily::NonGit
            } else {
                RepositoryFamily::Failed
            },
            repo_root_display: None,
            branch: None,
            main_repo_display: None,
            worktree_label: None,
        },
    };

    let repo_root_display = repo.workdir().map(collapse_home);

    // The empty-string sentinel means "detached HEAD"; the pager's notification path uses the same convention
    let branch = repo.head().ok().map(|head| {
        head.shorthand()
            .filter(|s| *s != "HEAD")
            .map(str::to_string)
            .unwrap_or_default()
    });

    let linked_main_repo = (repo.path() != repo.commondir())
        .then(|| repo.commondir().parent().map(Path::to_path_buf))
        .flatten();

    // Standalone grok worktrees are CoW clones (`.git` is a directory), so `path != commondir` is false; the source marker is the back-pointer
    let mut marker_main_repo = None;
    for ancestor in cwd.ancestors() {
        let git = ancestor.join(".git");
        if let Ok(contents) = std::fs::read_to_string(git.join("grok-worktree-source"))
            && let trimmed = contents.trim()
            && !trimmed.is_empty()
        {
            marker_main_repo = Some(PathBuf::from(trimmed));
            break;
        }
        // Don't inherit a parent checkout's marker across a nested repo root.
        if git.exists() {
            break;
        }
    }

    let (worktree_label, db_source_repo) = lookup_worktree_record(cwd);
    let db_main_repo = db_source_repo;

    let main_repo = marker_main_repo.or(db_main_repo).or(linked_main_repo);
    let family = match main_repo.as_deref() {
        Some(source) => git2::Repository::discover(source).ok()
            .and_then(|source| canonical_family_root(&source)),
        None => canonical_family_root(&repo),
    }.map_or(RepositoryFamily::Failed, RepositoryFamily::Resolved);
    GitSnapshot {
        family,
        repo_root_display,
        branch,
        main_repo_display: main_repo.as_deref().map(collapse_home),
        worktree_label,
    }
}

fn canonical_family_root(repo: &git2::Repository) -> Option<PathBuf> {
    let root = if repo.path() != repo.commondir() {
        repo.commondir().parent()
    } else {
        repo.workdir()
    }?;
    dunce::canonicalize(root).ok()
}

/// Map of worktree root path to human label for every managed worktree that has a non-empty label. Opens the
/// worktree metadata DB once and returns an empty map on any error. Keys are canonicalized so callers can match
/// against `dunce::canonicalize`d candidate paths.
pub fn worktree_label_index() -> std::collections::HashMap<PathBuf, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(db) = xai_fast_worktree::db::WorktreeDb::open_default() else {
        return map;
    };
    let Ok(records) = db.list(&Default::default()) else {
        return map;
    };
    for rec in records {
        let Some(label) = rec.label() else {
            continue;
        };
        let label = label.to_owned();
        let key = dunce::canonicalize(&rec.path).unwrap_or(rec.path);
        map.insert(key, label);
    }
    map
}

/// Look up the worktree label and source repo from the metadata DB.
/// Returns `(None, None)` silently on any error (missing DB, no record).
/// Called from `spawn_blocking` so DB I/O is fine.
fn lookup_worktree_record(cwd: &Path) -> (Option<String>, Option<PathBuf>) {
    let Ok(db) = xai_fast_worktree::db::WorktreeDb::open_default() else {
        return (None, None);
    };
    // Try exact match first, then walk up ancestors to find the worktree root.
    for ancestor in cwd.ancestors() {
        if let Ok(Some(record)) = db.get(&ancestor.to_string_lossy()) {
            let label = record.label().map(str::to_owned);
            let source = (!record.source_repo.as_os_str().is_empty()).then_some(record.source_repo);
            return (label, source);
        }
        // Nested checkout inside a registered worktree is its own repo.
        if ancestor.join(".git").exists() {
            break;
        }
    }
    (None, None)
}

fn collapse_home(path: &Path) -> String {
    collapse_home_path(path, home_dir().as_deref().map(Path::new))
}

/// Tilde-collapse with a path-component prefix, so `HOME=/Users/u` does not match `/Users/user/xai`.
/// Trailing slashes on HOME do not break the join.
fn collapse_home_path(path: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return path.display().to_string();
    };
    let home = PathBuf::from(
        home.as_os_str()
            .to_string_lossy()
            .trim_end_matches(['/', '\\']),
    );
    path.strip_prefix(&home)
        .map(|rest| {
            if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.display())
            }
        })
        .unwrap_or_else(|_| path.display().to_string())
}

/// Branch glyph for the git display, cached for process lifetime. The Powerline glyph (`\u{e0a0}`) is a Nerd
/// Font-only Private Use Area codepoint, so it renders as "tofu" without a patched font. Where one can't be assumed
/// we fall back to a glyph the platform's stock fonts cover.
pub(crate) fn branch_icon() -> &'static str {
    static ICON: OnceLock<&str> = OnceLock::new();
    ICON.get_or_init(|| {
        decide_branch_icon(
            std::env::var("GROK_NERD_FONTS").ok().as_deref(),
            HostOs::current(),
            terminal_context().brand,
        )
    })
}

/// Pure decision function for [`branch_icon`] so tests can drive inputs without touching ambient env/host state.
fn decide_branch_icon(nerd_fonts: Option<&str>, host: HostOs, brand: TerminalName) -> &'static str {
    const POWERLINE: &str = "\u{e0a0}";
    // Windows console fonts lack `⎇`, so use `≡` (also in the legacy CP437 font).
    let fallback = if host == HostOs::Windows {
        "\u{2261}" // ≡
    } else {
        "\u{2387}" // ⎇
    };

    if decide_nerd_fonts(nerd_fonts, host, brand) {
        POWERLINE
    } else {
        fallback
    }
}

/// Whether a Nerd Font (Private Use Area glyphs) is plausible for this host/terminal. An explicit `GROK_NERD_FONTS`
/// override always wins: `0`/`false` means off, anything else means on. Otherwise PUA glyphs are assumed everywhere
/// except Windows consoles and the macOS terminals that ship stock fonts (Apple Terminal, iTerm2).
fn decide_nerd_fonts(nerd_fonts: Option<&str>, host: HostOs, brand: TerminalName) -> bool {
    if let Some(val) = nerd_fonts {
        return !matches!(val, "0" | "false");
    }
    let stock_font_terminal = matches!(brand, TerminalName::AppleTerminal | TerminalName::Iterm2);
    !(host == HostOs::Windows || stock_font_terminal)
}

pub(crate) fn home_dir() -> Option<String> {
    xai_dirs::home_dir().map(|home| home.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cwd_git_info_lazy` returns `None` for a non-repo path and never panics when called without a tokio runtime (no refresh spawns).
    /// Uses a unique path so it doesn't race the shared cache with other tests.
    #[test]
    fn cwd_git_info_lazy_non_repo_is_none() {
        let p = Path::new("/nonexistent-xai-git-info-lazy-test-zzz");
        assert!(cwd_git_info_lazy(p).is_none());
        // Second call hits the reserved (None) entry and is still None
        assert!(cwd_git_info_lazy(p).is_none());
    }

    /// `cwd_cache_insert` bounds the map at `CWD_GIT_CACHE_CAP`, evicting the least-recently-refreshed entry when a new key would overflow it.
    /// It never evicts when overwriting an existing key.
    /// Drives a local map so it can't race the shared static cache.
    #[test]
    fn cwd_cache_insert_evicts_least_recently_refreshed() {
        let mut cache = CwdGitCache::default();
        let base = Instant::now();
        // Fill to capacity with strictly increasing timestamps; `/d/0` is oldest.
        for i in 0..CWD_GIT_CACHE_CAP {
            let ts = base + Duration::from_secs(i as u64);
            cwd_cache_insert(&mut cache, PathBuf::from(format!("/d/{i}")), (None, ts));
        }
        assert_eq!(cache.entries.len(), CWD_GIT_CACHE_CAP);
        // A new key overflows: the oldest entry is evicted and len stays capped
        let newest = base + Duration::from_secs(10_000);
        cwd_cache_insert(&mut cache, PathBuf::from("/d/new"), (None, newest));
        assert_eq!(cache.entries.len(), CWD_GIT_CACHE_CAP);
        assert!(
            !cache.entries.contains_key(Path::new("/d/0")),
            "least-recently-refreshed entry must be evicted"
        );
        assert!(
            cache.entries.contains_key(Path::new("/d/new")),
            "new entry inserted"
        );
        // Overwriting an existing key must not evict (no overflow).
        cwd_cache_insert(
            &mut cache,
            PathBuf::from("/d/new"),
            (None, base + Duration::from_secs(20_000)),
        );
        assert_eq!(cache.entries.len(), CWD_GIT_CACHE_CAP);
    }

    /// Discovery failure preserves display data, not resolved scope membership.
    #[test]
    fn apply_cwd_git_refresh_preserves_last_good_on_none() {
        let mut cache = CwdGitCache::default();
        let dir = PathBuf::from("/repo");
        let good = CwdGitInfo {
            branch: Some("main".into()),
            is_worktree: false,
            main_repo: None,
            repo_root: None,
            family: RepositoryFamily::Resolved(PathBuf::from("/repo")),
            worktree_label: None,
        };
        // Seed a resolved entry with an old timestamp.
        let old_ts = Instant::now() - Duration::from_secs(60);
        cwd_cache_insert(&mut cache, dir.clone(), (Some(good), old_ts));

        // A `None` refresh must keep the branch and bump the timestamp.
        apply_cwd_git_refresh(&mut cache, dir.clone(), None);
        let (kept, ts) = cache.entries.get(&dir).expect("entry retained");
        assert_eq!(
            kept.as_ref().and_then(|i| i.branch.as_deref()),
            Some("main"),
            "a transient None must not blank a resolved branch"
        );
        assert!(*ts > old_ts, "timestamp must advance on refresh");

        assert_eq!(kept.as_ref().unwrap().family, RepositoryFamily::Failed);
        // A failed first lookup is distinct from a confirmed non-repository.
        let fresh = PathBuf::from("/not-a-repo");
        apply_cwd_git_refresh(&mut cache, fresh.clone(), None);
        assert_eq!(cache.entries.get(&fresh).unwrap().0.as_ref().unwrap().family,
            RepositoryFamily::Failed);
    }

    const POWERLINE: &str = "\u{e0a0}";
    const ALT_KEY: &str = "\u{2387}"; // ⎇
    const WIN_FALLBACK: &str = "\u{2261}"; // ≡

    #[test]
    fn windows_default_avoids_powerline_and_alt_key() {
        let icon = decide_branch_icon(None, HostOs::Windows, TerminalName::Unknown);
        assert_eq!(icon, WIN_FALLBACK);
        assert_ne!(icon, POWERLINE);
        assert_ne!(icon, ALT_KEY);
    }

    #[test]
    fn windows_terminal_brand_is_irrelevant_without_nerd_fonts() {
        for brand in [
            TerminalName::Unknown,
            TerminalName::VsCode,
            TerminalName::AppleTerminal,
            TerminalName::WezTerm,
            TerminalName::WindowsTerminal,
        ] {
            assert_eq!(
                decide_branch_icon(None, HostOs::Windows, brand),
                WIN_FALLBACK
            );
        }
    }

    #[test]
    fn nerd_fonts_opt_in_forces_powerline_even_on_windows() {
        for val in ["1", "true", "yes", "on"] {
            assert_eq!(
                decide_branch_icon(Some(val), HostOs::Windows, TerminalName::Unknown),
                POWERLINE
            );
            assert_eq!(
                decide_branch_icon(Some(val), HostOs::Macos, TerminalName::Unknown),
                POWERLINE
            );
        }
    }

    #[test]
    fn nerd_fonts_opt_out_forces_platform_fallback() {
        for val in ["0", "false"] {
            assert_eq!(
                decide_branch_icon(Some(val), HostOs::Windows, TerminalName::Unknown),
                WIN_FALLBACK
            );
            assert_eq!(
                decide_branch_icon(Some(val), HostOs::Macos, TerminalName::Unknown),
                ALT_KEY
            );
        }
    }

    #[test]
    fn non_windows_defaults_to_powerline() {
        assert_eq!(
            decide_branch_icon(None, HostOs::Macos, TerminalName::Unknown),
            POWERLINE
        );
        assert_eq!(
            decide_branch_icon(None, HostOs::Macos, TerminalName::VsCode),
            POWERLINE
        );
        assert_eq!(
            decide_branch_icon(None, HostOs::Linux, TerminalName::Ghostty),
            POWERLINE
        );
    }

    #[test]
    fn macos_stock_font_terminals_use_alt_key() {
        // Stock-font macOS terminals tofu the PUA glyph, so both must use `⎇`.
        for brand in [TerminalName::AppleTerminal, TerminalName::Iterm2] {
            assert_eq!(decide_branch_icon(None, HostOs::Macos, brand), ALT_KEY);
            assert_ne!(decide_branch_icon(None, HostOs::Macos, brand), POWERLINE);
        }
    }

    #[test]
    fn iterm2_nerd_fonts_opt_in_forces_powerline() {
        assert_eq!(
            decide_branch_icon(Some("1"), HostOs::Macos, TerminalName::Iterm2),
            POWERLINE
        );
    }

    #[test]
    fn collapse_home_requires_whole_path_component() {
        let home = Path::new("/Users/u");
        assert_eq!(
            collapse_home_path(Path::new("/Users/user/xai"), Some(home)),
            "/Users/user/xai"
        );
        assert_eq!(
            collapse_home_path(Path::new("/Users/u/xai"), Some(home)),
            "~/xai"
        );
        assert_eq!(
            collapse_home_path(Path::new("/Users/u/"), Some(Path::new("/Users/u/"))),
            "~"
        );
        assert_eq!(
            collapse_home_path(Path::new("/Users/u"), Some(Path::new("/Users/u"))),
            "~"
        );
    }

    #[test]
    fn compute_cwd_git_info_standalone_grok_worktree() {
        let main = crate::test_util::TempGitRepo::init("main-only");
        let clone = main.standalone_clone("wt-branch");
        assert!(
            clone.path.join(".git").is_dir(),
            "standalone clone .git is a directory"
        );
        assert!(
            clone
                .path
                .join(".git")
                .join("grok-worktree-source")
                .is_file(),
            "standalone clone carries the source marker"
        );
        let info = compute_cwd_git_info(&clone.path).expect("clone is a repo");
        assert!(info.is_worktree);
        assert_eq!(info.branch.as_deref(), Some("wt-branch"));
        assert_eq!(
            info.main_repo.as_deref(),
            Some(crate::test_util::collapsed_path_display(&main.path).as_str())
        );
    }

    #[test]
    fn compute_cwd_git_info_standalone_marker_from_nested_cwd() {
        let main = crate::test_util::TempGitRepo::init("main-only");
        let clone = main.standalone_clone("wt-branch");
        let nested = clone.path.join("sub").join("dir");
        std::fs::create_dir_all(&nested).unwrap();
        let info = compute_cwd_git_info(&nested).expect("nested path is in the clone");
        assert!(info.is_worktree);
        assert_eq!(info.branch.as_deref(), Some("wt-branch"));
        assert_eq!(
            info.main_repo.as_deref(),
            Some(crate::test_util::collapsed_path_display(&main.path).as_str())
        );
    }

    #[test]
    fn compute_cwd_git_info_nested_plain_repo_does_not_inherit_marker() {
        let main = crate::test_util::TempGitRepo::init("main-only");
        let clone = main.standalone_clone("wt-branch");
        let nested = clone.path.join("vendor").join("dep");
        crate::test_util::init_git_repo_on_branch(&nested, "dep-branch");
        let info = compute_cwd_git_info(&nested).expect("nested init is a repo");
        assert!(!info.is_worktree);
        assert!(info.main_repo.is_none());
        assert_eq!(info.branch.as_deref(), Some("dep-branch"));
    }

    #[serial_test::serial(GROK_HOME)]
    #[test]
    fn compute_cwd_git_info_nested_repo_does_not_inherit_db_record() {
        let home = tempfile::tempdir().unwrap();
        // serial(GROK_HOME) orders peers; EnvVarGuard restores on drop so later `open_default()` callers do not see a deleted temp home
        let _grok_home = crate::test_util::EnvVarGuard::set("GROK_HOME", home.path());
        let _ = xai_fast_worktree::db::WorktreeDb::open(home.path());

        let wt = crate::test_util::TempGitRepo::init("wt-branch");
        let canon_wt = dunce::canonicalize(&wt.path).unwrap_or_else(|_| wt.path.clone());
        let db = xai_fast_worktree::db::WorktreeDb::open(home.path()).unwrap();
        db.register(&xai_fast_worktree::db::WorktreeRecord {
            id: "db-wt".into(),
            path: canon_wt,
            source_repo: PathBuf::from("/src/main-repo"),
            repo_name: "main-repo".into(),
            kind: xai_fast_worktree::db::WorktreeKind::Session,
            creation_mode: "standalone".into(),
            git_ref: None,
            head_commit: None,
            session_id: None,
            creator_pid: None,
            created_at: 1,
            last_accessed_at: None,
            status: xai_fast_worktree::db::WorktreeStatus::Alive,
            metadata: Some(serde_json::json!({ "label": "db-label" })),
        })
        .unwrap();

        let nested = wt.path.join("vendor").join("dep");
        crate::test_util::init_git_repo_on_branch(&nested, "dep-branch");
        let info = compute_cwd_git_info(&nested).expect("nested init is a repo");
        assert!(!info.is_worktree);
        assert!(info.main_repo.is_none());
        assert!(info.worktree_label.is_none());
        assert_eq!(info.branch.as_deref(), Some("dep-branch"));
    }

    #[test]
    fn update_from_notification_ors_cached_label_into_is_worktree() {
        let dir = PathBuf::from("/nonexistent-xai-notif-label-wt");
        {
            let mut cache = CWD_GIT_CACHE.lock().expect("cache lock");
            cwd_cache_insert(
                &mut cache,
                dir.clone(),
                (
                    Some(CwdGitInfo {
                        branch: Some("main".into()),
                        is_worktree: true,
                        main_repo: None,
                        repo_root: None,
                        family: RepositoryFamily::Pending,
                        worktree_label: Some("my-label".into()),
                    }),
                    Instant::now(),
                ),
            );
        }
        update_from_notification(&dir, Some("main"), None, /* is_worktree */ false);
        let cache = CWD_GIT_CACHE.lock().expect("cache lock");
        let info = cache
            .entries.get(&dir)
            .and_then(|(info, _)| info.clone())
            .expect("cache entry");
        assert!(info.is_worktree);
        assert_eq!(info.worktree_label.as_deref(), Some("my-label"));
    }

    #[test]
    fn compute_cwd_git_info_plain_repo_is_not_worktree() {
        let repo = crate::test_util::TempGitRepo::init("main");
        let info = compute_cwd_git_info(&repo.path).expect("plain repo");
        assert!(!info.is_worktree);
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert!(info.main_repo.is_none());
    }

    #[test]
    fn compute_cwd_git_info_linked_git_worktree() {
        let main = crate::test_util::TempGitRepo::init("main-only");
        let wt = main.add_linked_worktree("wt", "wt-branch");
        assert!(wt.join(".git").is_file(), "linked worktree .git is a file");
        let info = compute_cwd_git_info(&wt).expect("linked worktree is a repo");
        assert!(info.is_worktree);
        assert_eq!(info.branch.as_deref(), Some("wt-branch"));
        let main_repo = info.main_repo.expect("linked worktree has main_repo");
        let expected = crate::test_util::collapsed_path_display(&main.path);
        let expected_canon = dunce::canonicalize(&main.path)
            .map(|p| crate::test_util::collapsed_path_display(&p))
            .unwrap_or_else(|_| expected.clone());
        assert!(
            main_repo == expected || main_repo == expected_canon,
            "main_repo={main_repo}, expected {expected} or {expected_canon}"
        );
    }

    #[test]
    fn family_root_unifies_main_checkout_subdirs_and_linked_worktrees() {
        let main = crate::test_util::TempGitRepo::init("main-only");
        let expected = dunce::canonicalize(&main.path).unwrap();
        let subdir = main.path.join("crates/inner");
        std::fs::create_dir_all(&subdir).unwrap();
        let wt = main.add_linked_worktree("wt", "wt-branch");
        for cwd in [&main.path, &subdir, &wt] {
            let info = compute_cwd_git_info(cwd).unwrap();
            assert_eq!(info.family_root(), Some(expected.as_path()));
        }
    }

    /// A git-head notification carries no repo root; a previously resolved one survives, so the
    /// main checkout keeps its family identity across branch-switch notifications.
    #[test]
    fn update_from_notification_preserves_resolved_repo_root() {
        let main = crate::test_util::TempGitRepo::init("main-only");
        let dir = main.path.clone();
        let seeded = compute_cwd_git_info(&dir).expect("main checkout is a repo");
        seed_cwd_cache_for_tests(&dir, Some(seeded.clone()));
        update_from_notification(&dir, Some("feature"), None, /* is_worktree */ false);
        let cache = CWD_GIT_CACHE.lock().expect("cache lock");
        let info = cache
            .entries.get(&dir)
            .and_then(|(info, _)| info.clone())
            .expect("cache entry");
        assert_eq!(info.branch.as_deref(), Some("feature"));
        assert_eq!(info.repo_root, seeded.repo_root);
        assert_eq!(info.family_root(), seeded.family_root());
    }
    #[test]
    fn active_scope_and_in_flight_entries_survive_cache_pressure() {
        let mut cache = CwdGitCache::default();
        let active: HashSet<_> = (0..96).map(|i| PathBuf::from(format!("/scope/{i}"))).collect();
        cache.active_scope = active.clone();
        let pending = PathBuf::from("/pending");
        cache.in_flight.insert(pending.clone());
        cwd_cache_insert(&mut cache, pending.clone(), (None, Instant::now()));
        for path in &active {
            cwd_cache_insert(&mut cache, path.clone(), (None, Instant::now()));
        }
        for i in 0..128 {
            cwd_cache_insert(&mut cache, PathBuf::from(format!("/display/{i}")), (None, Instant::now()));
        }
        for path in &active {
            assert!(cache.entries.contains_key(path), "active family cannot be evicted");
        }
        assert!(cache.entries.contains_key(&pending));
        if !cache.active_scope.is_empty() {
            cache.active_scope.clear();
            trim_cwd_cache(&mut cache);
        }
        assert!(cache.entries.len() <= CWD_GIT_CACHE_CAP);
        assert!(cache.entries.contains_key(&pending));
    }

    #[test]
    fn notification_seed_is_pending_until_discovery_establishes_family() {
        let repo = crate::test_util::TempGitRepo::init("main");
        update_from_notification(&repo.path, Some("notified"), None, false);
        {
            let cache = CWD_GIT_CACHE.lock().unwrap();
            let seeded = cache.entries.get(&repo.path).unwrap().0.as_ref().unwrap();
            assert_eq!(seeded.family, RepositoryFamily::Pending);
            assert_eq!(seeded.family_root(), None);
        }
        let fresh = compute_cwd_context(&repo.path);
        assert_eq!(fresh.family, RepositoryFamily::Resolved(dunce::canonicalize(&repo.path).unwrap()));
        let mut cache = CWD_GIT_CACHE.lock().unwrap();
        apply_cwd_git_refresh(&mut cache, repo.path.clone(), Some(fresh));
        assert_eq!(cache.entries.get(&repo.path).unwrap().0.as_ref().unwrap().family_root(),
            Some(dunce::canonicalize(&repo.path).unwrap().as_path()));
    }

    #[test]
    fn non_git_discovery_is_not_pending_or_failed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(compute_cwd_context(dir.path()).family, RepositoryFamily::NonGit);
    }

    #[cfg(unix)]
    #[test]
    fn family_identity_canonicalizes_aliases_and_marker_sources() {
        let main = crate::test_util::TempGitRepo::init("main");
        let clone = crate::test_util::TempGitRepo::init("feature");
        let dir = tempfile::tempdir().unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&main.path, &alias).unwrap();
        std::fs::create_dir(main.path.join("nested")).unwrap();
        std::fs::write(clone.path.join(".git/grok-worktree-source"), alias.join("nested").to_str().unwrap()).unwrap();
        let expected = dunce::canonicalize(&main.path).unwrap();
        for cwd in [&main.path, &alias, &clone.path] {
            assert_eq!(compute_cwd_git_info(cwd).unwrap().family_root(), Some(expected.as_path()));
        }
    }

    #[tokio::test]
    async fn pending_scope_jobs_are_not_reserved_again_after_ttl_or_capacity_pressure() {
        let mut cache = CwdGitCache::default();
        let reserved_at = Instant::now() - CWD_GIT_REFRESH_TTL * 2;
        let paths: Vec<_> = (0..96).map(|i| PathBuf::from(format!("/pending-scope/{i}"))).collect();
        cache.active_scope.extend(paths.iter().cloned());
        cache.in_flight.extend(paths.iter().cloned());
        for path in &paths {
            cwd_cache_insert(&mut cache, path.clone(), (None, reserved_at));
        }
        for path in &paths {
            reserve_cwd_git_refresh(&mut cache, path, GitRead::Scope);
            assert_eq!(cache.entries.get(path).unwrap().1, reserved_at,
                "a running discovery cannot acquire a replacement reservation");
        }
        assert_eq!(cache.in_flight.len(), 96);
        assert_eq!(cache.entries.len(), 96);
    }

    #[test]
    fn missing_directory_is_failed_discovery_not_confirmed_non_git() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(compute_cwd_context(&temp.path().join("missing")).family,
            RepositoryFamily::Failed);
    }

}
