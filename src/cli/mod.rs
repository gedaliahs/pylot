use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};

use crate::context::ProjectContext;
use crate::services::ServiceManager;
use crate::style;

#[derive(Parser)]
#[command(
    name = "pylot",
    about = "Switch between projects without losing your place",
    version,
    after_help = format!(
        "{}{}Quick start:{}\n  \
        pylot save api            Save this project as 'api'\n  \
        pylot switch api          Jump back to 'api' anytime\n  \
        pylot                     Browse all projects interactively\n\n  \
        {}First time?{} Run {}pylot init{} in your project, then {}pylot save <name>{}\n",
        style::BOLD, style::WHITE, style::RESET,
        style::DIM, style::RESET,
        style::CYAN, style::RESET,
        style::CYAN, style::RESET,
    ),
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Save this project so you can come back to it later
    Save {
        /// A short name for this project (e.g. 'api', 'frontend', 'mobile')
        name: Option<String>,
    },
    /// Jump to a saved project (restores branch, env, services)
    Switch {
        /// Project name (supports partial matching)
        name: String,
        /// Skip the uncommitted changes warning
        #[arg(long)]
        force: bool,
    },
    /// Show all your saved projects
    List,
    /// Show what's happening in the current directory
    Status,
    /// Forget a saved project
    Remove {
        /// Project name to forget
        name: String,
    },
    /// Set up a .pylot.toml config for this project
    Init,
    /// Stop running services for a project
    Stop {
        /// Project name (defaults to current directory)
        name: Option<String>,
    },
    /// Check your pylot setup for issues
    Doctor,
    /// Print shell hook (add to your .zshrc/.bashrc)
    ShellInit {
        /// Shell type
        #[arg(default_value = "zsh")]
        shell: String,
    },
    /// Generate shell completions
    Completions {
        /// Shell type
        shell: Shell,
    },
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Commands::Save { name }) => cmd_save(name.as_deref()),
        Some(Commands::Switch { name, force }) => cmd_switch(&name, force),
        Some(Commands::List) => cmd_list(),
        Some(Commands::Status) => cmd_status(),
        Some(Commands::Remove { name }) => cmd_remove(&name),
        Some(Commands::Init) => cmd_init(),
        Some(Commands::Stop { name }) => cmd_stop(name.as_deref()),
        Some(Commands::Doctor) => cmd_doctor(),
        Some(Commands::ShellInit { shell }) => cmd_shell_init(&shell),
        Some(Commands::Completions { shell }) => cmd_completions(shell),
        None => {
            let contexts = ProjectContext::list_all()?;
            if contexts.is_empty() {
                cmd_onboarding()
            } else {
                crate::tui::run_dashboard()
            }
        }
    }
}

// ── Onboarding (first run with no contexts) ─────────────

fn cmd_onboarding() -> Result<()> {
    style::banner();
    style::divider();
    style::blank();
    eprintln!(
        "  {}Welcome!{} pylot saves and restores your dev environment",
        style::BOLD, style::RESET,
    );
    eprintln!(
        "  so you can switch between projects without losing your place.",
    );
    style::blank();
    style::divider();
    style::blank();
    eprintln!(
        "  {}{}Get started in 3 steps:{}",
        style::BOLD, style::WHITE, style::RESET,
    );
    style::blank();
    eprintln!(
        "  {}1.{} Go to a project directory:",
        style::CYAN, style::RESET,
    );
    eprintln!(
        "     {}$ cd ~/Projects/my-api{}",
        style::DIM, style::RESET,
    );
    style::blank();
    eprintln!(
        "  {}2.{} Save it with a short name:",
        style::CYAN, style::RESET,
    );
    eprintln!(
        "     {}$ pylot save api{}",
        style::DIM, style::RESET,
    );
    style::blank();
    eprintln!(
        "  {}3.{} Switch back to it from anywhere:",
        style::CYAN, style::RESET,
    );
    eprintln!(
        "     {}$ pylot switch api{}",
        style::DIM, style::RESET,
    );
    style::blank();
    style::divider();
    style::blank();
    style::hint("Optional: run 'pylot init' first to set up services & ports");
    style::hint("Run 'pylot doctor' to check your setup");
    style::blank();

    Ok(())
}

// ── Save ────────────────────────────────────────────────

fn cmd_save(name: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;

    // Auto-generate name from directory if not provided
    let name = match name {
        Some(n) => n.to_string(),
        None => {
            let dir_name = cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "project".to_string())
                .to_lowercase()
                .replace(' ', "-");
            dir_name
        }
    };

    let ctx = ProjectContext::capture_current(&name)?;
    ctx.save()?;

    style::blank();
    style::success(&format!(
        "Saved as {}{}{}",
        style::BOLD, name, style::RESET
    ));
    style::blank();
    style::item("path", &ctx.path.display().to_string());
    style::item_colored(
        "branch",
        ctx.git_branch.as_deref().unwrap_or("–"),
        style::MAGENTA,
    );
    style::item("env vars", &format!("{}", ctx.env_vars.len()));
    if !ctx.services.is_empty() {
        style::item(
            "services",
            &ctx.services.keys().cloned().collect::<Vec<_>>().join(", "),
        );
    }
    if !ctx.ports_required.is_empty() {
        style::item(
            "ports",
            &ctx.ports_required
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    style::blank();
    style::hint(&format!("Jump back with: pylot switch {}", name));
    style::blank();

    Ok(())
}

// ── Switch ──────────────────────────────────────────────

fn cmd_switch(name: &str, force: bool) -> Result<()> {
    // Fuzzy match: find the best matching context
    let resolved = fuzzy_resolve(name)?;

    // Warn about dirty state
    if !force {
        let cwd = std::env::current_dir()?;
        if ProjectContext::has_dirty_git_state(&cwd) {
            if let Some(summary) = ProjectContext::dirty_summary(&cwd) {
                style::blank();
                style::warn(&format!("You have uncommitted changes: {}", summary));
                if !style::confirm("Switch anyway?") {
                    style::blank();
                    style::hint("Commit or stash first, or use --force");
                    style::blank();
                    return Ok(());
                }
            }
        }
    }

    // Auto-stop services for the current context before switching
    let cwd = std::env::current_dir()?;
    let current_dir_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let service_mgr = ServiceManager::new();
    let current_health = service_mgr.service_health(&current_dir_name);
    if !current_health.is_empty() {
        let running: Vec<_> = current_health.iter().filter(|(_, _, a)| *a).collect();
        if !running.is_empty() {
            eprintln!(
                "  {}Stopping {} service(s) for '{}'...{}",
                style::DIM,
                running.len(),
                current_dir_name,
                style::RESET,
            );
            service_mgr.stop_services(&current_dir_name)?;
        }
    }

    let ctx = ProjectContext::load(&resolved)?;

    // Port conflicts
    let conflicts = service_mgr.check_port_conflicts(&ctx.ports_required);
    if !conflicts.is_empty() {
        style::blank();
        style::warn("Port conflicts:");
        for (port, pid, proc_name) in &conflicts {
            style::item(
                &format!(":{}", port),
                &format!("{} (PID {})", proc_name, pid),
            );
        }
        style::blank();
        if style::confirm("Kill these processes?") {
            for (_, pid, _) in &conflicts {
                service_mgr.kill_process(*pid)?;
            }
        }
    }

    // Start services for the new context
    if !ctx.services.is_empty() {
        style::blank();
        service_mgr.start_services(&ctx)?;
    }

    // Summary
    style::blank();
    style::success(&format!(
        "Now in {}{}{}",
        style::BOLD, resolved, style::RESET,
    ));
    style::blank();
    style::item("path", &ctx.path.display().to_string());
    if let Some(ref branch) = ctx.git_branch {
        style::item_colored("branch", branch, style::MAGENTA);
    }
    if let Some(ref last) = ctx.last_accessed {
        style::item("last used", &last.format("%b %d at %H:%M").to_string());
    }
    style::blank();

    // Shell commands for the wrapper to eval
    ctx.print_shell_commands();

    Ok(())
}

// ── List ────────────────────────────────────────────────

fn cmd_list() -> Result<()> {
    let contexts = ProjectContext::list_all()?;
    if contexts.is_empty() {
        style::empty_state(
            "No projects saved yet.",
            "cd into a project and run: pylot save <name>",
        );
        return Ok(());
    }

    let service_mgr = ServiceManager::new();

    style::blank();
    style::heading(&format!("Your Projects ({})", contexts.len()));
    style::blank();
    style::table_header();

    for ctx in contexts {
        let health = service_mgr.service_health(&ctx.name);
        let svc_status = if health.is_empty() {
            "–".to_string()
        } else {
            let alive = health.iter().filter(|(_, _, a)| *a).count();
            format!("{}/{}", alive, health.len())
        };

        style::table_row(
            &ctx.name,
            &ctx.path.display().to_string(),
            ctx.git_branch.as_deref().unwrap_or("–"),
            &svc_status,
            &ctx.last_accessed
                .map(|t| t.format("%b %d %H:%M").to_string())
                .unwrap_or_else(|| "–".to_string()),
        );
    }
    style::blank();

    Ok(())
}

// ── Status ──────────────────────────────────────────────

fn cmd_status() -> Result<()> {
    let ctx = ProjectContext::detect_current()?;
    let service_mgr = ServiceManager::new();

    style::blank();
    style::heading(&ctx.name);
    style::blank();
    style::item("path", &ctx.path.display().to_string());
    style::item_colored(
        "branch",
        ctx.git_branch.as_deref().unwrap_or("–"),
        style::MAGENTA,
    );
    style::item("env vars", &format!("{}", ctx.env_vars.len()));

    if ProjectContext::has_dirty_git_state(&ctx.path) {
        if let Some(summary) = ProjectContext::dirty_summary(&ctx.path) {
            style::item_colored("git", &format!("dirty ({})", summary), style::YELLOW);
        }
    } else {
        style::item_colored("git", "clean", style::GREEN);
    }

    let health = service_mgr.service_health(&ctx.name);
    if !health.is_empty() {
        style::section("Services");
        for (name, pid, alive) in &health {
            if *alive {
                eprintln!(
                    "  {} {}  {}{}{} {}PID {}{}",
                    style::CHECK, name,
                    style::GREEN, "running", style::RESET,
                    style::DIM, pid, style::RESET,
                );
            } else {
                eprintln!(
                    "  {} {}  {}{}{} {}PID {}{}",
                    style::CROSS, name,
                    style::RED, "stopped", style::RESET,
                    style::DIM, pid, style::RESET,
                );
            }
        }
    }

    let active_ports = service_mgr.get_listening_ports();
    if !active_ports.is_empty() {
        style::section("Ports");
        for (port, proc_name) in &active_ports {
            eprintln!(
                "  {}  {}:{}{} {}{}{}",
                style::DOT,
                style::WHITE, port, style::RESET,
                style::DIM, proc_name, style::RESET,
            );
        }
    }

    style::blank();
    Ok(())
}

// ── Remove ──────────────────────────────────────────────

fn cmd_remove(name: &str) -> Result<()> {
    let resolved = fuzzy_resolve(name)?;

    style::blank();
    if !style::confirm(&format!("Forget project '{}'?", resolved)) {
        style::hint("Cancelled.");
        style::blank();
        return Ok(());
    }

    let service_mgr = ServiceManager::new();
    service_mgr.stop_services(&resolved)?;
    ProjectContext::remove(&resolved)?;

    style::success(&format!("'{}' forgotten", resolved));
    style::blank();

    Ok(())
}

// ── Init ────────────────────────────────────────────────

fn cmd_init() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config_path = cwd.join(".pylot.toml");

    style::blank();

    if config_path.exists() {
        style::warn(".pylot.toml already exists here.");
        style::blank();
        return Ok(());
    }

    let template = ProjectContext::generate_config_template(&cwd)?;
    std::fs::write(&config_path, &template)?;

    style::success("Created .pylot.toml");
    style::blank();

    let dir_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "myproject".to_string());

    style::hint("Edit .pylot.toml to add your services and ports, then:");
    style::blank();
    eprintln!(
        "    {}$ pylot save {}{}",
        style::DIM, dir_name, style::RESET,
    );
    style::blank();

    Ok(())
}

// ── Stop ────────────────────────────────────────────────

fn cmd_stop(name: Option<&str>) -> Result<()> {
    let context_name = match name {
        Some(n) => fuzzy_resolve(n)?,
        None => {
            let cwd = std::env::current_dir()?;
            cwd.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        }
    };

    let service_mgr = ServiceManager::new();
    let health = service_mgr.service_health(&context_name);

    style::blank();

    if health.is_empty() {
        style::hint(&format!("No services running for '{}'.", context_name));
        style::blank();
        return Ok(());
    }

    service_mgr.stop_services(&context_name)?;
    style::success(&format!("Stopped all services for '{}'", context_name));
    style::blank();

    Ok(())
}

// ── Doctor ──────────────────────────────────────────────

fn cmd_doctor() -> Result<()> {
    style::blank();
    style::heading("pylot doctor");
    style::blank();

    let mut issues = 0;

    // Check: shell hook installed?
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell_name = shell.rsplit('/').next().unwrap_or("zsh");
    let rc_file = match shell_name {
        "zsh" => dirs::home_dir().map(|h| h.join(".zshrc")),
        "bash" => dirs::home_dir().map(|h| h.join(".bashrc")),
        "fish" => dirs::home_dir().map(|h| h.join(".config/fish/config.fish")),
        _ => None,
    };

    let hook_installed = rc_file
        .as_ref()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|content| content.contains("pylot"))
        .unwrap_or(false);

    if hook_installed {
        eprintln!("  {} Shell hook      {}installed{}", style::CHECK, style::GREEN, style::RESET);
    } else {
        eprintln!("  {} Shell hook      {}not installed{}", style::CROSS, style::RED, style::RESET);
        eprintln!(
            "     {}Run: eval \"$(pylot shell-init {})\" >> ~/{}{}",
            style::DIM,
            shell_name,
            rc_file
                .as_ref()
                .map(|f| f.file_name().unwrap_or_default().to_string_lossy().to_string())
                .unwrap_or_else(|| ".zshrc".to_string()),
            style::RESET,
        );
        issues += 1;
    }

    // Check: git installed?
    let git_ok = std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if git_ok {
        eprintln!("  {} Git             {}found{}", style::CHECK, style::GREEN, style::RESET);
    } else {
        eprintln!("  {} Git             {}not found{}", style::CROSS, style::RED, style::RESET);
        issues += 1;
    }

    // Check: config dir exists?
    let config_dir = ProjectContext::config_dir()?;
    if config_dir.exists() {
        eprintln!("  {} Config dir      {}~/.pylot{}", style::CHECK, style::GREEN, style::RESET);
    } else {
        eprintln!("  {} Config dir      {}will be created on first save{}", style::DOT, style::DIM, style::RESET);
    }

    // Check: saved contexts
    let contexts = ProjectContext::list_all()?;
    eprintln!(
        "  {} Saved projects  {}{} project(s){}",
        style::CHECK,
        style::GREEN,
        contexts.len(),
        style::RESET,
    );

    // Check: any stale contexts (pointing to deleted directories)?
    let stale: Vec<_> = contexts
        .iter()
        .filter(|c| !c.path.exists())
        .collect();
    if !stale.is_empty() {
        eprintln!(
            "  {} Stale projects  {}{} point to missing directories{}",
            style::WARN,
            style::YELLOW,
            stale.len(),
            style::RESET,
        );
        for ctx in &stale {
            eprintln!(
                "     {}{} → {}{}",
                style::DIM, ctx.name, ctx.path.display(), style::RESET,
            );
        }
        issues += 1;
    }

    // Check: .pylot.toml in current directory?
    let cwd = std::env::current_dir()?;
    if cwd.join(".pylot.toml").exists() {
        eprintln!("  {} Project config  {}.pylot.toml found{}", style::CHECK, style::GREEN, style::RESET);
    } else {
        eprintln!("  {} Project config  {}no .pylot.toml here (optional){}", style::DOT, style::DIM, style::RESET);
    }

    style::blank();
    if issues == 0 {
        style::success("Everything looks good!");
    } else {
        style::warn(&format!("{} issue(s) found above", issues));
    }
    style::blank();

    Ok(())
}

// ── Shell init ──────────────────────────────────────────

fn cmd_shell_init(shell: &str) -> Result<()> {
    match shell {
        "bash" | "zsh" => {
            println!(r#"# pylot shell integration — add to your ~/.{}rc
pylot() {{
    if [ "$1" = "switch" ] && [ -n "$2" ]; then
        local output
        output=$(command pylot switch "${{@:2}}" 2>&1)
        local exit_code=$?

        if [ $exit_code -ne 0 ]; then
            echo "$output" >&2
            return $exit_code
        fi

        echo "$output" | sed -n '/^__PYLOT_COMMANDS__$/q;p' >&2

        local commands
        commands=$(echo "$output" | sed -n '/^__PYLOT_COMMANDS__$/,$ {{ /^__PYLOT_COMMANDS__$/d; p; }}')
        if [ -n "$commands" ]; then
            eval "$commands"
        fi
    else
        command pylot "$@"
    fi
}}"#, shell);
        }
        "fish" => {
            println!(r#"# pylot shell integration — add to your ~/.config/fish/config.fish
function pylot
    if test "$argv[1]" = "switch" -a -n "$argv[2]"
        set -l output (command pylot switch $argv[2..] 2>&1)
        set -l exit_code $status

        if test $exit_code -ne 0
            echo $output >&2
            return $exit_code
        end

        set -l found 0
        for line in $output
            if test "$line" = "__PYLOT_COMMANDS__"
                set found 1
                continue
            end
            if test $found -eq 1
                eval $line
            else
                echo $line >&2
            end
        end
    else
        command pylot $argv
    end
end"#);
        }
        _ => {
            style::error(&format!("Unsupported shell: {}. Use: bash, zsh, or fish", shell));
        }
    }
    Ok(())
}

// ── Completions ─────────────────────────────────────────

fn cmd_completions(shell: Shell) -> Result<()> {
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "pylot", &mut std::io::stdout());
    Ok(())
}

// ── Fuzzy matching ──────────────────────────────────────

fn fuzzy_resolve(input: &str) -> Result<String> {
    // Try exact match first
    let contexts = ProjectContext::list_all()?;

    // Exact match
    if let Some(ctx) = contexts.iter().find(|c| c.name == input) {
        return Ok(ctx.name.clone());
    }

    // Prefix match
    let prefix_matches: Vec<_> = contexts
        .iter()
        .filter(|c| c.name.starts_with(input))
        .collect();

    if prefix_matches.len() == 1 {
        let matched = &prefix_matches[0].name;
        eprintln!(
            "  {}Matched '{}' → '{}'{}",
            style::DIM, input, matched, style::RESET,
        );
        return Ok(matched.clone());
    }

    // Contains match
    let contains_matches: Vec<_> = contexts
        .iter()
        .filter(|c| c.name.contains(input))
        .collect();

    if contains_matches.len() == 1 {
        let matched = &contains_matches[0].name;
        eprintln!(
            "  {}Matched '{}' → '{}'{}",
            style::DIM, input, matched, style::RESET,
        );
        return Ok(matched.clone());
    }

    // Multiple matches
    if !prefix_matches.is_empty() || !contains_matches.is_empty() {
        let matches = if !prefix_matches.is_empty() {
            prefix_matches
        } else {
            contains_matches
        };
        style::blank();
        style::warn(&format!("'{}' matches multiple projects:", input));
        for m in &matches {
            eprintln!("     {} {}", style::ARROW, m.name);
        }
        style::blank();
        anyhow::bail!("Be more specific.");
    }

    // No match at all — suggest closest
    style::blank();
    style::error(&format!("No project named '{}'", input));

    if !contexts.is_empty() {
        style::blank();
        style::hint("Your saved projects:");
        for ctx in &contexts {
            eprintln!("     {} {}", style::ARROW, ctx.name);
        }
    } else {
        style::hint("No projects saved yet. Run: pylot save <name>");
    }
    style::blank();

    anyhow::bail!("Project not found.");
}
