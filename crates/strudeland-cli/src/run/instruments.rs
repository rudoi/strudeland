//! The instrument operations shared by CLI commands and MCP tools.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::json;
use strudeland_ctl::{InstrumentApplyParams, InstrumentApplyResult, Method};
use strudeland_project::{
    INSTRUMENTS_DIR, InstrumentEntry, InstrumentLibrary, Project, ProjectError,
};

use super::{
    Env, Failure, InstrumentCommand, Transport, call, emit, offline_project, params,
    read_utf8_file, stdout_only,
};

#[derive(Serialize)]
pub(super) struct Listing {
    directory: String,
    entries: Vec<InstrumentEntry>,
}

impl Listing {
    fn emit(&self, env: &mut Env, json: bool) -> i32 {
        let mut human = format!("{}\n", self.directory);
        for entry in &self.entries {
            writeln!(human, "{}  {}", entry.id, entry.fingerprint).expect("write to string");
        }
        emit(env, json, &json!(self), stdout_only(human))
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Apply {
    pub id: String,
    /// Project directory. Defaults to the enclosing project or the project open in the app.
    pub project: Option<PathBuf>,
    /// Assert the active song's existing project-relative path, e.g. songs/main.js.
    pub song: Option<String>,
    /// Zero-based UTF-16 start offset of the selected pattern expression.
    pub from: u32,
    /// Exclusive UTF-16 end offset.
    pub to: u32,
    /// Entire expected editor buffer including all unsaved edits and line endings.
    pub expect: String,
}

pub(super) struct Instruments<'a> {
    pub transport: &'a mut dyn Transport,
    pub cwd: &'a dyn Fn() -> Result<PathBuf, Failure>,
    pub library: &'a dyn Fn() -> Result<InstrumentLibrary, ProjectError>,
}

impl Instruments<'_> {
    fn open(&mut self, project: Option<&Path>) -> Result<Project, Failure> {
        offline_project(
            project,
            self.transport,
            self.cwd,
            Project::discover_from_dir,
        )
    }

    pub fn library_entries(&self) -> Result<Listing, Failure> {
        let library = (self.library)()?;
        Ok(Listing {
            directory: library.path().to_string_lossy().into_owned(),
            entries: library.entries()?,
        })
    }

    pub fn project_entries(&mut self, project: Option<&Path>) -> Result<Listing, Failure> {
        let project = self.open(project)?;
        Ok(Listing {
            directory: project
                .root()
                .join(INSTRUMENTS_DIR)
                .to_string_lossy()
                .into_owned(),
            entries: project.instrument_entries()?,
        })
    }

    pub fn create(&self, id: &str, definition: &str) -> Result<InstrumentEntry, Failure> {
        Ok((self.library)()?.create(id, definition)?)
    }

    pub fn pin(&mut self, id: &str, project: Option<&Path>) -> Result<InstrumentEntry, Failure> {
        Ok(self.open(project)?.pin_instrument(&(self.library)()?, id)?)
    }

    pub fn update(
        &mut self,
        id: &str,
        project: Option<&Path>,
        expect: &str,
    ) -> Result<InstrumentEntry, Failure> {
        Ok(self
            .open(project)?
            .update_instrument(&(self.library)()?, id, expect)?)
    }

    pub fn apply(&mut self, request: Apply) -> Result<InstrumentApplyResult, Failure> {
        let project = self.open(request.project.as_deref())?;
        let args = InstrumentApplyParams {
            root: project.root().to_string_lossy().into_owned(),
            id: request.id,
            song: request.song,
            from: request.from,
            to: request.to,
            expect: request.expect,
        };
        let (_, result) = call(self.transport, Method::InstrumentApply, params(&args))?;
        Ok(result)
    }
}

pub(super) fn run_instrument(
    command: &InstrumentCommand,
    json: bool,
    transport: &mut dyn Transport,
    env: &mut Env,
    cwd: &dyn Fn() -> Result<PathBuf, Failure>,
    library: impl Fn() -> Result<InstrumentLibrary, ProjectError>,
) -> Result<i32, Failure> {
    let mut instruments = Instruments {
        transport,
        cwd,
        library: &library,
    };
    let entry = match command {
        InstrumentCommand::Library => return Ok(instruments.library_entries()?.emit(env, json)),
        InstrumentCommand::List { project } => {
            return Ok(instruments
                .project_entries(project.as_deref())?
                .emit(env, json));
        }
        InstrumentCommand::Create { id, file } => {
            instruments.create(id, &read_utf8_file(file, "definition file")?)?
        }
        InstrumentCommand::Pin { id, project } => instruments.pin(id, project.as_deref())?,
        InstrumentCommand::Update {
            id,
            project,
            expect,
        } => instruments.update(id, project.as_deref(), expect)?,
        InstrumentCommand::Apply {
            id,
            project,
            song,
            from,
            to,
            expect_file,
        } => {
            let result = instruments.apply(Apply {
                id: id.clone(),
                project: project.clone(),
                song: song.clone(),
                from: *from,
                to: *to,
                expect: read_utf8_file(expect_file, "expected buffer")?,
            })?;
            let human = format!(
                "applied {id} to {} [{}, {})\n",
                result.song, result.from, result.to
            );
            return Ok(emit(env, json, &json!(result), stdout_only(human)));
        }
    };
    Ok(emit(
        env,
        json,
        &json!(entry),
        stdout_only(format!("{}  {}\n", entry.id, entry.fingerprint)),
    ))
}
