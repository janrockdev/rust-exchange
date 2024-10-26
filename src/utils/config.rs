use std::fs::File;
use serde_yaml::from_reader;
use std::error::Error;

use crate::models::model::models::Config;

pub fn load_config() -> Result<Config, Box<dyn Error>> {
    let file = File::open("config.yaml")?;  // Open the YAML file
    let config: Config = from_reader(file)?; // Parse the YAML file into the Config struct
    Ok(config)
}