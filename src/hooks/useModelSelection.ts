import { useEffect } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { queryClient } from "@/lib/query/queryClient";
import {
  modelSelectionApi,
  type ModelSelection,
  type ToolModelSnapshot,
} from "@/lib/api/modelSelection";
import { ofoxGetApexState } from "@/lib/api/ofoxApex";
import { TOOL_PROTOCOL } from "@/lib/api/manageTool";
import {
  fetchOfoxModels,
  filterOfoxModelsByProtocol,
  filterOfoxModelsForOpenCode,
  filterOfoxModelsForWorkBuddy,
  type FetchedModel,
} from "@/lib/api/model-fetch";

export const MODEL_SELECTION_KEY = ["modelSelection"] as const;
export const MODEL_PROFILES_KEY = ["modelProfiles"] as const;

export function invalidateModelState() {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: MODEL_SELECTION_KEY }),
    queryClient.invalidateQueries({ queryKey: MODEL_PROFILES_KEY }),
  ]);
}

let subscribers = 0;
let subscription: Promise<Array<() => void>> | null = null;

/** One native subscription per webview, shared by tools, dialogs and profiles. */
function useModelStateEvents() {
  useEffect(() => {
    subscribers += 1;
    if (!subscription) {
      const onFocus = () => void invalidateModelState();
      window.addEventListener("focus", onFocus);
      subscription = Promise.all([
        ...[
          "model-selection-changed",
          "ofox-prefs-updated",
          "ofox-apex-changed",
          "ofox-auth-restored",
          "ofox-auth-expired",
        ].map((name) =>
          listen(name, () => {
            void invalidateModelState();
            if (name === "ofox-apex-changed") {
              void queryClient.invalidateQueries({
                queryKey: ["modelCatalog"],
              });
              void queryClient.invalidateQueries({ queryKey: ["modelApex"] });
            }
          }).catch(() => () => {}),
        ),
        Promise.resolve(() => window.removeEventListener("focus", onFocus)),
      ]);
    }
    const current = subscription;
    return () => {
      subscribers -= 1;
      if (subscribers === 0 && subscription === current) {
        subscription = null;
        void current.then((unlisten) => unlisten.forEach((off) => off()));
      }
    };
  }, []);
}

export function filterModelsForNativeTool(models: FetchedModel[]) {
  return models.filter(
    (model) =>
      (!model.supportedEndpoints ||
        model.supportedEndpoints.some((endpoint) =>
          [
            "/v1/chat/completions",
            "/v1/responses",
            "/anthropic/v1/messages",
          ].includes(endpoint),
        )) &&
      (!model.outputModalities || model.outputModalities.includes("text")) &&
      (!model.supportedParameters ||
        model.supportedParameters.includes("tools")),
  );
}

/** Share the public catalog request across every picker in one webview. */
export function useModelCatalog(tool: string, enabled = true) {
  const apex = useQuery(
    { queryKey: ["modelApex"], queryFn: ofoxGetApexState, staleTime: 0 },
    queryClient,
  );
  const protocol = TOOL_PROTOCOL[tool] ?? "openai";
  const catalog = useQuery(
    {
      queryKey: ["modelCatalog", protocol, apex.data?.apex],
      queryFn: () => fetchOfoxModels(protocol, true),
      enabled: enabled && !!apex.data,
      staleTime: 60_000,
      retry: false,
    },
    queryClient,
  );
  const all = catalog.data ?? [];
  const endpoint =
    tool === "codex" || tool === "chatgpt"
      ? "/v1/responses"
      : tool === "openclaw" || tool === "hermes"
        ? "/v1/chat/completions"
        : undefined;
  const models =
    tool === "workbuddy"
      ? filterOfoxModelsForWorkBuddy(all)
      : tool === "pi" || tool === "dsh"
        ? filterModelsForNativeTool(all)
        : tool === "opencode"
          ? filterOfoxModelsForOpenCode(all)
          : filterOfoxModelsByProtocol(all, protocol, endpoint);
  return {
    models,
    loading: apex.isPending || catalog.isFetching,
    error: apex.error ?? catalog.error,
    fetch: () => void catalog.refetch(),
  };
}

export function useModelSelection(tools: string[], enabled = true) {
  useModelStateEvents();
  const key = [...new Set(tools)].sort();
  return useQuery(
    {
      queryKey: [...MODEL_SELECTION_KEY, key],
      queryFn: () => modelSelectionApi.snapshot(key),
      enabled: enabled && key.length > 0,
      staleTime: 0,
      retry: false,
    },
    queryClient,
  );
}

export function useSaveModelSelection() {
  return useMutation(
    {
      mutationFn: (input: {
        tool: string;
        selection: ModelSelection;
        expectedRevision: string;
      }) =>
        modelSelectionApi.save(
          input.tool,
          input.selection,
          input.expectedRevision,
        ),
      onSuccess: (report) => {
        queryClient.setQueriesData<{ tools: ToolModelSnapshot[] }>(
          { queryKey: MODEL_SELECTION_KEY },
          (previous) =>
            previous
              ? {
                  tools: previous.tools.map((snapshot) =>
                    snapshot.toolId === report.snapshot.toolId
                      ? report.snapshot
                      : snapshot,
                  ),
                }
              : previous,
        );
      },
      onSettled: () => invalidateModelState(),
    },
    queryClient,
  );
}
