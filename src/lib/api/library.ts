import { invoke } from "@tauri-apps/api/core";

export type LibraryKind = "instructions" | "mcp" | "skills";
export interface InstructionSet {
  id: string;
  name: string;
  content: string;
  revision: number;
}
export interface ResourceTarget {
  toolId: string;
  path: string;
  targetGroupId: string;
  available: boolean;
  discoveryStatus: string;
  status: string;
  desired?: boolean;
  applied?: boolean;
  desiredResourceId?: string | null;
  readers?: string[] | null;
  readable?: boolean | null;
  readableResourceId?: string | null;
  lastAppliedResourceId?: string | null;
  lastAppliedRevision?: number | null;
  userContentLines?: number | null;
  managedContentLines?: number | null;
  nativeState?: string | null;
  disabledReason?: string | null;
  /** This entry was taken over in place (import), not written by Ofox. */
  adopted?: boolean;
  /** An identical native entry is already here and nobody owns it. */
  adoptable?: boolean;
  entryKind?: "link" | "directory" | null;
}
/** An agent the library can give skills to (never the shared folder). */
export interface SkillTarget {
  id: string;
  label: string;
  /** Installed, or its home folder exists: show the column. */
  detected: boolean;
  /** Reads ~/.agents/skills itself. */
  readsShared: boolean;
  /** Its skills folder resolved. */
  scopeKnown: boolean;
}
export interface McpField {
  path: string[];
  fieldRef: string;
  valueType: string;
  exists: boolean;
}
export interface McpDefinition {
  id: string;
  name: string;
  revision: number;
  transport: string;
  credentialState: string;
  /** Shipped inside an app bundle (Codex's node_repl); the app rewrites it. */
  appOwned?: boolean;
  serviceHost?: string | null;
  displaySummary?: string | null;
  /** `command args…` or `host/path`; credential-looking values are masked. */
  commandLine?: string | null;
  iconId?: string | null;
  fields: McpField[];
  targets: ResourceTarget[];
}
export interface SkillDefinition {
  id: string;
  name: string;
  description?: string;
  directory: string;
  revision?: number;
  artifactHash?: string;
  sourceState: string;
  sourceUrl?: string;
  sourceKind?: "repository" | "localOrArchive" | "unknown";
  repository?: string;
  trackingRef?: string;
  commitSha?: string | null;
  installedAtMs?: number;
  updatedAtMs?: number;
  targets: ResourceTarget[];
}
export interface LibraryOperation {
  operationId: string;
  kind: LibraryKind;
  resourceId: string;
  resourceRevision: number;
  status: string;
  targets: {
    tools: string[];
    readers: string[];
    path: string;
    targetGroupId: string;
    status: string;
    reason?: string | null;
    beforeManaged?: string | null;
    afterManaged?: string | null;
    changedFields: string[];
    method?: string | null;
    plannedMethod?: string | null;
    artifactHash?: string | null;
    fallbackUsed?: boolean | null;
  }[];
}
export interface LibraryData {
  epoch: number;
  instructions?: InstructionSet[];
  servers?: McpDefinition[];
  skills?: SkillDefinition[];
  skillTargets?: SkillTarget[];
  targets?: ResourceTarget[];
  detachedTargets?: { path: string; resourceId: string; tools: string[] }[];
  operations: LibraryOperation[];
}
export interface FieldEdit {
  path: string[];
  fieldRef?: string;
  action: "keep" | "set" | "clear";
  value?: unknown;
}
export interface SkillDetail {
  resourceId: string;
  artifactHash: string;
  path: string;
  content: string;
  files: {
    path: string;
    isDirectory: boolean;
    sizeBytes?: number;
    executable: boolean;
  }[];
}

export interface ResourcePackPreview {
  fingerprint: string;
  revision: number;
  epoch: number;
  items: {
    kind: LibraryKind;
    id: string;
    name: string;
    conflict: boolean;
    omittedFields: number;
  }[];
}
export interface ResourcePackResult {
  imported: number;
  items: {
    kind: LibraryKind;
    name: string;
    status: "imported" | "skipped" | "failed";
    reason?: string | null;
  }[];
}
export interface SkillUpdateCheck {
  resourceId: string;
  revision: number;
  epoch: number;
  state: "updateAvailable" | "upToDate" | "notTracked" | "sourceMissing";
  currentArtifactHash: string | null;
  sourceVerified?: boolean;
  targetConflicts?: string[];
  candidate?: {
    ticketId: string;
    expiresAtMs: number;
    artifactHash: string;
    commitSha: string;
    name: string;
    description?: string | null;
    sourceUrl?: string;
    canCopy?: boolean;
    copyBlockedReason?: string | null;
  };
}
export interface SkillUpdateComparison {
  beforeVerified?: boolean;
  canCopy?: boolean;
  beforeState?: "verified" | "unverified";
  path: string;
  before: string | null;
  after: string | null;
  files: {
    path: string;
    status: string;
    beforeExecutable: boolean | null;
    afterExecutable: boolean;
  }[];
}
export interface SkillDraftChange {
  state: "updated" | "copied" | "unchanged";
  resourceId: string;
  revision: number;
  artifactHash: string;
  nativeApplyRequired: boolean;
}
export interface McpCheckProgress {
  operationId: string;
  stage: "preflight" | "connect" | "negotiate" | "list" | "cleanup";
}
export interface McpCheckResult {
  operationId: string;
  state:
    | "ok"
    | "blocked"
    | "authRequired"
    | "forbidden"
    | "timeout"
    | "cancelled"
    | "error";
  reason?: string;
  tools: { name: string; description?: string | null }[];
}
export interface LibraryInventory {
  ticketId: string;
  kind: LibraryKind;
  expiresAtMs: number;
  observedAtMs?: number;
  sources: {
    sourceId: string;
    toolId: string;
    label: string;
    state: string;
    reason?: string | null;
    count: number;
    observedAtMs?: number;
    locationKind?: "file" | "directory" | "inline" | "legacy";
    displayPath?: string | null;
    sourceRef?: string | null;
  }[];
  entries: {
    entryId: string;
    kind: LibraryKind;
    name: string;
    summary?: string | null;
    displaySummary?: string | null;
    iconId?: string | null;
    serviceHost?: string | null;
    sources: { sourceId: string; toolId: string; disabled?: boolean }[];
    matchedResourceIds: string[];
    conflict: boolean;
    variant?: string | null;
    directory?: string | null;
    locations?: {
      /** A target id or "shared". */
      targetId: string;
      /** Home shown as ~. */
      displayPath: string;
      link: boolean;
      /** Can be taken over in place on import. */
      adoptable: boolean;
      /** A library.* key when not adoptable. */
      reason?: string | null;
    }[];
  }[];
}
export interface InventorySourceDetail {
  ticketId: string;
  entryId: string;
  sourceRef: string;
  toolId: string;
  label: string;
  locationKind: "file" | "directory" | "inline" | "legacy";
  displayPath?: string | null;
  logicalPath?: string | null;
  canonicalPath?: string | null;
  isSymlink: boolean;
  observedAtMs: number;
  canReveal: boolean;
  matchedResourceIds: string[];
  fields: { path: string[]; valueType: string }[];
}
export type McpComparisonSubject =
  | { kind: "native"; ticketId: string; entryId: string }
  | { kind: "resource"; resourceId: string; expectedRevision: number };
export interface McpComparison {
  fields: {
    path: string[];
    leftType?: string | null;
    rightType?: string | null;
    state: "equal" | "different" | "missingLeft" | "missingRight";
  }[];
}
export interface McpRecipeMatch {
  recipeId: string;
  exactResourceIds: string[];
  nativeEntryIds: string[];
  similarResourceIds: string[];
  similarNativeEntryIds: string[];
}
export interface InventoryImport {
  imported: number;
  items: {
    entryId: string;
    resourceId?: string | null;
    name: string;
    status: string;
    reason?: string | null;
    /** Target ids taken over in place. */
    adopted?: string[];
    notAdopted?: { targetId: string; reason: string }[];
  }[];
}
export interface RemovedFoundSkill {
  removed: { targetId: string; displayPath: string; backup?: string | null }[];
}
export interface BatchResource {
  resourceId: string;
  expectedRevision: number;
  addTools?: string[];
  withdrawTools?: string[];
}
export interface LibraryBatchChange {
  resourceId: string;
  name: string;
  action: "apply" | "withdraw";
  tools?: string[];
  changedFields: string[];
  method?: string | null;
  status?: string;
}
export interface LibraryBatchPlan {
  operationId: string;
  kind: LibraryKind;
  status: string;
  resources: { resourceId: string; name?: string; expectedRevision?: number }[];
  groups: {
    groupId: string;
    tools: string[];
    readers: string[];
    path: string;
    status: string;
    reason?: string | null;
    beforeManaged?: string | null;
    afterManaged?: string | null;
    resourceIds: string[];
    changedTools?: string[];
    changes?: LibraryBatchChange[];
  }[];
  metadataChanges?: LibraryBatchChange[];
  completed?: number;
  remaining?: number;
}
export interface LibraryBuiltinRecipe {
  recipeId: string;
  content?: string;
  repoOwner?: string;
  repoName?: string;
  directory?: string;
}
export const libraryApi = {
  matchBuiltinRecipes: (
    kind: LibraryKind,
    recipes: LibraryBuiltinRecipe[],
    inventoryTicketId: string | undefined,
    expectedEpoch: number,
  ) =>
    invoke<McpRecipeMatch[]>("match_library_builtin_recipes", {
      kind,
      recipes,
      inventoryTicketId,
      expectedEpoch,
    }),
  inventorySource: (
    ticketId: string,
    entryId: string,
    sourceRef: string,
    expectedEpoch: number,
  ) =>
    invoke<InventorySourceDetail>("get_library_inventory_source", {
      ticketId,
      entryId,
      sourceRef,
      expectedEpoch,
    }),
  revealInventorySource: (
    ticketId: string,
    entryId: string,
    sourceRef: string,
    expectedEpoch: number,
  ) =>
    invoke<void>("reveal_library_inventory_source", {
      ticketId,
      entryId,
      sourceRef,
      expectedEpoch,
    }),
  compareMcp: (
    left: McpComparisonSubject,
    right: McpComparisonSubject,
    expectedEpoch: number,
  ) =>
    invoke<McpComparison>("compare_library_mcp", {
      left,
      right,
      expectedEpoch,
    }),
  matchMcpRecipes: (
    recipes: { recipeId: string; name?: string; definition: unknown }[],
    inventoryTicketId: string | undefined,
    expectedEpoch: number,
  ) =>
    invoke<McpRecipeMatch[]>("match_library_mcp_recipes", {
      recipes,
      inventoryTicketId,
      expectedEpoch,
    }),
  discardInventory: (ticketId: string) =>
    invoke<boolean>("discard_library_inventory", { ticketId }),
  inventory: (kind: LibraryKind) =>
    invoke<LibraryInventory>("scan_library_inventory", { kind }),
  importInventory: (
    ticketId: string,
    entryIds: string[],
    expectedEpoch: number,
  ) =>
    invoke<InventoryImport>("import_library_inventory", {
      ticketId,
      entryIds,
      expectedEpoch,
    }),
  previewBatch: (
    kind: LibraryKind,
    resources: BatchResource[],
    tools: string[],
    action: "apply" | "withdraw",
    expectedEpoch: number,
  ) =>
    invoke<LibraryBatchPlan>("preview_library_batch", {
      kind,
      resources,
      tools,
      action,
      expectedEpoch,
    }),
  runBatch: (operationId: string) =>
    invoke<LibraryBatchPlan>("run_library_batch", { operationId }),
  cancelBatch: (operationId: string) =>
    invoke<LibraryBatchPlan>("cancel_library_batch", { operationId }),
  reconcileBatch: (operationId: string) =>
    invoke<LibraryBatchPlan>("reconcile_library_batch", { operationId }),
  batches: (kind: LibraryKind) =>
    invoke<LibraryBatchPlan[]>("list_library_batches", { kind }),
  prepareMcpCheck: (
    resourceId: string,
    expectedRevision: number,
    expectedEpoch: number,
  ) =>
    invoke<{ operationId: string }>("prepare_library_mcp_check", {
      resourceId,
      expectedRevision,
      expectedEpoch,
    }),
  runMcpCheck: (operationId: string) =>
    invoke<McpCheckResult>("run_library_mcp_check", { operationId }),
  cancelMcpCheck: (operationId: string) =>
    invoke<boolean>("cancel_library_mcp_check", { operationId }),
  checkSkillUpdate: (
    resourceId: string,
    expectedRevision: number,
    expectedEpoch: number,
  ) =>
    invoke<SkillUpdateCheck>("check_library_skill_update", {
      resourceId,
      expectedRevision,
      expectedEpoch,
    }),
  compareSkillUpdate: (ticketId: string, relativePath: string) =>
    invoke<SkillUpdateComparison>("compare_library_skill_update", {
      ticketId,
      relativePath,
    }),
  confirmSkillUpdate: (
    ticketId: string,
    expectedRevision: number,
    expectedEpoch: number,
    mode: "update" | "copy",
    name?: string,
    directory?: string,
  ) =>
    invoke<SkillDraftChange>("confirm_library_skill_update", {
      ticketId,
      expectedRevision,
      expectedEpoch,
      mode,
      name,
      directory,
    }),
  discardSkillUpdate: (ticketId: string) =>
    invoke<boolean>("discard_library_skill_update", { ticketId }),
  duplicateMcp: (
    sourceId: string,
    expectedRevision: number,
    expectedEpoch: number,
    newId: string,
    name: string,
  ) =>
    invoke<Omit<McpDefinition, "targets">>("duplicate_library_mcp", {
      sourceId,
      expectedRevision,
      expectedEpoch,
      newId,
      name,
    }),
  previewMcpAdoption: (
    resourceId: string,
    tools: string[],
    adoptTools: string[],
    expectedRevision: number,
    expectedEpoch: number,
  ) =>
    invoke<LibraryOperation>("preview_library_mcp_adoption", {
      resourceId,
      tools,
      adoptTools,
      expectedRevision,
      expectedEpoch,
    }),
  previewSkillAdoption: (
    resourceId: string,
    tools: string[],
    adoptTools: string[],
    expectedRevision: number,
    expectedEpoch: number,
  ) =>
    invoke<LibraryOperation>("preview_library_skill_adoption", {
      resourceId,
      tools,
      adoptTools,
      expectedRevision,
      expectedEpoch,
    }),
  /** `toolIds` [] removes every location; folders are archived, links unlinked. */
  removeFoundSkill: (
    ticketId: string,
    entryId: string,
    toolIds: string[],
    expectedEpoch: number,
  ) =>
    invoke<RemovedFoundSkill>("remove_library_found_skill", {
      ticketId,
      entryId,
      toolIds,
      expectedEpoch,
    }),
  pickPack: () => invoke<string | null>("pick_library_pack"),
  exportPack: () => invoke<boolean>("export_library_pack"),
  exportSelected: (kind: LibraryKind, resourceIds: string[]) =>
    invoke<boolean>("export_library_selection", {
      selection: { kind, resourceIds },
    }),
  previewPack: (path: string) =>
    invoke<ResourcePackPreview>("preview_library_pack", { path }),
  restorePack: (
    path: string,
    plan: ResourcePackPreview,
    conflicts: "skip" | "copy",
  ) =>
    invoke<ResourcePackResult>("restore_library_pack", {
      path,
      expectedFingerprint: plan.fingerprint,
      expectedRevision: plan.revision,
      expectedEpoch: plan.epoch,
      conflicts,
    }),
  summary: () => invoke<Record<LibraryKind, number>>("get_library_summary"),
  list: (kind: LibraryKind) => invoke<LibraryData>(`get_library_${kind}`),
  saveInstruction: (
    set: InstructionSet,
    expectedRevision: number | undefined,
    expectedEpoch: number,
  ) =>
    invoke<InstructionSet>("save_library_instruction", {
      set,
      expectedRevision,
      expectedEpoch,
    }),
  importInstruction: (name: string, content: string) =>
    invoke<InstructionSet>("import_library_instruction", { name, content }),
  deleteInstruction: (
    resourceId: string,
    expectedRevision: number,
    expectedEpoch: number,
  ) =>
    invoke<boolean>("delete_library_instruction", {
      resourceId,
      expectedRevision,
      expectedEpoch,
    }),
  saveMcp: (
    resourceId: string,
    name: string,
    expectedRevision: number | undefined,
    edits: FieldEdit[],
    expectedEpoch: number,
  ) =>
    invoke("save_library_mcp", {
      resourceId,
      name,
      expectedRevision,
      edits,
      expectedEpoch,
    }),
  importMcp: () =>
    invoke<{
      sources: {
        toolId: string;
        imported: number | null;
        error?: string | null;
      }[];
    }>("import_library_mcp"),
  deleteMcp: (
    resourceId: string,
    expectedRevision: number,
    expectedEpoch: number,
  ) =>
    invoke("delete_library_mcp", {
      resourceId,
      expectedRevision,
      expectedEpoch,
    }),
  deleteSkill: (
    resourceId: string,
    expectedRevision: number,
    expectedEpoch: number,
  ) =>
    invoke("delete_library_skill", {
      resourceId,
      expectedRevision,
      expectedEpoch,
    }),
  preview: (
    kind: LibraryKind,
    resourceId: string,
    tools: string[],
    expectedRevision: number | undefined,
    expectedEpoch: number,
  ) =>
    invoke<LibraryOperation>(`preview_library_${kind}`, {
      resourceId,
      tools,
      expectedRevision,
      expectedEpoch,
    }),
  apply: (operationId: string) =>
    invoke<LibraryOperation>("apply_library_operation", { operationId }),
  recover: (operationId: string) =>
    invoke<LibraryOperation>("recover_library_operation", { operationId }),
  cancel: (operationId: string) =>
    invoke<LibraryOperation>("cancel_library_operation", { operationId }),
  skillDetail: (
    resourceId: string,
    relativePath?: string,
    expectedRevision?: number,
    expectedEpoch?: number,
    expectedHash?: string,
  ) =>
    invoke<SkillDetail>("get_library_skill_detail", {
      resourceId,
      relativePath,
      expectedRevision,
      expectedEpoch,
      expectedHash,
    }),
  importSkillDirectory: (source: string, directory: string, name: string) =>
    invoke<{ id: string }>("import_library_skill_directory", {
      source,
      directory,
      name,
    }),
  pickDirectory: () =>
    invoke<string | null>("pick_directory", { defaultPath: null }),
};
