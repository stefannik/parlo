//! Finding things on disk: the bundled yt-dlp, a system yt-dlp / deno / curl (GUI apps don't inherit the shell PATH, so
//! a fixed list of directories is searched), and the browsers whose cookies yt-dlp could read. Every function takes its
//! search roots as arguments so the tests can use temp dirs instead of the real `/Applications`.

use std::path::{Path, PathBuf};

/// Where Homebrew & friends put binaries — what a Finder-launched app can't see in `$PATH`.
pub fn default_tool_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(h) = home {
        v.push(h.join(".local/bin"));
    }
    v
}

pub fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// First executable called `name` in `dirs` (in order), then in `$PATH` (a no-op for a GUI app, useful in dev/tests).
pub fn find_in_dirs(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    for d in dirs {
        let p = d.join(name);
        if is_executable(&p) {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

/// The settings override (a file, or a directory containing `yt-dlp`), else the search dirs.
pub fn find_system_ytdlp(override_path: Option<&str>, dirs: &[PathBuf]) -> Option<PathBuf> {
    if let Some(o) = override_path.map(str::trim).filter(|o| !o.is_empty()) {
        let p = PathBuf::from(o);
        if is_executable(&p) {
            return Some(p);
        }
        let inner = p.join("yt-dlp");
        if is_executable(&inner) {
            return Some(inner);
        }
        return None; // an explicit override that doesn't exist must not silently fall back to another binary
    }
    find_in_dirs("yt-dlp", dirs)
}

pub fn find_deno(dirs: &[PathBuf]) -> Option<PathBuf> {
    find_in_dirs("deno", dirs)
}

/// `/usr/bin/curl` (always there on macOS), else a search.
pub fn find_curl(dirs: &[PathBuf]) -> Option<PathBuf> {
    let sys = PathBuf::from("/usr/bin/curl");
    if is_executable(&sys) {
        return Some(sys);
    }
    find_in_dirs("curl", dirs)
}

/// The yt-dlp shipped inside the app / fetched for development. Layout `…/yt-dlp/yt-dlp` (+ `_internal/` for the onedir
/// build). Order: `PARLO_YTDLP` env → `<exe>/../Resources/yt-dlp/yt-dlp` (app bundle) → `tools/yt-dlp/yt-dlp` in the working
/// directory or one of its ancestors (dev).
pub fn bundled_ytdlp() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PARLO_YTDLP") {
        let p = PathBuf::from(p);
        return is_executable(&p).then_some(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(contents) = exe.parent().and_then(|d| d.parent()) {
            let p = contents.join("Resources/yt-dlp/yt-dlp");
            if is_executable(&p) {
                return Some(p);
            }
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let p = dir.join("tools/yt-dlp/yt-dlp");
        if is_executable(&p) {
            return Some(p);
        }
        if !dir.pop() {
            return None;
        }
    }
}

// ------------------------------------------------------------------------------------------------ browsers (cookies)

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Browser {
    /// yt-dlp's name for `--cookies-from-browser`
    pub id: &'static str,
    pub label: &'static str,
}

struct BrowserDef {
    id: &'static str,
    label: &'static str,
    app: &'static str,
    /// profile directory under `~/Library/Application Support`
    profile: &'static str,
}

const BROWSERS: &[BrowserDef] = &[
    BrowserDef {
        id: "chrome",
        label: "Google Chrome",
        app: "Google Chrome.app",
        profile: "Google/Chrome",
    },
    BrowserDef {
        id: "chromium",
        label: "Chromium",
        app: "Chromium.app",
        profile: "Chromium",
    },
    BrowserDef {
        id: "brave",
        label: "Brave",
        app: "Brave Browser.app",
        profile: "BraveSoftware/Brave-Browser",
    },
    BrowserDef {
        id: "edge",
        label: "Microsoft Edge",
        app: "Microsoft Edge.app",
        profile: "Microsoft Edge",
    },
    BrowserDef {
        id: "firefox",
        label: "Firefox",
        app: "Firefox.app",
        profile: "Firefox",
    },
    BrowserDef {
        id: "opera",
        label: "Opera",
        app: "Opera.app",
        profile: "com.operasoftware.Opera",
    },
    BrowserDef {
        id: "vivaldi",
        label: "Vivaldi",
        app: "Vivaldi.app",
        profile: "Vivaldi",
    },
    BrowserDef {
        id: "whale",
        label: "Whale",
        app: "Whale.app",
        profile: "Naver/Whale",
    },
];

/// The names `--cookies-from-browser` may be given (anything else is refused before it reaches a command line).
pub fn is_known_browser(id: &str) -> bool {
    id == "safari" || BROWSERS.iter().any(|b| b.id == id)
}

/// Where to look. The real app uses [`BrowserSearch::system`]; tests build one around a temp dir.
#[derive(Debug, Clone)]
pub struct BrowserSearch {
    pub app_dirs: Vec<PathBuf>,
    pub home: PathBuf,
    /// Safari lives in the system volume, not /Applications
    pub system_app_dirs: Vec<PathBuf>,
}

impl BrowserSearch {
    pub fn system() -> Self {
        let home = home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            app_dirs: vec![PathBuf::from("/Applications"), home.join("Applications")],
            system_app_dirs: vec![
                PathBuf::from("/Applications"),
                PathBuf::from("/System/Applications"),
                PathBuf::from("/System/Cryptexes/App/System/Applications"),
            ],
            home,
        }
    }
}

/// Browsers that are installed *and* have a profile on this account (a browser that was never opened has no cookies to
/// read). Safari counts when its app exists or its cookie store does (reading it needs Full Disk Access).
pub fn detect_browsers(s: &BrowserSearch) -> Vec<Browser> {
    let support = s.home.join("Library/Application Support");
    let mut out = Vec::new();
    for b in BROWSERS {
        let has_app = s.app_dirs.iter().any(|d| d.join(b.app).is_dir());
        if has_app && support.join(b.profile).is_dir() {
            out.push(Browser {
                id: b.id,
                label: b.label,
            });
        }
    }
    let safari_app = s
        .system_app_dirs
        .iter()
        .any(|d| d.join("Safari.app").is_dir());
    let safari_cookies = s
        .home
        .join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies")
        .exists()
        || s.home
            .join("Library/Cookies/Cookies.binarycookies")
            .exists();
    if safari_app || safari_cookies {
        out.push(Browser {
            id: "safari",
            label: "Safari",
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn exe(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn search_order_is_dirs_first() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        let in_b = exe(&b, "yt-dlp");
        assert_eq!(find_in_dirs("yt-dlp", &[a.clone(), b.clone()]), Some(in_b));
        let in_a = exe(&a, "yt-dlp");
        assert_eq!(find_in_dirs("yt-dlp", &[a, b]), Some(in_a));
    }

    #[test]
    fn non_executable_files_are_ignored() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("deno"), "x").unwrap();
        assert_eq!(
            find_deno(&[t.path().to_path_buf()]).filter(|p| p.starts_with(t.path())),
            None
        );
    }

    #[test]
    fn override_wins_and_never_falls_back() {
        let t = tempfile::tempdir().unwrap();
        let dirs = vec![t.path().join("bin")];
        let sys = exe(&dirs[0], "yt-dlp");
        let custom = exe(&t.path().join("custom"), "my-ytdlp");
        assert_eq!(
            find_system_ytdlp(Some(custom.to_str().unwrap()), &dirs),
            Some(custom.clone())
        );
        // a directory containing yt-dlp is accepted too
        let dir_override = t.path().join("custom2");
        let inner = exe(&dir_override, "yt-dlp");
        assert_eq!(
            find_system_ytdlp(Some(dir_override.to_str().unwrap()), &dirs),
            Some(inner)
        );
        // a broken override must not silently use another binary
        assert_eq!(find_system_ytdlp(Some("/nope/yt-dlp"), &dirs), None);
        // empty override = search
        assert_eq!(find_system_ytdlp(Some("  "), &dirs), Some(sys.clone()));
        assert_eq!(find_system_ytdlp(None, &dirs), Some(sys));
    }

    #[test]
    fn default_dirs_cover_homebrew_both_archs_and_local_bin() {
        let d = default_tool_dirs(Some(Path::new("/Users/me")));
        assert_eq!(
            d,
            vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/Users/me/.local/bin")
            ]
        );
    }

    fn fake_mac(t: &Path) -> BrowserSearch {
        let s = BrowserSearch {
            app_dirs: vec![t.join("Applications"), t.join("home/Applications")],
            system_app_dirs: vec![t.join("Applications"), t.join("System/Applications")],
            home: t.join("home"),
        };
        std::fs::create_dir_all(s.home.join("Library/Application Support")).unwrap();
        s
    }

    #[test]
    fn browsers_need_app_and_profile() {
        let t = tempfile::tempdir().unwrap();
        let s = fake_mac(t.path());
        let sup = s.home.join("Library/Application Support");
        // chrome: both → listed
        std::fs::create_dir_all(t.path().join("Applications/Google Chrome.app")).unwrap();
        std::fs::create_dir_all(sup.join("Google/Chrome")).unwrap();
        // firefox: app only (never launched) → not listed
        std::fs::create_dir_all(t.path().join("Applications/Firefox.app")).unwrap();
        // brave: profile only (app deleted) → not listed
        std::fs::create_dir_all(sup.join("BraveSoftware/Brave-Browser")).unwrap();
        // vivaldi: app in ~/Applications + profile → listed
        std::fs::create_dir_all(s.home.join("Applications/Vivaldi.app")).unwrap();
        std::fs::create_dir_all(sup.join("Vivaldi")).unwrap();
        let ids: Vec<_> = detect_browsers(&s).iter().map(|b| b.id).collect();
        assert_eq!(ids, vec!["chrome", "vivaldi"]);
    }

    #[test]
    fn safari_by_app_or_cookie_store() {
        let t = tempfile::tempdir().unwrap();
        let s = fake_mac(t.path());
        assert!(detect_browsers(&s).is_empty());
        std::fs::create_dir_all(t.path().join("System/Applications/Safari.app")).unwrap();
        assert_eq!(
            detect_browsers(&s).iter().map(|b| b.id).collect::<Vec<_>>(),
            vec!["safari"]
        );
        std::fs::remove_dir_all(t.path().join("System/Applications/Safari.app")).unwrap();
        let ck = s
            .home
            .join("Library/Containers/com.apple.Safari/Data/Library/Cookies");
        std::fs::create_dir_all(&ck).unwrap();
        std::fs::write(ck.join("Cookies.binarycookies"), b"cook").unwrap();
        assert_eq!(detect_browsers(&s).len(), 1);
    }

    #[test]
    fn all_nine_browsers_have_ytdlp_names() {
        for id in [
            "chrome", "chromium", "brave", "edge", "firefox", "opera", "vivaldi", "whale", "safari",
        ] {
            assert!(is_known_browser(id), "{id}");
        }
        assert!(!is_known_browser("chrome:Profile 1; rm"));
        assert!(!is_known_browser(""));
    }
}
