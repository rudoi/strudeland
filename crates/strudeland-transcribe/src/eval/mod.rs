//! Test support: a seeded synthetic melody generator, a note-level scorer and the evaluation set
//! built from them. Pure code, no extra dependency, so integration tests, the example and other
//! crates' tests can use it.

pub mod cases;
pub mod score;
pub mod synth;
