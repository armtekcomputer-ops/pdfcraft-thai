//! Platform preference location, separate from logs and crash recovery.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use pdfcraft_ui_egui::PRODUCT_DIR;

static STAGING_ID: AtomicU64 = AtomicU64::new(0);

pub fn directory(
    xdg: bool,
    portable: Option<&Path>,
    legacy: Option<PathBuf>,
    config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> std::io::Result<Option<PathBuf>> {
    if let Some(portable) = portable {
        return Ok(Some(portable.to_path_buf()));
    }
    if !xdg {
        return Ok(legacy);
    }
    // XDG requires absolute environment paths; empty and relative overrides are ignored.
    let preferred = config_home.filter(|p| p.is_absolute()).or_else(|| home.filter(|p| p.is_absolute()).map(|p| p.join(".config")));
    let Some(preferred) = preferred.map(|p| p.join(PRODUCT_DIR)) else { return Ok(legacy) };
    let target = preferred.join("app.ron");
    if std::fs::symlink_metadata(&target).is_ok() {
        return Ok(Some(preferred));
    }
    create_private_directory(&preferred)?;
    if let Some(source) = legacy.map(|p| p.join("app.ron")) {
        match std::fs::File::open(&source) {
            Ok(mut source) => migrate(&mut source, &preferred, &target)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Some(preferred))
}

/// The folder `app.ron` is read from and saved to: [`directory`]'s choice, or `data` (this build's
/// per-user data folder) when the platform has no configuration base, so a launch without
/// `HOME`/`XDG_CONFIG_HOME` still saves preferences instead of silently dropping them.
///
/// The fallback folder is made owner-only even when it already exists: the logger runs first and
/// creates it with the umask, and preferences are private.
///
/// No legacy source is passed: this build starts with its own preferences rather than importing
/// another PdfCraft's recents and settings.
pub fn preferences_dir(
    xdg: bool,
    portable: Option<&Path>,
    data: Option<PathBuf>,
    config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> std::io::Result<Option<PathBuf>> {
    if let Some(dir) = directory(xdg, portable, None, config_home, home)? {
        return Ok(Some(dir));
    }
    match data {
        Some(dir) => {
            create_private_directory(&dir)?;
            tighten_private(&dir);
            Ok(Some(dir))
        }
        None => Ok(None),
    }
}

/// The per-user data folder to keep logs and fallback preferences in: `data` when it is absolute.
/// eframe filters a relative `XDG_DATA_HOME` but not a relative `HOME`, and a relative folder
/// would put preferences and logs in the working directory.
pub fn data_dir(data: Option<PathBuf>) -> Option<PathBuf> {
    data.filter(|d| d.is_absolute())
}

/// Make an existing folder owner-only on Unix. [`create_private_directory`] leaves an existing
/// folder's permissions alone, but the data folder may already exist because the logger made it
/// with the umask. A volume that can't change modes (FAT, some network shares) works as before,
/// with one warning in the log.
fn tighten_private(directory: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let open_to_others = std::fs::metadata(directory).is_ok_and(|m| m.permissions().mode() & 0o077 != 0);
        if open_to_others && let Err(e) = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)) {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| log::warn!("preferences folder {} can't be made owner-only: {e}", directory.display()));
        }
    }
    #[cfg(not(unix))]
    let _ = directory;
}

/// Create private configuration directories without changing existing directory permissions.
fn create_private_directory(directory: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)
}

/// Publish a complete copy without replacing a concurrent launch's preferences. Keep the
/// original file and all other data (especially recovery and digital IDs) where they were.
fn migrate(source: &mut std::fs::File, directory: &Path, target: &Path) -> std::io::Result<()> {
    create_private_directory(directory)?;
    for _ in 0..32 {
        let id = STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let staging = directory.join(format!(".app.ron-migrate-{}-{id}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&staging) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let result = (|| {
            std::io::copy(source, &mut file)?;
            file.sync_all()?;
            // Both files are in the same directory/filesystem. Unlike rename, hard_link
            // atomically refuses an existing destination on Unix as well as Windows.
            match std::fs::hard_link(&staging, target) {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                result => result,
            }
        })();
        drop(file);
        let _ = std::fs::remove_file(&staging);
        return result;
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free settings migration staging name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_preferences_use_config_instead_of_data() {
        let fixture = Fixture::new();
        let home = fixture.0.join("home");
        let data = home.join(".local/share/pdfcraft");
        assert_eq!(directory(true, None, Some(data), None, Some(home.clone())).unwrap(), Some(home.join(".config/pdfcraft-thai")));
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let id = STAGING_ID.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("pdfcraft-settings-{}-{id}", std::process::id()));
            std::fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn xdg_override_and_invalid_overrides() {
        let f = Fixture::new();
        let home = f.0.join("home");
        let config = f.0.join("custom-config");
        assert_eq!(directory(true, None, None, Some(config.clone()), Some(home.clone())).unwrap(), Some(config.join("pdfcraft-thai")));
        for invalid in [PathBuf::new(), PathBuf::from("relative")] {
            assert_eq!(directory(true, None, None, Some(invalid), Some(home.clone())).unwrap(), Some(home.join(".config/pdfcraft-thai")));
        }
        assert_eq!(directory(true, None, Some(f.0.clone()), None, None).unwrap(), Some(f.0.clone()));
    }

    #[test]
    fn migration_preserves_preferences_and_other_data() {
        let f = Fixture::new();
        let data = f.0.join("data/pdfcraft");
        let config = f.0.join("config");
        std::fs::create_dir_all(data.join("recovery")).unwrap();
        std::fs::write(data.join("app.ron"), b"(theme:Dark,language:German)").unwrap();
        std::fs::write(data.join("recovery/unsaved.pdf"), b"UNSAVED").unwrap();
        std::fs::write(data.join("digital-id.p12"), b"KEY").unwrap();
        let selected = directory(true, None, Some(data.clone()), Some(config.clone()), None).unwrap().unwrap();
        assert_eq!(selected, config.join("pdfcraft-thai"));
        assert_eq!(std::fs::read(selected.join("app.ron")).unwrap(), std::fs::read(data.join("app.ron")).unwrap());
        assert_eq!(std::fs::read(data.join("recovery/unsaved.pdf")).unwrap(), b"UNSAVED");
        assert_eq!(std::fs::read(data.join("digital-id.p12")).unwrap(), b"KEY");
        // The migrated file is a copy, not a link to the original settings.
        std::fs::write(selected.join("app.ron"), b"NEW").unwrap();
        assert_eq!(std::fs::read(data.join("app.ron")).unwrap(), b"(theme:Dark,language:German)");
        assert_eq!(directory(true, None, Some(data), Some(config), None).unwrap(), Some(selected.clone()));
        assert_eq!(std::fs::read(selected.join("app.ron")).unwrap(), b"NEW");
        assert_eq!(std::fs::read_dir(selected).unwrap().count(), 1);
    }

    #[test]
    fn existing_or_concurrently_created_preferences_are_never_replaced() {
        let f = Fixture::new();
        let source = f.0.join("old.ron");
        let config = f.0.join("config/pdfcraft");
        std::fs::write(&source, b"OLD").unwrap();
        std::fs::create_dir_all(&config).unwrap();
        let target = config.join("app.ron");
        std::fs::write(&target, b"CURRENT").unwrap();
        migrate(&mut std::fs::File::open(source).unwrap(), &config, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"CURRENT");
        assert_eq!(std::fs::read_dir(config).unwrap().count(), 1);
    }

    #[test]
    fn failed_migration_keeps_original_available() {
        let f = Fixture::new();
        std::fs::write(f.0.join("app.ron"), b"OLD").unwrap();
        let blocked = f.0.join("blocked");
        std::fs::write(&blocked, b"NOT A DIRECTORY").unwrap();
        assert!(directory(true, None, Some(f.0.clone()), Some(blocked), None).is_err());
        assert_eq!(std::fs::read(f.0.join("app.ron")).unwrap(), b"OLD");
    }

    #[test]
    fn portable_and_non_xdg_platforms_keep_their_locations() {
        let f = Fixture::new();
        let portable = f.0.join("PdfCraftData");
        for xdg in [true, false] {
            assert_eq!(directory(xdg, Some(&portable), Some(f.0.clone()), Some(f.0.clone()), None).unwrap(), Some(portable.clone()));
        }
        assert_eq!(directory(false, None, Some(f.0.clone()), Some(portable), None).unwrap(), Some(f.0.clone()));
        assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 0, "no migration outside XDG mode");
    }

    #[test]
    fn preferences_dir_falls_back_to_the_data_folder_without_a_configuration_base() {
        let f = Fixture::new();
        let data = f.0.join("data/pdfcraft-thai");
        // Neither XDG_CONFIG_HOME nor HOME: `directory` names no folder. Preferences must still
        // go somewhere (the data folder eframe uses), not be dropped.
        assert_eq!(preferences_dir(true, None, Some(data.clone()), None, None).unwrap(), Some(data.clone()));
        assert!(data.is_dir(), "the fallback folder is created");
        assert_eq!(preferences_dir(true, None, None, None, None).unwrap(), None, "nothing to fall back to");
    }

    #[cfg(unix)]
    #[test]
    fn the_data_folder_fallback_is_private_like_the_xdg_folder() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let selected = preferences_dir(true, None, Some(f.0.join("data/pdfcraft-thai")), None, None).unwrap().unwrap();
        assert_eq!(std::fs::metadata(&selected).unwrap().permissions().mode() & 0o077, 0, "only the owner may open the preferences folder");
    }

    /// The logger runs before the settings are read and creates the data folder with the umask;
    /// the fallback must still end owner-only.
    #[cfg(unix)]
    #[test]
    fn the_data_folder_fallback_tightens_a_folder_the_logger_made() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let data = f.0.join("data/pdfcraft-thai");
        std::fs::create_dir_all(data.join("logs")).unwrap();
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
        let selected = preferences_dir(true, None, Some(data.clone()), None, None).unwrap().unwrap();
        assert_eq!(selected, data);
        assert_eq!(std::fs::metadata(&selected).unwrap().permissions().mode() & 0o777, 0o700, "the fallback folder is tightened");
        assert!(selected.join("logs").is_dir(), "the logger's folder is untouched");
    }

    #[test]
    fn a_relative_data_folder_is_ignored() {
        let f = Fixture::new();
        assert_eq!(data_dir(Some(PathBuf::from("relative/data"))), None);
        assert_eq!(data_dir(None), None);
        assert_eq!(data_dir(Some(f.0.clone())), Some(f.0.clone()), "an absolute data folder is kept");
    }

    #[test]
    fn preferences_dir_never_imports_another_pdfcrafts_settings() {
        let f = Fixture::new();
        let home = f.0.join("home");
        // An upstream PdfCraft's data folder, with preferences in it.
        let upstream = home.join(".local/share/pdfcraft");
        std::fs::create_dir_all(&upstream).unwrap();
        std::fs::write(upstream.join("app.ron"), b"(theme:Dark)").unwrap();
        let selected = preferences_dir(true, None, Some(upstream.clone()), None, Some(home.clone())).unwrap().unwrap();
        assert_eq!(selected, home.join(".config/pdfcraft-thai"));
        assert!(selected.ends_with("pdfcraft-thai"), "this build's own folder: {}", selected.display());
        assert!(!selected.join("app.ron").exists(), "nothing is imported from another PdfCraft");
        assert_eq!(std::fs::read(upstream.join("app.ron")).unwrap(), b"(theme:Dark)", "the other app's file is untouched");
    }

    #[cfg(unix)]
    #[test]
    fn new_config_directory_is_private_and_existing_permissions_are_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let selected = directory(true, None, None, Some(f.0.join("config")), None).unwrap().unwrap();
        assert_eq!(std::fs::metadata(&selected).unwrap().permissions().mode() & 0o777, 0o700);
        std::fs::set_permissions(&selected, std::fs::Permissions::from_mode(0o750)).unwrap();
        create_private_directory(&selected).unwrap();
        assert_eq!(std::fs::metadata(selected).unwrap().permissions().mode() & 0o777, 0o750);
    }
}
