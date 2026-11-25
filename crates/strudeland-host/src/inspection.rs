use serde::{Deserialize, Serialize};
use strudeland_project::{InstrumentSnapshot, validate_instrument_snapshot};

use crate::HostError;

const MAX_CODE_BYTES: usize = 256 * 1024;
const MAX_BAR: u32 = 1_000_000;
const MAX_WINDOW_BARS: u32 = 16;
pub(crate) const MAX_RESULT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InspectionView {
    Expression,
    Song,
}

/// Half-open range in the original UTF-16 editor document.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SourceSpan {
    pub from: u32,
    pub to: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionRequest {
    pub view: InspectionView,
    /// Only Expression executes the capture hook.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<SourceSpan>,
    pub from: u32,
    pub to: u32,
    /// Written bar corresponding to Expression's local cycle zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_bar: Option<u32>,
}

impl InspectionRequest {
    pub(crate) fn json(
        &self,
        code: &str,
        instruments: &InstrumentSnapshot,
    ) -> Result<String, HostError> {
        if code.len() > MAX_CODE_BYTES {
            return Err(HostError::Plan("The song is too large to inspect.".into()));
        }
        if self.from >= self.to
            || self.to > MAX_BAR
            || self.to - self.from > MAX_WINDOW_BARS
            || self.anchor_bar.is_some_and(|bar| bar > MAX_BAR)
        {
            return Err(HostError::Plan("Invalid piano roll window.".into()));
        }
        if matches!(self.view, InspectionView::Expression)
            && !self.expression.is_some_and(|span| {
                span.from < span.to && span.to as usize <= code.encode_utf16().count()
            })
        {
            return Err(HostError::Plan("Select a pattern expression.".into()));
        }
        validate_instrument_snapshot(instruments)
            .map_err(|e| HostError::Plan(format!("instrument pins: {e}")))?;
        #[derive(Serialize)]
        struct WithInstruments<'a> {
            #[serde(flatten)]
            request: &'a InspectionRequest,
            instruments: &'a InstrumentSnapshot,
        }
        serde_json::to_string(&WithInstruments {
            request: self,
            instruments,
        })
        .map_err(|e| HostError::Plan(format!("invalid inspection request: {e}")))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedNote {
    pub id: String,
    pub pitch: f64,
    pub start: String,
    pub end: String,
    pub visible_start: String,
    pub visible_end: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contributors: Vec<SourceSpan>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct InspectionResult {
    pub notes: Vec<InspectedNote>,
    pub truncated: bool,
}
