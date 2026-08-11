use std::path::PathBuf;

use crate::tenant::org::fs::organization_path;

/// Returns the writeable path for a specific user
pub fn user_path(organization_id: Option<&str>, user_id: &str) -> PathBuf {
    if let Some(organization_id) = organization_id {
        let org_dir = organization_path(organization_id);
        let user_dir = org_dir.join("users").join(user_id);

        if !user_dir.exists() {
            std::fs::create_dir_all(&user_dir)
                .unwrap_or_else(|_| panic!("Failed to create user directory: {:?}", user_dir));
        }

        user_dir
    } else {
        let user_dir = organization_path(user_id);

        if !user_dir.exists() {
            std::fs::create_dir_all(&user_dir)
                .unwrap_or_else(|_| panic!("Failed to create user directory: {:?}", user_dir));
        }

        user_dir
    }
}
