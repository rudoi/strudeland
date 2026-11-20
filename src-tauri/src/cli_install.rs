use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use rustix::fs::{StatVfsMountFlags, statvfs};
use tauri::menu::MenuItem;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const MENU_ID: &str = "install-command-line-tool";
const TITLE: &str = "install command line tool";

fn packaged_cli(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let bundle = macos.parent()?.parent()?;
    (macos.ends_with("Contents/MacOS") && bundle.extension()? == "app")
        .then(|| macos.join("strudeland-cli"))
}

pub fn register(app: &AppHandle) -> tauri::Result<()> {
    if tauri::is_dev() {
        return Ok(());
    }
    let Some(source) = tauri::utils::platform::current_exe()
        .ok()
        .and_then(|exe| packaged_cli(&exe))
        .filter(|path| path.is_file())
    else {
        return Ok(());
    };
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    // Tauri's default macOS menu starts with the application submenu and its About item.
    let items = menu.items()?;
    if let Some(submenu) = items.first().and_then(|item| item.as_submenu()) {
        let action = MenuItem::with_id(
            app,
            MENU_ID,
            "install command line tool…",
            true,
            None::<&str>,
        )?;
        submenu.insert(&action, 1)?;
        app.on_menu_event(move |app, event| {
            if event.id() == MENU_ID {
                confirm(app, &source);
            }
        });
    }
    Ok(())
}

fn confirm(app: &AppHandle, source: &Path) {
    let destination = match app.path().home_dir() {
        Ok(home) => home.join(".local/bin/strudeland"),
        Err(error) => {
            report_error(
                app,
                format!("could not locate ~/.local/bin/strudeland. {error}"),
            );
            return;
        }
    };
    let source = source.to_owned();
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "create a link at {} to the command line tool in this copy of strudeland?\n\n\
            if you plan to move the app to Applications, open it from there first.",
            destination.display()
        ))
        .title(TITLE)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "install".into(),
            "cancel".into(),
        ))
        .show(move |confirmed| {
            if !confirmed {
                return;
            }
            if let Err(error) = install(&source, &destination) {
                report_error(&handle, error);
                return;
            }
            handle
                .dialog()
                .message(format!(
                    "the command line tool is installed at {}.\n\n\
                    in zsh or bash, run this command to add its directory to PATH.\n\n\
                    export PATH=\"$HOME/.local/bin:$PATH\"\n\n\
                    to keep this setting, add the same line to your shell startup file.",
                    destination.display()
                ))
                .title(TITLE)
                .show(|_| {});
        });
}

fn report_error(app: &AppHandle, message: String) {
    app.dialog()
        .message(message)
        .title(TITLE)
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

fn check_location(source: &Path, flags: StatVfsMountFlags) -> Result<(), String> {
    // Finder can mount the read-only installer at a custom mountpoint.
    if flags.contains(StatVfsMountFlags::RDONLY)
        || source.iter().any(|part| part == "AppTranslocation")
    {
        return Err(format!(
            "{} is in a temporary or read-only location. copy strudeland to Applications, open that copy, and install the tool again.",
            source.display()
        ));
    }
    Ok(())
}

fn install(source: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::symlink_metadata(source).map_err(|e| format!("{}: {e}", source.display()))?;
    if !file.is_file() || file.permissions().mode() & 0o111 == 0 {
        return Err(format!(
            "{} must be an executable file in the app bundle.",
            source.display()
        ));
    }
    let mount = statvfs(source).map_err(|e| format!("{}: {e}", source.display()))?;
    check_location(source, mount.f_flag)?;

    let bin = destination
        .parent()
        .ok_or_else(|| format!("{} has no parent directory.", destination.display()))?;
    fs::create_dir_all(bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    match symlink(source, destination) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if fs::read_link(destination).is_ok_and(|target| {
                bin.join(target)
                    .canonicalize()
                    .is_ok_and(|path| path == source)
            }) {
                Ok(())
            } else {
                Err(format!(
                    "{} already exists. move or remove it yourself if you want to install this copy.",
                    destination.display()
                ))
            }
        }
        Err(e) => Err(format!("{}: {e}", destination.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_a_cli_inside_an_application_bundle() {
        let exe = Path::new("/Applications/music app.app/Contents/MacOS/strudeland");
        assert_eq!(
            packaged_cli(exe).unwrap(),
            exe.with_file_name("strudeland-cli")
        );
        for exe in [
            "/target/debug/strudeland",
            "/target/Contents/MacOS/strudeland",
            "/app.app/MacOS/strudeland",
        ] {
            assert!(packaged_cli(Path::new(exe)).is_none());
        }
    }

    #[test]
    fn rejects_translocation_and_read_only_mounts() {
        let source = Path::new(
            "/private/var/folders/xx/AppTranslocation/uuid/d/app.app/Contents/MacOS/strudeland-cli",
        );
        assert!(
            check_location(source, StatVfsMountFlags::empty())
                .unwrap_err()
                .contains("Applications")
        );
        for source in ["/Volumes/strudeland/app.app", "/tmp/custom mount/app.app"] {
            assert!(check_location(Path::new(source), StatVfsMountFlags::RDONLY).is_err());
        }
    }

    fn bundled_cli(dir: &Path) -> PathBuf {
        let source = dir
            .canonicalize()
            .unwrap()
            .join("strudeland.app/Contents/MacOS/strudeland-cli");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "version one").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        source
    }

    #[test]
    fn creates_the_link_once_and_tracks_app_replacement() {
        let tmp = tempfile::tempdir().unwrap();
        let source = bundled_cli(tmp.path());
        let link = tmp.path().join(".local/bin/strudeland");
        install(&source, &link).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), source);
        install(&source, &link).unwrap();
        fs::rename(&source, source.with_extension("old")).unwrap();
        fs::write(&source, "version two").unwrap();
        assert_eq!(fs::read_to_string(link).unwrap(), "version two");
    }

    #[test]
    fn conflicts_leave_every_other_destination_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let source = bundled_cli(tmp.path());
        let other = tmp.path().join("other");
        fs::write(&other, "other command").unwrap();
        for kind in ["file", "directory", "other link", "dangling link"] {
            let bin = tmp.path().join(kind);
            fs::create_dir(&bin).unwrap();
            let dest = bin.join("strudeland");
            match kind {
                "file" => fs::write(&dest, "user command").unwrap(),
                "directory" => fs::create_dir(&dest).unwrap(),
                "other link" => symlink(&other, &dest).unwrap(),
                _ => symlink("missing", &dest).unwrap(),
            }
            let error = install(&source, &dest).unwrap_err();
            assert!(error.contains(dest.to_string_lossy().as_ref()), "{error}");
            match kind {
                "file" => assert_eq!(fs::read_to_string(dest).unwrap(), "user command"),
                "directory" => assert!(dest.is_dir()),
                "other link" => assert_eq!(fs::read_link(dest).unwrap(), other),
                _ => assert_eq!(fs::read_link(dest).unwrap(), Path::new("missing")),
            }
        }
    }

    #[test]
    fn reports_invalid_sources_and_an_unwritable_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let source = bundled_cli(tmp.path());
        let bin = tmp.path().join(".local/bin");
        let dest = bin.join("strudeland");
        fs::write(tmp.path().join(".local"), "occupied").unwrap();
        assert!(
            install(&source, &dest)
                .unwrap_err()
                .contains(bin.to_string_lossy().as_ref())
        );
        fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(install(&source, &dest).unwrap_err().contains("executable"));
        fs::remove_file(&source).unwrap();
        assert!(
            install(&source, &dest)
                .unwrap_err()
                .contains(source.to_string_lossy().as_ref())
        );
        assert!(!bin.exists());
    }
}
