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
  disabledReason?: string | null;
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
  targets?: ResourceTarget[];
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
export const libraryApi = {
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
  pickPack: () => invoke<string | null>("pick_library_pack"),
  exportPack: () => invoke<boolean>("export_library_pack"),
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
  skillDetail: (resourceId: string, relativePath?: string) =>
    invoke<SkillDetail>("get_library_skill_detail", {
      resourceId,
      relativePath,
    }),
  importSkillDirectory: (source: string, directory: string, name: string) =>
    invoke("import_library_skill_directory", { source, directory, name }),
  pickDirectory: () =>
    invoke<string | null>("pick_directory", { defaultPath: null }),
};
