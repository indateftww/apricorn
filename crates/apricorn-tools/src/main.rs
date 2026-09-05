//! ROM and asset pipeline CLI.
//!
//! Phase 1 will grow subcommands here (`extract`, `verify`, `convert`).
//! This stub only validates that the crate runs and points at the plan.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("verify") if args.len() == 2 => {
            println!(
                "verify: not implemented yet — the NDS container parser is PLAN.md Phase 1.\
                 \nRun it against {} for now.",
                args[1]
            );
            ExitCode::from(2)
        }
        _ => {
            println!("apricorn-tools — ROM and asset pipeline");
            println!("usage: apricorn-tools verify <rom>");
            println!("subcommands arrive in PLAN.md Phase 1.");
            ExitCode::from(64)
        }
    }
}