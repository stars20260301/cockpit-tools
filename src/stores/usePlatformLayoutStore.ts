import { create } from 'zustand';
import { ALL_PLATFORM_IDS, PlatformId } from '../types/platform';

const PLATFORM_LAYOUT_STORAGE_KEY = 'agtools.platform_layout.v1';

type PersistedPlatformLayout = {
  orderedPlatformIds?: PlatformId[];
  hiddenPlatformIds?: PlatformId[];
  sidebarPlatformIds?: PlatformId[];
};

interface PlatformLayoutState {
  orderedPlatformIds: PlatformId[];
  hiddenPlatformIds: PlatformId[];
  sidebarPlatformIds: PlatformId[];

  movePlatform: (fromIndex: number, toIndex: number) => void;
  toggleHiddenPlatform: (id: PlatformId) => void;
  setHiddenPlatform: (id: PlatformId, hidden: boolean) => void;
  toggleSidebarPlatform: (id: PlatformId) => void;
  setSidebarPlatform: (id: PlatformId, enabled: boolean) => void;
  resetPlatformLayout: () => void;
}

function sanitizePlatformIds(list: unknown): PlatformId[] {
  if (!Array.isArray(list)) return [];
  const seen = new Set<PlatformId>();
  const result: PlatformId[] = [];
  for (const item of list) {
    if (typeof item !== 'string') continue;
    if (!ALL_PLATFORM_IDS.includes(item as PlatformId)) continue;
    const id = item as PlatformId;
    if (seen.has(id)) continue;
    seen.add(id);
    result.push(id);
  }
  return result;
}

function normalizeOrder(order: PlatformId[]): PlatformId[] {
  const next = sanitizePlatformIds(order);
  for (const id of ALL_PLATFORM_IDS) {
    if (!next.includes(id)) {
      next.push(id);
    }
  }
  return next;
}

function normalizeHidden(hidden: PlatformId[]): PlatformId[] {
  return sanitizePlatformIds(hidden);
}

function normalizeSidebar(sidebar: PlatformId[], hidden: PlatformId[]): PlatformId[] {
  const normalized = sanitizePlatformIds(sidebar).filter((id) => !hidden.includes(id));
  return normalized.slice(0, 2);
}

function loadPersistedState(): Pick<
  PlatformLayoutState,
  'orderedPlatformIds' | 'hiddenPlatformIds' | 'sidebarPlatformIds'
> {
  try {
    const raw = localStorage.getItem(PLATFORM_LAYOUT_STORAGE_KEY);
    if (!raw) {
      return {
        orderedPlatformIds: [...ALL_PLATFORM_IDS],
        hiddenPlatformIds: [],
        sidebarPlatformIds: ['antigravity', 'codex'],
      };
    }
    const parsed = JSON.parse(raw) as PersistedPlatformLayout;
    const hiddenPlatformIds = normalizeHidden(parsed.hiddenPlatformIds ?? []);
    const orderedPlatformIds = normalizeOrder(parsed.orderedPlatformIds ?? ALL_PLATFORM_IDS);
    const sidebarPlatformIds = normalizeSidebar(parsed.sidebarPlatformIds ?? ['antigravity', 'codex'], hiddenPlatformIds);
    return {
      orderedPlatformIds,
      hiddenPlatformIds,
      sidebarPlatformIds,
    };
  } catch {
    return {
      orderedPlatformIds: [...ALL_PLATFORM_IDS],
      hiddenPlatformIds: [],
      sidebarPlatformIds: ['antigravity', 'codex'],
    };
  }
}

function persist(state: Pick<PlatformLayoutState, 'orderedPlatformIds' | 'hiddenPlatformIds' | 'sidebarPlatformIds'>) {
  try {
    localStorage.setItem(PLATFORM_LAYOUT_STORAGE_KEY, JSON.stringify(state));
  } catch {
    // ignore persistence failures
  }
}

export const usePlatformLayoutStore = create<PlatformLayoutState>((set, get) => ({
  ...loadPersistedState(),

  movePlatform: (fromIndex, toIndex) => {
    const current = [...get().orderedPlatformIds];
    if (fromIndex < 0 || toIndex < 0 || fromIndex >= current.length || toIndex >= current.length) return;
    if (fromIndex === toIndex) return;
    const [item] = current.splice(fromIndex, 1);
    current.splice(toIndex, 0, item);
    const orderedPlatformIds = normalizeOrder(current);
    const next = {
      orderedPlatformIds,
      hiddenPlatformIds: [...get().hiddenPlatformIds],
      sidebarPlatformIds: [...get().sidebarPlatformIds],
    };
    set(next);
    persist(next);
  },

  toggleHiddenPlatform: (id) => {
    const hidden = [...get().hiddenPlatformIds];
    const exists = hidden.includes(id);
    const nextHidden = exists ? hidden.filter((item) => item !== id) : [...hidden, id];
    const hiddenPlatformIds = normalizeHidden(nextHidden);
    const sidebarPlatformIds = normalizeSidebar(get().sidebarPlatformIds, hiddenPlatformIds);
    const next = {
      orderedPlatformIds: [...get().orderedPlatformIds],
      hiddenPlatformIds,
      sidebarPlatformIds,
    };
    set(next);
    persist(next);
  },

  setHiddenPlatform: (id, hidden) => {
    const current = get().hiddenPlatformIds;
    const has = current.includes(id);
    if ((hidden && has) || (!hidden && !has)) return;
    get().toggleHiddenPlatform(id);
  },

  toggleSidebarPlatform: (id) => {
    const hiddenPlatformIds = [...get().hiddenPlatformIds];
    if (hiddenPlatformIds.includes(id)) return;

    const current = [...get().sidebarPlatformIds];
    let nextSidebar: PlatformId[] = [];

    if (current.includes(id)) {
      nextSidebar = current.filter((item) => item !== id);
    } else if (current.length < 2) {
      nextSidebar = [...current, id];
    } else {
      return;
    }

    const sidebarPlatformIds = normalizeSidebar(nextSidebar, hiddenPlatformIds);
    const next = {
      orderedPlatformIds: [...get().orderedPlatformIds],
      hiddenPlatformIds,
      sidebarPlatformIds,
    };
    set(next);
    persist(next);
  },

  setSidebarPlatform: (id, enabled) => {
    const current = get().sidebarPlatformIds.includes(id);
    if (current === enabled) return;
    get().toggleSidebarPlatform(id);
  },

  resetPlatformLayout: () => {
    const next = {
      orderedPlatformIds: [...ALL_PLATFORM_IDS],
      hiddenPlatformIds: [],
      sidebarPlatformIds: ['antigravity', 'codex'] as PlatformId[],
    };
    set(next);
    persist(next);
  },
}));

