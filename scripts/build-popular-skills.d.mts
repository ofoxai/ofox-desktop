export type MarketSkill = {
  source: string;
  skillId: string;
  name: string;
  installs: number;
  isOfficial?: boolean;
  [key: string]: unknown;
};
export type SnapshotSkill = {
  source: string;
  skillId: string;
  name: string;
  installs: number;
  official: boolean;
};
export function selectPopularSkills(entries: MarketSkill[]): SnapshotSkill[];
export function metaDescription(html: string): string | undefined;
