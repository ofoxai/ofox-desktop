import { invoke } from "@tauri-apps/api/core";
import type { AssistantAttachment } from "./assistant";

export interface FilePreviewRequest {
  sessionId: string;
  expectedOwner: string;
  expectedGeneration: number;
  attachmentId: string;
}
export interface FilePreviewManifest {
  attachmentId: string;
  name: string;
  version: string;
  kind: "pptx" | "html";
  totalPages: number;
  width: number;
  height: number;
  readyPages: number[];
  limits: { maxPagesPerRequest: 1; maxPage: number | null };
  warnings: string[];
}
export interface FilePreviewPage {
  attachmentId: string;
  version: string;
  page: number;
  mimeType: "image/jpeg";
  dataBase64: string;
  width: number;
  height: number;
  mathFallback?: boolean;
  mathUnverified?: boolean;
}
export interface FilePreviewHtml {
  attachmentId: string;
  version: string;
  url: string;
  missingResources: string[];
}
export interface FilePageReference {
  attachmentId: string;
  version: string;
  name: string;
  page: number | null;
}
export interface PresentationRevision {
  parentAttachmentId: string;
  parentVersion: string;
  pageNumbers: number[];
  revisionId: string;
}
export function assistantPresentationRevision(
  file: AssistantAttachment,
): PresentationRevision | null {
  if (
    file.origin !== "generated" ||
    file.mimeType !==
      "application/vnd.openxmlformats-officedocument.presentationml.presentation"
  )
    return null;
  const value = file.details?.presentationRevision as
    | Partial<PresentationRevision>
    | undefined;
  if (
    !value ||
    typeof value !== "object" ||
    typeof value.parentAttachmentId !== "string" ||
    !/^[A-Za-z0-9_-]{1,100}$/.test(value.parentAttachmentId) ||
    value.parentAttachmentId === file.id ||
    typeof value.parentVersion !== "string" ||
    !/^[a-f0-9]{64}$/.test(value.parentVersion) ||
    typeof value.revisionId !== "string" ||
    !/^[A-Za-z0-9_-]{1,100}$/.test(value.revisionId) ||
    !Array.isArray(value.pageNumbers) ||
    value.pageNumbers.length === 0 ||
    value.pageNumbers.length > 10000 ||
    value.pageNumbers.some(
      (page) => !Number.isInteger(page) || page < 1 || page > 10000,
    ) ||
    new Set(value.pageNumbers).size !== value.pageNumbers.length
  )
    return null;
  return {
    parentAttachmentId: value.parentAttachmentId,
    parentVersion: value.parentVersion,
    revisionId: value.revisionId,
    pageNumbers: [...value.pageNumbers],
  };
}

export const canPreviewAssistantFile = (file: AssistantAttachment) =>
  file.status !== "missing" &&
  [
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "text/html",
  ].includes(file.mimeType);

const fail = (): never => {
  throw new Error("assistant_preview_unavailable");
};
const integer = (value: unknown, min: number, max: number): value is number =>
  typeof value === "number" &&
  Number.isInteger(value) &&
  value >= min &&
  value <= max;
const versioned = (
  value: { attachmentId: string; version: string },
  request: FilePreviewRequest,
  version?: string,
) =>
  value.attachmentId === request.attachmentId &&
  typeof value.version === "string" &&
  /^[a-f0-9]{64}$/.test(value.version) &&
  (version === undefined || value.version === version);
const strings = (value: unknown): value is string[] =>
  Array.isArray(value) &&
  value.length <= 100 &&
  value.every((item) => typeof item === "string" && item.length <= 2000);

/** Only a dedicated, token-scoped loopback server may host active HTML. */
export function isAssistantHtmlPreviewUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      url.protocol === "http:" &&
      url.hostname === "127.0.0.1" &&
      url.port === "43763" &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash &&
      /^\/[a-zA-Z0-9_-]{24,128}\/index\.html$/.test(url.pathname)
    );
  } catch {
    return false;
  }
}

export const assistantPreviewApi = {
  async manifest(request: FilePreviewRequest): Promise<FilePreviewManifest> {
    const value = await invoke<FilePreviewManifest>("assistant_file_preview", {
      request,
    });
    if (
      !value ||
      !versioned(value, request) ||
      !["pptx", "html"].includes(value.kind) ||
      typeof value.name !== "string" ||
      value.name.length > 1000 ||
      !integer(value.totalPages, 0, 10000) ||
      !integer(value.width, 0, 16384) ||
      !integer(value.height, 0, 16384) ||
      (value.kind === "pptx" && (!value.width || !value.height)) ||
      !Array.isArray(value.readyPages) ||
      value.readyPages.length > value.totalPages ||
      value.readyPages.some((page) => !integer(page, 1, value.totalPages)) ||
      new Set(value.readyPages).size !== value.readyPages.length ||
      value.limits?.maxPagesPerRequest !== 1 ||
      (value.limits.maxPage !== null &&
        !integer(value.limits.maxPage, 1, 10000)) ||
      !strings(value.warnings)
    )
      fail();
    return value;
  },
  async page(
    request: FilePreviewRequest & {
      version: string;
      page: number;
      scale: 1 | 2;
    },
  ): Promise<FilePreviewPage> {
    if (!integer(request.page, 1, 10000) || ![1, 2].includes(request.scale))
      fail();
    const value = await invoke<FilePreviewPage>("assistant_file_preview_page", {
      request,
    });
    if (
      !value ||
      !versioned(value, request, request.version) ||
      value.page !== request.page ||
      value.mimeType !== "image/jpeg" ||
      (value.mathFallback !== undefined &&
        typeof value.mathFallback !== "boolean") ||
      (value.mathUnverified !== undefined &&
        typeof value.mathUnverified !== "boolean") ||
      !integer(value.width, 1, 8192) ||
      !integer(value.height, 1, 8192) ||
      typeof value.dataBase64 !== "string" ||
      value.dataBase64.length > 16 * 1024 * 1024 ||
      !/^\/9j\/[A-Za-z0-9+/]*={0,2}$/.test(value.dataBase64)
    )
      fail();
    return value;
  },
  async html(
    request: FilePreviewRequest & { version: string },
  ): Promise<FilePreviewHtml> {
    const value = await invoke<FilePreviewHtml>("assistant_file_preview_html", {
      request,
    });
    if (
      !value ||
      !versioned(value, request, request.version) ||
      typeof value.url !== "string" ||
      !isAssistantHtmlPreviewUrl(value.url) ||
      !strings(value.missingResources)
    )
      fail();
    return value;
  },
};
