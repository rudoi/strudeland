//! The committed host bundle (`js/host.js`, written by `bun run host:build` only).

/// The bundle's source: an IIFE that sets `globalThis.strudelandHost`.
pub const BUNDLE: &str = include_str!("../js/host.js");

#[cfg(test)]
mod tests {
    #[test]
    fn bundle_defines_the_entry() {
        assert!(super::BUNDLE.contains("strudelandHost"));
        assert!(super::BUNDLE.len() > 100_000);
    }
}
