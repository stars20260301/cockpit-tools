use crate::models::{InstanceProfile, InstanceStore};
use crate::modules;
use crate::modules::instance_store;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATION_CLOSE_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoragePathStat {
    pub key: String,
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
    pub item_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedInstanceStat {
    pub platform: String,
    pub instance_id: String,
    pub name: String,
    pub user_data_dir: String,
    pub size_bytes: u64,
    pub item_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageState {
    pub supported: bool,
    pub app_data_path: String,
    pub legacy_app_data_path: String,
    pub app_data_customized: bool,
    pub local_app_data_path: String,
    pub legacy_local_app_data_path: String,
    pub local_app_data_customized: bool,
    pub instance_base_dir: String,
    pub legacy_instance_base_dir: String,
    pub instance_base_customized: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageOverview {
    pub supported: bool,
    pub scanned_at: i64,
    pub total_bytes: u64,
    pub app_data_size_bytes: u64,
    pub instances_total_bytes: u64,
    pub state: StorageState,
    pub path_stats: Vec<StoragePathStat>,
    pub managed_instances: Vec<ManagedInstanceStat>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageMigrationRequest {
    pub target_root: String,
    pub migrate_app_data: bool,
    pub migrate_instances: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageMigrationResult {
    pub app_data_path: String,
    pub instance_base_dir: String,
    pub migrated_app_data: bool,
    pub migrated_instances: usize,
    pub requires_restart: bool,
    pub notes: Vec<String>,
}

struct PlatformInstanceHandle {
    key: &'static str,
    load: fn() -> Result<InstanceStore, String>,
    save: fn(&InstanceStore) -> Result<(), String>,
    close: fn(&[String], u64) -> Result<(), String>,
    root_dir: fn() -> Result<PathBuf, String>,
}

fn close_noop(_user_data_dirs: &[String], _timeout_secs: u64) -> Result<(), String> {
    Ok(())
}

fn close_codebuddy_instances_for_migration(
    user_data_dirs: &[String],
    timeout_secs: u64,
) -> Result<(), String> {
    for user_data_dir in user_data_dirs {
        if let Some(pid) = modules::process::resolve_codebuddy_pid(None, Some(user_data_dir)) {
            let _ = modules::process::close_pid(pid, timeout_secs);
        }
    }
    let _ = modules::codebuddy_instance::clear_all_pids();
    Ok(())
}

struct LoadedPlatformStore {
    handle: &'static PlatformInstanceHandle,
    store: InstanceStore,
}

fn managed_platforms() -> &'static [PlatformInstanceHandle] {
    &[
        PlatformInstanceHandle {
            key: "antigravity",
            load: modules::instance::load_instance_store,
            save: modules::instance::save_instance_store,
            close: modules::process::close_antigravity_instances,
            root_dir: modules::instance::get_default_instances_root_dir,
        },
        PlatformInstanceHandle {
            key: "github_copilot",
            load: modules::github_copilot_instance::load_instance_store,
            save: modules::github_copilot_instance::save_instance_store,
            close: modules::process::close_vscode,
            root_dir: modules::github_copilot_instance::get_default_instances_root_dir,
        },
        PlatformInstanceHandle {
            key: "windsurf",
            load: modules::windsurf_instance::load_instance_store,
            save: modules::windsurf_instance::save_instance_store,
            close: modules::windsurf_instance::close_windsurf,
            root_dir: modules::windsurf_instance::get_default_instances_root_dir,
        },
        PlatformInstanceHandle {
            key: "kiro",
            load: modules::kiro_instance::load_instance_store,
            save: modules::kiro_instance::save_instance_store,
            close: modules::kiro_instance::close_kiro,
            root_dir: modules::kiro_instance::get_default_instances_root_dir,
        },
        PlatformInstanceHandle {
            key: "cursor",
            load: modules::cursor_instance::load_instance_store,
            save: modules::cursor_instance::save_instance_store,
            close: modules::cursor_instance::close_cursor,
            root_dir: modules::cursor_instance::get_default_instances_root_dir,
        },
        PlatformInstanceHandle {
            key: "gemini",
            load: modules::gemini_instance::load_instance_store,
            save: modules::gemini_instance::save_instance_store,
            close: close_noop,
            root_dir: modules::gemini_instance::get_default_instances_root_dir,
        },
        PlatformInstanceHandle {
            key: "codebuddy",
            load: modules::codebuddy_instance::load_instance_store,
            save: modules::codebuddy_instance::save_instance_store,
            close: close_codebuddy_instances_for_migration,
            root_dir: modules::codebuddy_instance::get_default_instances_root_dir,
        },
    ]
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

fn measure_path(path: &Path) -> (u64, u64) {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(_) => return (0, 0),
    };

    if metadata.is_file() {
        return (metadata.len(), 1);
    }

    if !metadata.is_dir() {
        return (0, 0);
    }

    let mut total_size = 0u64;
    let mut total_items = 0u64;
    let read_dir = match fs::read_dir(path) {
        Ok(iter) => iter,
        Err(_) => return (0, 0),
    };

    for entry in read_dir.flatten() {
        let child = entry.path();
        let (child_size, child_items) = measure_path(&child);
        total_size = total_size.saturating_add(child_size);
        total_items = total_items.saturating_add(child_items.saturating_add(1));
    }

    (total_size, total_items)
}

fn path_stat(key: &str, path: &Path) -> StoragePathStat {
    let exists = path.exists();
    let (size_bytes, item_count) = if exists { measure_path(path) } else { (0, 0) };
    StoragePathStat {
        key: key.to_string(),
        path: path.to_string_lossy().to_string(),
        exists,
        size_bytes,
        item_count,
    }
}

fn build_state() -> Result<StorageState, String> {
    let app_data = modules::storage_paths::get_app_data_dir()?;
    let legacy_app_data = modules::storage_paths::legacy_app_data_dir()?;
    let local_app_data = modules::storage_paths::get_local_app_data_dir()?;
    let legacy_local_app_data = modules::storage_paths::legacy_local_app_data_dir()?;
    let instance_base = modules::storage_paths::get_instance_base_dir()?;
    let legacy_instance_base = modules::storage_paths::legacy_instance_base_dir()?;

    Ok(StorageState {
        supported: modules::storage_paths::is_windows_storage_supported(),
        app_data_path: app_data.to_string_lossy().to_string(),
        legacy_app_data_path: legacy_app_data.to_string_lossy().to_string(),
        app_data_customized: app_data != legacy_app_data,
        local_app_data_path: local_app_data.to_string_lossy().to_string(),
        legacy_local_app_data_path: legacy_local_app_data.to_string_lossy().to_string(),
        local_app_data_customized: local_app_data != legacy_local_app_data,
        instance_base_dir: instance_base.to_string_lossy().to_string(),
        legacy_instance_base_dir: legacy_instance_base.to_string_lossy().to_string(),
        instance_base_customized: instance_base != legacy_instance_base,
    })
}

fn load_managed_platform_stores() -> Result<Vec<LoadedPlatformStore>, String> {
    let mut stores = Vec::new();
    for handle in managed_platforms() {
        stores.push(LoadedPlatformStore {
            handle,
            store: (handle.load)()?,
        });
    }
    Ok(stores)
}

fn sanitize_target_root(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("目标目录不能为空".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("目标目录必须是绝对路径".to_string());
    }
    Ok(path)
}

fn validate_target_root(
    target_root: &Path,
    current_app_data: &Path,
    current_local_app_data: &Path,
) -> Result<(), String> {
    if target_root == current_app_data || target_root.starts_with(current_app_data) {
        return Err("目标目录不能位于当前应用数据目录内部".to_string());
    }
    if target_root == current_local_app_data || target_root.starts_with(current_local_app_data) {
        return Err("目标目录不能位于当前本地应用数据目录内部".to_string());
    }
    Ok(())
}

fn ensure_empty_or_missing(path: &Path, label: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        return Err(format!("{label} 不是目录: {}", path.display()));
    }
    let has_entries = fs::read_dir(path)
        .map_err(|e| format!("读取 {label} 失败: {}", e))?
        .next()
        .is_some();
    if has_entries {
        return Err(format!("{label} 已存在且不为空: {}", path.display()));
    }
    Ok(())
}

fn save_runtime_config_to_current_dir() -> Result<(), String> {
    let current = modules::config::get_user_config();
    modules::config::save_user_config(&current)?;
    if let Some(actual_port) = modules::config::get_actual_port() {
        let status = modules::config::ServerStatus {
            ws_port: actual_port,
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
            started_at: chrono::Utc::now().timestamp(),
        };
        let _ = modules::config::save_server_status(&status);
    }
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        }
    }
    Ok(())
}

fn choose_target_instance_dir(
    platform_key: &str,
    instance: &InstanceProfile,
    target_base: &Path,
    used_targets: &mut HashSet<String>,
) -> PathBuf {
    let current = PathBuf::from(&instance.user_data_dir);
    let default_leaf = current
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| instance.id.clone());

    let mut leaf = default_leaf.clone();
    let mut candidate = target_base.join(platform_key).join(&leaf);
    let mut index = 1usize;
    while !used_targets.insert(candidate.to_string_lossy().to_ascii_lowercase()) {
        leaf = format!("{}-{}", default_leaf, index);
        candidate = target_base.join(platform_key).join(&leaf);
        index += 1;
    }
    candidate
}

fn migrate_instance_store(
    loaded: &mut LoadedPlatformStore,
    target_instance_base: &Path,
) -> Result<usize, String> {
    let user_dirs = loaded
        .store
        .instances
        .iter()
        .map(|value| value.user_data_dir.clone())
        .collect::<Vec<_>>();
    if !user_dirs.is_empty() {
        (loaded.handle.close)(&user_dirs, MIGRATION_CLOSE_TIMEOUT_SECS)?;
    }

    let mut migrated = 0usize;
    let mut used_targets = HashSet::new();
    for instance in &mut loaded.store.instances {
        let current_path = PathBuf::from(&instance.user_data_dir);
        if !current_path.exists() {
            continue;
        }

        let target_path =
            choose_target_instance_dir(loaded.handle.key, instance, target_instance_base, &mut used_targets);
        if target_path == current_path {
            continue;
        }

        ensure_parent_dir(&target_path)?;
        ensure_empty_or_missing(&target_path, "目标实例目录")?;
        instance_store::copy_dir_recursive(&current_path, &target_path)?;
        if current_path != target_path {
            let _ = fs::remove_dir_all(&current_path);
        }
        instance.user_data_dir = target_path.to_string_lossy().to_string();
        migrated += 1;
    }

    (loaded.handle.save)(&loaded.store)?;
    Ok(migrated)
}

pub fn get_storage_state() -> Result<StorageState, String> {
    build_state()
}

pub fn set_default_instance_base_dir(path: Option<String>) -> Result<StorageState, String> {
    if !modules::storage_paths::is_windows_storage_supported() {
        return build_state();
    }

    if let Some(raw) = path.as_deref() {
        let target = sanitize_target_root(raw)?;
        fs::create_dir_all(&target).map_err(|e| format!("创建实例默认目录失败: {}", e))?;
        modules::storage_paths::set_instance_base_dir(Some(&target))?;
    } else {
        modules::storage_paths::set_instance_base_dir(None)?;
    }

    build_state()
}

pub fn collect_storage_overview() -> Result<StorageOverview, String> {
    let state = build_state()?;
    let app_data_path = PathBuf::from(&state.app_data_path);
    let local_app_data_path = PathBuf::from(&state.local_app_data_path);
    let app_data_stat = path_stat("appData", &app_data_path);
    let local_app_data_stat = path_stat("localAppData", &local_app_data_path);

    let mut path_stats = vec![app_data_stat.clone(), local_app_data_stat.clone()];
    let mut managed_instances = Vec::new();
    let mut instances_total_bytes = 0u64;
    let stores = load_managed_platform_stores()?;

    for loaded in stores {
        let root_dir = (loaded.handle.root_dir)()?;
        let root_stat = path_stat(&format!("instanceRoot:{}", loaded.handle.key), &root_dir);
        instances_total_bytes = instances_total_bytes.saturating_add(root_stat.size_bytes);
        path_stats.push(root_stat);

        for instance in loaded.store.instances {
            let path = PathBuf::from(&instance.user_data_dir);
            let (size_bytes, item_count) = if path.exists() {
                measure_path(&path)
            } else {
                (0, 0)
            };
            managed_instances.push(ManagedInstanceStat {
                platform: loaded.handle.key.to_string(),
                instance_id: instance.id,
                name: instance.name,
                user_data_dir: instance.user_data_dir,
                size_bytes,
                item_count,
            });
        }
    }

    managed_instances.sort_by(|a, b| {
        b.size_bytes
            .cmp(&a.size_bytes)
            .then_with(|| a.platform.cmp(&b.platform))
            .then_with(|| a.name.cmp(&b.name))
    });
    path_stats.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| a.key.cmp(&b.key)));

    let mut notes = Vec::new();
    if !state.supported {
        notes.push("当前版本的目录迁移仅在 Windows 上可用。".to_string());
    } else {
        notes.push("当前页会统计并迁移 Antigravity 管理的应用数据目录、本地私有数据目录与多开实例目录。".to_string());
        notes.push("官方客户端自身的默认目录不会在此页自动迁移。".to_string());
    }

    Ok(StorageOverview {
        supported: state.supported,
        scanned_at: now_ts(),
        total_bytes: app_data_stat
            .size_bytes
            .saturating_add(local_app_data_stat.size_bytes)
            .saturating_add(instances_total_bytes),
        app_data_size_bytes: app_data_stat
            .size_bytes
            .saturating_add(local_app_data_stat.size_bytes),
        instances_total_bytes,
        state,
        path_stats,
        managed_instances,
        notes,
    })
}

pub fn migrate_storage(request: StorageMigrationRequest) -> Result<StorageMigrationResult, String> {
    if !modules::storage_paths::is_windows_storage_supported() {
        return Err("当前平台暂不支持此迁移功能".to_string());
    }
    if !request.migrate_app_data && !request.migrate_instances {
        return Err("至少需要选择一项迁移范围".to_string());
    }

    let current_app_data = modules::storage_paths::get_app_data_dir()?;
    let current_local_app_data = modules::storage_paths::get_local_app_data_dir()?;
    let target_root = sanitize_target_root(&request.target_root)?;
    validate_target_root(&target_root, &current_app_data, &current_local_app_data)?;
    if !target_root.exists() {
        fs::create_dir_all(&target_root).map_err(|e| format!("创建目标目录失败: {}", e))?;
    }

    let target_app_data = modules::storage_paths::build_target_app_data_dir(&target_root);
    let target_local_app_data = modules::storage_paths::build_target_local_app_data_dir(&target_root);
    let target_instance_base = modules::storage_paths::build_target_instance_base_dir(&target_root);

    let mut notes = Vec::new();
    let mut stores = load_managed_platform_stores()?;
    let mut migrated_instances = 0usize;
    let mut migrated_app_data = false;

    if request.migrate_app_data && target_app_data != current_app_data {
        ensure_empty_or_missing(&target_app_data, "目标应用数据目录")?;
        instance_store::copy_dir_recursive(&current_app_data, &target_app_data)?;
        modules::storage_paths::set_app_data_dir(Some(&target_app_data))?;
        save_runtime_config_to_current_dir()?;
        migrated_app_data = true;
        notes.push("应用数据目录已复制到新目录，旧目录保留作为回滚备份。".to_string());
    }

    if request.migrate_app_data && target_local_app_data != current_local_app_data {
        ensure_empty_or_missing(&target_local_app_data, "目标本地应用数据目录")?;
        instance_store::copy_dir_recursive(&current_local_app_data, &target_local_app_data)?;
        modules::storage_paths::set_local_app_data_dir(Some(&target_local_app_data))?;
        migrated_app_data = true;
        notes.push("本地应用私有数据目录已复制到新目录，旧目录保留作为回滚备份。".to_string());
    }

    if request.migrate_instances {
        for loaded in &mut stores {
            migrated_instances += migrate_instance_store(loaded, &target_instance_base)?;
        }
        modules::storage_paths::set_instance_base_dir(Some(&target_instance_base))?;
        notes.push("已更新多开实例记录中的实例目录路径。".to_string());
    }

    let state = build_state()?;
    Ok(StorageMigrationResult {
        app_data_path: state.app_data_path,
        instance_base_dir: state.instance_base_dir,
        migrated_app_data,
        migrated_instances,
        requires_restart: migrated_app_data,
        notes,
    })
}
