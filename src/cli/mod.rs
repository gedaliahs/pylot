use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::context::ProjectContext;
use crate::services::ServiceManager;
use crate::style;

#[derive(Parser)]
#[command(
    name = "pylot",
    about = "Project context switcher",
    version,
    after_help = format!(
        "{}{}Examples:{}\n  \
        pylot save myproject      Save current directory as a context\n  \
        pylot switch myproject    Switch to a saved context\n  \
        pylot list                Show all saved contexts\n  \
        pylot                     Open interactive dashboard\n\n  \
        {}Get started:{} pylot init\n",
        style::BOLD, style::WHITE, style::RESET,
        style::DIM, style::RESET,
    ),
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Save the current project context
    Save {
        /// Name for this context
        name: String,
    },
    /// Switch to a saved project context
    Switch {
        /// Context name to switch to
        name: String,
        /// Skip dirty state warning
        #[arg(long)]
        force: bool,
    },
    /// List all saved contexts
    List,
    /// Show status of the current context
    Status,
    /// Remove a saved context
    Remove {
        /// Context name to remove
        name: String,
    },
    /// Initialize a .pylot.toml config in the current directory
    Init,
    /// Stop all services for a context
    Stop {
        /// Context name (defaults to current directory name)
        name: Option<String>,
    },
    /// Show the shell hook script to add to your shell profile
    ShellInit {
        /// Shell type (bash, zsh, fish)
        #[arg(default_value = "zsh")]
        shell: String,
    },
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Commands::Save { name }) => cmd_save(&name),
        Some(Commands::Switch { name, force }) => cmd_switch(&name, force),
        Some(Commands::List) => cmd_list(),
        Some(Commands::Status) => cmd_status(),
        Some(Commands::Remove { name }) => cmd_remove(&name),
        Some(Commands::Init) => cmd_init(),
        Some(Commands::Stop { name }) => cmd_stop(name.as_deref()),
        Some(Commands::ShellInit { shell }) => cmd_shell_init(&shell),
        None => crate::tui::run_dashboard(),
    }
}

fn cmd_save(name: &str) -> Result<()> {
    let ctx = ProjectContext::capture_current(name)?;
    ctx.save()?;

    style::blank();
    style::success(&format!("Context {}{}{} saved", style::BOLD, name, style::RESET));
    style::blank();
    style::item("path", &ctx.path.display().to_string());
    style::item_colored(
        "branch",
        ctx.git_branch.as_deref().unwrap_or("n/a"),
        style::MAGENTA,
    );
    style::item("env vars", &format!("{}", ctx.env_vars.len()));
    if !ctx.services.is_empty() {
        style::item(
            "services",
            &ctx.services.keys().cloned().collect::<Vec<_>>().join(", "),
        );
    }
    style::blank();
    style::hint(&format!("Switch with: pylot switch {}", name));
    style::blank();

    Ok(())
}

fn cmd_switch(name: &str, force: bool) -> Result<()> {
    if !force {
        let cwd = std::env::current_dir()?;
        if ProjectContext::has_dirty_git_state(&cwd) {
            if let Some(summary) = ProjectContext::dirty_summary(&cwd) {
                style::blank();
                style::warn(&format!("Uncommitted changes: {}", summary));
                if !style::confirm("Switch anyway?") {
                    style::blank();
                    style::hint("Commit or stash your changes, or use --force");
                    style::blank();
                    return Ok(());
                }
            }
        }
    }

    let ctx = ProjectContext::load(name)?;
    let service_mgr = ServiceManager::new();

    let conflicts = service_mgr.check_port_conflicts(&ctx.ports_required);
    if !conflicts.is_empty() {
        style::blank();
        style::warn("Port conflicts detected:");
        for (port, pid, proc_name) in &conflicts {
            style::item(
                &format!(":{}", port),
                &format!("{} (PID {})", proc_name, pid),
            );
        }
        style::blank();
        if style::confirm("Kill conflicting processes?") {
            for (_, pid, _) in &conflicts {
                service_mgr.kill_process(*pid)?;
            }
            style::success("Processes killed");
        }
    }

    if !ctx.services.is_empty() {
        style::section("Services");
        service_mgr.start_services(&ctx)?;
    }

    style::blank();
    style::success(&format!(
        "Switched to {}{}{}",
        style::BOLD, name, style::RESET
    ));
    style::blank();
    style::item("path", &ctx.path.display().to_string());
    if let Some(ref branch) = ctx.git_branch {
        style::item_colored("branch", branch, style::MAGENTA);
    }
    if let Some(ref last) = ctx.last_accessed {
        style::item("last used", &last.format("%Y-%m-%d %H:%M").to_string());
    }
    style::blank();

    ctx.print_shell_commands();

    Ok(())
}

fn cmd_list() -> Result<()> {
    let contexts = ProjectContext::list_all()?;
    if contexts.is_empty() {
        style::empty_state(
            "No saved contexts yet.",
            "Run pylot save <name> in a project directory to get started.",
        );
        return Ok(());
    }

    let service_mgr = ServiceManager::new();

    style::blank();
    style::heading("Your Contexts");
    style::blank();
    style::table_header();

    for ctx in contexts {
        let health = service_mgr.service_health(&ctx.name);
        let svc_status = if health.is_empty() {
            "–".to_string()
        } else {
            let alive = health.iter().filter(|(_, _, a)| *a).count();
            format!("{}/{} up", alive, health.len())
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

fn cmd_status() -> Result<()> {
    let ctx = ProjectContext::detect_current()?;
    let service_mgr = ServiceManager::new();

    style::blank();
    style::heading(&format!("Status: {}", ctx.name));
    style::blank();
    style::item("path", &ctx.path.display().to_string());
    style::item_colored(
        "branch",
        ctx.git_branch.as_deref().unwrap_or("n/a"),
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
        style::section("Active Ports");
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

fn cmd_remove(name: &str) -> Result<()> {
    style::blank();
    if !style::confirm(&format!("Remove context '{}'?", name)) {
        style::hint("Cancelled.");
        style::blank();
        return Ok(());
    }

    let service_mgr = ServiceManager::new();
    service_mgr.stop_services(name)?;
    ProjectContext::remove(name)?;

    style::success(&format!("Context '{}' removed", name));
    style::blank();

    Ok(())
}

fn cmd_init() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config_path = cwd.join(".pylot.toml");

    style::blank();

    if config_path.exists() {
        style::warn(".pylot.toml already exists in this directory.");
        style::blank();
        return Ok(());
    }

    let template = ProjectContext::generate_config_template(&cwd)?;
    std::fs::write(&config_path, &template)?;

    style::success("Created .pylot.toml");
    style::blank();
    style::hint("Edit it to define your services and required ports, then:");
    style::blank();
    let dir_name = cwd.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "myproject".to_string());
    eprintln!(
        "    {}$ pylot save {}{}",
        style::DIM, dir_name, style::RESET,
    );
    style::blank();

    Ok(())
}

fn cmd_stop(name: Option<&str>) -> Result<()> {
    let context_name = match name {
        Some(n) => n.to_string(),
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
    style::success(&format!("All services stopped for '{}'", context_name));
    style::blank();

    Ok(())
}

fn cmd_shell_init(shell: &str) -> Result<()> {
    match shell {
        "bash" | "zsh" => {
            println!(r#"# Add this to your ~/.{}rc
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
            println!(r#"# Add this to your ~/.config/fish/config.fish
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
            style::error(&format!("Unsupported shell: {}. Supported: bash, zsh, fish", shell));
        }
    }
    Ok(())
}
