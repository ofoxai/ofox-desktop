import { queryOptions } from "@tanstack/react-query";
import { sessionsApi } from "@/lib/api/sessions";

export const sessionListOptions = (apex: string) =>
  queryOptions({
    queryKey: ["sessions", apex] as const,
    queryFn: sessionsApi.list,
    retry: false,
    // Local IPC remains available when the WebView reports no network.
    networkMode: "always",
  });
