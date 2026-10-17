import type { LibraryKind } from "@/lib/api/library";
import { LIBRARY_RECOMMENDATIONS } from "./libraryRecommendations";

export interface LibraryScenario {
  id: "frontend" | "docs" | "review";
  resources: { recipeId: string; optional?: boolean }[];
}
export const LIBRARY_SCENARIOS: readonly LibraryScenario[] = [
  {
    id: "frontend",
    resources: [
      { recipeId: "coding" },
      { recipeId: "context7" },
      { recipeId: "frontend-design" },
      { recipeId: "webapp-testing" },
      { recipeId: "playwright", optional: true },
    ],
  },
  {
    id: "docs",
    resources: [
      { recipeId: "microsoft-learn" },
      { recipeId: "deepwiki" },
      { recipeId: "context7", optional: true },
      { recipeId: "chinese", optional: true },
    ],
  },
  {
    id: "review",
    resources: [
      { recipeId: "review" },
      { recipeId: "context7" },
      { recipeId: "github", optional: true },
    ],
  },
];
export const SCENARIO_KIND_ORDER: LibraryKind[] = [
  "instructions",
  "mcp",
  "skills",
];
export function scenarioRecipes(ids: string[]) {
  return SCENARIO_KIND_ORDER.flatMap((kind) =>
    LIBRARY_RECOMMENDATIONS.filter(
      (recipe) => recipe.kind === kind && ids.includes(recipe.id),
    ),
  );
}
