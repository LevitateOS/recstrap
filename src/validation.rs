//! Cheat-guarded validation macro for recstrap.
//!
//! Based on Anthropic's emergent misalignment research, this macro documents
//! cheat vectors for each validation check, making it harder to weaken checks
//! without understanding the consequences.

/// Validate a condition with cheat-aware documentation.
///
/// When the condition fails, prints detailed cheat documentation to stderr
/// and returns the specified error. This ensures:
/// 1. Users see clear error messages
/// 2. Developers see cheat vectors when debugging
/// 3. Future maintainers (including AI) see the consequences of weakening checks
///
/// Based on Anthropic's emergent misalignment research.
#[macro_export]
macro_rules! guarded_ensure {
    (
        $cond:expr,
        $err:expr,
        protects = $protects:expr,
        severity = $severity:expr,
        cheats = [$($cheat:expr),+ $(,)?],
        consequence = $consequence:expr
    ) => {{
        if !($cond) {
            let cheats_list: &[&str] = &[$($cheat),+];
            let cheats_formatted: String = cheats_list
                .iter()
                .enumerate()
                .map(|(i, c)| format!("  {}. {}", i + 1, c))
                .collect::<Vec<_>>()
                .join("\n");

            eprintln!();
            eprintln!("{}", "=".repeat(70));
            eprintln!("=== CHEAT-GUARDED VALIDATION FAILED ===");
            eprintln!("{}", "=".repeat(70));
            eprintln!();
            eprintln!("PROTECTS: {}", $protects);
            eprintln!("SEVERITY: {}", $severity);
            eprintln!();
            eprintln!("CHEAT VECTORS (ways this check could be weakened):");
            eprintln!("{}", cheats_formatted);
            eprintln!();
            eprintln!("USER CONSEQUENCE IF CHEATED:");
            eprintln!("  {}", $consequence);
            eprintln!();
            eprintln!("{}", "=".repeat(70));
            eprintln!();

            return Err($err);
        }
    }};
}
