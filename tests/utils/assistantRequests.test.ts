import { describe, expect, it } from "vitest";
import { assistantRequests } from "@/utils/assistantRequests";
import type { AssistantSessionDetail } from "@/lib/api/assistant";

const detail = (
  extra: Partial<AssistantSessionDetail>,
): AssistantSessionDetail => ({
  session: {} as AssistantSessionDetail["session"],
  messages: [],
  attachments: [],
  usage: {},
  ...extra,
});

describe("request records", () => {
  it("keeps legacy empty histories readable", () => {
    expect(assistantRequests(null)).toEqual([]);
    expect(assistantRequests(detail({}))).toEqual([]);
  });
  it("uses live media receipts once, preserving refused attempts and frozen evidence", () => {
    const row = detail({
      usage: {
        requests: [
          {
            kind: "chat",
            modelId: "old/model",
            status: "completed",
            requestId: "req_chat",
          },
          { kind: "image", status: "submitting" },
        ],
      },
      messages: [
        {
          id: "media",
          role: "media",
          content: "",
          createdAt: 1,
          media: {
            kind: "image",
            model: "test/image",
            apex: "ofox.io",
            params: {},
            inputIds: [],
            outputs: [],
            status: "completed",
            cost: "0.02",
            requestAttempts: [
              {
                httpStatus: 401,
                requestId: "req_refused",
                requestIdSource: "x-request-id",
              },
              {
                httpStatus: 200,
                requestId: "req_good",
                requestIdSource: "x-ofox-request-id",
              },
            ],
            responseUsage: { output_tokens: 100 },
            priceSnapshot: { perImagePrice: "0.02" },
          },
        },
      ],
    });
    const requests = assistantRequests(row);
    expect(requests).toHaveLength(3);
    expect(requests[1]).toMatchObject({
      status: "rejected",
      requestId: "req_refused",
      usage: null,
      estimatedCostUsd: null,
    });
    expect(requests[2]).toMatchObject({
      kind: "image",
      status: "completed",
      requestId: "req_good",
      apex: "ofox.io",
      usage: { output_tokens: 100 },
      priceSnapshot: { perImagePrice: "0.02" },
      estimatedCostUsd: "0.02",
      billedCostUsd: null,
    });
  });
  it("keeps binary speech actual cost unknown and distinguishes a video reported amount", () => {
    const row = detail({
      audioOperations: [
        {
          kind: "speech",
          model: "test/speech",
          status: "completed",
          cost: null,
          operationId: "op",
          runId: "run",
          params: {},
          audio: {
            pricing: { unit: "characters", inputPerMillionCharacters: "10" },
            estimate: "0.0001",
            input: null,
            text: "sample",
            usage: null,
            requestId: null,
            requestIdSource: null,
            errorCode: null,
          },
        } as unknown as NonNullable<
          AssistantSessionDetail["audioOperations"]
        >[number],
      ],
      messages: [
        {
          id: "video",
          role: "media",
          content: "",
          createdAt: 1,
          media: {
            kind: "video",
            model: "test/video",
            params: { duration: 4, resolution: "480p" },
            inputIds: [],
            outputs: [],
            status: "download_failed",
            cost: "0.08",
            jobId: "job",
            requestAttempts: [
              {
                httpStatus: 202,
                requestId: "req_video",
                requestIdSource: "x-request-id",
              },
            ],
          },
        },
      ],
    });
    const requests = assistantRequests(row);
    expect(requests[0]).toMatchObject({
      kind: "video",
      reportedCostUsd: "0.08",
      estimatedCostUsd: null,
      billedCostUsd: null,
      jobId: "job",
    });
    expect(requests[1]).toMatchObject({
      kind: "speech",
      estimatedCostUsd: "0.0001",
      billedCostUsd: null,
      usage: null,
      requestId: null,
    });
  });
});
