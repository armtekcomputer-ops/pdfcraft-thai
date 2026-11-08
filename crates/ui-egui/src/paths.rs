//! Per-user folder conventions shared by crash recovery, the OCR dictionary, digital IDs and the
//! copied-image clipboard cache.
//!
//! Every per-user folder is [`PRODUCT_DIR`]'s, never PdfCraft's own `pdfcraft`/`PdfCraft`, so the
//! two installs don't read or write each other's preferences, recovery snapshots, dictionaries or
//! clipboard file. (The installer, the bundle id and portable mode's `PdfCraftData` folder keep
//! their existing names.) The platform is a parameter rather than a `cfg!` inside each formula, so
//! tests check every platform's path on whatever machine runs them.

use std::path::{Path, PathBuf};

use crate::PRODUCT_DIR;

/// The platform whose per-user folder convention applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    MacOs,
    Windows,
    Unix,
}

impl Platform {
    /// The platform this build runs on.
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// `<base>/<PRODUCT_DIR>/<sub>`, where `base` is the platform's per-user folder for that kind of
/// file and `sub` keeps that platform's spelling (`Recovery`/`Dictionary`/`Clipboard` on macOS and
/// Windows, lower case elsewhere).
pub(crate) fn product_dir_in(platform: Platform, base: &Path, sub: &str) -> PathBuf {
    let sub = match platform {
        Platform::MacOs | Platform::Windows => sub.to_string(),
        Platform::Unix => sub.to_ascii_lowercase(),
    };
    base.join(PRODUCT_DIR).join(sub)
}

/// The per-user data base that holds crash recovery, the OCR dictionary and digital IDs:
/// `~/Library/Application Support` (macOS), `%LOCALAPPDATA%` (Windows), or `$XDG_DATA_HOME` /
/// `~/.local/share` (others). Relative environment paths are ignored, as the XDG spec requires:
/// a relative base would put the files in the working folder.
pub(crate) fn data_base(platform: Platform, home: Option<&Path>, xdg_data_home: Option<&Path>, local_app_data: Option<&Path>) -> Option<PathBuf> {
    match platform {
        Platform::MacOs => home.filter(|p| p.is_absolute()).map(|h| h.join("Library/Application Support")),
        Platform::Windows => local_app_data.filter(|p| p.is_absolute()).map(Path::to_path_buf),
        Platform::Unix => xdg_data_home
            .filter(|p| p.is_absolute())
            .map(Path::to_path_buf)
            .or_else(|| home.filter(|p| p.is_absolute()).map(|h| h.join(".local/share"))),
    }
}

/// The per-user cache base that holds the copied-image clipboard file: `~/Library/Caches` (macOS),
/// `%LOCALAPPDATA%` (Windows), or `$XDG_CACHE_HOME` / `~/.cache` (others). Relative environment
/// paths are ignored, like [`data_base`]. The web build has no clipboard cache, so the only caller
/// is not compiled there.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn cache_base(platform: Platform, home: Option<&Path>, xdg_cache_home: Option<&Path>, local_app_data: Option<&Path>) -> Option<PathBuf> {
    match platform {
        Platform::MacOs => home.filter(|p| p.is_absolute()).map(|h| h.join("Library/Caches")),
        Platform::Windows => local_app_data.filter(|p| p.is_absolute()).map(Path::to_path_buf),
        Platform::Unix => {
            xdg_cache_home.filter(|p| p.is_absolute()).map(Path::to_path_buf).or_else(|| home.filter(|p| p.is_absolute()).map(|h| h.join(".cache")))
        }
    }
}

/// Folders an earlier PdfCraft, or this build before its folders were separated, kept per-user
/// files in: recovery, the OCR dictionary, digital IDs and the copied-image clipboard file.
/// Nothing here is read any more; the list only lets a start-up note name data the user may want
/// to copy by hand. Relative environment paths are ignored, as in [`data_base`].
fn upstream_dirs(
    platform: Platform,
    home: Option<&Path>,
    xdg_data_home: Option<&Path>,
    xdg_cache_home: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Vec<PathBuf> {
    let capitalised = !matches!(platform, Platform::Unix);
    let mut dirs = Vec::new();
    let data = match platform {
        Platform::MacOs => home.filter(|p| p.is_absolute()).map(|h| h.join("Library/Application Support/PdfCraft")),
        Platform::Windows => local_app_data.filter(|p| p.is_absolute()).map(|d| d.join("PdfCraft")),
        Platform::Unix => data_base(platform, home, xdg_data_home, None).map(|d| d.join("pdfcraft")),
    };
    if let Some(data) = data {
        let sub = if capitalised { "Recovery" } else { "recovery" };
        dirs.push(data.join(sub));
        dirs.push(data.join(if capitalised { "Dictionary" } else { "dictionary" }));
        dirs.push(data.join("Digital IDs"));
    }
    let cache = match platform {
        Platform::MacOs => home.filter(|p| p.is_absolute()).map(|h| h.join("Library/Caches/PdfCraft")),
        Platform::Windows => local_app_data.filter(|p| p.is_absolute()).map(|d| d.join("PdfCraft")),
        Platform::Unix => cache_base(platform, home, xdg_cache_home, None).map(|d| d.join("pdfcraft")),
    };
    if let Some(cache) = cache {
        dirs.push(cache.join(if capitalised { "Clipboard" } else { "clipboard" }));
    }
    dirs
}

/// [`upstream_dirs`] on this platform and environment, keeping only folders that exist and hold at
/// least one entry. Best effort for a start-up log note; this build never imports them.
pub fn upstream_dirs_with_files() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let dirs = upstream_dirs(
        Platform::current(),
        env("HOME").as_deref(),
        env("XDG_DATA_HOME").as_deref(),
        env("XDG_CACHE_HOME").as_deref(),
        env("LOCALAPPDATA").as_deref(),
    );
    dirs.into_iter().filter(|d| std::fs::read_dir(d).is_ok_and(|mut entries| entries.next().is_some())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute path on whatever machine runs the tests: the platform branches under test only
    /// join folder names onto a base, and `is_absolute` is a host question.
    fn absolute(tag: &str) -> PathBuf {
        if cfg!(windows) { PathBuf::from(format!(r"C:\{tag}")) } else { PathBuf::from(format!("/{tag}")) }
    }

    #[test]
    fn every_platform_puts_the_product_folder_under_its_base_and_never_upstreams() {
        let home = absolute("home/someone");
        let local = absolute("local/appdata");
        let data = absolute("var/lib/someone");
        let cache = absolute("var/cache/someone");
        let (home, local, data, cache) = (home.as_path(), local.as_path(), data.as_path(), cache.as_path());
        let mac_data = home.join("Library/Application Support");
        for (platform, base, sub) in
            [(Platform::Unix, data, "recovery"), (Platform::MacOs, mac_data.as_path(), "Recovery"), (Platform::Windows, local, "Recovery")]
        {
            let dir = product_dir_in(platform, base, "Recovery");
            assert_eq!(dir, base.join(PRODUCT_DIR).join(sub));
            assert!(dir.components().all(|c| c.as_os_str() != "pdfcraft" && c.as_os_str() != "PdfCraft"), "{dir:?}");
        }
        assert_eq!(data_base(Platform::Unix, Some(home), None, None), Some(home.join(".local/share")));
        assert_eq!(data_base(Platform::Unix, Some(home), Some(data), None), Some(data.to_path_buf()));
        assert_eq!(data_base(Platform::MacOs, Some(home), None, None), Some(home.join("Library/Application Support")));
        assert_eq!(data_base(Platform::Windows, None, None, Some(local)), Some(local.to_path_buf()));
        assert_eq!(data_base(Platform::Windows, Some(home), None, None), None, "Windows uses %LOCALAPPDATA%, not HOME");
        assert_eq!(cache_base(Platform::Unix, Some(home), None, None), Some(home.join(".cache")));
        assert_eq!(cache_base(Platform::Unix, Some(home), Some(cache), None), Some(cache.to_path_buf()));
        assert_eq!(cache_base(Platform::MacOs, Some(home), None, None), Some(home.join("Library/Caches")));
        assert_eq!(cache_base(Platform::Windows, None, None, Some(local)), Some(local.to_path_buf()));
    }

    #[test]
    fn relative_environment_paths_are_ignored_and_home_is_used_instead() {
        let home = absolute("home/someone");
        let relative = Path::new("relative/base");
        assert_eq!(
            data_base(Platform::Unix, Some(&home), Some(relative), None),
            Some(home.join(".local/share")),
            "relative XDG_DATA_HOME is ignored"
        );
        assert_eq!(cache_base(Platform::Unix, Some(&home), Some(relative), None), Some(home.join(".cache")), "relative XDG_CACHE_HOME is ignored");
        assert_eq!(data_base(Platform::Unix, Some(relative), None, None), None, "relative HOME is ignored");
        assert_eq!(cache_base(Platform::Unix, Some(relative), None, None), None);
        assert_eq!(data_base(Platform::MacOs, Some(&home), None, Some(relative)), Some(home.join("Library/Application Support")));
        assert_eq!(data_base(Platform::Windows, Some(&home), None, Some(relative)), None, "relative LOCALAPPDATA is ignored");
        assert_eq!(cache_base(Platform::Windows, Some(&home), None, Some(relative)), None);
    }

    #[test]
    fn the_upstream_folders_a_start_up_note_can_name_are_the_old_ones() {
        let home = absolute("home/someone");
        let local = absolute("local/appdata");
        let unix = upstream_dirs(Platform::Unix, Some(&home), None, None, None);
        assert_eq!(
            unix,
            [
                home.join(".local/share/pdfcraft/recovery"),
                home.join(".local/share/pdfcraft/dictionary"),
                home.join(".local/share/pdfcraft/Digital IDs"),
                home.join(".cache/pdfcraft/clipboard"),
            ]
        );
        let macos = upstream_dirs(Platform::MacOs, Some(&home), None, None, None);
        assert_eq!(
            macos,
            [
                home.join("Library/Application Support/PdfCraft/Recovery"),
                home.join("Library/Application Support/PdfCraft/Dictionary"),
                home.join("Library/Application Support/PdfCraft/Digital IDs"),
                home.join("Library/Caches/PdfCraft/Clipboard"),
            ]
        );
        let windows = upstream_dirs(Platform::Windows, None, None, None, Some(&local));
        assert_eq!(
            windows,
            [
                local.join("PdfCraft").join("Recovery"),
                local.join("PdfCraft").join("Dictionary"),
                local.join("PdfCraft").join("Digital IDs"),
                local.join("PdfCraft").join("Clipboard"),
            ]
        );
    }
}
