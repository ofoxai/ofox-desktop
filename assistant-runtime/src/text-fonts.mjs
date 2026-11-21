// Skia selects the first available family and does not fall back per glyph.
// Keep Latin text in its requested face; use an installed CJK face for CJK.
export const CJK_FONT_FAMILIES =
  "'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei', 'Yu Gothic', 'Noto Sans CJK SC', 'Noto Sans SC'";

// A circled decimal digit can be measured and drawn apart from its CJK label.
const CJK_TEXT =
  /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}\u2460-\u2473\u3000-\u303f\uff00-\uffef]/u;
const CJK_FAMILY =
  /CJK|PingFang|Hiragino|YaHei|JhengHei|Yu (?:Gothic|Mincho)|Noto (?:Sans|Serif) (?:SC|TC|JP|KR)|SimSun|SimHei|Songti|Heiti|Meiryo|Malgun|Batang|Gulim|KaiTi|FangSong|\p{Script=Han}/iu;

export const hasCjkText = (text) => CJK_TEXT.test(text);

export function cjkFontFamilies(family) {
  // An inherited, invalid family must not be copied into every text element.
  const requested =
    family && family.length <= 512 && family.trim() ? family : undefined;
  // Preserve an explicitly requested CJK face (including a serif face).
  const preferred = requested
    ?.split(",")
    .filter((item) => CJK_FAMILY.test(item));
  return [
    ...(preferred ?? []),
    CJK_FONT_FAMILIES,
    ...(requested === undefined ? [] : [requested]),
    "sans-serif",
  ].join(", ");
}

export function fontFamiliesForText(family, text) {
  if (hasCjkText(text)) return cjkFontFamilies(family);
  return family === undefined ? "sans-serif" : `${family}, sans-serif`;
}
