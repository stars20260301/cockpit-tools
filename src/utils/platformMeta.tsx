import { ReactNode } from 'react';
import { Github } from 'lucide-react';
import { TFunction } from 'i18next';
import { PlatformId } from '../types/platform';
import { RobotIcon } from '../components/icons/RobotIcon';
import { CodexIcon } from '../components/icons/CodexIcon';
import { WindsurfIcon } from '../components/icons/WindsurfIcon';

export function getPlatformLabel(platformId: PlatformId, t: TFunction): string {
  switch (platformId) {
    case 'antigravity':
      return t('nav.overview', 'Antigravity');
    case 'codex':
      return t('nav.codex', 'Codex');
    case 'github-copilot':
      return t('nav.githubCopilot', 'GitHub Copilot');
    case 'windsurf':
      return 'Windsurf';
    default:
      return platformId;
  }
}

export function renderPlatformIcon(platformId: PlatformId, size = 20): ReactNode {
  switch (platformId) {
    case 'antigravity':
      return <RobotIcon style={{ width: size, height: size }} />;
    case 'codex':
      return <CodexIcon size={size} />;
    case 'github-copilot':
      return <Github size={size} />;
    case 'windsurf':
      return <WindsurfIcon style={{ width: size, height: size }} />;
    default:
      return null;
  }
}

