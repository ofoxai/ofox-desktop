import { Type } from "typebox";
import { ADAPTED_STYLES, PRESENTATION_CATALOG } from "./deck/catalog.mjs";
import { themeFor } from "./deck/theme.mjs";

const baseline = {
  id: "navy",
  name: "Navy (compatible default)",
  scenarios: ["general"],
  description: "Original Desktop layout and typography.",
  validation: { visualQuality: "pending" },
};

const workflow = {
  pagePlan: [
    "claim and evidence/source",
    "information relationship and primary visual",
    "composition, density and text hierarchy",
    "native editing requirements and fit checks",
  ],
  select:
    "Choose by audience, purpose, density and contrast; retain one coherent type system. Do not pick a color alone.",
  review:
    "Review the exported PPTX overview and reported issues; verify facts, reading order, clipping, chart labels and formula editing. Fix at most twice. Without vision, state that visual review is unavailable.",
  references: [
    {
      label: "Public style hierarchy",
      url: "https://github.com/zarazhangrui/beautiful-html-templates/tree/e5e204fb1f3b06290846e7dcd7aceddabeceec8c/templates",
    },
    {
      label: "Deck creation workflow",
      url: "https://github.com/manalkaff/opendesign/blob/cecd9bb6b59408cb96a3974449b8e6ef9f5b17bb/skills/make-a-deck/SKILL.md",
    },
    {
      label: "Exported deck review and source invalidation",
      url: "https://github.com/icip-cas/PPTAgent/tree/9dacac53ac9ef39346461063df729396551d929e",
    },
  ],
};

function mappingCounts(mapping) {
  const counts = { adapted: 0, approximate: 0, unsupported: 0 };
  for (const page of mapping) counts[page.status] += 1;
  return counts;
}

export function presentationStyleTool(runtime, context) {
  return runtime.wrapTool(
    context,
    "get_presentation_style",
    "List available PPTX styles or get one style's native adaptation, provenance, composition recipe and design checks. Choices are experimental; candidate resources are not verified PPTX templates. Does not run HTML or fetch assets.",
    Type.Object({ id: Type.Optional(Type.String({ maxLength: 80 })) }),
    async ({ id }) => {
      const styles = [baseline, ...ADAPTED_STYLES];
      let result;
      if (id === undefined)
        result = {
          candidateCount: PRESENTATION_CATALOG.styles.length,
          styles: styles.map((style) => ({
            id: style.id,
            name: style.name,
            family: style.family,
            scheme: style.routing?.scheme ?? style.scheme,
            scenarios: style.scenarios,
            density: style.density,
            description: style.intro?.en ?? style.description,
            avoid: style.routing?.avoid || undefined,
            ...(style.pageMapping
              ? {
                  sourcePages: style.pageMapping.length,
                  nativePages: mappingCounts(style.pageMapping),
                }
              : {}),
            validation: style.validation,
          })),
          workflow,
          note: "Adapted choices are selectable for evaluation; enabled does not mean visual quality or PowerPoint round-trip acceptance has passed. sourcePages is the upstream deck's actual page count; nativePages counts how those pages map to native objects (adapted, approximate with a named difference, unsupported). HTML assets require separate adaptation, not a screenshot conversion.",
        };
      else {
        const style = styles.find((item) => item.id === id);
        if (!style)
          throw new Error(
            `Unknown presentation style: ${id}. List available IDs first.`,
          );
        const source = PRESENTATION_CATALOG.sources.find(
          (item) => item.id === style.sourceId,
        );
        result = {
          id,
          name: style.name,
          scenarios: style.scenarios,
          description: style.description,
          source: source
            ? {
                repository: source.repository,
                revision: source.revision,
                license: source.license,
                path: style.sourcePath,
                attribution: source.attribution,
              }
            : { repository: "Ofox Desktop", revision: "compatible-default" },
          theme: themeFor(id),
          fontPolicy: style.fontPolicy,
          assetReview: style.assetReview,
          recipe: style.recipe ?? {
            composition: "Original centered cover and card pages",
          },
          family: style.family,
          routing: style.routing,
          // How each actual source page is drawn natively, and what differs.
          pageMapping: style.pageMapping,
          validation: style.validation,
          workflow,
          editability: {
            text: "native",
            charts:
              "Python native_chart/add_chart embeds chart data; Node deck media.chart requires separate data-editing verification",
            formulas:
              "Python equation helper only; formula and SVG/plot remain non-semantic",
            htmlConversion: "unsupported",
            roundTrip: "pending",
          },
        };
      }
      if (context.params.documents?.presentationPlanning === "v1")
        result.planning = {
          requirements: "get_presentation_candidates",
          freeze: "freeze_presentation_plan",
          generation: "Pass the frozen planId and revision to create_document",
          imageBudget: "Host-controlled; a page plan cannot authorize spending",
        };
      return {
        content: [{ type: "text", text: JSON.stringify(result) }],
        details: result,
      };
    },
  );
}
