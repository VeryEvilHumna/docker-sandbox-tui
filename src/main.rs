use anyhow::{Context, Result};
use colored::Colorize;
use inquire::{error::InquireError, Confirm, Select, Text};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = "1.0.0";
const APP_NAME: &str = "docker-sandbox-tui";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    /// Default agent (e.g. "claude", "codex")
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<String>,

    /// Whether to open VS Code by default
    #[serde(skip_serializing_if = "Option::is_none")]
    open_vscode: Option<bool>,

    /// When false, skip the "Open VS Code?" prompt entirely (and don't open it)
    #[serde(skip_serializing_if = "Option::is_none")]
    show_vscode_prompt: Option<bool>,
}

impl Config {
    /// True when any value has been explicitly set (i.e. config file exists with content)
    fn is_set(&self) -> bool {
        self.agent.is_some() || self.open_vscode.is_some() || self.show_vscode_prompt.is_some()
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join("config.yaml"))
}

fn load_config() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    serde_yaml::from_str(&contents).unwrap_or_default()
}

fn save_config(config: &Config) -> Result<()> {
    let path = config_path().context("Could not determine config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create config directory")?;
    }
    let yaml = serde_yaml::to_string(config).context("Failed to serialize config")?;
    std::fs::write(&path, yaml).context("Failed to write config file")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// App types
// ---------------------------------------------------------------------------

struct SandboxInfo {
    name: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    if let Err(e) = run() {
        let interrupted = e.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<InquireError>(),
                Some(InquireError::OperationInterrupted | InquireError::OperationCanceled)
            )
        });
        if interrupted {
            println!("\n  Cancelled.");
            std::process::exit(130);
        }
        eprintln!("\n  {} {e:#}", "Error:".red());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path_arg = args.get(1).map(|s| s.as_str());

    let config = load_config();
    print_banner(&config);

    // Resolve workspace: default to cwd when no arg or arg is "."
    let workspace = match path_arg {
        Some(".") | None => std::env::current_dir().context("Failed to get current directory")?,
        Some(p) => resolve_path(p)?,
    };

    let is_quick_launch = path_arg == Some(".");
    let existing = find_existing_sandbox(&workspace);

    if is_quick_launch {
        return quick_launch(existing, &workspace, &config);
    }

    // ---- Interactive mode ------------------------------------------------

    if let Some(sandbox) = existing {
        // Offer the existing sandbox as the first choice
        let choice = Select::new(
            "A sandbox already exists for this directory. What would you like to do?",
            vec![
                format!("Resume existing sandbox: {}", sandbox.name),
                "Create new sandbox".to_string(),
                "Cancel".to_string(),
            ],
        )
        .with_starting_cursor(0)
        .prompt()
        .context("Action select failed")?;

        if choice.starts_with("Resume") {
            return resume_sandbox(sandbox, &workspace, &config);
        } else if choice == "Cancel" {
            println!("  Cancelled.");
            return Ok(());
        }
        // fall through to wizard
    }

    run_wizard(path_arg, &workspace, &config)
}

// ---------------------------------------------------------------------------
// Launch modes
// ---------------------------------------------------------------------------

/// `agent .` — zero-interaction fast path
fn quick_launch(existing: Option<SandboxInfo>, workspace: &Path, config: &Config) -> Result<()> {
    if let Some(sandbox) = existing {
        println!(
            "  {}  {}",
            "Resuming sandbox:".green(),
            sandbox.name.green().bold()
        );
        resume_sandbox_run(&sandbox.name)?;
        if config.open_vscode.unwrap_or(false) {
            launch_vscode(workspace)?;
        }
        return Ok(());
    }

    // No existing sandbox — create with defaults (no prompts)
    let agent = config
        .agent
        .as_deref()
        .unwrap_or("claude")
        .to_string();
    let open_vscode = config.open_vscode.unwrap_or(false);

    println!(
        "  {}  {} {}",
        "Quick-launching with defaults:".cyan(),
        agent.bold(),
        workspace.display()
    );
    println!();

    print_summary(workspace, &agent, &None);
    run_sandbox(&agent, workspace, &None)?;

    if open_vscode {
        launch_vscode(workspace)?;
    }

    Ok(())
}

/// Resume an existing sandbox interactively (ask about VS Code, offer save)
fn resume_sandbox(sandbox: SandboxInfo, workspace: &Path, config: &Config) -> Result<()> {
    resume_sandbox_run(&sandbox.name)?;

    let open_vscode = prompt_vscode(config)?;
    if open_vscode {
        launch_vscode(workspace)?;
    }

    Ok(())
}

/// Full wizard flow
fn run_wizard(path_arg: Option<&str>, workspace: &Path, config: &Config) -> Result<()> {
    let agent = prompt_agent(config)?;
    let workspace = prompt_workspace(path_arg, workspace)?;
    let name = prompt_name(&agent, &workspace)?;
    let template = prompt_template()?;
    let open_vscode = prompt_vscode(config)?;

    print_summary(&workspace, &agent, &template);
    run_sandbox(&agent, &workspace, &template)?;

    if open_vscode {
        launch_vscode(&workspace)?;
    }

    // Offer to save as new defaults (last step, explicit)
    maybe_save_config(&agent, open_vscode, config)?;

    print_hints(&name);
    Ok(())
}

// ---------------------------------------------------------------------------
// UI helpers
// ---------------------------------------------------------------------------

fn print_banner(config: &Config) {
    let title = format!("docker-sandbox-tui  v{VERSION}");
    let subtitle = "Because docker sandbox UX sucks";

    // Show config file path only when config has been set at least once
    let cfg_path_line = if config.is_set() {
        config_path().map(|p| format!("config: {}", p.display()))
    } else {
        None
    };

    let mut lines: Vec<String> = vec![title, subtitle.to_string()];
    if let Some(ref p) = cfg_path_line {
        lines.push(p.clone());
    }

    let content_width = lines.iter().map(|l| l.len()).max().unwrap_or(0);
    let inner_width = content_width + 4;
    let hr = "─".repeat(inner_width);

    println!();
    let empty_row = format!("│{}│", " ".repeat(inner_width));
    println!("  {}", format!("┌{hr}┐").cyan().bold());
    println!("  {}", empty_row.cyan().bold());
    for (i, line) in lines.iter().enumerate() {
        let right_pad = " ".repeat(inner_width - 2 - line.len());
        let formatted = format!("│  {line}{right_pad}│");
        if i == lines.len() - 1 && cfg_path_line.is_some() {
            println!("  {}", formatted.dimmed());
        } else {
            println!("  {}", formatted.cyan().bold());
        }
    }
    println!("  {}", empty_row.cyan().bold());
    println!("  {}", format!("└{hr}┘").cyan().bold());
    println!();
}

fn resolve_path(path: &str) -> Result<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        let cwd = std::env::current_dir().context("Failed to get current directory")?;
        let joined = cwd.join(p);
        Ok(joined.canonicalize().unwrap_or(joined))
    }
}

/// Returns `Some(SandboxInfo)` if docker reports a sandbox bound to `abs_path`.
/// Returns `None` on any failure (no Docker, command error, no match).
fn find_existing_sandbox(abs_path: &Path) -> Option<SandboxInfo> {
    let output = Command::new("docker")
        .args(["sandbox", "ls"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let path_str = abs_path.to_string_lossy();

    for line in stdout.lines().skip(1) {
        if line.contains(path_str.as_ref()) {
            let name = line.split_whitespace().next().unwrap_or("").to_string();
            if !name.is_empty() {
                return Some(SandboxInfo { name });
            }
        }
    }

    None
}

fn resume_sandbox_run(name: &str) -> Result<()> {
    println!(
        "  {}",
        format!("Running: docker sandbox run {name}").yellow()
    );

    let status = Command::new("docker")
        .args(["sandbox", "run", name])
        .status()
        .context("Failed to resume sandbox")?;

    println!();

    if status.success() {
        println!("{} Sandbox session ended", "✔".green());
    } else {
        println!("{} Sandbox run failed", "✗".red());
    }

    Ok(())
}

fn prompt_agent(config: &Config) -> Result<String> {
    let agents = vec![
        "claude",
        "codex",
        "gemini",
        "opencode",
        "copilot",
        "kiro",
        "cagent",
        "shell",
        "Enter agent name...",
    ];

    let starting = config
        .agent
        .as_deref()
        .and_then(|saved| agents.iter().position(|&a| a == saved))
        .unwrap_or(0);

    let selected = Select::new("Agent", agents)
        .with_starting_cursor(starting)
        .prompt()
        .context("Agent prompt failed")?;

    if selected == "Enter agent name..." {
        let custom = Text::new("Agent name:")
            .prompt()
            .context("Agent name prompt failed")?;
        Ok(custom.trim().to_string())
    } else {
        Ok(selected.to_string())
    }
}

fn prompt_workspace(path_arg: Option<&str>, resolved: &Path) -> Result<PathBuf> {
    let default_str = match path_arg {
        Some(".") | None => resolved.to_string_lossy().to_string(),
        Some(p) => p.to_string(),
    };

    let result = Text::new("Workspace path")
        .with_default(&default_str)
        .prompt()
        .context("Workspace prompt failed")?;

    Ok(PathBuf::from(result))
}

fn prompt_name(agent: &str, workspace: &Path) -> Result<String> {
    let dirname = workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("sandbox");
    let default_name = format!("{agent}-{dirname}");

    Text::new("Sandbox name")
        .with_default(&default_name)
        .prompt()
        .context("Name prompt failed")
}

fn prompt_template() -> Result<Option<String>> {
    let choice = Select::new("Template", vec!["No template", "Yes, use a template"])
        .with_starting_cursor(0)
        .prompt()
        .context("Template prompt failed")?;

    if choice == "No template" {
        return Ok(None);
    }

    let images = get_local_images();
    let mut options = vec!["Enter a custom image name or URL...".to_string()];
    options.extend(images);

    let selected = Select::new("Select image", options)
        .with_starting_cursor(0)
        .prompt()
        .context("Image select prompt failed")?;

    if selected == "Enter a custom image name or URL..." {
        let custom = Text::new("Image name or URL:")
            .prompt()
            .context("Image name prompt failed")?;
        Ok(Some(custom.trim().to_string()))
    } else {
        Ok(Some(selected))
    }
}

fn get_local_images() -> Vec<String> {
    let output = Command::new("docker")
        .args(["image", "ls", "--format", "{{.Repository}}:{{.Tag}}"])
        .output();

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.contains("<none>"))
            .map(|l| l.to_string())
            .collect(),
        _ => vec![],
    }
}

fn prompt_vscode(config: &Config) -> Result<bool> {
    if config.show_vscode_prompt == Some(false) {
        return Ok(false);
    }
    let default = config.open_vscode.unwrap_or(true);
    Confirm::new("Open VS Code?")
        .with_default(default)
        .prompt()
        .context("VS Code prompt failed")
}

fn maybe_save_config(agent: &str, open_vscode: bool, current: &Config) -> Result<()> {
    // Only ask if the new values differ from what's already saved
    let already_saved = current.agent.as_deref() == Some(agent)
        && current.open_vscode == Some(open_vscode);

    if already_saved {
        return Ok(());
    }

    // First launch (nothing saved yet) → default yes; subsequent → default no
    let is_first_launch = current.agent.is_none() && current.open_vscode.is_none();
    let label = format!("Save as new default? (agent={agent}  vscode={})", if open_vscode { "yes" } else { "no" });
    let save = Confirm::new(&label)
        .with_default(is_first_launch)
        .prompt()
        .context("Save config prompt failed")?;

    if save {
        let updated = Config {
            agent: Some(agent.to_string()),
            open_vscode: Some(open_vscode),
            show_vscode_prompt: Some(true)
        };
        save_config(&updated)?;
        if let Some(p) = config_path() {
            println!("  {} Config saved → {}", "✔".green(), p.display());
        }
    }

    Ok(())
}

fn print_summary(workspace: &Path, agent: &str, template: &Option<String>) {
    let mut cmd = format!(
        "  Running: docker sandbox run {} {}",
        agent,
        workspace.display()
    );
    if let Some(t) = template {
        cmd.push_str(&format!(" --image {t}"));
    }
    println!("{}", cmd.yellow());
    println!();
}

fn run_sandbox(agent: &str, workspace: &Path, template: &Option<String>) -> Result<()> {
    let mut args = vec![
        "sandbox".to_string(),
        "run".to_string(),
        agent.to_string(),
        workspace.to_string_lossy().to_string(),
    ];

    if let Some(t) = template {
        args.push("--image".to_string());
        args.push(t.clone());
    }

    let status = Command::new("docker")
        .args(&args)
        .status()
        .context("Failed to run docker sandbox")?;

    if status.success() {
        println!("  {} Sandbox ready", "✔".green());
    } else {
        println!("  {} Sandbox creation failed", "✗".red());
        anyhow::bail!("docker sandbox run failed");
    }

    Ok(())
}

fn launch_vscode(workspace: &Path) -> Result<()> {
    println!();
    println!("  Opening VS Code...");

    let status = Command::new("code")
        .arg(workspace)
        .status()
        .context("Failed to launch VS Code")?;

    if status.success() {
        println!("  {} VS Code launched", "✔".green());
    } else {
        println!("  {} VS Code launch failed", "✗".red());
    }

    Ok(())
}

fn print_hints(name: &str) {
    println!();
    println!("  {}", "Hint: docker sandbox ls".dimmed());
    println!(
        "  {}",
        format!("Hint: docker sandbox run {name}").dimmed()
    );
    println!("  {}", "Hint: agent .  (quick-launch in current dir)".dimmed());
    println!();
}
