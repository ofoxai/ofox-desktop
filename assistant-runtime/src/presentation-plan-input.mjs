// Model input aliases only. Canonical plan validators and Host enums stay strict.
// Preserve every other field so malformed or unauthorized input is still refused.
export function normalizePresentationPlanInput(given) {
  const args = JSON.parse(JSON.stringify(given));
  if (Array.isArray(args?.slides)) {
    for (const page of args.slides) {
      if (page && !Array.isArray(page) && page.relationship === "process")
        page.relationship = "sequence";
    }
  }
  return args;
}
