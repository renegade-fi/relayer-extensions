use crate::helpers::{
    create_ecr_client, get_commit_hash, get_ecr_registry, run_command_with_output,
    run_command_with_stdin,
};
use anyhow::Result;
use base64::{engine::general_purpose, Engine};

/// Build and push Docker image to ECR
pub async fn build_and_push(
    dockerfile: String,
    ecr_repo: String,
    region: String,
    cargo_features: String,
) -> Result<()> {
    println!("Building and pushing Docker image...");
    let ecr_registry = get_ecr_registry(&ecr_repo, &region);
    let image_name = &ecr_repo;

    // Get current commit hash to tag the image with
    let commit_hash = get_commit_hash()?;
    println!("Using commit hash: {commit_hash}");

    // Build the Docker image
    build_docker_image(&dockerfile, image_name, &cargo_features)?;
    push_docker_image(&ecr_registry, image_name, &commit_hash, &region).await?;
    println!("Successfully built and pushed Docker image!");
    Ok(())
}

/// Build Docker image locally
fn build_docker_image(dockerfile: &str, image_name: &str, cargo_features: &str) -> Result<()> {
    println!("Building Docker image...");
    let build_args = if cargo_features.is_empty() {
        String::new()
    } else {
        format!("--build-arg CARGO_FEATURES={cargo_features}")
    };

    run_command_with_output(&format!(
        "docker build -t {image_name}:latest -f {dockerfile} {build_args} ."
    ))
}

/// Push Docker image to ECR
async fn push_docker_image(
    ecr_registry: &str,
    image_name: &str,
    commit_hash: &str,
    region: &str,
) -> Result<()> {
    // Login to ECR
    println!("Logging into ECR...");
    let password = get_ecr_auth_token(region).await?;
    run_command_with_stdin(
        &format!("docker login --username AWS --password-stdin {ecr_registry}"),
        &password,
    )?;

    // Tag images
    let tag1 = format!("{ecr_registry}:{commit_hash}");
    let tag2 = format!("{ecr_registry}:latest");
    run_command_with_output(&format!("docker tag {image_name}:latest {tag1}"))?;
    run_command_with_output(&format!("docker tag {image_name}:latest {tag2}"))?;

    // Push images
    println!("Pushing image with tags: {tag1} and {tag2}");
    run_command_with_output(&format!("docker push {tag1}"))?;
    run_command_with_output(&format!("docker push {tag2}"))
}

/// Get ECR authorization token password for Docker login
async fn get_ecr_auth_token(region: &str) -> Result<String> {
    let ecr_client = create_ecr_client(region).await?;

    // Get an authorization token for the ECR registry
    let auth_result = ecr_client.get_authorization_token().send().await?;
    let auth_data_slice = auth_result.authorization_data();
    let auth_data = auth_data_slice
        .first()
        .ok_or_else(|| anyhow::anyhow!("No authorization data in response"))?;
    let token = auth_data
        .authorization_token()
        .ok_or_else(|| anyhow::anyhow!("No authorization token in response"))?;

    // Decode the token to get the password
    parse_password_from_token(token)
}

/// Decode base64 ECR authorization token to extract the password
fn parse_password_from_token(token: &str) -> Result<String> {
    // Decode the base64 token (format is "AWS:password")
    let decoded = general_purpose::STANDARD.decode(token)?;
    let token_str = String::from_utf8(decoded)?;
    let password =
        token_str.strip_prefix("AWS:").ok_or_else(|| anyhow::anyhow!("Invalid token format"))?;

    Ok(password.to_string())
}
