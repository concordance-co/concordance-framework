pub mod types;
pub mod utils;

wit_bindgen::generate!({
    world: "host-types",
    path: "../../wit",
    additional_derives: [
        serde::Serialize,
        serde::Deserialize,
        Clone,
        PartialEq,
    ],
});

use crate::plugin::injector::env::env_var;

pub trait TryFromEnvVar: for<'de> serde::Deserialize<'de> {
    fn try_from_env_var(var_name: &str) -> Result<Self, String> {
        let var_str = env_var(var_name)
            .map_err(|e| format!("Unable to retrieve environment variable: {}", e))?;
        let var = serde_json::from_str::<Self>(&var_str)
            .map_err(|e| format!("Unable to parse: {}", e))?;
        Ok(var)
    }
}

impl<T: for<'de> serde::Deserialize<'de>> TryFromEnvVar for T {}
