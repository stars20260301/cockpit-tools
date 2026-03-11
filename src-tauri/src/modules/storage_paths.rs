use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const LEGACY_DATA_DIR: &str = ".antigravity_cockpit";
const INSTANCES_DIR: &str = "instances";
const LOCAL_APP_DATA_DIR: &str = "com.antigravity.cockpit-tools";

#[cfg(target_os = "windows")]
const WINDOWS_BOOTSTRAP_DIR: &str = "AntigravityCockpit";
#[cfg(target_os = "windows")]
const WINDOWS_BOOTSTRAP_FILE: &str = "storage-bootstrap.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageBootstrap {
    #[serde(default)]
    pub app_data_dir: Option<String>,
    #[serde(default)]
    pub instance_base_dir: Option<String>,
    #[serde(default)]
    pub local_app_data_dir: Option<String>,
}

fn normalize_absolute_path(raw: Option<&str>) -> Option<PathBuf> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return None;
    }
    Some(path)
}

fn ensure_dir(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        fs::create_dir_all(path).map_err(|e| format!("创建目录失败 ({}): {}", path.display(), e))?;
    }
    Ok(path.to_path_buf())
}

pub fn is_windows_storage_supported() -> bool {
    cfg!(target_os = "windows")
}

pub fn legacy_app_data_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录".to_string())?;
    Ok(home.join(LEGACY_DATA_DIR))
}

pub fn legacy_instance_base_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join(format!("{LEGACY_DATA_DIR}\\{INSTANCES_DIR}")));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录".to_string())?;
        Ok(home.join(LEGACY_DATA_DIR).join(INSTANCES_DIR))
    }
}

pub fn legacy_local_app_data_dir() -> Result<PathBuf, String> {
    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .map(|path| path.join(LOCAL_APP_DATA_DIR))
        .ok_or_else(|| "无法获取本地应用数据目录".to_string())
}

#[cfg(target_os = "windows")]
fn bootstrap_file_path() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or("无法获取配置目录".to_string())?;
    Ok(base.join(WINDOWS_BOOTSTRAP_DIR).join(WINDOWS_BOOTSTRAP_FILE))
}

#[cfg(not(target_os = "windows"))]
fn bootstrap_file_path() -> Result<PathBuf, String> {
    Err("仅 Windows 支持存储路径引导配置".to_string())
}

pub fn load_bootstrap() -> Result<StorageBootstrap, String> {
    if !cfg!(target_os = "windows") {
        return Ok(StorageBootstrap::default());
    }

    let path = bootstrap_file_path()?;
    if !path.exists() {
        return Ok(StorageBootstrap::default());
    }

    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取存储引导配置失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(StorageBootstrap::default());
    }

    serde_json::from_str(&content).map_err(|e| format!("解析存储引导配置失败: {}", e))
}

pub fn save_bootstrap(bootstrap: &StorageBootstrap) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Ok(());
    }

    let path = bootstrap_file_path()?;
    let parent = path.parent().ok_or("无法获取存储引导配置目录".to_string())?;
    if !parent.exists() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建存储引导配置目录失败: {}", e))?;
    }

    let content = serde_json::to_string_pretty(bootstrap)
        .map_err(|e| format!("序列化存储引导配置失败: {}", e))?;
    fs::write(path, content).map_err(|e| format!("写入存储引导配置失败: {}", e))
}

pub fn get_app_data_dir() -> Result<PathBuf, String> {
    if cfg!(target_os = "windows") {
        let bootstrap = load_bootstrap()?;
        if let Some(custom) = normalize_absolute_path(bootstrap.app_data_dir.as_deref()) {
            return ensure_dir(&custom);
        }
    }
    ensure_dir(&legacy_app_data_dir()?)
}

pub fn get_instance_base_dir() -> Result<PathBuf, String> {
    if cfg!(target_os = "windows") {
        let bootstrap = load_bootstrap()?;
        if let Some(custom) = normalize_absolute_path(bootstrap.instance_base_dir.as_deref()) {
            return ensure_dir(&custom);
        }
    }
    ensure_dir(&legacy_instance_base_dir()?)
}

pub fn get_local_app_data_dir() -> Result<PathBuf, String> {
    if cfg!(target_os = "windows") {
        let bootstrap = load_bootstrap()?;
        if let Some(custom) = normalize_absolute_path(bootstrap.local_app_data_dir.as_deref()) {
            return ensure_dir(&custom);
        }
    }
    ensure_dir(&legacy_local_app_data_dir()?)
}

pub fn get_instance_root_dir(platform: &str) -> Result<PathBuf, String> {
    ensure_dir(&get_instance_base_dir()?.join(platform))
}

pub fn set_app_data_dir(path: Option<&Path>) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Ok(());
    }

    let mut bootstrap = load_bootstrap()?;
    bootstrap.app_data_dir = path.map(|value| value.to_string_lossy().to_string());
    save_bootstrap(&bootstrap)
}

pub fn set_instance_base_dir(path: Option<&Path>) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Ok(());
    }

    let mut bootstrap = load_bootstrap()?;
    bootstrap.instance_base_dir = path.map(|value| value.to_string_lossy().to_string());
    save_bootstrap(&bootstrap)
}

pub fn set_local_app_data_dir(path: Option<&Path>) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Ok(());
    }

    let mut bootstrap = load_bootstrap()?;
    bootstrap.local_app_data_dir = path.map(|value| value.to_string_lossy().to_string());
    save_bootstrap(&bootstrap)
}

pub fn build_target_app_data_dir(target_root: &Path) -> PathBuf {
    target_root.join("app-data")
}

pub fn build_target_instance_base_dir(target_root: &Path) -> PathBuf {
    target_root.join(INSTANCES_DIR)
}

pub fn build_target_local_app_data_dir(target_root: &Path) -> PathBuf {
    target_root.join("local-app-data")
}
