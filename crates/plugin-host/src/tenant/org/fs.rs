use std::path::PathBuf;

/// Returns the writeable path for an organization
pub fn organization_path(organization_id: &str) -> PathBuf {
    let base_path = match home::home_dir() {
        Some(path) if !path.as_os_str().is_empty() => path,
        _ => std::env::current_dir().expect("Failed to get current directory"),
    };

    let org_dir = base_path
        .join(".concordance")
        .join("organizations")
        .join(organization_id);

    if !org_dir.exists() {
        std::fs::create_dir_all(&org_dir)
            .unwrap_or_else(|_| panic!("Failed to create organization directory: {:?}", org_dir));
    }

    org_dir
}

/// Returns the shared path for organization-wide resources
pub fn shared_path(organization_id: &str) -> PathBuf {
    let org_dir = organization_path(organization_id);
    let shared_dir = org_dir.join("shared");

    if !shared_dir.exists() {
        std::fs::create_dir_all(&shared_dir)
            .unwrap_or_else(|_| panic!("Failed to create shared directory: {:?}", shared_dir));
    }

    shared_dir
}

/// Returns the shared path for organization-wide resources
pub fn vecdb_path(organization_id: &str) -> PathBuf {
    let org_dir = organization_path(organization_id);
    let vector_dbs_dir = org_dir.join("vector_dbs");

    if !vector_dbs_dir.exists() {
        std::fs::create_dir_all(&vector_dbs_dir).unwrap_or_else(|_| {
            panic!(
                "Failed to create vector databases directory: {:?}",
                vector_dbs_dir
            )
        });
    }

    vector_dbs_dir
}
