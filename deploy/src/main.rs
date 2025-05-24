use anyhow::Result;
use clap::{Parser, Subcommand};
use dialoguer::Select;

mod build;
mod config;
mod deploy;
mod helpers;

use build::build_and_push;
use config::Config;
use deploy::deploy;

// -------
// | Cli |
// -------

#[derive(Parser)]
#[command(name = "deploy")]
#[command(about = "A CLI tool for building and deploying Docker images to AWS ECS")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build and push Docker image to ECR
    Build {
        /// Service name from config (optional - will prompt if not provided)
        #[arg(long)]
        service: Option<String>,
    },
    /// Deploy to ECS
    Deploy {
        /// Service name from config (optional - will prompt if not provided)
        #[arg(long)]
        service: Option<String>,
        /// Image tag (defaults to current git commit hash)
        #[arg(long)]
        image_tag: Option<String>,
    },
    /// List available services
    List,
}

// --------------
// | Helpers    |
// --------------

fn prompt_for_service(config: &Config) -> Result<String> {
    let services: Vec<String> = config.list_services().into_iter().cloned().collect();
    if services.is_empty() {
        return Err(anyhow::anyhow!("No services found in config"));
    }

    println!("Available services:");
    let selection = Select::new().with_prompt("Select a service").items(&services).interact()?;
    Ok(services[selection].clone())
}

fn get_service_name(config: &Config, service_arg: Option<String>) -> Result<String> {
    match service_arg {
        Some(service) => {
            // Validate that the service exists
            config.get_service(&service)?;
            Ok(service)
        },
        None => prompt_for_service(config),
    }
}

// --------------
// | Entrypoint |
// --------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;

    match cli.command {
        Commands::Build { service } => {
            let service_name = get_service_name(&config, service)?;
            let service_config = config.get_service(&service_name)?;
            let build_config = service_config.build;

            println!("Building service: {}", service_name);
            build_and_push(
                build_config.dockerfile,
                build_config.ecr_repo,
                build_config.region,
                build_config.cargo_features,
            )
            .await?;
        },
        Commands::Deploy { service, image_tag } => {
            let service_name = get_service_name(&config, service)?;
            let service_config = config.get_service(&service_name)?;
            let deploy_config = service_config.deploy;

            println!("Deploying service: {}", service_name);
            deploy(
                deploy_config.environment,
                deploy_config.resource,
                deploy_config.region,
                image_tag,
            )
            .await?;
        },
        Commands::List => {
            println!("Available services:");
            for service_name in config.list_services() {
                println!("  - {}", service_name);
            }
        },
    }

    Ok(())
}
