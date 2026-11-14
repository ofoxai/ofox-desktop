import assert from "node:assert/strict";
import test from "node:test";
import {
  presentationCandidates,
  styleFit,
} from "../src/presentation-planning.mjs";
import { ADAPTED_STYLES } from "../src/deck/catalog.mjs";
import { Check } from "typebox/value";
import { FREEZE_PRESENTATION_SCHEMA } from "../src/presentation-planning.mjs";

const requirements = (extra = {}) => ({
  audience: "Engineers",
  purpose: "Explain a system design",
  scenario: "technical",
  language: "zh",
  slideCount: 8,
  aspectRatio: "16:9",
  density: "medium",
  nativeEditing: { text: true, charts: true, formulas: false, diagrams: true },
  visualNeeds: ["process"],
  roundTrip: "unverifiedAllowed",
  ...extra,
});
const style = (routing) => ({
  id: "sample",
  scenarios: routing.scenarios,
  density: routing.density,
  routing,
});

test("rule-based fit scores hard signals only and explains each caution", () => {
  const poster = style({
    scenarios: ["creative"],
    density: "low",
    scheme: "dark",
    suits: { data: 1, formulas: 0, images: 2, longText: 0 },
    avoid: "Text-heavy material.",
  });
  const paper = style({
    scenarios: ["technical", "research"],
    density: "medium",
    scheme: "light",
    suits: { data: 2, formulas: 2, images: 1, longText: 2 },
    avoid: "",
  });
  const mathHeavy = requirements({
    contentMix: { data: 0.3, formulas: 0.3, images: 0 },
    nativeEditing: { text: true, charts: true, formulas: true, diagrams: true },
  });
  const weak = styleFit(poster, mathHeavy);
  const strong = styleFit(paper, mathHeavy);
  assert.ok(strong.score > weak.score);
  assert.equal(strong.scenario, true);
  assert.equal(weak.scenario, false);
  assert.ok(weak.cautions.some((note) => /formula/i.test(note)));
  assert.deepEqual(strong.cautions, []);
  // A dense deck on a style that cannot carry long text is cautioned, not excluded.
  assert.ok(
    styleFit(poster, requirements({ density: "high" })).cautions.some((note) =>
      /dense|long text/i.test(note),
    ),
  );
  // A style without routing data (built-in navy) stays neutral.
  assert.deepEqual(
    styleFit({ id: "navy", scenarios: ["general"] }, mathHeavy).cautions,
    [],
  );
});

test("every enabled style is an ordered candidate with family, scheme, page kinds and a fit record", () => {
  const result = presentationCandidates(requirements(), { python: true });
  assert.equal(result.eligibleIds.length, ADAPTED_STYLES.length + 1);
  const scores = result.candidates.map((item) => item.fit.score);
  assert.deepEqual(
    scores,
    [...scores].sort((a, b) => b - a),
    "ordered by rule-based fit",
  );
  for (const candidate of result.candidates) {
    assert.equal(typeof candidate.fit.scenario, "boolean");
    assert.ok(Array.isArray(candidate.fit.cautions));
    if (candidate.id === "navy") continue;
    const source = ADAPTED_STYLES.find((item) => item.id === candidate.id);
    if (!source.pageMapping) continue;
    assert.equal(candidate.family, source.family);
    assert.equal(candidate.scheme, source.routing.scheme);
    const kinds = [
      ...candidate.pageKinds.adapted,
      ...candidate.pageKinds.approximate,
      ...candidate.pageKinds.unsupported,
    ];
    assert.deepEqual(
      [...new Set(kinds)].sort(),
      [...new Set(source.pageMapping.map((page) => page.archetype))].sort(),
    );
  }
  // The Host compares eligibility in catalog order, independent of ranking.
  assert.deepEqual(result.eligibleIds, [
    "navy",
    ...ADAPTED_STYLES.map((item) => item.id),
  ]);
  assert.match(result.nextAction, /decisionSummary/);
  assert.match(result.nextAction, /not.*(colou?r|keyword)/i);
});

test("a manual choice stays the only candidate however poorly it scores", () => {
  const manual = ADAPTED_STYLES[0].id;
  const result = presentationCandidates(
    requirements({ scenario: "no-such-scenario", density: "high" }),
    { python: true, selectedStyle: manual },
  );
  assert.deepEqual(
    result.candidates.map((item) => item.id),
    [manual],
  );
});

test("an empty deck image look is refused by the runtime as the Host refuses it", () => {
  const plan = {
    requirements: requirements({ slideCount: 1 }),
    styleId: "navy",
    decisionSummary: "Plain",
    slides: [
      {
        id: "only",
        claim: "A claim",
        evidence: [],
        relationship: "single-claim",
        composition: "Open",
        editing: { text: true, charts: true, formulas: false, diagrams: true },
        visual: { kind: "none", purpose: "Text", fallback: "omit" },
      },
    ],
  };
  assert.equal(Check(FREEZE_PRESENTATION_SCHEMA, plan), true);
  assert.equal(
    Check(FREEZE_PRESENTATION_SCHEMA, { ...plan, imageStyle: "" }),
    false,
  );
});

test("the model reads a short list in full, the rest briefly, and may still choose any eligible style", () => {
  const result = presentationCandidates(requirements(), { python: true });
  assert.equal(result.candidates.length, 6);
  assert.equal(
    result.candidates.length + result.otherCandidates.length,
    result.eligibleIds.length,
  );
  for (const other of result.otherCandidates) {
    assert.deepEqual(Object.keys(other).sort(), [
      "family",
      "fitScore",
      "id",
      "scheme",
    ]);
    assert.ok(result.eligibleIds.includes(other.id));
  }
  // The short list is the best rule-based fit.
  const lowest = Math.min(...result.candidates.map((item) => item.fit.score));
  assert.ok(result.otherCandidates.every((item) => item.fitScore <= lowest));
  assert.match(result.nextAction, /otherCandidates/);
});
