use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Wry};

const NEW_PROJECT: &str = "newProject";
const EXPORT_AUDIO: &str = "exportAudio";
const OPEN_PROJECT: &str = "openProject";
const SETTINGS: &str = "settings";
const PROJECT_MENU_EVENT: &str = "project://menu";

pub fn build(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    let new = MenuItem::with_id(app, NEW_PROJECT, "New Project…", true, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN_PROJECT, "Open Project…", true, None::<&str>)?;

    let export = MenuItem::with_id(app, EXPORT_AUDIO, "Export Audio…", false, None::<&str>)?;
    let settings = MenuItem::with_id(
        app,
        SETTINGS,
        "Settings…",
        true,
        cfg!(target_os = "macos").then_some("Cmd+,"),
    )?;

    #[cfg(target_os = "macos")]
    {
        let menu = Menu::default(app)?;
        let items = menu.items()?;
        // The first submenu is the application menu. Its default items start with About and a separator.
        if let Some(application) = items.first().and_then(|item| item.as_submenu()) {
            application.insert_items(&[&settings, &PredefinedMenuItem::separator(app)?], 2)?;
        }
        for item in &items {
            if let Some(file) = item.as_submenu()
                && file.text()? == "File"
            {
                let separator = PredefinedMenuItem::separator(app)?;
                file.insert_items(&[&new, &open, &export, &separator], 0)?;
                return Ok(menu);
            }
        }
        let file = Submenu::with_items(app, "File", true, &[&new, &open, &export])?;
        menu.insert(&file, 1)?;
        Ok(menu)
    }

    // Windows and Linux previously had no app menu. In particular, the predefined Windows
    // Exit posts a quit message directly, bypassing the webview's unsaved-changes prompt.
    #[cfg(not(target_os = "macos"))]
    {
        let separator = PredefinedMenuItem::separator(app)?;
        let file = Submenu::with_items(
            app,
            "File",
            true,
            &[&new, &open, &export, &separator, &settings],
        )?;
        Menu::with_items(app, &[&file])
    }
}

pub fn dispatch(app: &AppHandle<Wry>, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    if matches!(id, NEW_PROJECT | OPEN_PROJECT | EXPORT_AUDIO | SETTINGS)
        && let Err(error) = app.emit_to("main", PROJECT_MENU_EVENT, id)
    {
        eprintln!("could not send project menu action: {error}");
    }
}

#[tauri::command]
pub fn export_audio_set_available(app: AppHandle, available: bool) -> Result<(), String> {
    let menu = app.menu().ok_or("no application menu")?;
    let items = menu.items().map_err(|error| error.to_string())?;
    for submenu in items.iter().filter_map(|item| item.as_submenu()) {
        if let Some(export) = submenu.get(EXPORT_AUDIO)
            && let Some(item) = export.as_menuitem()
        {
            return item
                .set_enabled(available)
                .map_err(|error| error.to_string());
        }
    }
    Err("the export menu item is unavailable".into())
}
