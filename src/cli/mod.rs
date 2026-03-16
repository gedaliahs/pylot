use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::context::ProjectContext;
use crate::services::ServiceManager;

#[derive(Parser)]
#[command(name = "pylot", about = "Project context switcher", version)]
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
    println!("Context '{}' saved.", name);
    println!("  Path:   {}", ctx.path.display());
    println!("  Branch: {}", ctx.git_branch.as_deref().unwrap_or("n/a"));
    println!("  Env:    {} variables", ctx.env_vars.len());
    if !ctx.services.is_empty() {
        println!("  Services: {}", ctx.services.keys().cloned().collect::<Vec<_>>().join(", "));
    }
    Ok(())
}

fn cmd_switch(name: &str, force: bool) -> Result<()> {
    // Check for dirty state in the current directory before switching
    if !force {
        let cwd = std::env::current_dir()?;
        if ProjectContext::has_dirty_git_state(&cwd) {
            if let Some(summary) = ProjectContext::dirty_summary(&cwd) {
                eprintln!("Warning: Current directory has uncommitted changes ({}).", summary);
                eprintln!("Use --force to switch anyway, or commit/stash your changes first.");
                eprint!("Switch anyway? [y/N] ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Aborted.");
                    return Ok(());
                }
            }
        }
    }

    let ctx = ProjectContext::load(name)?;
    let service_mgr = ServiceManager::new();

    // Check for port conflicts before switching
    let conflicts = service_mgr.check_port_conflicts(&ctx.ports_required);
    if !conflicts.is_empty() {
        eprintln!("Port conflicts detected:");
        for (port, pid, proc_name) in &conflicts {
            eprintln!("  Port {} in use by {} (PID {})", port, proc_name, pid);
        }
        eprint!("Kill conflicting processes? [y/N] ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if input.trim().eq_ignore_ascii_case("y") {
            for (_, pid, _) in &conflicts {
                service_mgr.kill_process(*pid)?;
            }
        }
    }

    // Start services
    if !ctx.services.is_empty() {
        eprintln!("Starting services...");
        service_mgr.start_services(&ctx)?;
    }

    // Print switch summary to stderr (stdout is reserved for shell commands)
    eprintln!("Switched to '{}'", name);
    eprintln!("  Path:   {}", ctx.path.display());
    if let Some(ref branch) = ctx.git_branch {
        eprintln!("  Branch: {}", branch);
    }
    if let Some(ref last) = ctx.last_accessed {
        eprintln!("  Last accessed: {}", last.format("%Y-%m-%d %H:%M"));
    }

    // Generate shell commands to stdout (for eval by the shell wrapper)
    ctx.print_shell_commands();

    Ok(())
}

fn cmd_list() -> Result<()> {
    let contexts = ProjectContext::list_all()?;
    if contexts.is_empty() {
        println!("No saved contexts. Use 'pylot save <name>' to create one.");
        return Ok(());
    }

    let service_mgr = ServiceManager::new();

    println!("{:<16} {:<40} {:<12} {:<10} {}", "NAME", "PATH", "BRANCH", "SERVICES", "LAST ACCESSED");
    println!("{}", "-".repeat(96));
    for ctx in contexts {
        let health = service_mgr.service_health(&ctx.name);
        let svc_status = if health.is_empty() {
            "-".to_string()
        } else {
            let alive = health.iter().filter(|(_, _, a)| *a).count();
            format!("{}/{}", alive, health.len())
        };

        println!(
            "{:<16} {:<40} {:<12} {:<10} {}",
            ctx.name,
            ctx.path.display(),
            ctx.git_branch.as_deref().unwrap_or("-"),
            svc_status,
            ctx.last_accessed
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "-".to_string()),
        );
    }
    Ok(())
}

fn cmd_status() -> Result<()> {
    let ctx = ProjectContext::detect_current()?;
    let service_mgr = ServiceManager::new();

    println!("Current context:");
    println!("  Path:     {}", ctx.path.display());
    println!("  Branch:   {}", ctx.git_branch.as_deref().unwrap_or("n/a"));
    println!("  Env vars: {}", ctx.env_vars.len());

    // Dirty state
    if ProjectContext::has_dirty_git_state(&ctx.path) {
        if let Some(summary) = ProjectContext::dirty_summary(&ctx.path) {
            println!("  Git:      dirty ({})", summary);
        }
    } else {
        println!("  Git:      clean");
    }

    // Service health
    let health = service_mgr.service_health(&ctx.name);
    if !health.is_empty() {
        println!("  Services:");
        for (name, pid, alive) in &health {
            let status = if *alive { "running" } else { "stopped" };
            println!("    {} (PID {}) — {}", name, pid, status);
        }
    }

    // Active ports
    let active_ports = service_mgr.get_listening_ports();
    if !active_ports.is_empty() {
        println!("  Active ports:");
        for (port, proc_name) in &active_ports {
            println!("    :{} ({})", port, proc_name);
        }
    }
    Ok(())
}

fn cmd_remove(name: &str) -> Result<()> {
    // Stop any running services first
    let service_mgr = ServiceManager::new();
    service_mgr.stop_services(name)?;

    ProjectContext::remove(name)?;
    println!("Context '{}' removed.", name);
    Ok(())
}

fn cmd_init() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config_path = cwd.join(".pylot.toml");

    if config_path.exists() {
        println!(".pylot.toml already exists in this directory.");
        return Ok(());
    }

    let template = ProjectContext::generate_config_template(&cwd)?;
    std::fs::write(&config_path, &template)?;
    println!("Created .pylot.toml");
    println!("Edit it to define your services and required ports, then run:");
    println!("  pylot save <name>");
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

    if health.is_empty() {
        println!("No services running for '{}'.", context_name);
        return Ok(());
    }

    println!("Stopping services for '{}'...", context_name);
    service_mgr.stop_services(&context_name)?;
    println!("Done.");
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

        # Print everything before __DEVCTX_COMMANDS__ as info
        echo "$output" | sed -n '/^__DEVCTX_COMMANDS__$/q;p' >&2

        # Eval everything after __DEVCTX_COMMANDS__
        local commands
        commands=$(echo "$output" | sed -n '/^__DEVCTX_COMMANDS__$/,$ {{ /^__DEVCTX_COMMANDS__$/d; p; }}')
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

        # Eval shell commands after the marker
        set -l found 0
        for line in $output
            if test "$line" = "__DEVCTX_COMMANDS__"
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
            println!("Unsupported shell: {}. Supported: bash, zsh, fish", shell);
        }
    }
    Ok(())
}
