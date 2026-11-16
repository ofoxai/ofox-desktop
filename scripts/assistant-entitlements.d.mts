export const ENTITLEMENTS: Readonly<Record<string, string | null>>;
export function entitlementsFor(
  filename: string,
  enginesDirectory: string,
): string | null;
