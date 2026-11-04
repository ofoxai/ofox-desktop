/**
 * The preview's own policy: inline styles and `data:` images and fonts, and
 * nothing else — no script, no request to the network or the app. A page's
 * own policy can only add restrictions to it, never lift one.
 */
export const HTML_PREVIEW_POLICY =
  "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; media-src data:; form-action 'none'";

/**
 * The document an HTML code block is previewed as: the policy first, before
 * any element of the page can ask for something, and the page's own refresh
 * (which would navigate the frame) left out.
 */
export function htmlPreviewDocument(html: string): string {
  const page = html
    .replace(/^\s*<!doctype[^>]*>/i, "")
    .replace(/<meta\b[^>]*http-equiv\s*=\s*["']?refresh[^>]*>/gi, "");
  return `<!doctype html><meta http-equiv="Content-Security-Policy" content="${HTML_PREVIEW_POLICY}">${page}`;
}

/** Whether the page has script the preview will not run. */
export function htmlHasScripts(html: string): boolean {
  return /<script\b|\son[a-z]+\s*=|javascript:/i.test(html);
}
