use anyhow::{anyhow, Result};
use aws_sdk_ecs::{types::TaskDefinition, Client as EcsClient};

use crate::helpers::{create_ecs_client, get_commit_hash, get_ecr_registry};

/// Deploy to ECS
pub async fn deploy(
    environment: String,
    resource: String,
    region: String,
    image_tag: Option<String>,
) -> Result<()> {
    let commit_hash_or_latest = get_commit_hash().unwrap_or_else(|_| "latest".to_string());
    let image_tag = image_tag.unwrap_or(commit_hash_or_latest);

    let cluster_name = format!("{environment}-{resource}-cluster");
    let service_name = format!("{environment}-{resource}-service");
    let task_def_name = format!("{environment}-{resource}-task-def");
    let ecr_repo = format!("{resource}-{environment}");
    let ecr_registry = get_ecr_registry(&ecr_repo, &region);
    let full_image_uri = format!("{ecr_registry}:{image_tag}");

    println!("Deploying to ECS...");
    println!("Using image URI: {full_image_uri}");

    // Create new ECS revision
    let ecs_client = create_ecs_client(&region).await?;
    let new_revision = create_ecs_revision(&ecs_client, &task_def_name, &full_image_uri).await?;

    // Deploy the new revision
    deploy_ecs_revision(&ecs_client, &cluster_name, &service_name, &task_def_name, new_revision)
        .await?;
    println!("Deployment completed successfully! 🎉");

    // Print the AWS console URL for easy access
    let console_url = format!(
        "https://{}.console.aws.amazon.com/ecs/v2/clusters/{}/services/{}/health?region={}",
        region, cluster_name, service_name, region
    );
    println!("View service in AWS Console:\n\t {console_url}");
    Ok(())
}

/// Create a new ECS task definition revision with updated image
async fn create_ecs_revision(
    ecs_client: &EcsClient,
    task_def_name: &str,
    new_image_uri: &str,
) -> Result<i32> {
    println!("Creating new task revision...");
    let new_revision = register_task_definition(ecs_client, task_def_name, new_image_uri).await?;
    println!("Created new task revision: {new_revision}");
    Ok(new_revision)
}

/// Register a new task definition by fetching the existing one and updating the
/// image
async fn register_task_definition(
    ecs_client: &EcsClient,
    task_def_name: &str,
    new_image_uri: &str,
) -> Result<i32> {
    println!("Updating task definition...");
    // Fetch existing task definition
    let mut task_def = fetch_latest_task_definition(ecs_client, task_def_name).await?;
    set_container_image(&mut task_def, new_image_uri)?;
    task_def.revision += 1;

    // Remove fields that ECS will set for us
    task_def.task_definition_arn = None;
    task_def.status = None;
    task_def.requires_attributes = None;
    task_def.compatibilities = None;
    task_def.registered_at = None;
    task_def.registered_by = None;

    // Register the task definition using the builder pattern
    let register_result = ecs_client
        .register_task_definition()
        .family(task_def.family().unwrap_or_default())
        .set_task_role_arn(task_def.task_role_arn().map(|s| s.to_string()))
        .set_execution_role_arn(task_def.execution_role_arn().map(|s| s.to_string()))
        .set_network_mode(task_def.network_mode().cloned())
        .set_container_definitions(Some(task_def.container_definitions().to_vec()))
        .set_volumes(Some(task_def.volumes().to_vec()))
        .set_requires_compatibilities(Some(task_def.requires_compatibilities().to_vec()))
        .set_cpu(task_def.cpu().map(|s| s.to_string()))
        .set_memory(task_def.memory().map(|s| s.to_string()))
        .send()
        .await?;

    let new_revision = register_result
        .task_definition()
        .ok_or_else(|| anyhow!("No task definition returned"))?
        .revision();
    Ok(new_revision)
}

/// Fetch the latest task definition
async fn fetch_latest_task_definition(
    ecs_client: &EcsClient,
    task_def_name: &str,
) -> Result<TaskDefinition> {
    let task_def_output =
        ecs_client.describe_task_definition().task_definition(task_def_name).send().await?;
    let task_def =
        task_def_output.task_definition().ok_or_else(|| anyhow!("No task definition found"))?;
    Ok(task_def.clone())
}

/// Modify the container in the task definition to use the new image
fn set_container_image(task_def: &mut TaskDefinition, new_image_uri: &str) -> Result<()> {
    // Create a new container definition with updated image
    let containers = task_def.container_definitions.as_mut().expect("no container definitions");
    let n_containers = containers.len();
    if n_containers != 1 {
        anyhow::bail!("Expected 1 container definition, got {n_containers}");
    }

    let container = &mut containers[0];
    container.image = Some(new_image_uri.to_string());
    Ok(())
}

/// Deploy the new ECS revision by updating the service
async fn deploy_ecs_revision(
    ecs_client: &EcsClient,
    cluster_name: &str,
    service_name: &str,
    task_def_name: &str,
    revision: i32,
) -> Result<()> {
    println!("Updating ECS service...");
    ecs_client
        .update_service()
        .cluster(cluster_name)
        .service(service_name)
        .task_definition(format!("{task_def_name}:{revision}"))
        .send()
        .await?;

    println!("ECS cluster updated to new revision");
    Ok(())
}
