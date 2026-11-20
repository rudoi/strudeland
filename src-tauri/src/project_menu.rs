use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Wry};

const NEW_PROJECT: &str = "newProject";
const OPEN_PROJECT: &str = "openProject";
const PROJECT_MENU_EVENT: &str = "project://menu";

pub fn build(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::default(app)?;
    let new = MenuItem::with_id(app, NEW_PROJECT, "New Project…", true, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN_PROJECT, "Open Project…", true, None::<&str>)?;
    for item in menu.items()? {
        if let Some(file) = item.as_submenu()
            && file.text()? == "File"
        {
            let separator = PredefinedMenuItem::separator(app)?;
            file.insert_items(&[&new, &open, &separator], 0)?;
            return Ok(menu);
        }
    }

    // The Tauri default menu on Linux starts with Edit.
    let file = Submenu::with_items(app, "File", true, &[&new, &open])?;
    menu.insert(&file, 0)?;
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
