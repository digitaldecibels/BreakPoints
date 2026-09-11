//! Point the scanner at a real project and print what it found.
//!
//! This is how a detector gets improved: run it against a project that came up
//! empty, read the log, add the case.
//!
//!     cargo run --example scan -- /path/to/project [--log]

use breakpoints_lib::generate;
use breakpoints_lib::model::HeightStrategy;
use breakpoints_lib::scanner;

fn main() {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run --example scan -- <project folder> [--log]");
        std::process::exit(2);
    };
    let root = std::path::PathBuf::from(root);
    let verbose = std::env::args().any(|a| a == "--log");

    let outcome = scanner::scan(&root, |row| {
        println!(
            "  {} {:<46} {}",
            match row.status.as_str() {
                "ok" => "✓",
                "warn" => "⚠",
                _ => "✕",
            },
            row.label,
            row.source.unwrap_or_default()
        );
    });

    let report = outcome.report;
    println!(
        "\n{} — {}ms, {} files indexed, {} skipped, {} stylesheet{}",
        report.project_name,
        report.duration_ms,
        report.scanned_files,
        report.skipped_files,
        report.css_files,
        if report.css_files == 1 { "" } else { "s" }
    );
    if report.truncated {
        println!("  (the walk hit a budget and stopped early)");
    }

    println!("\nbreakpoints");
    for found in &report.breakpoints {
        println!(
            "  {:>6}  {:<12} {:<12} {:<40} {}",
            found.width as i64,
            found.name.clone().unwrap_or_default(),
            format!("{:?}", found.kind).to_lowercase(),
            found.source,
            found
                .line
                .map(|l| format!("{}:{}", found.source_file, l))
                .unwrap_or_else(|| found.source_file.clone()),
        );
    }

    if !report.dev_servers.is_empty() {
        println!("\ndev servers");
        for server in &report.dev_servers {
            println!(
                "  {:<42} {:<30} {}",
                server.url, server.source, server.confidence
            );
        }
    }

    if !report.conflicts.is_empty() {
        println!("\nconflicts");
        for conflict in &report.conflicts {
            println!(
                "  {}: {} says {} ({}), {} says {} ({})",
                conflict.name,
                conflict.left.label,
                conflict.left.width,
                conflict.left.source_file,
                conflict.right.label,
                conflict.right.width,
                conflict.right.source_file,
            );
        }
    }

    if !report.warnings.is_empty() {
        println!("\nwarnings");
        for warning in &report.warnings {
            println!("  ⚠ {} ({})", warning.message, warning.file);
            for line in &warning.detail {
                println!("      {line}");
            }
        }
    }

    let recommendation =
        generate::recommend(&report.breakpoints, HeightStrategy::DeviceRatio, 900.0, false);
    println!("\nrecommended testing environment");
    for candidate in &recommendation.recommended {
        println!(
            "  {:<20} {:>5} × {:<5}  {}",
            candidate.viewport.name,
            candidate.viewport.width as i64,
            candidate.viewport.height as i64,
            candidate.detail
        );
    }
    if !recommendation.also_found.is_empty() {
        println!("\nalso found in your CSS");
        for candidate in &recommendation.also_found {
            println!(
                "  {:>5} px              {}",
                candidate.viewport.width as i64, candidate.detail
            );
        }
    }

    if verbose {
        println!("\nlog");
        for entry in outcome.log.entries() {
            println!("  {}", scanner::log::render_line(entry));
        }
    }
}
