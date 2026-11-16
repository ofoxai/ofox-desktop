import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { Type } from "typebox";
import { Check } from "typebox/value";
import { ADAPTED_STYLES } from "./deck/catalog.mjs";
import { THEME_NAMES } from "./deck/theme.mjs";
import { imageSize } from "./deck/image.mjs";

const object = (properties) =>
  Type.Object(properties, { additionalProperties: false });
const choice = (values) => Type.String({ enum: values });
const text = (maxLength) => Type.String({ minLength: 1, maxLength });
const id = Type.String({ pattern: "^[A-Za-z0-9_-]{1,80}$" });
const editing = Type.Object(
  Object.fromEntries(
    ["text", "charts", "formulas", "diagrams"].map((role) => [
      role,
      Type.Boolean({
        description: `Native ${role} editing policy. In slides[].editing, inherit true whenever requirements.nativeEditing.${role} is true, even if this page has no such object.`,
      }),
    ]),
  ),
  {
    additionalProperties: false,
    description:
      "Native editing policy, not an inventory of objects present. In every slides[].editing object keep every required true flag from requirements.nativeEditing, even on pages with no chart, formula or diagram. True requires native editability if that kind is used; it does not require adding that object or certify target-application round-trip. A code-figure (an SVG or matplotlib picture the deck code draws: an architecture sketch, attention map or curve) illustrates rather than carries editable data, so diagrams=true alone does not bar it; when the user asks for every diagram to be editable, use native diagrams instead. Chart data and formulas still use native_chart and equation.",
  },
);
const unknownOr = (schema) => Type.Optional(Type.Union([schema, Type.Null()]));
const MAX_ASSETS = 12;

export const PRESENTATION_REQUIREMENTS_SCHEMA = object({
  audience: text(300),
  purpose: text(300),
  scenario: text(80),
  language: choice(["zh", "en", "ja"]),
  slideCount: Type.Integer({
    minimum: 1,
    maximum: 30,
    description:
      "Exact number of plan entries; must equal slides.length. For a new deck include cover and closing in this requested total, not extra pages. For a source edit count only authorized target pages, not the unchanged deck pages.",
  }),
  aspectRatio: choice(["16:9", "4:3"]),
  density: Type.String({
    enum: ["low", "medium", "high"],
    description:
      "Required overall document density. Keep this field when reusing the complete candidate requirements for freezing; per-page density belongs in each slide's composition text.",
  }),
  nativeEditing: editing,
  visualNeeds: Type.Array(
    choice(["none", "photo", "concept", "data", "process"]),
    {
      maxItems: 5,
      description:
        "Document-wide visual-need tags, at most five distinct values; not a per-slide list. Put each page's chosen visual in slides[].visual.",
    },
  ),
  roundTrip: Type.String({
    enum: ["unverifiedAllowed", "verifiedRequired"],
    description:
      "Native-editable content does not imply certified target-application round-trip acceptance. Use unverifiedAllowed for editable text, charts, equations and shapes unless the user explicitly requires already-certified editing, saving and reopening in the target application. verifiedRequired is that additional hard constraint; never infer it from nativeEditing or from a request to check the output. unverifiedAllowed preserves every nativeEditing requirement and does not claim certification.",
  }),
  durationMinutes: unknownOr(Type.Integer({ minimum: 1, maximum: 240 })),
  contentMix: unknownOr(
    object({
      data: Type.Number({ minimum: 0, maximum: 1 }),
      formulas: Type.Number({ minimum: 0, maximum: 1 }),
      images: Type.Number({ minimum: 0, maximum: 1 }),
    }),
  ),
  brand: unknownOr(
    object({
      description: Type.String({ maxLength: 300 }),
      colors: Type.Array(Type.String({ pattern: "^#?[A-Fa-f0-9]{6}$" }), {
        maxItems: 8,
      }),
      fonts: Type.Array(text(100), { maxItems: 4 }),
      strict: Type.Boolean(),
    }),
  ),
});
// Page kinds a plan may name; the same list the style page mappings use.
export const PAGE_KINDS = [
  "cover",
  "chapter",
  "agenda",
  "statement",
  "quote",
  "list",
  "split-image",
  "cards",
  "stats",
  "comparison",
  "table",
  "chart-bar",
  "chart-line",
  "chart-pie",
  "process",
  "timeline",
  "cycle",
  "hierarchy",
  "diagram",
  "code",
  "team",
  "lesson",
  "pricing",
  "equation",
  "closing",
];
export const PRESENTATION_SLIDE_SCHEMA = object({
  id,
  claim: text(1000),
  evidence: Type.Array(text(1000), { maxItems: 12 }),
  relationship: Type.String({
    enum: [
      "sequence",
      "comparison",
      "hierarchy",
      "part-whole",
      "trend",
      "spatial",
      "single-claim",
    ],
    description:
      "Page information relationship. For a sequential process or ordered flow choose sequence. The process tag in requirements.visualNeeds and Python relation(kind='process') are different fields; process is not a relationship value.",
  }),
  composition: text(300),
  pageKind: Type.Optional(
    Type.String({
      enum: PAGE_KINDS,
      description:
        "The page kind this page is built as; supply it for every page. Choose from the chosen style's pageKinds and vary kinds across the deck: three consecutive pages of one kind are refused unless continuity gives the reason.",
    }),
  ),
  continuity: Type.Optional(
    Type.String({
      maxLength: 200,
      description:
        "Only when this page deliberately repeats the two previous pages' kind: the reason the run reads as one series.",
    }),
  ),
  facts: Type.Optional(
    Type.Array(
      object({
        text: text(300),
        source: Type.String({
          enum: ["user", "attachment", "computed", "illustrative", "proposed"],
          description:
            "Where this figure, feature or claim comes from. user/attachment: stated by the user or their files. computed: derived from those. illustrative: a made-up example value, which the page must label as an example. proposed: a suggestion the user has not confirmed, which the page must label as proposed.",
        }),
      }),
      {
        maxItems: 12,
        description:
          "Every number, product capability, result or benefit this page states, with its source. Do not present illustrative or proposed items as established facts.",
      },
    ),
  ),
  editing,
  visual: object({
    kind: Type.String({
      enum: [
        "none",
        "native-chart",
        "native-diagram",
        "native-math",
        "code-figure",
        "user-image",
        "generated-image",
      ],
      description:
        "native-chart: data the user edits (native_chart). native-math: a formula (equation). native-diagram: simple steps, relations, fork-join, timeline or pyramid helpers. code-figure: a richer diagram, map or curve the deck code draws with figure() (SVG) or plot() (matplotlib), such as an architecture, an attention heatmap or a function curve; free, not a generated image. user-image or generated-image: a picture, with an assetId.",
    }),
    purpose: text(500),
    fallback: choice(["native-summary", "omit", "stop"]),
    assetId: Type.Optional(id),
    placement: Type.Optional(
      Type.String({
        enum: ["side", "inset", "band", "background"],
        description:
          "Image visuals only; give placement, textZone and aspect together. side: a column beside the text. inset: a framed picture inside the body. band: a strip across the page. background: under the text, which needs a calm text zone.",
      }),
    ),
    textZone: Type.Optional(
      Type.String({
        enum: ["left", "right", "top", "bottom", "center", "none"],
        description:
          "Where the page's title and text sit; the image keeps that part of the page clear (or visually quiet for a background).",
      }),
    ),
    aspect: Type.Optional(
      Type.String({
        enum: ["16:9", "4:3", "1:1", "3:4", "9:16"],
        description: "Aspect ratio of the region the image fills on the page.",
      }),
    ),
  }),
});
export const PRESENTATION_SOURCE_SCHEMA = object({
  attachmentId: id,
  version: Type.String({ pattern: "^[a-f0-9]{64}$" }),
  pageNumbers: Type.Array(Type.Integer({ minimum: 1, maximum: 1000 }), {
    minItems: 1,
    maxItems: 30,
    uniqueItems: true,
  }),
});
export const FREEZE_PRESENTATION_SCHEMA = object({
  requirements: PRESENTATION_REQUIREMENTS_SCHEMA,
  styleId: id,
  decisionSummary: text(1000),
  imageStyle: Type.Optional(
    Type.String({
      minLength: 1,
      maxLength: 300,
      description:
        "One sentence for the look every generated image in this deck shares (medium, palette, line or lighting, level of detail), following the chosen style. Supply it whenever any page plans a generated image. Images carry no slide text, formulas or data.",
    }),
  ),
  slides: Type.Array(PRESENTATION_SLIDE_SCHEMA, {
    minItems: 1,
    maxItems: 30,
    description:
      "Exactly requirements.slideCount entries. For a new deck include cover and closing within that count; for source editing include only the authorized target pages. Each page inherits every required native-editing flag, regardless of which objects are present.",
  }),
  source: Type.Optional(PRESENTATION_SOURCE_SCHEMA),
});
export const CATALOG_REVISION = createHash("sha256")
  .update(
    readFileSync(
      new URL(
        "../skills/ofox-office-pptx/resources/catalog.json",
        import.meta.url,
      ),
    ),
  )
  .digest("hex");
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const invalid = (why) =>
  new Error("assistant_presentation_plan_invalid: " + why);
const assetInvalid = (why) =>
  new Error("assistant_presentation_asset_invalid: " + why);
const clone = (value) => JSON.parse(JSON.stringify(value));

function noCandidateAction(requirements) {
  const cause =
    requirements.roundTrip === "verifiedRequired"
      ? "Target-application round-trip acceptance is not certified for any enabled style. If verifiedRequired was inferred only from native editability or a request to check the output, correct roundTrip to unverifiedAllowed, keep all nativeEditing flags and disclose that target-application round-trip remains unverified. If the user explicitly requires prior certification, keep verifiedRequired, explain the unavailable capability and ask whether an unverified deliverable is acceptable; otherwise stop without generation."
      : "No enabled style satisfies these hard constraints. Read the excluded reasons and correct only requirements you inferred incorrectly from the user's request. Keep explicit user constraints; explain the conflict and ask about an alternative, or stop without generation.";
  return (
    cause +
    " Do not freeze an excluded style, repeat the rejected requirements unchanged or bypass plan freezing. A PDF or page screenshot is not a native-editable PPTX and cannot satisfy editable-math requirements."
  );
}

function validate(schema, value) {
  if (!Check(schema, value))
    throw invalid(
      "Malformed plan, claim, requirements or unknown fields (including budget)",
    );
  const visit = (node) => {
    if (typeof node === "string" && node.length > 0 && !node.trim())
      throw invalid("Plan text and claim must not be blank");
    if (Array.isArray(node)) node.forEach(visit);
    else if (node && typeof node === "object")
      Object.values(node).forEach(visit);
  };
  visit(value);
}

// What a planned page kind must contain, as the deck helper's receipt tags:
// C/S cover or section layout, cb/cl/cp charts, tb table, cc code, py pyramid,
// fj fork-join, rl relation, ls lesson rail, im picture, eq equation, k cards,
// n figures, st steps, tl timeline, bu bullets.
const KIND_TAGS = {
  cover: ["C"],
  closing: ["C", "S"],
  chapter: ["S"],
  statement: ["S"],
  quote: ["S"],
  "chart-bar": ["cb"],
  "chart-line": ["cl"],
  "chart-pie": ["cp"],
  table: ["tb"],
  code: ["cc"],
  hierarchy: ["py"],
  diagram: ["fj", "rl", "st"],
  process: ["st", "rl", "tl"],
  cycle: ["rl", "st"],
  timeline: ["tl", "st"],
  stats: ["n"],
  cards: ["k"],
  comparison: ["k", "rl", "tb"],
  team: ["k", "tb"],
  pricing: ["k", "tb"],
  list: ["bu", "k"],
  agenda: ["st", "bu", "k"],
  "split-image": ["im"],
  lesson: ["ls"],
  equation: ["eq"],
};
const DIAGRAM_TAGS = ["fj", "rl", "st", "tl", "py"];

/**
 * Takes the deck helper's built-page receipt out of the host's issues and
 * reports where a page was not built as the frozen plan says. Advisory: the
 * model fixes the page or says what changed; the plan itself stays frozen.
 */
export function planDrift(issues, plan) {
  const text = (issue) =>
    typeof issue === "string" ? issue : String(issue?.message ?? "");
  const list = Array.isArray(issues) ? issues : [];
  const at = list.findIndex((issue) => text(issue).startsWith("plan-receipt:"));
  if (at < 0) return { issues: list, receipt: [] };
  const rest = list.filter((_, index) => index !== at);
  const receipt = text(list[at])
    .slice(13)
    .split(";")
    .map((entry) => (entry.split("=")[1] ?? "").split(",").filter(Boolean));
  if (!plan?.slides) return { issues: rest, receipt };
  const drift = [];
  plan.slides.forEach((slide, index) => {
    const tags = receipt[index];
    if (!tags) return;
    const has = (codes) =>
      codes.some((code) =>
        tags.some(
          (tag) => tag === code || (/^[kn]\d+$/.test(tag) && tag[0] === code),
        ),
      );
    const built = tags.length ? tags.join(", ") : "text only";
    const kind = slide.visual?.kind;
    if (kind === "native-chart" && !has(["cb", "cl", "cp"]))
      drift.push(
        `plan drift: page ${index + 1} planned a native chart; none was built (built: ${built}). Build it with native_chart, or say in your reply why the page changed.`,
      );
    if (kind === "native-math" && !has(["eq"]))
      drift.push(
        `plan drift: page ${index + 1} planned native math; no equation was built (built: ${built}).`,
      );
    if (kind === "code-figure" && !has(["fg", "pl"]))
      drift.push(
        `plan drift: page ${index + 1} planned a code figure; no figure() or plot() picture was drawn (built: ${built}).`,
      );
    if (kind === "native-diagram" && !has(DIAGRAM_TAGS))
      drift.push(
        `plan drift: page ${index + 1} planned a native diagram; no steps, relation, fork-join, timeline or pyramid was built (built: ${built}).`,
      );
    // A diagram or curve drawn in code is what a body page planned with a
    // code-figure visual shows; cover and section layouts still need theirs.
    const expected = KIND_TAGS[slide.pageKind];
    const drawn =
      kind === "code-figure" &&
      !expected?.some((tag) => tag === "C" || tag === "S")
        ? ["fg", "pl"]
        : [];
    if (expected && !has([...expected, ...drawn]))
      drift.push(
        `plan drift: page ${index + 1} planned page kind "${slide.pageKind}" but was built as: ${built}. Rebuild it as planned, or say in your reply what changed and why.`,
      );
  });
  return { issues: [...rest, ...drift], receipt };
}

/** The listed WxH size whose ratio is nearest a planned aspect such as "4:3". */
export function nearestSize(aspect, sizes) {
  const [w, h] = String(aspect).split(":").map(Number);
  if (!(w > 0 && h > 0)) return undefined;
  let best;
  for (const size of sizes ?? []) {
    const match = /^(\d+)x(\d+)$/.exec(size);
    if (!match) continue;
    const gap = Math.abs(
      Math.log(Number(match[1]) / Number(match[2]) / (w / h)),
    );
    if (!best || gap < best.gap) best = { size, gap };
  }
  return best?.size;
}

const DENSITY = ["low", "medium", "high"];
const SHORT_LIST = 6;

/**
 * Rule-based signals for one style against the stated requirements. They order
 * the candidate list and name cautions; they never exclude a style and are not
 * the design decision, which the model makes among the eligible candidates.
 */
export function styleFit(style, requirements) {
  const routing = style.routing;
  const scenario = (style.scenarios ?? []).includes(requirements.scenario);
  const cautions = [];
  let score = scenario ? 3 : 0;
  if (!routing) return { score, scenario, density: null, cautions };
  const gap = Math.abs(
    DENSITY.indexOf(routing.density) - DENSITY.indexOf(requirements.density),
  );
  score += 2 - gap * 2;
  const mix = requirements.contentMix ?? {};
  const needs = {
    data: (mix.data ?? 0) >= 0.15 || requirements.nativeEditing.charts,
    formulas:
      (mix.formulas ?? 0) >= 0.15 || requirements.nativeEditing.formulas,
    images:
      (mix.images ?? 0) >= 0.15 ||
      requirements.visualNeeds.some((need) =>
        ["photo", "concept"].includes(need),
      ),
  };
  const labels = {
    data: "data and chart pages",
    formulas: "formula pages",
    images: "image-led pages",
  };
  for (const key of Object.keys(needs)) {
    if (!needs[key]) continue;
    const suit = routing.suits[key];
    score += suit - 1;
    if (suit === 0)
      cautions.push(`This style is a poor fit for ${labels[key]}`);
  }
  if (requirements.density === "high" && routing.suits.longText === 0) {
    score -= 2;
    cautions.push("This style cannot carry dense pages or long text");
  }
  return { score, scenario, density: gap === 0, cautions };
}

function pageKinds(mapping) {
  const kinds = { adapted: [], approximate: [], unsupported: [] };
  for (const page of mapping ?? [])
    if (!kinds[page.status].includes(page.archetype))
      kinds[page.status].push(page.archetype);
  return kinds;
}

export function presentationCandidates(requirements, documents, source) {
  validate(PRESENTATION_REQUIREMENTS_SCHEMA, requirements);
  if (source !== undefined) validate(PRESENTATION_SOURCE_SCHEMA, source);
  if (
    requirements.contentMix &&
    Object.values(requirements.contentMix).reduce(
      (sum, value) => sum + value,
      0,
    ) >
      1 + 1e-9
  )
    throw invalid("Content mix proportions must sum to at most one");
  const manual =
    documents.selectedStyle === "auto" ? undefined : documents.selectedStyle;
  if (manual !== undefined && !THEME_NAMES.includes(manual))
    throw invalid("Unknown manual style");
  const sourceFile =
    source &&
    documents.sourceFiles?.find(
      (file) =>
        file.attachmentId === source.attachmentId &&
        file.version === source.version,
    );
  const inherited =
    !manual && THEME_NAMES.includes(sourceFile?.styleId)
      ? sourceFile.styleId
      : undefined;
  const styles = [
    { id: "navy", name: "Navy", scenarios: ["general"], validation: {} },
    ...ADAPTED_STYLES,
  ];
  const candidates = [],
    excluded = [];
  for (const style of styles) {
    const reasons = [];
    if (manual && style.id !== manual)
      reasons.push("User manual choice has priority");
    else if (inherited && style.id !== inherited)
      reasons.push("Preserve the source presentation's known style");
    if (requirements.aspectRatio !== "16:9")
      reasons.push("Current native adaptations use a 16:9 canvas");
    if (requirements.nativeEditing.formulas && documents.python !== true)
      reasons.push("Native equations require the Python engine");
    if (requirements.nativeEditing.charts && documents.python !== true)
      reasons.push("Editable chart data requires the Python engine");
    if (requirements.roundTrip === "verifiedRequired")
      reasons.push("Target application round-trip acceptance is not certified");
    if (
      requirements.brand?.strict &&
      (requirements.brand.description ||
        requirements.brand.colors.length ||
        requirements.brand.fonts.length)
    )
      reasons.push(
        "Requested brand and font constraints are not certified by the current native adaptations",
      );
    if (reasons.length) excluded.push({ id: style.id, reasons });
    else {
      const fit = styleFit(style, requirements);
      candidates.push({
        id: style.id,
        name: style.name,
        family: style.family,
        scheme: style.routing?.scheme ?? style.scheme,
        scenarios: style.scenarios,
        density: style.density ?? "medium",
        intro: style.intro?.en ?? style.description,
        avoid: style.routing?.avoid || undefined,
        recipe: style.recipe,
        pageKinds: pageKinds(style.pageMapping),
        scenarioMatch: fit.scenario,
        fit,
        limitations: [
          "Partial native adaptation; font fallback and target-application editing require verification",
        ],
      });
    }
  }
  // The Host checks eligibility against the catalog order, whatever the ranking.
  const eligibleIds = candidates.map((item) => item.id);
  // Best rule-based fit first; equal scores keep the catalog order. Only a
  // short list is described in full: every style in full is too much to weigh
  // in one turn. The rest stay eligible and can be read with get_presentation_style.
  candidates.sort((a, b) => b.fit.score - a.fit.score);
  const otherCandidates = candidates.splice(SHORT_LIST).map((item) => ({
    id: item.id,
    family: item.family,
    scheme: item.scheme,
    fitScore: item.fit.score,
  }));
  return {
    status: candidates.length ? "eligible" : "blocked",
    catalogRevision: CATALOG_REVISION,
    candidates,
    otherCandidates,
    eligibleIds,
    excluded,
    manualStyle: manual ?? null,
    inheritedStyle: inherited ?? null,
    nextAction: candidates.length
      ? "Choose one eligible style, then freeze the page plan. candidates is the best-fitting short list in full; otherCandidates are also eligible, listed briefly: read one with get_presentation_style only if it may serve better. Do not list every style. The order and fit records are rule-based signals, not the decision: judge which eligible style serves this audience, purpose, density and content (data, formulas, images), reading each style's recipe, pageKinds and cautions. Do not choose by colour or keyword alone, and do not pick at random. State the reason in one sentence in decisionSummary so the user can see why and override it. Within the chosen style vary the composition page by page using its adapted page kinds. Native editability and prior target-application round-trip certification are separate requirements."
      : noCandidateAction(requirements),
  };
}

function checkedPlan(args, documents) {
  validate(FREEZE_PRESENTATION_SCHEMA, args);
  if (
    args.source &&
    (documents.python !== true ||
      args.source.pageNumbers.length !== args.requirements.slideCount)
  )
    throw invalid(
      "Source editing requires Python and one plan per target page",
    );
  const candidates = presentationCandidates(
    args.requirements,
    documents,
    args.source,
  );
  if (!candidates.candidates.length) throw invalid(candidates.nextAction);
  if (!candidates.eligibleIds.includes(args.styleId))
    throw invalid(
      "Style is not an eligible candidate or conflicts with the manual choice",
    );
  if (args.slides.length !== args.requirements.slideCount)
    throw invalid(
      `Plan page count differs from requirements: expected ${args.requirements.slideCount}, actual ${args.slides.length} (requirements.slideCount=${args.requirements.slideCount}, slides.length=${args.slides.length}). Revise the slides array to the exact requested count, including cover and closing for a new deck or only authorized target pages for a source edit. Do not change the requested count merely to fit extra plan entries.`,
    );
  if (new Set(args.slides.map((slide) => slide.id)).size !== args.slides.length)
    throw invalid("Duplicate page id");
  const assets = [];
  for (const [index, slide] of args.slides.entries()) {
    for (const [role, required] of Object.entries(
      args.requirements.nativeEditing,
    ))
      if (required && !slide.editing[role])
        throw invalid(
          `A page weakens required native editing: ${role}; plan page ${index + 1} (id=${slide.id}) has slides[${index}].editing.${role}=false but requirements.nativeEditing.${role}=true. Editing is a policy, not an inventory of objects present: set this page's ${role} editing flag to true even when it has no such object, retaining the global requirement. Do not add unnecessary objects or weaken requirements.nativeEditing to pass validation.`,
        );
    const image = ["user-image", "generated-image"].includes(slide.visual.kind);
    if (image !== (slide.visual.assetId !== undefined))
      throw invalid(
        "Image visuals require an assetId; native visuals must not name one",
      );
    if (image) assets.push(slide.visual.assetId);
    const { placement, textZone, aspect } = slide.visual;
    const given = [placement, textZone, aspect].filter(
      (value) => value !== undefined,
    ).length;
    if (given && !image)
      throw invalid(
        `Plan page ${index + 1} (id=${slide.id}): only an image visual takes placement, textZone and aspect`,
      );
    if (given && given < 3)
      throw invalid(
        `Plan page ${index + 1} (id=${slide.id}): give the image's placement, textZone and aspect together`,
      );
    if (placement === "background" && textZone === "none")
      throw invalid(
        `Plan page ${index + 1} (id=${slide.id}): a background image needs a text zone that it keeps visually quiet`,
      );
    // Three pages in a row built the same way read as a template, not a deck.
    if (
      index >= 2 &&
      slide.pageKind !== undefined &&
      slide.pageKind === args.slides[index - 1].pageKind &&
      slide.pageKind === args.slides[index - 2].pageKind &&
      !slide.continuity?.trim()
    )
      throw invalid(
        `Plan pages ${index - 1}–${index + 1} repeat the page kind "${slide.pageKind}". Give page ${index + 1} a different page kind suited to its claim (the chosen style's pageKinds list what it draws natively), or state in its continuity field why the three pages read as one series.`,
      );
    if (
      ["native-math", "native-chart", "code-figure"].includes(
        slide.visual.kind,
      ) &&
      documents.python !== true
    )
      throw invalid(
        "Native math, editable chart data and code figures require Python",
      );
  }
  if (new Set(assets).size > MAX_ASSETS)
    throw invalid(`At most ${MAX_ASSETS} image assets in a plan`);
  return candidates;
}

// This is a private runtime result, never a model tool result or a host path.
// A deck places at most this many pictures: up to eight generated for it, and
// pictures the conversation already holds (the Host's MAX_ASSETS).
export function checkedPresentationAssets(
  result,
  requested,
  { maxTotalBytes = 20 * 1024 * 1024 } = {},
) {
  if (
    !result ||
    !Array.isArray(result.assets) ||
    result.assets.length !== requested.length ||
    requested.length > MAX_ASSETS
  )
    throw assetInvalid("Incomplete asset resolution");
  const mapped = new Map();
  let total = 0;
  for (const asset of result.assets) {
    if (
      !asset ||
      Object.keys(asset).some(
        (key) =>
          ![
            "assetId",
            "mimeType",
            "sha256",
            "dataBase64",
            "width",
            "height",
          ].includes(key),
      ) ||
      !Check(id, asset.assetId)
    )
      throw assetInvalid("Unexpected asset fields or invalid ID");
    if (!requested.includes(asset.assetId) || mapped.has(asset.assetId))
      throw assetInvalid("Unknown or duplicate asset ID");
    if (
      !["image/png", "image/jpeg"].includes(asset.mimeType) ||
      typeof asset.dataBase64 !== "string" ||
      asset.dataBase64.length > 14_000_000
    )
      throw assetInvalid("Unsupported or oversized asset");
    const bytes = Buffer.from(asset.dataBase64, "base64");
    total += bytes.length;
    if (
      !bytes.length ||
      bytes.length > 10 * 1024 * 1024 ||
      total > maxTotalBytes ||
      bytes.toString("base64") !== asset.dataBase64 ||
      sha(bytes) !== asset.sha256
    )
      throw assetInvalid("Asset bytes or hash do not match");
    if (
      asset.mimeType === "image/png"
        ? !bytes
            .subarray(0, 8)
            .equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))
        : !bytes.subarray(0, 3).equals(Buffer.from([255, 216, 255]))
    )
      throw assetInvalid("Asset MIME does not match bytes");
    const uri = "data:" + asset.mimeType + ";base64," + asset.dataBase64;
    const dimensions = imageSize(uri);
    if (!dimensions || dimensions.width * dimensions.height > 40_000_000)
      throw assetInvalid("Invalid image dimensions");
    if (
      (asset.width !== undefined && asset.width !== dimensions.width) ||
      (asset.height !== undefined && asset.height !== dimensions.height)
    )
      throw assetInvalid("Asset dimensions do not match");
    mapped.set(asset.assetId, uri);
  }
  return mapped;
}

// Display only a bounded Host-produced snapshot. This is not a reservation,
// and neither a model argument nor the user's prose can create this authority.
function visualBudgetSnapshot(value) {
  const fields = [
    "mode",
    "maxImages",
    "maxCostUsd",
    "remainingImages",
    "remainingCostUsd",
    "canAttempt",
    "reason",
    "scope",
  ];
  const micros = (amount) => {
    if (typeof amount !== "string" || amount.length > 32) return null;
    const match = /^(\d*)(?:\.(\d{0,6}))?$/.exec(amount);
    if (!match || !(match[1] || match[2])) return null;
    const result =
      BigInt(match[1] || "0") * 1_000_000n +
      BigInt((match[2] || "").padEnd(6, "0"));
    return result <= 999_000_000n ? result : null;
  };
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).some((key) => !fields.includes(key)) ||
    !["off", "auto", "illustration", "exact"].includes(value.mode) ||
    !Number.isSafeInteger(value.maxImages) ||
    value.maxImages < 0 ||
    value.maxImages > 8 ||
    !Number.isSafeInteger(value.remainingImages) ||
    value.remainingImages < 0 ||
    value.remainingImages > value.maxImages ||
    typeof value.canAttempt !== "boolean" ||
    typeof value.reason !== "string" ||
    !/^[a-z][a-z-]{0,79}$/.test(value.reason) ||
    value.scope !== "freeze-snapshot-not-reservation"
  )
    return null;
  const countOnly = value.maxCostUsd === "count-only";
  const maximum = countOnly ? null : micros(value.maxCostUsd);
  const remaining = countOnly ? null : micros(value.remainingCostUsd);
  const unknownRemainder =
    value.remainingCostUsd === null &&
    value.canAttempt === false &&
    value.reason === "unknown-spend";
  if (
    (countOnly
      ? value.remainingCostUsd !== null
      : maximum === null ||
        (!unknownRemainder && (remaining === null || remaining > maximum))) ||
    (value.canAttempt &&
      (value.mode === "off" ||
        value.remainingImages === 0 ||
        (!countOnly && remaining === 0n) ||
        value.reason !==
          (countOnly
            ? "authorized-count-only"
            : "authorized-price-check-required")))
  )
    return null;
  return Object.fromEntries(fields.map((key) => [key, value[key]]));
}

export function createPresentationPlanning(runtime, context) {
  const documents = context.params.documents ?? {};
  if (
    documents.presentationPlanning !== "v1" ||
    !documents.formats?.includes("pptx")
  )
    return null;
  let frozen = null;
  let freezing = false;
  const shownPlan = () => ({
    status: "frozen",
    planId: frozen.planId,
    revision: frozen.revision,
    effectiveStyle: frozen.effectiveStyle,
    ...(frozen.source ? { source: clone(frozen.source) } : {}),
    visualBudget: frozen.visualBudget
      ? clone(frozen.visualBudget)
      : {
          canAttempt: false,
          reason: "budget-summary-unavailable",
          scope: "freeze-snapshot-not-reservation",
        },
    // Each planned image's region ratio, as the nearest size the image model offers.
    imageSizes: Object.fromEntries(
      (frozen.slides ?? [])
        .filter((slide) => slide.visual?.assetId && slide.visual.aspect)
        .map((slide) => [
          slide.visual.assetId,
          nearestSize(slide.visual.aspect, context.params.media?.image?.sizes),
        ])
        .filter(([, size]) => size),
    ),
    nextAction: frozen.visualBudget?.canAttempt
      ? "For an asset listed in imageSizes pass that size to generate_image, so the picture is composed for its region on the page. Visual spending is already authorized within this Host snapshot; do not ask for that authorization again. For a needed planned image, call generate_image with its frozen references and the model named by the user, if any. This snapshot is not a reservation: price and reservation remain Host-checked on every attempt and may be refused. On failure use the declared fallback and report it; never claim an image was generated without success."
      : frozen.visualBudget
        ? "This Host snapshot does not permit a paid image attempt. Use supplied assets or the declared native/omit fallback and report the limitation; do not bypass Host guards or infer new spending permission."
        : "Budget summary unavailable; do not infer paid authorization from a plan or prompt. Use supplied assets or the declared fallback and report that the trusted summary is unavailable.",
  });
  const canonical = (value) =>
    Array.isArray(value)
      ? value.map(canonical)
      : value && typeof value === "object"
        ? Object.fromEntries(
            Object.keys(value)
              .sort()
              .map((key) => [key, canonical(value[key])]),
          )
        : value;
  const tools = [
    runtime.wrapTool(
      context,
      "get_presentation_candidates",
      "Filter enabled native PPTX styles by structured requirements. Native-editable objects do not imply pre-certified target-application round-trip: normally use roundTrip=unverifiedAllowed and preserve nativeEditing flags. verifiedRequired is only for an explicit demand for prior target-application certification. Scenario matching is advisory. If status is blocked, follow nextAction; do not freeze excluded styles, bypass freezing or repeat unchanged rejected constraints. Does not authorize image spending.",
      object({
        requirements: PRESENTATION_REQUIREMENTS_SCHEMA,
        source: Type.Optional(PRESENTATION_SOURCE_SCHEMA),
      }),
      async (args) => {
        validate(
          object({
            requirements: PRESENTATION_REQUIREMENTS_SCHEMA,
            source: Type.Optional(PRESENTATION_SOURCE_SCHEMA),
          }),
          args,
        );
        const result = presentationCandidates(
          args.requirements,
          documents,
          args.source,
        );
        return {
          content: [{ type: "text", text: JSON.stringify(result) }],
          details: result,
        };
      },
    ),
    runtime.wrapTool(
      context,
      "freeze_presentation_plan",
      "Submit a structured PPTX page plan and a style from nonempty eligible candidates for authoritative Host freezing. User manual choice has priority. Native editing is independent of prior target-application round-trip certification. If no candidate is eligible, explain the constraint conflict instead of repeating an unchanged rejected freeze or skipping this tool. A plan grants no image budget: the returned visualBudget is the Host-approved freeze-time snapshot. If canAttempt is true, do not request the same authorization again; every image call still needs Host price/budget checks. Unavailable images follow each page's fallback.",
      FREEZE_PRESENTATION_SCHEMA,
      async (given, signal, toolCallId) => {
        const args = clone(given);
        const candidates = checkedPlan(args, documents);
        const request = {
          ...clone(args),
          catalogRevision: CATALOG_REVISION,
          candidateIds: candidates.eligibleIds,
        };
        const requestHash = sha(JSON.stringify(canonical(request)));
        if (frozen && frozen.requestHash !== requestHash)
          throw invalid("A different plan is already frozen for this run");
        if (!frozen) {
          if (freezing) throw invalid("A plan freeze is already in progress");
          freezing = true;
          try {
            const result = await runtime.hostTool(
              context,
              "freeze_presentation_plan",
              request,
              signal,
              toolCallId,
            );
            if (
              result?.status !== "frozen" ||
              !Check(id, result.planId) ||
              !Number.isSafeInteger(result.revision) ||
              result.revision < 1 ||
              result.effectiveStyle !== args.styleId
            )
              throw invalid(
                "Host did not freeze the submitted effective style",
              );
            frozen = {
              planId: result.planId,
              revision: result.revision,
              effectiveStyle: result.effectiveStyle,
              visualBudget: visualBudgetSnapshot(result.visualBudget),
              slides: clone(args.slides),
              ...(args.source ? { source: clone(args.source) } : {}),
              requestHash,
            };
          } finally {
            freezing = false;
          }
        }
        const shown = shownPlan();
        return {
          content: [{ type: "text", text: JSON.stringify(shown) }],
          details: shown,
        };
      },
    ),
  ];
  return {
    tools,
    current: () => frozen,
    bindDocument(given) {
      // Take a call snapshot before any asset or Host await. Only this run's
      // Host-accepted immutable tuple may fill absent courier fields.
      const args = clone(given);
      if (
        args.format === "pptx" &&
        frozen &&
        !freezing &&
        (args.planId === undefined || args.planId === frozen.planId) &&
        (args.revision === undefined || args.revision === frozen.revision)
      ) {
        if (args.planId === undefined) args.planId = frozen.planId;
        if (args.revision === undefined) args.revision = frozen.revision;
      }
      return args;
    },
    problem(args) {
      if (args.format !== "pptx")
        return args.planId !== undefined || args.revision !== undefined
          ? "Plan references apply only to PPTX"
          : null;
      if (!frozen)
        return freezing
          ? "The Host has not accepted the pending presentation plan yet. Wait for that freeze result before generating or delivering a PPTX."
          : "No Host-frozen presentation plan exists for this run. Get eligible candidates, then freeze one plan containing all pages before generating or delivering a PPTX. Historical plan references cannot create a current plan.";
      if (args.planId !== frozen.planId || args.revision !== frozen.revision)
        return "Explicit planId or revision does not match the current Host-frozen plan. Omit these references to use this run's existing plan, or use its exact returned tuple. Do not freeze again or replace the existing plan.";
      return null;
    },
    async hydrate(deck, args, signal, toolCallId) {
      const snapshot = frozen;
      if (snapshot?.source)
        throw invalid(
          "Source editing requires Python source_deck; Node reconstruction is unsupported",
        );
      const copy = clone(deck);
      if (
        !snapshot ||
        !Array.isArray(copy.slides) ||
        copy.slides.length !== snapshot.slides.length
      )
        throw invalid("Deck count differs from the frozen page plan");
      const fallbacks = [];
      for (const [index, slide] of copy.slides.entries()) {
        const planned = snapshot.slides[index];
        if (
          ["native-math", "native-chart", "code-figure"].includes(
            planned.visual.kind,
          )
        )
          throw invalid(
            "Planned native chart, native math or code figure requires the Python route",
          );
        const needsImage = ["user-image", "generated-image"].includes(
          planned.visual.kind,
        );
        if (slide.media?.image !== undefined)
          throw assetInvalid("Planned images must use a controlled assetId");
        if (
          slide.media?.assetId !== undefined &&
          (Object.keys(slide.media).length !== 1 ||
            slide.media.assetId !== planned.visual.assetId)
        )
          throw assetInvalid("Image asset does not match this frozen page");
        if (slide.media?.assetId !== undefined && slide.type !== "showcase")
          throw assetInvalid(
            "A controlled image requires an actual showcase page; other slide types do not render media",
          );
        if (needsImage && slide.media?.assetId === undefined) {
          if (planned.visual.fallback === "stop")
            throw assetInvalid(
              "Required image is missing from page " +
                (index + 1) +
                "; the frozen fallback is stop",
            );
          fallbacks.push({
            page: index + 1,
            slideId: planned.id,
            assetId: planned.visual.assetId,
            fallback: planned.visual.fallback,
          });
        }
      }
      const assetIds = [
        ...new Set(
          copy.slides.flatMap((slide) =>
            slide.media?.assetId === undefined ? [] : [slide.media.assetId],
          ),
        ),
      ];
      if (!assetIds.length) return { deck: copy, fallbacks };
      const allowed = snapshot.slides.flatMap((slide) =>
        slide.visual.assetId ? [slide.visual.assetId] : [],
      );
      if (assetIds.some((assetId) => !allowed.includes(assetId)))
        throw assetInvalid("Asset is not part of the frozen plan");
      const result = await runtime.hostTool(
        context,
        "resolve_presentation_assets",
        { planId: args.planId, revision: args.revision, assetIds },
        signal,
        toolCallId,
      );
      if (frozen !== snapshot)
        throw invalid("Plan changed during asset resolution");
      const assets = checkedPresentationAssets(result, assetIds, {
        maxTotalBytes: 1024 * 1024,
      });
      for (const slide of copy.slides)
        if (slide.media?.assetId !== undefined) {
          if (Object.keys(slide.media).length !== 1)
            throw assetInvalid("assetId cannot be combined with other media");
          slide.media = { image: assets.get(slide.media.assetId) };
        }
      return { deck: copy, fallbacks };
    },
  };
}
