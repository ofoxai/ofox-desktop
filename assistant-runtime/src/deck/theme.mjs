// The deck stylesheet: colors, fonts and type sizes the renderer draws with.
// Values from OfficeCLI skills/morph-ppt/reference/styles/light--minimal-corporate + light--project-proposal (Apache-2.0)
//
// A theme holds roles, never coordinates: layout.mjs places things and the
// renderer picks a role (primary, card, onDark) for each. Every `sizes` list
// runs from large to small so text.mjs `fitSize` can take it as it is.
// Rounded cards keep officecli's default corner: the host refuses the `adj`
// that would set another one, so a theme has no corner radius.
// This module imports nothing, so schema.mjs can read the theme names from it.

const navy = {
  name: "navy",
  colors: {
    background: "FFFFFF",
    card: "E8EEF4",
    primary: "1E3A5F",
    secondary: "D4A84B",
    accent: "3498DB",
    text: "333333",
    textSecondary: "666666",
    textMuted: "999999",
    dark: "1E3A5F",
    onDark: "FFFFFF",
    onDarkMuted: "CADCFC",
    chart: ["1E3A5F", "D4A84B", "3498DB", "7F9CC0", "E5C87A", "8FC1E8"],
  },
  fonts: {
    latin: { title: "Arial Black", body: "Arial", data: "Arial" },
    zh: { title: "Microsoft YaHei", body: "Microsoft YaHei" },
    ja: { title: "Yu Gothic", body: "Yu Gothic" },
  },
  sizes: {
    hero: [60, 54, 48, 44],
    statement: [40, 36, 32],
    title: [44, 40, 36],
    subtitle: [24, 22, 20],
    body: [24, 22, 20, 18],
    card: [18, 16, 14],
    caption: [16, 14, 13],
    kpi: [72, 60, 48],
  },
  decor: { titleRule: "bar", pageNumber: true },
};

export const THEMES = { navy };
export const THEME_NAMES = Object.keys(THEMES);
export const DEFAULT_THEME = "navy";

// A name the stylesheet does not have gets the default theme.
export function themeFor(name) {
  return typeof name === "string" && Object.hasOwn(THEMES, name)
    ? THEMES[name]
    : THEMES[DEFAULT_THEME];
}

// Languages whose text needs a CJK font; any other language is set in Latin.
const CJK_LANGUAGES = ["zh", "ja"];

// The fonts a deck in `language` is set in. English uses the theme's Latin
// fonts. Chinese and Japanese use a CJK font for `title` and `body`, and
// name the Latin fonts as `latinTitle` and `latinBody` for the `font.latin`
// slot, so Latin words and digits inside CJK text keep the theme's look.
// `data` (numbers in charts and big figures) is always Latin.
export function fontsFor(theme, language) {
  const { latin } = theme.fonts;
  if (!CJK_LANGUAGES.includes(language))
    return { title: latin.title, body: latin.body, data: latin.data };
  const cjk = theme.fonts[language];
  return {
    title: cjk.title,
    body: cjk.body,
    data: latin.data,
    latinTitle: latin.title,
    latinBody: latin.body,
  };
}
