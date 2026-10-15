import { invoke } from "@tauri-apps/api/core";
import type {
  UsageSummary,
  DailyStats,
  ProviderStats,
  ModelStats,
  RequestLog,
  LogFilters,
  ModelPricing,
  ProviderLimitStatus,
  PaginatedLogs,
  SessionSyncResult,
  DataSourceSummary,
  RequestQueryOptions,
} from "@/types/usage";
import type { UsageCsvExport, UsageSnapshotReceipt } from "@/types/usageQuery";
import {
  assertUsageReceipt,
  assertSnapshotReceipt,
  sameRequestFilters,
} from "@/lib/usageWorkbench";
import type { UsageResult } from "@/types";
import type { AppId } from "./types";
import type { TemplateType } from "@/config/constants";

export const usageApi = {
  // Provider usage script methods
  query: async (providerId: string, appId: AppId): Promise<UsageResult> => {
    return invoke("queryProviderUsage", { providerId, app: appId });
  },

  testScript: async (
    providerId: string,
    appId: AppId,
    scriptCode: string,
    timeout?: number,
    apiKey?: string,
    baseUrl?: string,
    accessToken?: string,
    userId?: string,
    templateType?: TemplateType,
  ): Promise<UsageResult> => {
    return invoke("testUsageScript", {
      providerId,
      app: appId,
      scriptCode,
      timeout,
      apiKey,
      baseUrl,
      accessToken,
      userId,
      templateType,
    });
  },

  // Proxy usage statistics methods
  /**
   * Aggregate usage stats over a time window.
   *
   * Scope = everything in cc-switch's `proxy_request_logs` table:
   * proxy real-time intercepts AND parsed offline session logs from
   * claude / codex / gemini. Tools cc-switch never saw (e.g. OpenCode
   * with proxy turned off, direct vendor traffic) physically aren't in
   * the table and naturally return 0 — no SQL filter needed.
   *
   * Numbers here are advisory; surface them with a "供参考" tooltip and
   * point users at the OFox dashboard for the authoritative billing view.
   */
  getUsageSummary: async (
    startDate?: number,
    endDate?: number,
    appType?: string,
  ): Promise<UsageSummary> => {
    return invoke("get_usage_summary", { startDate, endDate, appType });
  },

  getUsageTrends: async (
    startDate?: number,
    endDate?: number,
    appType?: string,
  ): Promise<DailyStats[]> => {
    return invoke("get_usage_trends", { startDate, endDate, appType });
  },

  getProviderStats: async (
    startDate?: number,
    endDate?: number,
    appType?: string,
  ): Promise<ProviderStats[]> => {
    return invoke("get_provider_stats", { startDate, endDate, appType });
  },

  getModelStats: async (
    startDate?: number,
    endDate?: number,
    appType?: string,
  ): Promise<ModelStats[]> => {
    return invoke("get_model_stats", { startDate, endDate, appType });
  },

  getRequestLogs: async (
    filters: LogFilters,
    page: number = 0,
    pageSize: number = 20,
    options?: RequestQueryOptions,
  ): Promise<PaginatedLogs> => {
    const { expectedReceipt, ...ipcOptions } = options ?? {};
    const response = await invoke<PaginatedLogs>("get_request_logs", {
      filters,
      page,
      pageSize,
      ...ipcOptions,
    });
    if (options) {
      assertUsageReceipt(response, options.query);
      if (!response.requestFilters) throw new Error("usage.queryUnsupported");
      if (!sameRequestFilters(response.requestFilters, options.requestFilters))
        throw new Error("usage.queryMismatch");
      if (expectedReceipt) assertSnapshotReceipt(response, expectedReceipt);
      if (!response.snapshotId) throw new Error("usage.queryUnsupported");
      if (options.snapshotId && response.snapshotId !== options.snapshotId)
        throw new Error("usage.queryMismatch");
    }
    return response;
  },

  getRequestDetail: async (
    requestId: string,
    snapshotId?: string,
    expectedReceipt?: UsageSnapshotReceipt,
  ): Promise<RequestLog | null> => {
    const response = await invoke<RequestLog | null>("get_request_detail", {
      requestId,
      ...(snapshotId ? { snapshotId } : {}),
    });
    if (snapshotId && response) {
      assertSnapshotReceipt(response.receipt, expectedReceipt);
      if (response.requestId !== requestId)
        throw new Error("usage.queryMismatch");
    }
    return response;
  },

  exportRequestLogsCsv: async (
    snapshotId: string,
    destination?: string,
    expectedReceipt?: UsageSnapshotReceipt,
  ): Promise<UsageCsvExport> => {
    const response = await invoke<UsageCsvExport>("export_request_logs_csv", {
      snapshotId,
      ...(destination ? { destination } : {}),
    });
    assertSnapshotReceipt(response, expectedReceipt);
    return response;
  },

  getModelPricing: async (): Promise<ModelPricing[]> => {
    return invoke("get_model_pricing");
  },

  updateModelPricing: async (
    modelId: string,
    displayName: string,
    inputCost: string,
    outputCost: string,
    cacheReadCost: string,
    cacheCreationCost: string,
  ): Promise<void> => {
    return invoke("update_model_pricing", {
      modelId,
      displayName,
      inputCost,
      outputCost,
      cacheReadCost,
      cacheCreationCost,
    });
  },

  deleteModelPricing: async (modelId: string): Promise<void> => {
    return invoke("delete_model_pricing", { modelId });
  },

  checkProviderLimits: async (
    providerId: string,
    appType: string,
  ): Promise<ProviderLimitStatus> => {
    return invoke("check_provider_limits", { providerId, appType });
  },

  // Session usage sync
  syncSessionUsage: async (): Promise<SessionSyncResult> => {
    return invoke("sync_session_usage");
  },

  getDataSourceBreakdown: async (): Promise<DataSourceSummary[]> => {
    return invoke("get_usage_data_sources");
  },
};
