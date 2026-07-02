import { invoke } from "@tauri-apps/api/core";
import type { TFunction } from "i18next";
import { toast } from "sonner";

export interface FetchedModel {
  id: string;
  ownedBy: string | null;
  /**
   * 上游 `pricing.prompt`——per-token 输入价的字符串形式（如 "0.000001"）。
   * 免费模型为 "0"；上游缺字段 / Gemini 端口未返时为 null。
   *
   * 不在前端预先解析成 number：上游若未来加单位或换形态，原样透传更稳。
   * 取值/比较时再走 `Number(pricingPrompt)`——NaN/<=0 都按"未知或免费"处理。
   */
  pricingPrompt: string | null;
  /**
   * 上游 `supported_endpoints`——形如
   * `["/v1/chat/completions", "/v1/responses"]`。
   *
   * 用途：codex CLI 只能走 responses 协议，选到只支持 chat/completions 的
   * 模型会导致 "wire_api not supported" 报错。UI 层用这个字段过滤。
   *
   * 缺字段（旧 catalog / gemini 端点）时为 null——按"未知则不过滤"处理，
   * 避免安全降级把合法模型误砍。
   */
  supportedEndpoints: string[] | null;
}

/**
 * 从候选列表里挑「最便宜但不免费」的 model id。
 *
 * 算法：
 * - 解析 `pricingPrompt` 为 number；NaN 或 ≤0 视为"免费 / 未知"，剔除
 * - 剩余按价格升序，取第一个
 * - 全部被剔除时回落到原列表首个非空 id（兜底，避免空字符串触发 Codex CLI
 *   "Thread model is unavailable" 报错；调用方还会对 Codex 单独再兜一层）
 * - 列表为空返回 ""
 */
export function pickCheapestPaidModel(models: FetchedModel[]): string {
  const paid = models
    .map((m) => ({ id: m.id, price: Number(m.pricingPrompt ?? "") }))
    .filter((x) => Number.isFinite(x.price) && x.price > 0);
  if (paid.length > 0) {
    paid.sort((a, b) => a.price - b.price);
    return paid[0].id;
  }
  return models.find((m) => m.id)?.id ?? "";
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
 *
 * `requiredEndpoint` 是可选二次过滤：命中值形如 `"/v1/responses"`，只保留
 * `supportedEndpoints` 包含该端点的模型。Codex CLI 强绑 responses 协议
 * （见 `codex_config.rs` 里 `wire_api = "responses"` 硬编码），必须过掉
 * 只支持 chat/completions 的模型。`supportedEndpoints === null` 视为"未知"
 * ——不过滤（兼容老 catalog / 未接入该字段的端点）。
 */
export function filterOfoxModelsByProtocol(
  models: FetchedModel[],
  protocol: OfoxProtocol,
  requiredEndpoint?: string,
): FetchedModel[] {
  let result = models;
  if (protocol === "openai") {
    result = result.filter((m) => {
      const slashIdx = m.id.indexOf("/");
      if (slashIdx <= 0) return true;
      const vendor = m.id.slice(0, slashIdx).toLowerCase();
      return !OPENAI_VENDOR_EXCLUDE.includes(vendor);
    });
  }
  if (requiredEndpoint) {
    result = result.filter((m) => {
      // null / undefined → 未知，放行
      if (!m.supportedEndpoints) return true;
      return m.supportedEndpoints.includes(requiredEndpoint);
    });
  }
  return result;
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
