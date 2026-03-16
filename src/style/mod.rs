/// Consistent styling, colors, and symbols for pylot's terminal output.

// ── Symbols ──────────────────────────────────────────────
pub const CHECK: &str = "\x1b[32m✓\x1b[0m";
pub const CROSS: &str = "\x1b[31m✗\x1b[0m";
pub const ARROW: &str = "\x1b[36m▸\x1b[0m";
pub const DOT: &str = "\x1b[90m●\x1b[0m";
pub const WARN: &str = "\x1b[33m!\x1b[0m";
pub const DASH: &str = "\x1b[90m─\x1b[0m";

// ── ANSI helpers ─────────────────────────────────────────
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const ITALIC: &str = "\x1b[3m";
pub const RESET: &str = "\x1b[0m";
pub const CYAN: &str = "\x1b[36m";
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const YELLOW: &str = "\x1b[33m";
pub const MAGENTA: &str = "\x1b[35m";
pub const WHITE: &str = "\x1b[97m";
pub const GRAY: &str = "\x1b[90m";
pub const BG_GRAY: &str = "\x1b[48;5;236m";

// ── Gradient banner colors (blue → cyan → white) ────────
const GRADIENT: &[&str] = &[
    "\x1b[38;5;27m",  // deep blue
    "\x1b[38;5;33m",  // blue
    "\x1b[38;5;39m",  // light blue
    "\x1b[38;5;44m",  // cyan-blue
    "\x1b[38;5;50m",  // cyan
    "\x1b[38;5;87m",  // light cyan
];

pub fn banner() {
    let lines = [
        r"             __      __  ",
        r"    ____    / /_    / /_ ",
        r"   / __ \  / / /   / __ \",
        r"  / /_/ / / / /   / /_/ /",
        r" / .___/ /_/ /   /_.___/ ",
        r"/_/     /_/_/    /_ /    ",
    ];

    eprintln!();
    for (i, line) in lines.iter().enumerate() {
        let color = GRADIENT[i % GRADIENT.len()];
        eprintln!("  {}{}{}", color, line, RESET);
    }
    eprintln!();
    eprintln!(
        "  {}{}pylot{} {}v{}{}",
        BOLD, WHITE, RESET,
        DIM, env!("CARGO_PKG_VERSION"), RESET
    );
    eprintln!("  {}Project context switcher{}", DIM, RESET);
    eprintln!();
}

pub fn divider() {
    eprintln!("  {}{}{}", DIM, "─".repeat(48), RESET);
}

pub fn heading(text: &str) {
    eprintln!("  {}{}{}{}", BOLD, WHITE, text, RESET);
}

pub fn item(label: &str, value: &str) {
    eprintln!(
        "  {}  {}{:<12}{} {}",
        DOT, DIM, label, RESET, value
    );
}

pub fn item_colored(label: &str, value: &str, color: &str) {
    eprintln!(
        "  {}  {}{:<12}{} {}{}{}",
        DOT, DIM, label, RESET, color, value, RESET
    );
}

pub fn success(msg: &str) {
    eprintln!("  {} {}", CHECK, msg);
}

pub fn warn(msg: &str) {
    eprintln!("  {} {}{}{}",  WARN, YELLOW, msg, RESET);
}

pub fn error(msg: &str) {
    eprintln!("  {} {}{}{}",  CROSS, RED, msg, RESET);
}

pub fn hint(msg: &str) {
    eprintln!("  {}  {}{}", GRAY, msg, RESET);
}

pub fn blank() {
    eprintln!();
}

/// Print a section with a header and items
pub fn section(title: &str) {
    eprintln!();
    eprintln!("  {}{}  {}{}", CYAN, BOLD, title, RESET);
}

/// Print a table row for list command
pub fn table_row(name: &str, path: &str, branch: &str, services: &str, last: &str) {
    eprintln!(
        "  {}▸{} {}{:<14}{} {}{:<32}{} {}{:<10}{} {}{:<8}{} {}{}{}",
        CYAN, RESET,
        BOLD, name, RESET,
        DIM, truncate(path, 32), RESET,
        MAGENTA, branch, RESET,
        GREEN, services, RESET,
        DIM, last, RESET,
    );
}

pub fn table_header() {
    eprintln!(
        "    {}{:<14} {:<32} {:<10} {:<8} {}{}",
        DIM, "NAME", "PATH", "BRANCH", "SERVICES", "LAST USED", RESET,
    );
    eprintln!("  {}  {}{}", DIM, "─".repeat(82), RESET);
}

pub fn empty_state(msg: &str, hint_msg: &str) {
    eprintln!();
    eprintln!("  {}  {}{}", DIM, msg, RESET);
    eprintln!();
    eprintln!("  {}  {}{}", GRAY, hint_msg, RESET);
    eprintln!();
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

/// Prompt the user for y/n confirmation
pub fn confirm(msg: &str) -> bool {
    eprint!("  {} {} {}[y/N]{} ", WARN, msg, DIM, RESET);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap_or(0);
    input.trim().eq_ignore_ascii_case("y")
}
