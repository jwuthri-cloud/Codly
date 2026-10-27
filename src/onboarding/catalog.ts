import type { Provider } from '../lib/gen/Provider';
import raw from './catalog.json';

export type Tier = 'best' | 'fast' | 'good' | 'none';
export type Pick = { tier: Tier; provider: Provider; name: string; why: string; needs?: string; tip?: string; verify?: boolean };
export type Capability = { id: string; title: string; icon: string; blurb: string; picks: Pick[] };
export type Mission = { id: string; title: string; text: string };
export type Catalog = {
  capabilities: Capability[];
  brains: { steps: { title: string; body: string; url?: string }[]; local: string };
  missions: Mission[];
};

export const catalog = raw as unknown as Catalog;
