//! Prints the evaluation table: every tier of the synthetic set through `transcribe`, scored by
//! COnP (onset within 50 ms, pitch within 50 cents, one to one), micro-averaged per tier.
//!
//! ```text
//! cargo run --release -p strudeland-transcribe --features eval --example eval
//! ```

use std::process::ExitCode;

use strudeland_transcribe::eval::cases::{KeyOutcome, TIERS, Tier, TierResult, run_tier};

fn main() -> ExitCode {
    let mut results: Vec<TierResult> = Vec::new();
    for (tier, _) in TIERS {
        match run_tier(tier) {
            Ok(r) => results.push(r),
            Err(e) => {
                eprintln!("error: {}: {e}", tier.name());
                return ExitCode::FAILURE;
            }
        }
    }

    println!(
        "| tier | cases | precision | recall | F1 | exact notation | key exact | key related | key none |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for r in &results {
        let n = r.cases.len();
        println!(
            "| {} | {n} | {:.3} | {:.3} | {:.3} | {}/{n} | {}/{n} | {}/{n} | {}/{n} |",
            r.tier.name(),
            r.score.precision,
            r.score.recall,
            r.score.f1,
            r.exact_count(),
            r.key_count(KeyOutcome::Exact),
            r.key_count(KeyOutcome::Related),
            r.key_count(KeyOutcome::None),
        );
    }

    println!();
    println!(
        "| tier | seed | tp | fp | fn | F1 | exact | key (truth) | key (detected) | tempo error | phase error |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for r in &results {
        for c in &r.cases {
            let (tempo, phase) = if r.tier == Tier::NoGrid {
                (
                    c.tempo_error
                        .map_or("none".to_string(), |e| format!("{:+.2}%", e * 100.0)),
                    c.phase_error
                        .map_or("none".to_string(), |e| format!("{:+.1} ms", e * 1000.0)),
                )
            } else {
                ("-".to_string(), "-".to_string())
            };
            println!(
                "| {} | {} | {} | {} | {} | {:.3} | {} | {} | {} ({}) | {tempo} | {phase} |",
                r.tier.name(),
                c.seed,
                c.score.tp,
                c.score.fp,
                c.score.fn_,
                c.score.f1,
                if c.exact { "yes" } else { "no" },
                c.truth_key,
                c.key_name.as_deref().unwrap_or("none"),
                c.key.label(),
            );
        }
    }
    ExitCode::SUCCESS
}
