export type SkillSource =
  | {
      kind: "repository";
      owner: string;
      name: string;
      /** Absent when not typed: the saved branch, else the repository's default. */
      branch?: string;
      /** A folder inside the repository, from a /tree/<ref>/<path> link. */
      path?: string;
    }
  | { kind: "folder"; path: string; leaf: string };

const PART = /^[A-Za-z0-9_.-]+$/;

/** Reads what was typed into the install card: owner/repo, a GitHub link, or a folder. */
export function parseSkillSource(input: string): SkillSource | null {
  const text = input.trim();
  if (!text) return null;
  if (/^(?:[/~]|[A-Za-z]:[\\/])/.test(text)) {
    const trimmed = text.replace(/[\\/]+$/, "");
    const leaf = trimmed.split(/[\\/]/).pop() ?? "";
    if (!leaf || leaf === "~") return null;
    return { kind: "folder", path: trimmed, leaf };
  }
  const link = text.match(
    /^(?:https?:\/\/)?(?:www\.)?github\.com\/([^/\s]+)\/([^/\s?#]+?)(?:\.git)?(?:\/tree\/([^/\s?#]+)(?:\/([^?#\s]*))?)?\/?(?:[?#].*)?$/i,
  );
  if (link) {
    const [, owner, name, branch, path] = link;
    if (!PART.test(owner) || !PART.test(name)) return null;
    const folder = path?.replace(/\/+$/, "");
    return {
      kind: "repository",
      owner,
      name,
      ...(branch ? { branch: decodeURIComponent(branch) } : {}),
      ...(folder ? { path: decodeURIComponent(folder) } : {}),
    };
  }
  const short = text.match(/^([^/\s]+)\/([^/\s]+?)(?:\.git)?$/);
  if (short && PART.test(short[1]) && PART.test(short[2]))
    return {
      kind: "repository",
      owner: short[1],
      name: short[2],
    };
  return null;
}
