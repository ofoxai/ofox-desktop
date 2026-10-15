/** Shared with the analytics DTO: an identified session with no known directory. */
export const UNKNOWN_USAGE_PROJECT_ID = "__unknown_project__";

export function usageProjectLabel(
  id: string | null,
  label: string | null | undefined,
  path: string | null | undefined,
  names: { unknown: string; unparsed: string },
): string {
  if (id === null) return names.unparsed;
  if (id === UNKNOWN_USAGE_PROJECT_ID) return names.unknown;
  return label?.trim() || path || id;
}
