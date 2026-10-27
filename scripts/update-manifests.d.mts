export interface UpdateFeed {
  version: string;
  pubDate: string;
  notes: string;
  downloads: Record<string, string>;
  downloadPage: string;
  schemaVersion?: number;
  minimumMacOS?: string;
}
export interface UpdateFeedPair {
  legacy: UpdateFeed;
  modern: UpdateFeed;
}
export function buildUpdateManifests(input: {
  legacy: unknown;
  modern: unknown;
  version: string;
  platforms: string[];
  pubDate: string;
  notes?: string;
  minimumMacOS: string;
}): UpdateFeedPair;
export function verifyPublishedManifests(input: {
  expected: UpdateFeedPair;
  published: UpdateFeedPair;
}): void;
export function publishUpdateRelease(input: {
  bucket: string;
  endpoint: string;
  version: string;
  pubDate: string;
  minimumMacOS: string;
  output: string;
  notes?: string;
  dryRun?: boolean;
  artifacts: Array<{
    platform: string;
    path: string;
    sha256?: string;
    bytes?: number;
  }>;
  run?: (tool: string, args: string[]) => Promise<string>;
  verifyPublic?: (expected: UpdateFeedPair) => Promise<void>;
  validateArtifact?: (artifact: {
    platform: string;
    path: string;
  }) => Promise<void>;
}): Promise<UpdateFeedPair>;
export const LEGACY_MAC_VERSION: string;
export const UPDATE_FEED_LOCK_KEY: string;
