use crate::modules::storage_manager;

#[tauri::command]
pub async fn storage_get_state() -> Result<storage_manager::StorageState, String> {
    storage_manager::get_storage_state()
}

#[tauri::command]
pub async fn storage_get_overview() -> Result<storage_manager::StorageOverview, String> {
    storage_manager::collect_storage_overview()
}

#[tauri::command]
pub async fn storage_set_default_instance_base_dir(
    path: Option<String>,
) -> Result<storage_manager::StorageState, String> {
    storage_manager::set_default_instance_base_dir(path)
}

#[tauri::command]
pub async fn storage_migrate(
    target_root: String,
    migrate_app_data: bool,
    migrate_instances: bool,
) -> Result<storage_manager::StorageMigrationResult, String> {
    storage_manager::migrate_storage(storage_manager::StorageMigrationRequest {
        target_root,
        migrate_app_data,
        migrate_instances,
    })
}
