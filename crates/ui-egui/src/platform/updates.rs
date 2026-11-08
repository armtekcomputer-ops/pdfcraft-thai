//! Help ▸ Check for updates and the check at start, through `craft_update`: the flow itself is its
//! `Updater`; this module supplies the suite and the quit path, and shows the result in
//! `craft_update`'s `CheckResultDialog`.

#[cfg(not(target_arch = "wasm32"))]
use craft_update::updater::{Action, CheckSource, Status, Updater};
#[cfg(not(target_arch = "wasm32"))]
use craft_update::{CheckResultDialog, CheckResultResponse};

use crate::PdfCraftApp;

/// Where every PdfCraft release is listed.
pub const RELEASES_PAGE: &str = "https://github.com/armtekcomputer-ops/pdfcraft-thai/releases";

/// Stands in for the network in tests.
#[cfg(not(target_arch = "wasm32"))]
pub type UpdateSource = CheckSource;

pub(crate) struct Updates {
    #[cfg(not(target_arch = "wasm32"))]
    updater: Updater,
    /// The Updates dialog is showing.
    pub(crate) open: bool,
    /// Ask for a newer release at start, unless Preferences says not to; the desktop app turns
    /// it on, so tests never reach the network.
    #[cfg(not(target_arch = "wasm32"))]
    startup_check: bool,
    #[cfg(not(target_arch = "wasm32"))]
    check_requested: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            updater: Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")),
            open: false,
            #[cfg(not(target_arch = "wasm32"))]
            startup_check: false,
            #[cfg(not(target_arch = "wasm32"))]
            check_requested: false,
        }
    }
}

impl PdfCraftApp {
    /// Whether the app asks for a newer release on its first frame (off until the desktop app turns it on).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_startup_update_check(&mut self, on: bool) {
        self.updates.startup_check = on;
    }

    /// Replaces the network with `source` (tests).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_update_source(&mut self, source: UpdateSource) {
        self.updates.updater = Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")).with_source(source);
    }

    /// Help ▸ Check for updates: ask for the latest release and show the outcome.
    pub fn check_for_updates(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.updates.open = true;
            self.updates.check_requested = true;
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.open_url(RELEASES_PAGE);
        }
    }

    /// Starts the checks and picks up their answers (each frame).
    pub(crate) fn poll_updates(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(ctx) = self.ctx.clone() else { return };
            // Settled on the first frame: a preference turned on later waits for the next start.
            let at_start = std::mem::take(&mut self.updates.startup_check) && self.update_check_at_start;
            let updates = &mut self.updates;
            if std::mem::take(&mut updates.check_requested) {
                updates.updater.check_now(&ctx);
            } else if at_start {
                updates.updater.check_at_start(&ctx);
            }
            updates.updater.tick(&ctx);
        }
    }
}

/// The startup prompt, and the Updates dialog that answers Help ▸ Check for updates.
pub(crate) fn dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The restart helper is scheduled before the app closes, so no update starts over unsaved work.
        let clean = app.first_dirty().is_none();
        match app.updates.updater.show_if(ctx, clean) {
            Action::None => {}
            Action::OpenUrl(url) => app.open_url(&url),
            Action::Close => {
                app.request_quit(ctx);
            }
        }
        // A requested check that found a release is answered by the prompt, not by this dialog.
        if app.updates.open && !app.updates.check_requested && matches!(app.updates.updater.status(), Status::Idle) {
            app.updates.open = false;
        }
        if app.updates.open {
            let dialog = CheckResultDialog::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION"));
            match dialog.show(ctx, app.updates.updater.status()) {
                CheckResultResponse::None => {}
                CheckResultResponse::Close => {
                    app.updates.open = false;
                    app.updates.updater.dismiss_status();
                }
                CheckResultResponse::CheckAgain => app.updates.check_requested = true,
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// A Thai build must not offer to "update" to upstream PdfCraft: both the check and the
    /// releases link name this fork.
    #[test]
    fn updates_come_from_this_forks_releases() {
        assert_eq!(crate::UPDATE_APP.repo, "armtekcomputer-ops/pdfcraft-thai");
        assert_eq!(RELEASES_PAGE, "https://github.com/armtekcomputer-ops/pdfcraft-thai/releases");
        assert_eq!(crate::UPDATE_APP.releases_url(), RELEASES_PAGE);
    }

    /// The published Linux asset is matched by `craft_update`'s own parser: its stem is the app
    /// id, `pdfcraft`, and the platform token is Linux x86_64. The updater would silently find
    /// nothing if the release used a `pdfcraft-thai-` prefix, so the asset names are pinned here.
    #[test]
    fn the_published_linux_asset_is_accepted_under_the_app_id() {
        // The app id is the asset stem the updater strips before reading the version: renaming
        // it to "pdfcraft-thai" would make `pdfcraft-<v>-linux-x86_64.tar.gz` match nothing (and
        // force renaming every published asset in the same release).
        assert_eq!(crate::UPDATE_APP.id, "pdfcraft");

        let name = "pdfcraft-0.18.1-linux-x86_64.tar.gz";
        let parsed = craft_update::platform::parse_asset_name(name, &[crate::UPDATE_APP.id]).expect("the published asset matches the app id");
        assert_eq!(parsed.stem, "pdfcraft");
        assert_eq!(parsed.version, "0.18.1");
        assert_eq!(parsed.token, "linux-x86_64");
        assert_eq!(parsed.kind, craft_update::PackageKind::TarGz);

        // The updater's own selection path (`pick_asset`) finds that asset for the kind the
        // install was made as. Production passes the detected install's kind (the tarball, an
        // AppImage, a .deb, …), never every kind Linux offers.
        use craft_update::release::{Asset, Release};
        use craft_update::{Arch, Os, PackageKind, Platform};
        let platform = Platform { os: Os::Linux, arch: Arch::X64 };
        let asset = |name: &str| Asset { name: name.into(), size: 1, url: format!("https://example.invalid/{name}"), sha256: None };
        let appimage = "pdfcraft-0.18.1-linux-x86_64.AppImage";
        let release = Release { assets: vec![asset(name), asset(appimage)], ..Release::default() };
        let (picked, kind) =
            craft_update::pick_asset(&release, &[crate::UPDATE_APP.id], platform, &[PackageKind::TarGz]).expect("the published tarball is found");
        assert_eq!(picked.name, name);
        assert_eq!(kind, PackageKind::TarGz, "a Linux x86_64 tarball updates a tarball install");
        // The AppImage is published beside the tarball and is picked for an AppImage install.
        let (picked, kind) =
            craft_update::pick_asset(&release, &[crate::UPDATE_APP.id], platform, &[PackageKind::AppImage]).expect("the published AppImage is found");
        assert_eq!(picked.name, appimage);
        assert_eq!(kind, PackageKind::AppImage, "the AppImage name parses and is picked for an AppImage install");

        // The same file under the fork's display name is not an asset the updater can see.
        assert!(
            craft_update::platform::parse_asset_name("pdfcraft-thai-0.18.1-linux-x86_64.tar.gz", &[crate::UPDATE_APP.id]).is_none(),
            "a pdfcraft-thai prefix must not be used for release assets"
        );
    }
}
