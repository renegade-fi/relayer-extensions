use anyhow::{anyhow, Result};
use aws_sdk_ecr::Client as EcrClient;
use aws_sdk_ecs::Client as EcsClient;
use std::process::{Command, Stdio};

// -------
// | ECR |
// -------

/// Get the ECR registry template
pub fn get_ecr_registry(repo: &str, region: &str) -> String {
    format!("377928551571.dkr.ecr.{region}.amazonaws.com/{repo}")
}

/// Create an ECR client for the specified region
pub async fn create_ecr_client(region: &str) -> Result<EcrClient> {
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()))
        .load()
        .await;
    Ok(EcrClient::new(&config))
}

// -------
// | ECS |
// -------

/// Create an ECS client for the specified region
pub async fn create_ecs_client(region: &str) -> Result<EcsClient> {
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()))
        .load()
        .await;
    Ok(EcsClient::new(&config))
}

// -------
// | Git |
// -------

/// Get the current git commit hash (short form)
pub fn get_commit_hash() -> Result<String> {
    let output = run_command("git rev-parse --short HEAD")?;
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(hash)
}

// ---------------------
// | Shell Interaction |
// ---------------------

/// Internal helper to execute a shell command with optional output display
/// If show_output is true, output is displayed in real-time; otherwise it's
/// captured
fn run_command_internal(command_str: &str, show_output: bool) -> Result<std::process::Output> {
    let parts: Vec<&str> = command_str.split_whitespace().collect();

    if parts.is_empty() {
        return Err(anyhow!("Empty command string"));
    }

    let (cmd, args) = parts.split_first().unwrap();

    if show_output {
        // Run with real-time output
        let status = Command::new(cmd).args(args).status()?;

        if !status.success() {
            return Err(anyhow!("Command '{}' failed", command_str));
        }

        // Return empty output since we showed it in real-time
        Ok(std::process::Output { status, stdout: Vec::new(), stderr: Vec::new() })
    } else {
        // Capture output
        let output = Command::new(cmd).args(args).output()?;

        if !output.status.success() {
            return Err(anyhow!(
                "Command '{}' failed: {}",
                command_str,
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(output)
    }
}

/// Execute a shell command and check if it was successful
/// Returns an error if the command fails
pub fn run_command(command_str: &str) -> Result<std::process::Output> {
    run_command_internal(command_str, false)
}

/// Execute a shell command with real-time output to terminal
/// Returns an error if the command fails
pub fn run_command_with_output(command_str: &str) -> Result<()> {
    run_command_internal(command_str, true)?;
    Ok(())
}

/// Execute a shell command with stdin input
/// Returns an error if the command fails
pub fn run_command_with_stdin(command_str: &str, stdin_input: &str) -> Result<()> {
    let parts: Vec<&str> = command_str.split_whitespace().collect();

    if parts.is_empty() {
        return Err(anyhow!("Empty command string"));
    }

    let (cmd, args) = parts.split_first().unwrap();

    let mut child = Command::new(cmd).args(args).stdin(Stdio::piped()).spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(stdin_input.as_bytes())?;
    }

    let result = child.wait()?;
    if !result.success() {
        return Err(anyhow!("Command '{}' failed", command_str));
    }

    Ok(())
}
