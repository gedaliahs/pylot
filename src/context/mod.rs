use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectContext {
    pub name: String,
    pub path: PathBuf,
    pub git_branch: Option<String>,
    pub env_vars: HashMap<String, String>,
    pub env_file: Option<String>,
    pub services: HashMap<String, String>,
    pub ports_required: Vec<u16>,
    pub last_accessed: Option<DateTime<Utc>>,
}

impl ProjectContext {
    /// Capture context from the current working directory
    pub fn capture_current(name: &str) -> Result<Self> {
        let path = std::env::current_dir()?;
        let git_branch = Self::detect_git_branch(&path);
        let env_vars = Self::detect_env_vars(&path);
        let env_file = Self::detect_env_file(&path);

        let mut ctx = Self {
            name: name.to_string(),
            path,
            git_branch,
            env_vars,
            env_file,
            services: HashMap::new(),
            ports_required: Vec::new(),
            last_accessed: Some(Utc::now()),
        };

        // Load .pylot.toml if it exists in the project
        ctx.load_project_config()?;

        Ok(ctx)
    }

    /// Detect context from current directory without a name
    pub fn detect_current() -> Result<Self> {
        let path = std::env::current_dir()?;
        let dir_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Ok(Self {
            name: dir_name,
            path: path.clone(),
            git_branch: Self::detect_git_branch(&path),
            env_vars: Self::detect_env_vars(&path),
            env_file: Self::detect_env_file(&path),
            services: HashMap::new(),
            ports_required: Vec::new(),
            last_accessed: Some(Utc::now()),
        })
    }

    /// Save context to disk
    pub fn save(&self) -> Result<()> {
        let dir = Self::config_dir()?;
        fs::create_dir_all(&dir)?;
        let file = dir.join(format!("{}.json", self.name));
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&file, json).context("Failed to save context")?;
        Ok(())
    }

    /// Load a saved context by name
    pub fn load(name: &str) -> Result<Self> {
        let file = Self::config_dir()?.join(format!("{}.json", name));
        if !file.exists() {
            bail!("Context '{}' not found. Run 'pylot list' to see available contexts.", name);
        }
        let data = fs::read_to_string(&file)?;
        let mut ctx: Self = serde_json::from_str(&data)?;
        ctx.last_accessed = Some(Utc::now());

        // Also check for a .pylot.toml in the project directory
        ctx.load_project_config()?;

        // Re-save with updated last_accessed
        ctx.save()?;

        Ok(ctx)
    }

    /// List all saved contexts
    pub fn list_all() -> Result<Vec<Self>> {
        let dir = Self::config_dir()?;
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut contexts = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                let data = fs::read_to_string(&path)?;
                if let Ok(ctx) = serde_json::from_str::<Self>(&data) {
                    contexts.push(ctx);
                }
            }
        }
        contexts.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));
        Ok(contexts)
    }

    /// Remove a saved context
    pub fn remove(name: &str) -> Result<()> {
        let file = Self::config_dir()?.join(format!("{}.json", name));
        if !file.exists() {
            bail!("Context '{}' not found.", name);
        }
        fs::remove_file(&file)?;
        Ok(())
    }

    /// Check if the project directory has uncommitted git changes
    pub fn has_dirty_git_state(path: &Path) -> bool {
        Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(path)
            .output()
            .ok()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false)
    }

    /// Get a summary of uncommitted changes
    pub fn dirty_summary(path: &Path) -> Option<String> {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(path)
            .output()
            .ok()?;

        if !output.status.success() || output.stdout.is_empty() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        let modified = lines.iter().filter(|l| l.starts_with('M')).count();
        let added = lines.iter().filter(|l| l.starts_with('A') || l.starts_with('?')).count();
        let deleted = lines.iter().filter(|l| l.starts_with('D')).count();

        let mut parts = Vec::new();
        if modified > 0 {
            parts.push(format!("{} modified", modified));
        }
        if added > 0 {
            parts.push(format!("{} untracked/added", added));
        }
        if deleted > 0 {
            parts.push(format!("{} deleted", deleted));
        }

        if parts.is_empty() {
            Some(format!("{} changes", lines.len()))
        } else {
            Some(parts.join(", "))
        }
    }

    /// Print shell commands that the parent shell should execute
    pub fn print_shell_commands(&self) {
        // These get eval'd by the shell wrapper function
        println!("__DEVCTX_COMMANDS__");
        println!("cd \"{}\"", self.path.display());

        if let Some(ref branch) = self.git_branch {
            println!("git checkout {} 2>/dev/null", branch);
        }

        if let Some(ref env_file) = self.env_file {
            println!("set -a && source \"{}\" && set +a", env_file);
        }

        for (key, value) in &self.env_vars {
            println!("export {}=\"{}\"", key, value);
        }

        for (name, cmd) in &self.services {
            println!("# Starting service: {}", name);
            println!("{} &", cmd);
        }
    }

    /// Generate a .pylot.toml template for the current directory
    pub fn generate_config_template(path: &Path) -> Result<String> {
        let env_file = Self::detect_env_file(&path.to_path_buf())
            .unwrap_or_else(|| ".env".to_string());

        let mut toml = String::new();
        toml.push_str("# pylot project configuration\n");
        toml.push_str("# This file defines your project's dev environment\n\n");
        toml.push_str(&format!("env_file = \"{}\"\n\n", env_file));
        toml.push_str("[services]\n");
        toml.push_str("# Define services that should run when switching to this project\n");

        // Try to detect common project types and suggest services
        if path.join("package.json").exists() {
            toml.push_str("# dev = \"npm run dev\"\n");
        }
        if path.join("Cargo.toml").exists() {
            toml.push_str("# dev = \"cargo watch -x run\"\n");
        }
        if path.join("go.mod").exists() {
            toml.push_str("# dev = \"go run .\"\n");
        }
        if path.join("requirements.txt").exists() || path.join("pyproject.toml").exists() {
            toml.push_str("# dev = \"python manage.py runserver\"\n");
        }
        if path.join("docker-compose.yml").exists() || path.join("docker-compose.yaml").exists() || path.join("compose.yml").exists() {
            toml.push_str("# docker = \"docker compose up -d\"\n");
        }

        toml.push_str("\n[ports]\n");
        toml.push_str("# Ports your project needs — pylot will warn about conflicts\n");
        toml.push_str("required = []\n");

        Ok(toml)
    }

    /// Load project-local .pylot.toml config
    fn load_project_config(&mut self) -> Result<()> {
        let config_path = self.path.join(".pylot.toml");
        if !config_path.exists() {
            return Ok(());
        }

        let data = fs::read_to_string(&config_path)?;
        let config: toml::Value = toml::from_str(&data)?;

        if let Some(env_file) = config.get("env_file").and_then(|v| v.as_str()) {
            self.env_file = Some(env_file.to_string());
        }

        if let Some(services) = config.get("services").and_then(|v| v.as_table()) {
            for (name, cmd) in services {
                if let Some(cmd_str) = cmd.as_str() {
                    self.services.insert(name.clone(), cmd_str.to_string());
                }
            }
        }

        if let Some(ports) = config.get("ports").and_then(|v| v.get("required")).and_then(|v| v.as_array()) {
            self.ports_required = ports
                .iter()
                .filter_map(|v| v.as_integer().map(|p| p as u16))
                .collect();
        }

        Ok(())
    }

    pub fn config_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not find home directory")?;
        Ok(home.join(".pylot"))
    }

    fn detect_git_branch(path: &PathBuf) -> Option<String> {
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(path)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    }

    fn detect_env_vars(path: &PathBuf) -> HashMap<String, String> {
        let env_path = path.join(".env");
        let mut vars = HashMap::new();
        if let Ok(content) = fs::read_to_string(&env_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    vars.insert(
                        key.trim().to_string(),
                        value.trim().trim_matches('"').to_string(),
                    );
                }
            }
        }
        vars
    }

    fn detect_env_file(path: &PathBuf) -> Option<String> {
        for name in &[".env", ".env.local", ".env.development"] {
            if path.join(name).exists() {
                return Some(name.to_string());
            }
        }
        None
    }
}
