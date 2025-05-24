use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// The filename of the deployment config
const CONFIG_FILENAME: &str = "deploy-config.toml";

/// Default region for AWS operations
fn default_region() -> String {
    "us-east-2".to_string()
}

/// The deployment config in the repo
#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    /// The list of services available
    pub services: HashMap<String, ServiceConfig>,
}

/// The configuration for a service
#[derive(Deserialize, Debug, Clone)]
pub struct ServiceConfig {
    /// The build configuration for the service
    pub build: BuildConfig,
    /// The deploy configuration for the service
    pub deploy: DeployConfig,
}

/// The build configuration for a service
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BuildConfig {
    pub dockerfile: String,
    pub ecr_repo: String,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default)]
    pub cargo_features: String,
}

/// The deploy configuration for a service
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeployConfig {
    pub environment: String,
    pub resource: String,
    #[serde(default = "default_region")]
    pub region: String,
}

impl Config {
    /// Load the config from the expected location
    pub fn load() -> Result<Self> {
        // Find the config
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let config_path = Path::new(manifest_dir).join(CONFIG_FILENAME);
        if !config_path.exists() {
            return Err(anyhow!(
                "Could not find {CONFIG_FILENAME} at expected location: {}",
                config_path.display()
            ));
        }

        // Read the config
        let config_str = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow!("Failed to read config file {}: {}", config_path.display(), e))?;
        let config: Config = toml::from_str(&config_str)
            .map_err(|e| anyhow!("Failed to parse config file {}: {}", config_path.display(), e))?;
        Ok(config)
    }

    /// Get the service config for a given service name
    pub fn get_service(&self, name: &str) -> Result<ServiceConfig> {
        self.services
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("Service '{name}' not found in config"))
    }

    /// List all service names
    pub fn list_services(&self) -> Vec<&String> {
        let mut services: Vec<&String> = self.services.keys().collect();
        services.sort();
        services
    }
}
