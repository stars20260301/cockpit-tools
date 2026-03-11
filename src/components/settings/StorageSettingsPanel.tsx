import { useCallback, useEffect, useMemo, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { ArrowRightLeft, FolderOpen, HardDrive, RefreshCw, Save } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  getStorageOverview,
  migrateStorage,
  openFolder,
  setDefaultInstanceBaseDir,
  type ManagedInstanceStat,
  type StorageOverview,
  type StoragePathStat,
} from '../../services/storageService';
import { getPlatformLabel } from '../../utils/platformMeta';
import type { PlatformId } from '../../types/platform';

const PLATFORM_KEY_MAP: Record<string, PlatformId> = {
  antigravity: 'antigravity',
  github_copilot: 'github-copilot',
  windsurf: 'windsurf',
  kiro: 'kiro',
  cursor: 'cursor',
  gemini: 'gemini',
  codebuddy: 'codebuddy',
};

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(value >= 100 || index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatScanTime(scannedAt: number): string {
  if (!Number.isFinite(scannedAt) || scannedAt <= 0) return '-';
  return new Date(scannedAt * 1000).toLocaleString();
}

function parentDir(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const next = normalized.replace(/[\\/][^\\/]+$/, '');
  return next || normalized;
}

function PathStatRow({
  stat,
  label,
  onOpen,
}: {
  stat: StoragePathStat;
  label: string;
  onOpen: (path: string) => void;
}) {
  return (
    <div className="storage-settings-item">
      <div className="storage-settings-item-main">
        <div className="storage-settings-item-title">{label}</div>
        <div className="storage-settings-item-path" title={stat.path}>{stat.path}</div>
      </div>
      <div className="storage-settings-item-meta">
        <span>{formatBytes(stat.sizeBytes)}</span>
        <button className="btn btn-secondary" onClick={() => onOpen(stat.path)}>
          <FolderOpen size={16} />
        </button>
      </div>
    </div>
  );
}

function ManagedInstanceRow({
  instance,
  onOpen,
}: {
  instance: ManagedInstanceStat;
  onOpen: (path: string) => void;
}) {
  return (
    <div className="storage-settings-item">
      <div className="storage-settings-item-main">
        <div className="storage-settings-item-title">{instance.name}</div>
        <div className="storage-settings-item-sub">
          <span>{instance.userDataDir}</span>
        </div>
      </div>
      <div className="storage-settings-item-meta">
        <span>{formatBytes(instance.sizeBytes)}</span>
        <button className="btn btn-secondary" onClick={() => onOpen(instance.userDataDir)}>
          <FolderOpen size={16} />
        </button>
      </div>
    </div>
  );
}

export function StorageSettingsPanel() {
  const { t } = useTranslation();
  const [overview, setOverview] = useState<StorageOverview | null>(null);
  const [loading, setLoading] = useState(false);
  const [savingDefault, setSavingDefault] = useState(false);
  const [migrating, setMigrating] = useState(false);
  const [message, setMessage] = useState<{ tone: 'success' | 'error'; text: string } | null>(null);
  const [defaultInstanceBase, setDefaultInstanceBase] = useState('');
  const [migrationTargetRoot, setMigrationTargetRoot] = useState('');
  const [migrateAppData, setMigrateAppData] = useState(true);
  const [migrateInstances, setMigrateInstances] = useState(true);

  const loadOverview = useCallback(async () => {
    setLoading(true);
    try {
      const data = await getStorageOverview();
      setOverview(data);
      setDefaultInstanceBase(data.state.instanceBaseDir);
      setMigrationTargetRoot((current) => current || parentDir(data.state.instanceBaseDir));
      setMessage(null);
    } catch (error) {
      setMessage({ tone: 'error', text: String(error) });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadOverview();
  }, [loadOverview]);

  const pathStats = useMemo(() => overview?.pathStats ?? [], [overview]);
  const managedInstances = useMemo(() => overview?.managedInstances ?? [], [overview]);

  const getPathLabel = (stat: StoragePathStat) => {
    if (stat.key === 'appData') {
      return t('settings.storage.currentAppData');
    }
    if (stat.key === 'localAppData') {
      return t('settings.storage.currentLocalAppData');
    }
    const platformKey = stat.key.split(':')[1];
    const platform = PLATFORM_KEY_MAP[platformKey];
    if (platform) {
      return t('settings.storage.instanceRootLabel', {
        platform: getPlatformLabel(platform, t),
      });
    }
    return stat.key;
  };

  const getInstanceTitle = (instance: ManagedInstanceStat) => {
    const platform = PLATFORM_KEY_MAP[instance.platform];
    if (!platform) return instance.name;
    return `${getPlatformLabel(platform, t)} · ${instance.name}`;
  };

  const handleOpenFolder = async (path: string) => {
    try {
      await openFolder(path);
    } catch (error) {
      setMessage({ tone: 'error', text: String(error) });
    }
  };

  const pickDirectory = async (setter: (value: string) => void, defaultPath?: string) => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath: defaultPath || undefined,
      });
      if (selected && typeof selected === 'string') {
        setter(selected);
      }
    } catch (error) {
      setMessage({ tone: 'error', text: String(error) });
    }
  };

  const handleSaveDefaultBase = async () => {
    setSavingDefault(true);
    try {
      await setDefaultInstanceBaseDir(defaultInstanceBase.trim() || null);
      await loadOverview();
      setMessage({ tone: 'success', text: t('settings.storage.saveDefaultSuccess') });
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('settings.storage.saveDefaultFailed', { error: String(error) }),
      });
    } finally {
      setSavingDefault(false);
    }
  };

  const handleResetDefaultBase = async () => {
    setSavingDefault(true);
    try {
      await setDefaultInstanceBaseDir(null);
      await loadOverview();
      setMessage({ tone: 'success', text: t('settings.storage.saveDefaultSuccess') });
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('settings.storage.saveDefaultFailed', { error: String(error) }),
      });
    } finally {
      setSavingDefault(false);
    }
  };

  const handleMigrate = async () => {
    if (!migrationTargetRoot.trim()) {
      setMessage({ tone: 'error', text: t('settings.storage.migrationTargetRequired') });
      return;
    }
    if (!window.confirm(t('settings.storage.confirmMigration'))) {
      return;
    }

    setMigrating(true);
    try {
      const result = await migrateStorage(
        migrationTargetRoot.trim(),
        migrateAppData,
        migrateInstances,
      );
      await loadOverview();
      const messageKey = result.requiresRestart
        ? 'settings.storage.migrationSuccessRestart'
        : 'settings.storage.migrationSuccess';
      setMessage({
        tone: 'success',
        text: t(messageKey, {
          appDataPath: result.appDataPath,
          instanceBaseDir: result.instanceBaseDir,
          count: result.migratedInstances,
        }),
      });
    } catch (error) {
      setMessage({
        tone: 'error',
        text: t('settings.storage.migrationFailed', { error: String(error) }),
      });
    } finally {
      setMigrating(false);
    }
  };

  return (
    <div className="settings-storage-panel">
      <div className="group-title">{t('settings.general.storageTitle')}</div>

      <div className="settings-group">
        <div className="settings-row settings-row--top">
          <div className="row-label">
            <div className="row-title">{t('settings.general.storageTitle')}</div>
            <div className="row-desc">{t('settings.storage.summaryDesc')}</div>
          </div>
          <div className="row-control">
            <button className="btn btn-secondary" onClick={() => void loadOverview()} disabled={loading}>
              <RefreshCw size={16} className={loading ? 'spin' : undefined} />
              {t('common.refresh')}
            </button>
          </div>
        </div>

        <div className="storage-settings-summary">
          <div className="storage-settings-summary-card">
            <div className="storage-settings-summary-label">{t('settings.storage.summaryAppData')}</div>
            <div className="storage-settings-summary-value">{formatBytes(overview?.appDataSizeBytes ?? 0)}</div>
          </div>
          <div className="storage-settings-summary-card">
            <div className="storage-settings-summary-label">{t('settings.storage.summaryInstances')}</div>
            <div className="storage-settings-summary-value">{formatBytes(overview?.instancesTotalBytes ?? 0)}</div>
          </div>
          <div className="storage-settings-summary-card">
            <div className="storage-settings-summary-label">{t('settings.storage.summaryManagedInstances')}</div>
            <div className="storage-settings-summary-value">{managedInstances.length}</div>
            <div className="storage-settings-summary-meta">
              {overview ? formatScanTime(overview.scannedAt) : '-'}
            </div>
          </div>
        </div>

        {!overview?.supported && (
          <div className="storage-settings-note">{t('settings.storage.windowsOnly')}</div>
        )}
      </div>

      <div className="group-title">{t('settings.storage.directorySection')}</div>
      <div className="settings-group">
        <div className="settings-row settings-row--top">
          <div className="row-label">
            <div className="row-title">{t('settings.storage.currentAppData')}</div>
            <div className="row-desc">{t('settings.storage.currentAppDataDesc')}</div>
          </div>
          <div className="row-control row-control--grow">
            <div className="storage-settings-inline">
              <input
                className="settings-input settings-input--path"
                value={overview?.state.appDataPath ?? ''}
                readOnly
              />
              <button
                className="btn btn-secondary"
                onClick={() => overview && handleOpenFolder(overview.state.appDataPath)}
                disabled={!overview}
              >
                <FolderOpen size={16} />
                {t('common.open')}
              </button>
            </div>
          </div>
        </div>

        <div className="settings-row settings-row--top">
          <div className="row-label">
            <div className="row-title">{t('settings.storage.currentLocalAppData')}</div>
            <div className="row-desc">{t('settings.storage.currentLocalAppDataDesc')}</div>
          </div>
          <div className="row-control row-control--grow">
            <div className="storage-settings-inline">
              <input
                className="settings-input settings-input--path"
                value={overview?.state.localAppDataPath ?? ''}
                readOnly
              />
              <button
                className="btn btn-secondary"
                onClick={() => overview && handleOpenFolder(overview.state.localAppDataPath)}
                disabled={!overview}
              >
                <FolderOpen size={16} />
                {t('common.open')}
              </button>
            </div>
          </div>
        </div>

        <div className="settings-row settings-row--top">
          <div className="row-label">
            <div className="row-title">{t('settings.storage.defaultInstanceBase')}</div>
            <div className="row-desc">{t('settings.storage.defaultInstanceBaseDesc')}</div>
          </div>
          <div className="row-control row-control--grow">
            <div className="storage-settings-inline">
              <input
                className="settings-input settings-input--path"
                value={defaultInstanceBase}
                onChange={(event) => setDefaultInstanceBase(event.target.value)}
                placeholder={overview?.state.legacyInstanceBaseDir ?? ''}
              />
              <button
                className="btn btn-secondary"
                onClick={() => void pickDirectory(setDefaultInstanceBase, defaultInstanceBase || overview?.state.instanceBaseDir)}
                disabled={!overview?.supported}
              >
                <FolderOpen size={16} />
                {t('settings.storage.browse')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => void handleSaveDefaultBase()}
                disabled={savingDefault || !overview?.supported}
              >
                <Save size={16} />
                {t('common.save')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => void handleResetDefaultBase()}
                disabled={savingDefault || !overview?.supported}
              >
                {t('common.reset')}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div className="group-title">{t('settings.storage.migrationSection')}</div>
      <div className="settings-group">
        <div className="settings-row settings-row--top">
          <div className="row-label">
            <div className="row-title">{t('settings.storage.targetRoot')}</div>
            <div className="row-desc">{t('settings.storage.targetRootDesc')}</div>
          </div>
          <div className="row-control row-control--grow">
            <div className="storage-settings-inline">
              <input
                className="settings-input settings-input--path"
                value={migrationTargetRoot}
                onChange={(event) => setMigrationTargetRoot(event.target.value)}
              />
              <button
                className="btn btn-secondary"
                onClick={() => void pickDirectory(setMigrationTargetRoot, migrationTargetRoot || parentDir(defaultInstanceBase))}
              >
                <FolderOpen size={16} />
                {t('settings.storage.browse')}
              </button>
            </div>
          </div>
        </div>

        <div className="settings-row settings-row--top">
          <div className="row-label">
            <div className="row-title">{t('settings.storage.scopeTitle')}</div>
            <div className="row-desc">{t('settings.storage.scopeDesc')}</div>
          </div>
          <div className="row-control row-control--grow">
            <div className="storage-settings-checkboxes">
              <label className="storage-settings-checkbox">
                <input
                  type="checkbox"
                  checked={migrateAppData}
                  onChange={(event) => setMigrateAppData(event.target.checked)}
                  disabled={!overview?.supported}
                />
                <span>{t('settings.storage.migrateAppData')}</span>
              </label>
              <label className="storage-settings-checkbox">
                <input
                  type="checkbox"
                  checked={migrateInstances}
                  onChange={(event) => setMigrateInstances(event.target.checked)}
                  disabled={!overview?.supported}
                />
                <span>{t('settings.storage.migrateInstances')}</span>
              </label>
            </div>
          </div>
        </div>

        <div className="settings-row settings-row--top">
          <div className="row-label">
            <div className="row-title">{t('settings.storage.migrationHintTitle')}</div>
            <div className="row-desc">{t('settings.storage.migrationHint')}</div>
          </div>
          <div className="row-control">
            <button
              className="btn btn-primary"
              onClick={() => void handleMigrate()}
              disabled={migrating || !overview?.supported}
            >
              <ArrowRightLeft size={16} />
              {migrating ? t('common.loading') : t('settings.storage.startMigration')}
            </button>
          </div>
        </div>
      </div>

      <div className="group-title">{t('settings.storage.statsSection')}</div>
      <div className="settings-group">
        <div className="storage-settings-list">
          {pathStats.map((stat) => (
            <PathStatRow
              key={`${stat.key}-${stat.path}`}
              stat={stat}
              label={getPathLabel(stat)}
              onOpen={(path) => void handleOpenFolder(path)}
            />
          ))}
        </div>
      </div>

      <div className="group-title">{t('settings.storage.managedInstancesSection')}</div>
      <div className="settings-group">
        {managedInstances.length === 0 ? (
          <div className="storage-settings-empty">
            <HardDrive size={16} />
            <span>{t('settings.storage.noInstances')}</span>
          </div>
        ) : (
          <div className="storage-settings-list">
            {managedInstances.map((instance) => (
              <ManagedInstanceRow
                key={`${instance.platform}-${instance.instanceId}`}
                instance={{ ...instance, name: getInstanceTitle(instance) }}
                onOpen={(path) => void handleOpenFolder(path)}
              />
            ))}
          </div>
        )}
      </div>

      {message && (
        <div className={`storage-settings-message storage-settings-message--${message.tone}`}>
          {message.text}
        </div>
      )}

    </div>
  );
}
