import { invoke } from "@tauri-apps/api/core";
import type { TFunction } from "i18next";
import { toast } from "sonner";

export interface FetchedModel {
  id: string;
  ownedBy: string | null;
}

/**
 * 从供应商获取可用模型列表
 *
 * 使用 OpenAI 兼容的 GET /v1/models 端点。
 * 主要面向第三方聚合站（硅基流动、OpenRouter 等）。
 */
export async function fetchModelsForConfig(
  baseUrl: string,
  apiKey: string,
  isFullUrl?: boolean,
): Promise<FetchedModel[]> {
  return invoke("fetch_models_for_config", { baseUrl, apiKey, isFullUrl });
}

export type OfoxProtocol = "openai" | "anthropic" | "gemini";

/**
 * 从 Ofox 获取可用模型列表（公开接口，无需 API Key）
 *
 * 根据 protocol 选择对应的端点：
 * - "openai":    GET /v1/models
 * - "anthropic": GET /anthropic/v1/models
 * - "gemini":    GET /gemini/v1beta/models
 */
export async function fetchOfoxModels(
  protocol: OfoxProtocol,
): Promise<FetchedModel[]> {
  return invoke("fetch_ofox_models", { protocol });
}

/** openai 协议下需要排除的模型 vendor 前缀 */
const OPENAI_VENDOR_EXCLUDE = ["google", "anthropic"];

/**
 * 按 oFox 协议过滤掉不相关的 vendor 模型
 *
 * openai 协议排除 google/ 和 anthropic/ 前缀的模型；
 * anthropic / gemini 协议的 API 本身只返回对应模型，无需过滤。
 */
export function filterOfoxModelsByProtocol(
  models: FetchedModel[],
  protocol: OfoxProtocol,
): FetchedModel[] {
  if (protocol !== "openai") return models;
  return models.filter((m) => {
    const slashIdx = m.id.indexOf("/");
    if (slashIdx <= 0) return true;
    const vendor = m.id.slice(0, slashIdx).toLowerCase();
    return !OPENAI_VENDOR_EXCLUDE.includes(vendor);
  });
}

/**
 * 根据错误类型显示对应的 toast 提示
 */
export function showFetchModelsError(
  err: unknown,
  t: TFunction,
  opts?: { hasApiKey: boolean; hasBaseUrl: boolean },
): void {
  // 前端预检：缺少必填字段
  if (opts && !opts.hasBaseUrl && !opts.hasApiKey) {
    toast.error(t("providerForm.fetchModelsNeedConfig"));
    return;
  }
  if (opts && !opts.hasApiKey) {
    toast.error(t("providerForm.fetchModelsNeedApiKey"));
    return;
  }
  if (opts && !opts.hasBaseUrl) {
    toast.error(t("providerForm.fetchModelsNeedEndpoint"));
    return;
  }

  // 解析后端错误字符串
  const msg = String(err);

  if (msg.includes("HTTP 401") || msg.includes("HTTP 403")) {
    toast.error(t("providerForm.fetchModelsAuthFailed"));
    return;
  }
  if (msg.includes("HTTP 404") || msg.includes("HTTP 405")) {
    toast.error(t("providerForm.fetchModelsNotSupported"));
    return;
  }
  if (msg.includes("timeout") || msg.includes("timed out")) {
    toast.error(t("providerForm.fetchModelsTimeout"));
    return;
  }
  if (msg.includes("Failed to parse")) {
    toast.error(t("providerForm.fetchModelsNotSupported"));
    return;
  }

  // 通用兜底
  toast.error(t("providerForm.fetchModelsFailed"));
}
