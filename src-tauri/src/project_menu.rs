#[cfg(target_os = "macos")]
use tauri::menu::PredefinedMenuItem;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::{AppHandle, Emitter, Wry};

const NEW_PROJECT: &str = "newProject";
const OPEN_PROJECT: &str = "openProject";
const PROJECT_MENU_EVENT: &str = "project://menu";

pub fn build(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    let new = MenuItem::with_id(app, NEW_PROJECT, "New Project…", true, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN_PROJECT, "Open Project…", true, None::<&str>)?;

    #[cfg(target_os = "macos")]
    let menu = Menu::default(app)?;
    #[cfg(not(target_os = "macos"))]
    let menu = Menu::new(app)?;

    #[cfg(target_os = "macos")]
    for item in menu.items()? {
        if let Some(file) = item.as_submenu()
            && file.text()? == "File"
        {
            let separator = PredefinedMenuItem::separator(app)?;
            file.insert_items(&[&new, &open, &separator], 0)?;
            return Ok(menu);
        }
    }

    // Windows and Linux previously had no app menu. In particular, the predefined Windows
    // Exit posts a quit message directly, bypassing the webview's unsaved-changes prompt.
    let file = Submenu::with_items(app, "File", true, &[&new, &open])?;
    #[cfg(target_os = "macos")]
    menu.insert(&file, 1)?;
    #[cfg(not(target_os = "macos"))]
    menu.append(&file)?;
    Ok(menu)
}

pub fn dispatch(app: &AppHandle<Wry>, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    if matches!(id, NEW_PROJECT | OPEN_PROJECT)
        && let Err(error) = app.emit_to("main", PROJECT_MENU_EVENT, id)
    {
        eprintln!("could not send project menu action: {error}");
    }
}
