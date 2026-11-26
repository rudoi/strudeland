//! Bounded, read-only inspection of the editor snapshot for the piano roll.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use strudeland_host::{Host, InspectionRequest, InspectionResult};
use tauri::State;
use tokio::sync::Mutex;

use crate::command_error::CommandError;
use crate::project::ProjectState;

const INSPECTION_TIMEOUT: Duration = Duration::from_secs(2);

/// One admitted inspector per window. The frontend coalesces pending requests, while the
/// command also rejects concurrent callers before submitting any work to the host.
#[derive(Default)]
pub struct PianoRollState(Arc<Mutex<Option<Host>>>);

#[derive(Deserialize)]
pub struct PianoRollRequest {
    root: String,
    code: String,
    #[serde(flatten)]
    inspection: InspectionRequest,
}

#[tauri::command]
pub async fn piano_roll_inspect(
    state: State<'_, PianoRollState>,
    projects: State<'_, ProjectState>,
    request: PianoRollRequest,
) -> Result<InspectionResult, CommandError> {
    let PianoRollRequest {
        root,
        code,
        inspection,
    } = request;
    let project = projects.current_for(&root)?;
    let mut host = Arc::clone(&state.0)
        .try_lock_owned()
        .map_err(|_| CommandError::busy("A piano roll preview is already running."))?;
    let inspected_project = Arc::clone(&project);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let pins = inspected_project
            .instrument_snapshot()
            .map_err(CommandError::other)?;
        let host = match &mut *host {
            Some(host) => host,
            slot => slot.insert(Host::new().map_err(CommandError::other)?),
        };
        host.inspect(&code, &inspection, &pins, INSPECTION_TIMEOUT)
            .map_err(CommandError::other)
    })
    .await
    .map_err(CommandError::other)??;
    if !Arc::ptr_eq(&project, &projects.current_for(&root)?) {
        return Err(CommandError::project_changed());
    }
    Ok(result)
}
