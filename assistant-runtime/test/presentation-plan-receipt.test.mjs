import assert from "node:assert/strict";
import test from "node:test";
import { nearestSize, planDrift } from "../src/presentation-planning.mjs";

const page = (pageKind, kind = "none") => ({
  id: "p",
  pageKind,
  visual: { kind, purpose: "x", fallback: "omit" },
});

test("a built-page receipt is taken out of the issues and compared with the frozen plan", () => {
  const plan = {
    slides: [
      page("cover"),
      page("chart-bar", "native-chart"),
      page("stats"),
      page("table"),
      page("process", "native-diagram"),
      page("closing"),
    ],
  };
  const issues = [
    "plan-receipt:1=C;2=bu;3=n3;4=tb;5=k2;6=C",
    "slide 3: something real",
  ];
  const result = planDrift(issues, plan);
  // The receipt itself is never shown as an issue.
  assert.deepEqual(result.issues.slice(0, 1), ["slide 3: something real"]);
  const drift = result.issues.slice(1).join("\n");
  assert.match(drift, /page 2 .*native chart.*none/);
  assert.match(drift, /page 2 .*page kind "chart-bar"/);
  assert.match(drift, /page 5 .*native diagram/);
  assert.doesNotMatch(drift, /page (1|3|4|6) /);
  assert.equal(result.receipt.length, 6);
  // Without a plan or a receipt, issues pass through untouched.
  assert.deepEqual(planDrift(["a"], undefined).issues, ["a"]);
  assert.deepEqual(planDrift(["a"], plan).issues, ["a"]);
});

test("a code figure is planned as one and is drift only when no figure or plot was drawn", () => {
  const plan = {
    slides: [
      page("process", "code-figure"),
      page("cards", "code-figure"),
      page("cards", "code-figure"),
    ],
  };
  const { issues } = planDrift(["plan-receipt:1=fg,st;2=pl,k2;3=k3"], plan);
  assert.equal(issues.length, 1);
  assert.match(issues[0], /page 3 planned a code figure/);
  assert.match(issues[0], /figure\(\) or plot\(\)/);
});

test("a code figure meets a diagram or chart page kind", () => {
  const plan = {
    slides: [
      page("diagram", "code-figure"),
      page("chart-line", "code-figure"),
      page("diagram", "native-diagram"),
    ],
  };
  const { issues } = planDrift(["plan-receipt:1=fg;2=pl;3=fg"], plan);
  assert.equal(issues.length, 2, issues.join("\n"));
  assert.ok(issues.every((issue) => /page 3 /.test(issue)));
});

test("a planned image region maps to the nearest size the image model offers", () => {
  const sizes = ["1024x1024", "1536x1024", "1024x1536", "auto"];
  assert.equal(nearestSize("16:9", sizes), "1536x1024");
  assert.equal(nearestSize("4:3", sizes), "1536x1024");
  assert.equal(nearestSize("1:1", sizes), "1024x1024");
  assert.equal(nearestSize("9:16", sizes), "1024x1536");
  assert.equal(nearestSize("4:3", ["auto"]), undefined);
  assert.equal(nearestSize("4:3", undefined), undefined);
});
