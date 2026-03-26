use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{types::Value, Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use url::Url;

use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const STATE_DB_FILE: &str = "state_5.sqlite";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const SESSION_TRASH_ROOT_DIR: &str = "cockpit-tools-codex-session-trash";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionLocation {
    pub instance_id: String,
    pub instance_name: String,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRecord {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: Option<i64>,
    pub location_count: usize,
    pub locations: Vec<CodexSessionLocation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionTrashSummary {
    pub requested_session_count: usize,
    pub trashed_session_count: usize,
    pub trashed_instance_count: usize,
    pub trash_dirs: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct CodexSyncInstance {
    id: String,
    name: String,
    data_dir: PathBuf,
    last_pid: Option<u32>,
}

#[derive(Debug, Clone)]
struct ThreadRowData {
    columns: Vec<String>,
    values: Vec<Value>,
}

impl ThreadRowData {
    fn get_value(&self, column: &str) -> Option<&Value> {
        self.columns
            .iter()
            .position(|item| item == column)
            .and_then(|index| self.values.get(index))
    }

    fn get_text(&self, column: &str) -> Option<String> {
        match self.get_value(column)? {
            Value::Text(value) => Some(value.clone()),
            Value::Integer(value) => Some(value.to_string()),
            Value::Real(value) => Some(value.to_string()),
            _ => None,
        }
    }

    fn get_i64(&self, column: &str) -> Option<i64> {
        match self.get_value(column)? {
            Value::Integer(value) => Some(*value),
            Value::Text(value) => value.parse::<i64>().ok(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ThreadSnapshot {
    id: String,
    title: String,
    cwd: String,
    updated_at: Option<i64>,
    rollout_path: PathBuf,
    row_data: ThreadRowData,
    session_index_entry: JsonValue,
    source_root: PathBuf,
}

pub fn list_sessions_across_instances() -> Result<Vec<CodexSessionRecord>, String> {
    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let mut session_map = HashMap::<String, CodexSessionRecord>::new();

    for instance in &instances {
        let running = is_instance_running(instance, &process_entries);
        for snapshot in load_thread_snapshots(instance)? {
            let entry = session_map
                .entry(snapshot.id.clone())
                .or_insert_with(|| CodexSessionRecord {
                    session_id: snapshot.id.clone(),
                    title: snapshot.title.clone(),
                    cwd: snapshot.cwd.clone(),
                    updated_at: snapshot.updated_at,
                    location_count: 0,
                    locations: Vec::new(),
                });

            if entry.updated_at.is_none() {
                entry.updated_at = snapshot.updated_at;
            }
            if entry.title.trim().is_empty() {
                entry.title = snapshot.title.clone();
            }
            if entry.cwd.trim().is_empty() {
                entry.cwd = snapshot.cwd.clone();
            }

            entry.locations.push(CodexSessionLocation {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                running,
            });
            entry.location_count = entry.locations.len();
        }
    }

    let mut sessions = session_map.into_values().collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .unwrap_or_default()
            .cmp(&left.updated_at.unwrap_or_default())
            .then_with(|| left.cwd.cmp(&right.cwd))
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(sessions)
}

pub fn move_sessions_to_trash_across_instances(
    session_ids: Vec<String>,
) -> Result<CodexSessionTrashSummary, String> {
    let requested_ids = session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }

    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let trash_root = create_trash_root_dir()?;
    let mut trashed_session_ids = HashSet::new();
    let mut trashed_instance_count = 0usize;
    let mut mutated_running_instance_count = 0usize;

    for instance in &instances {
        let snapshots = load_thread_snapshots(instance)?
            .into_iter()
            .filter(|snapshot| requested_ids.contains(&snapshot.id))
            .collect::<Vec<_>>();
        if snapshots.is_empty() {
            continue;
        }

        if is_instance_running(instance, &process_entries) {
            mutated_running_instance_count += 1;
        }

        trash_snapshots_for_instance(instance, &trash_root, &snapshots)?;
        trashed_instance_count += 1;
        for snapshot in snapshots {
            trashed_session_ids.insert(snapshot.id);
        }
    }

    if trashed_instance_count == 0 {
        return Ok(CodexSessionTrashSummary {
            requested_session_count: requested_ids.len(),
            trashed_session_count: 0,
            trashed_instance_count: 0,
            trash_dirs: Vec::new(),
            message: "所选会话在当前实例集合中不存在，无需处理".to_string(),
        });
    }

    let message = if mutated_running_instance_count > 0 {
        format!(
            "已将 {} 条会话移到废纸篓，运行中的实例可能需要重启后显示",
            trashed_session_ids.len()
        )
    } else {
        format!("已将 {} 条会话移到废纸篓", trashed_session_ids.len())
    };

    Ok(CodexSessionTrashSummary {
        requested_session_count: requested_ids.len(),
        trashed_session_count: trashed_session_ids.len(),
        trashed_instance_count,
        trash_dirs: vec![trash_root.to_string_lossy().to_string()],
        message,
    })
}

fn collect_instances() -> Result<Vec<CodexSyncInstance>, String> {
    let mut instances = Vec::new();
    let default_dir = modules::codex_instance::get_default_codex_home()?;
    let store = modules::codex_instance::load_instance_store()?;
    instances.push(CodexSyncInstance {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: DEFAULT_INSTANCE_NAME.to_string(),
        data_dir: default_dir,
        last_pid: store.default_settings.last_pid,
    });

    for instance in store.instances {
        let user_data_dir = instance.user_data_dir.trim();
        if user_data_dir.is_empty() {
            continue;
        }
        instances.push(CodexSyncInstance {
            id: instance.id,
            name: instance.name,
            data_dir: PathBuf::from(user_data_dir),
            last_pid: instance.last_pid,
        });
    }

    Ok(instances)
}

fn is_instance_running(instance: &CodexSyncInstance, process_entries: &[(u32, Option<String>)]) -> bool {
    let codex_home = if instance.id == DEFAULT_INSTANCE_ID {
        None
    } else {
        instance.data_dir.to_str()
    };
    modules::process::resolve_codex_pid_from_entries(instance.last_pid, codex_home, process_entries)
        .is_some()
}

fn load_thread_snapshots(instance: &CodexSyncInstance) -> Result<Vec<ThreadSnapshot>, String> {
    let db_path = instance.data_dir.join(STATE_DB_FILE);
    let connection = open_readonly_connection(&db_path)?;
    let columns = read_thread_columns(&connection)?;
    let select_columns = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!("SELECT {} FROM threads", select_columns);
    let mut statement = connection
        .prepare(&query)
        .map_err(|error| format!("读取实例会话失败 ({}): {}", instance.name, error))?;
    let mut rows = statement
        .query([])
        .map_err(|error| format!("查询实例会话失败 ({}): {}", instance.name, error))?;
    let session_index_map = read_session_index_map(&instance.data_dir)?;

    let mut snapshots = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("迭代实例会话失败 ({}): {}", instance.name, error))?
    {
        let mut values = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            values.push(
                row.get::<usize, Value>(index)
                    .map_err(|error| format!("解析会话记录失败 ({}): {}", instance.name, error))?,
            );
        }

        let row_data = ThreadRowData {
            columns: columns.clone(),
            values,
        };
        let id = row_data
            .get_text("id")
            .ok_or_else(|| format!("会话缺少 id 字段 ({})", instance.name))?;
        let rollout_path = row_data
            .get_text("rollout_path")
            .ok_or_else(|| format!("会话 {} 缺少 rollout_path ({})", id, instance.name))?;
        let title = row_data
            .get_text("title")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        let cwd = row_data
            .get_text("cwd")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "未知工作目录".to_string());
        let updated_at = row_data.get_i64("updated_at");
        let session_index_entry = session_index_map
            .get(&id)
            .cloned()
            .unwrap_or_else(|| json!({ "id": id, "thread_name": title }));

        snapshots.push(ThreadSnapshot {
            id,
            title,
            cwd,
            updated_at,
            rollout_path: PathBuf::from(rollout_path),
            row_data,
            session_index_entry,
            source_root: instance.data_dir.clone(),
        });
    }

    Ok(snapshots)
}

fn trash_snapshots_for_instance(
    instance: &CodexSyncInstance,
    trash_root: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<(), String> {
    for snapshot in snapshots {
        move_snapshot_rollout_to_trash(instance, trash_root, snapshot)?;
    }

    remove_threads_from_db(&instance.data_dir, snapshots)?;
    rewrite_session_index_without_ids(&instance.data_dir, snapshots)?;
    Ok(())
}

fn create_trash_root_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let root = home
        .join(".Trash")
        .join(SESSION_TRASH_ROOT_DIR)
        .join(Utc::now().format("%Y%m%d-%H%M%S").to_string());
    fs::create_dir_all(&root)
        .map_err(|error| format!("创建会话废纸篓目录失败 ({}): {}", root.display(), error))?;
    Ok(root)
}

fn move_snapshot_rollout_to_trash(
    instance: &CodexSyncInstance,
    trash_root: &Path,
    snapshot: &ThreadSnapshot,
) -> Result<(), String> {
    if !snapshot.rollout_path.exists() {
        return Ok(());
    }

    let relative_path = snapshot
        .rollout_path
        .strip_prefix(&snapshot.source_root)
        .unwrap_or(snapshot.rollout_path.as_path());
    let entry_dir = trash_root.join(format!(
        "{}--{}",
        sanitize_for_file_name(&instance.id),
        sanitize_for_file_name(&snapshot.id)
    ));
    let file_target = entry_dir.join("files").join(relative_path);
    if let Some(parent) = file_target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建废纸篓会话目录失败 ({}): {}", parent.display(), error))?;
    }

    let manifest = json!({
        "sessionId": snapshot.id,
        "title": snapshot.title,
        "cwd": snapshot.cwd,
        "instanceId": instance.id,
        "instanceName": instance.name,
        "instanceRoot": instance.data_dir,
        "originalRolloutPath": snapshot.rollout_path,
        "relativeRolloutPath": relative_path.to_string_lossy(),
        "sessionIndexEntry": snapshot.session_index_entry,
        "threadRow": serialize_row_data(&snapshot.row_data),
        "deletedAt": Utc::now().to_rfc3339(),
    });

    fs::create_dir_all(&entry_dir)
        .map_err(|error| format!("创建废纸篓条目失败 ({}): {}", entry_dir.display(), error))?;
    fs::rename(&snapshot.rollout_path, &file_target).map_err(|error| {
        format!(
            "移动会话文件到废纸篓失败 ({} -> {}): {}",
            snapshot.rollout_path.display(),
            file_target.display(),
            error
        )
    })?;
    fs::write(
        entry_dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("序列化会话废纸篓清单失败: {}", error))?
        ),
    )
    .map_err(|error| format!("写入会话废纸篓清单失败 ({}): {}", entry_dir.display(), error))?;
    Ok(())
}

fn remove_threads_from_db(root_dir: &Path, snapshots: &[ThreadSnapshot]) -> Result<(), String> {
    let db_path = root_dir.join(STATE_DB_FILE);
    let mut connection = Connection::open(&db_path)
        .map_err(|error| format!("打开实例数据库失败 ({}): {}", db_path.display(), error))?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开启会话删除事务失败 ({}): {}", db_path.display(), error))?;

    for snapshot in snapshots {
        transaction
            .execute("DELETE FROM threads WHERE id = ?1", [&snapshot.id])
            .map_err(|error| format!("删除会话记录失败 ({}): {}", snapshot.id, error))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("提交会话删除事务失败 ({}): {}", db_path.display(), error))?;
    Ok(())
}

fn rewrite_session_index_without_ids(root_dir: &Path, snapshots: &[ThreadSnapshot]) -> Result<(), String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    if !path.exists() {
        return Ok(());
    }

    let removed_ids = snapshots
        .iter()
        .map(|snapshot| snapshot.id.as_str())
        .collect::<HashSet<_>>();
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取 session_index.jsonl 失败 ({}): {}", path.display(), error))?;
    let retained = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return false;
            }
            match serde_json::from_str::<JsonValue>(trimmed) {
                Ok(value) => value
                    .get("id")
                    .and_then(JsonValue::as_str)
                    .map(|id| !removed_ids.contains(id))
                    .unwrap_or(true),
                Err(_) => true,
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let final_content = if retained.is_empty() {
        String::new()
    } else {
        format!("{}\n", retained)
    };
    fs::write(&path, final_content)
        .map_err(|error| format!("重写 session_index.jsonl 失败 ({}): {}", path.display(), error))?;
    Ok(())
}

fn read_session_index_map(root_dir: &Path) -> Result<HashMap<String, JsonValue>, String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取 session_index.jsonl 失败 ({}): {}", path.display(), error))?;
    let mut entries = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        let Some(id) = parsed.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        entries.insert(id.to_string(), parsed);
    }

    Ok(entries)
}

fn open_readonly_connection(db_path: &Path) -> Result<Connection, String> {
    let mut uri = Url::from_file_path(db_path)
        .map_err(|_| format!("无法构建只读数据库 URI: {}", db_path.display()))?;
    uri.set_query(Some("mode=ro"));
    Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| format!("打开只读数据库失败 ({}): {}", db_path.display(), error))
}

fn read_thread_columns(connection: &Connection) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(|error| format!("读取 threads 表结构失败: {}", error))?;
    let mut rows = statement
        .query([])
        .map_err(|error| format!("查询 threads 表结构失败: {}", error))?;
    let mut columns = Vec::new();

    while let Some(row) = rows
        .next()
        .map_err(|error| format!("解析 threads 表结构失败: {}", error))?
    {
        columns.push(
            row.get::<usize, String>(1)
                .map_err(|error| format!("解析 threads 列失败: {}", error))?,
        );
    }

    if columns.is_empty() {
        return Err("threads 表不存在或没有列定义".to_string());
    }

    Ok(columns)
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn serialize_row_data(row_data: &ThreadRowData) -> JsonValue {
    let mut object = serde_json::Map::new();
    for (column, value) in row_data.columns.iter().zip(row_data.values.iter()) {
        object.insert(column.clone(), sqlite_value_to_json(value));
    }
    JsonValue::Object(object)
}

fn sqlite_value_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Integer(number) => json!(number),
        Value::Real(number) => json!(number),
        Value::Text(text) => json!(text),
        Value::Blob(bytes) => json!(
            bytes
                .iter()
                .map(|byte| format!("{:02X}", byte))
                .collect::<String>()
        ),
    }
}

fn sanitize_for_file_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
}
