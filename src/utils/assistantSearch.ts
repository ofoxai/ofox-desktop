/**
 * What to mark in the conversation list and transcript for what the search
 * box holds: the text as typed (the backend matches it as typed), but nothing
 * for a search of only spaces, which would mark every gap between words.
 */
export function searchHighlight(search: string): string {
  return search.trim() ? search : "";
}
