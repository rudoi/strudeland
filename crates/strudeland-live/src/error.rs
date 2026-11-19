use crate::output::OutputError;

/// Why a [`Player`](crate) could not start or keep running.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("{0}")]
    Output(#[from] OutputError),
    #[error("{0}")]
    Engine(String),
}
