use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use sysinfo::System;

use crate::context::ProjectContext;

/// Tracks PIDs of services started by pylot
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ServiceState {
    /// Map of context_name -> (service_name -> pid)
    pub running: HashMap<String, HashMap<String, u32>>,
}

impl ServiceState {
    fn state_path() -> Result<PathBuf> {
        let dir = ProjectContext::config_dir()?;
        Ok(dir.join("services.json"))
    }

    pub fn load() -> Self {
        Self::state_path()
            .ok()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::state_path()?;
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Record that a service was started
    pub fn register(&mut self, context: &str, service: &str, pid: u32) {
        self.running
            .entry(context.to_string())
            .or_default()
            .insert(service.to_string(), pid);
    }

    /// Remove a service record
    pub fn unregister(&mut self, context: &str, service: &str) {
        if let Some(services) = self.running.get_mut(context) {
            services.remove(service);
            if services.is_empty() {
                self.running.remove(context);
            }
        }
    }

    /// Get running services for a context
    pub fn get_services(&self, context: &str) -> Vec<(String, u32)> {
        self.running
            .get(context)
            .map(|s| s.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default()
    }
}

pub struct ServiceManager {
    system: System,
}

impl ServiceManager {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_all();
        Self { system }
    }

    /// Check which required ports are already in use
    pub fn check_port_conflicts(&self, required_ports: &[u16]) -> Vec<(u16, u32, String)> {
        let mut conflicts = Vec::new();
        for &port in required_ports {
            if let Some((pid, name)) = self.find_process_on_port(port) {
                conflicts.push((port, pid, name));
            }
        }
        conflicts
    }

    /// Get all listening ports and their processes
    pub fn get_listening_ports(&self) -> Vec<(u16, String)> {
        #[cfg(target_os = "macos")]
        let mut ports = self.get_listening_ports_lsof();

        #[cfg(target_os = "linux")]
        let mut ports = self.get_listening_ports_ss();

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let mut ports: Vec<(u16, String)> = Vec::new();

        ports.sort_by_key(|(p, _)| *p);
        ports.dedup_by_key(|(p, _)| *p);
        ports
    }

    /// Start a service command in the background, return its PID
    pub fn start_service(&self, name: &str, cmd: &str, working_dir: &std::path::Path) -> Result<u32> {
        let child = std::process::Command::new("sh")
            .args(["-c", cmd])
            .current_dir(working_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to start service '{}': {}", name, e))?;

        Ok(child.id())
    }

    /// Stop all services for a context
    pub fn stop_services(&self, context_name: &str) -> Result<()> {
        let mut state = ServiceState::load();
        let services = state.get_services(context_name);

        for (name, pid) in &services {
            if self.is_process_alive(*pid) {
                self.kill_process(*pid)?;
                eprintln!("  Stopped service '{}' (PID {})", name, pid);
            }
            state.unregister(context_name, name);
        }

        state.save()?;
        Ok(())
    }

    /// Start all services defined in a context
    pub fn start_services(&self, ctx: &ProjectContext) -> Result<()> {
        if ctx.services.is_empty() {
            return Ok(());
        }

        let mut state = ServiceState::load();

        for (name, cmd) in &ctx.services {
            let pid = self.start_service(name, cmd, &ctx.path)?;
            state.register(&ctx.name, name, pid);
            eprintln!("  Started service '{}' (PID {}): {}", name, pid, cmd);
        }

        state.save()?;
        Ok(())
    }

    /// Check if a process is still running
    pub fn is_process_alive(&self, pid: u32) -> bool {
        let spid = sysinfo::Pid::from_u32(pid);
        self.system.process(spid).is_some()
    }

    /// Kill a process by PID
    pub fn kill_process(&self, pid: u32) -> Result<()> {
        let spid = sysinfo::Pid::from_u32(pid);
        if let Some(process) = self.system.process(spid) {
            process.kill();
        } else {
            bail!("Process {} not found", pid);
        }
        Ok(())
    }

    /// Get service health status for a context
    pub fn service_health(&self, context_name: &str) -> Vec<(String, u32, bool)> {
        let state = ServiceState::load();
        state
            .get_services(context_name)
            .into_iter()
            .map(|(name, pid)| {
                let alive = self.is_process_alive(pid);
                (name, pid, alive)
            })
            .collect()
    }

    #[cfg(target_os = "macos")]
    fn get_listening_ports_lsof(&self) -> Vec<(u16, String)> {
        let mut ports = Vec::new();
        if let Ok(output) = std::process::Command::new("lsof")
            .args(["-iTCP", "-sTCP:LISTEN", "-nP"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 9 {
                        let name = parts[0].to_string();
                        if let Some(port_str) = parts[8].rsplit(':').next() {
                            if let Ok(port) = port_str.parse::<u16>() {
                                ports.push((port, name));
                            }
                        }
                    }
                }
            }
        }
        ports
    }

    #[cfg(target_os = "linux")]
    fn get_listening_ports_ss(&self) -> Vec<(u16, String)> {
        let mut ports = Vec::new();
        if let Ok(output) = std::process::Command::new("ss")
            .args(["-tlnp"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 5 {
                        // Local address is in column 4, format: addr:port
                        if let Some(port_str) = parts[3].rsplit(':').next() {
                            if let Ok(port) = port_str.parse::<u16>() {
                                // Process name is in the last column
                                let proc_name = parts
                                    .last()
                                    .and_then(|s| {
                                        // Format: users:(("name",pid=123,fd=4))
                                        s.split('"').nth(1).map(|n| n.to_string())
                                    })
                                    .unwrap_or_else(|| "unknown".to_string());
                                ports.push((port, proc_name));
                            }
                        }
                    }
                }
            }
        }
        ports
    }

    fn find_process_on_port(&self, port: u16) -> Option<(u32, String)> {
        #[cfg(target_os = "macos")]
        {
            self.find_process_on_port_lsof(port)
        }

        #[cfg(target_os = "linux")]
        {
            self.find_process_on_port_ss(port)
        }
    }

    #[cfg(target_os = "macos")]
    fn find_process_on_port_lsof(&self, port: u16) -> Option<(u32, String)> {
        let output = std::process::Command::new("lsof")
            .args(["-iTCP", &format!(":{}", port), "-sTCP:LISTEN", "-nP", "-t"])
            .output()
            .ok()?;

        if output.status.success() {
            let pid_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                let spid = sysinfo::Pid::from_u32(pid);
                let name = self
                    .system
                    .process(spid)
                    .map(|p| p.name().to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                return Some((pid, name));
            }
        }
        None
    }

    #[cfg(target_os = "linux")]
    fn find_process_on_port_ss(&self, port: u16) -> Option<(u32, String)> {
        let output = std::process::Command::new("ss")
            .args(["-tlnp", &format!("sport = :{}", port)])
            .output()
            .ok()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                // Extract PID from users:(("name",pid=123,fd=4))
                if let Some(pid_part) = line.split("pid=").nth(1) {
                    if let Ok(pid) = pid_part.split(',').next().unwrap_or("").parse::<u32>() {
                        let spid = sysinfo::Pid::from_u32(pid);
                        let name = self
                            .system
                            .process(spid)
                            .map(|p| p.name().to_string_lossy().to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        return Some((pid, name));
                    }
                }
            }
        }
        None
    }
}
