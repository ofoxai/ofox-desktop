# Builtin PPTX skill priming

The Host now loads the packaged `ofox-office-pptx` instruction body during admission of an eligible PPT run. The trigger is the Host's presentation context (a presentation style choice or a verified PPT page reference), together with a tool-capable model, an available runtime and PPTX engine. `presentationWorkflowVersion: 1` alone does not establish PPT intent, so ordinary chat does not receive this body.

An explicit slash skill keeps its existing selection path. A user skill that shadows the builtin name prevents automatic loading; the primer never falls through to reading that user skill. Automatic loading takes the fixed name and directory from the admitted builtin snapshot, checks the canonical directory and scoped file access, and validates the run and identity before and after the read. Missing, unreadable, invalid or oversized instruction bodies fail without claiming successful loading.

Priming reads only `SKILL.md`, bounded to 1 MiB and 60,000 code points, with its frontmatter name revalidated. The resource directory has more than 200 files; truncation of a general skill tool's file inventory must not be confused with incomplete instruction text. The primer supplies the complete body without enumerating that inventory.

The prompt labels the body as bundled source material, XML-escapes its original text and distinguishes the managed document engine from local shell execution. Only a successful read adds the Host-internal `documents.primedBuiltinSkills` marker. No user skill choice, synthetic tool activity, model request, spending authorization or tool permission is added by priming. Planned guidance consistently orders candidates, the chosen style recipe, and plan freezing before generation.

Compatibility limitation: eligible legacy PPT requests also receive the body, while legacy runtime guidance still asks for `use_skill`. A duplicate skill call is therefore possible. This preserves the existing legacy guidance contract; it does not expand execution or spending permissions. Automatic priming also does not prove that a model follows the helper APIs or that a generated presentation has passed visual or target-application editing acceptance.

Validation: the initial Host regression run had 5 behavioral failures and 1 passing shadowing case. After implementation, the root agent's focused Rust run passed all 8 primer and prompt tests. Independent read-only review found no new blocking issue. Broader gates, a rebuilt application and a fresh model end-to-end run remain separately recorded by the root agent.

Separate existing metric limitation: `modelRequests` currently adds document generation records to its provider-request count. A completed local document can therefore increase that metric without another provider request. This patch does not change that aggregation; native request receipts and invoice evidence remain distinct.
