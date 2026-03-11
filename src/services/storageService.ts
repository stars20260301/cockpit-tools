import { invoke } from '@tauri-apps/api/core';

export interface StorageState {
  supported: boolean;
  appDataPath: string;
  legacyAppDataPath: string;
  appDataCustomized: boolean;
  localAppDataPath: string;
  legacyLocalAppDataPath: string;
  localAppDataCustomized: boolean;
  instanceBaseDir: string;
  legacyInstanceBaseDir: string;
  instanceBaseCustomized: boolean;
}

export interface StoragePathStat {
  key: string;
  path: string;
  exists: boolean;
  sizeBytes: number;
  itemCount: number;
}

export interface ManagedInstanceStat {
  platform: string;
  instanceId: string;
  name: string;
  userDataDir: string;
  sizeBytes: number;
  itemCount: number;
}

export interface StorageOverview {
  supported: boolean;
  scannedAt: number;
  totalBytes: number;
  appDataSizeBytes: number;
  instancesTotalBytes: number;
  state: StorageState;
  pathStats: StoragePathStat[];
  managedInstances: ManagedInstanceStat[];
  notes: string[];
}

export interface StorageMigrationResult {
  appDataPath: string;
  instanceBaseDir: string;
  migratedAppData: boolean;
  migratedInstances: number;
  requiresRestart: boolean;
  notes: string[];
}

export async function getStorageState(): Promise<StorageState> {
  return await invoke('storage_get_state');
}

export async function getStorageOverview(): Promise<StorageOverview> {
  return await invoke('storage_get_overview');
}

export async function setDefaultInstanceBaseDir(path: string | null): Promise<StorageState> {
  return await invoke('storage_set_default_instance_base_dir', { path });
}

export async function migrateStorage(
  targetRoot: string,
  migrateAppData: boolean,
  migrateInstances: boolean,
): Promise<StorageMigrationResult> {
  return await invoke('storage_migrate', {
    targetRoot,
    migrateAppData,
    migrateInstances,
  });
}

export async function openFolder(path: string): Promise<void> {
  return await invoke('open_folder', { path });
}
