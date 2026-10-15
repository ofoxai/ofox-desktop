import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageFreshness } from "@/components/usage-overview/UsageFreshness";
it("separates source failures and price coverage from successful local and account times", () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  render(
    <UsageFreshness
      localUpdatedAt={null}
      accountUpdatedAt={1000}
      sources={[
        {
          sourceId: "claude_session",
          toolId: "claude",
          status: "failed",
          lastAttemptAtMs: 2000,
          lastSuccessAtMs: 500,
          errorCode: "private path must never render",
        },
      ]}
      pricing={{
        policy: "current-ofox-base-v1",
        apex: "ofox.io",
        catalogState: "invalid",
        catalogVersion: null,
        catalogUpdatedAtMs: null,
        stale: true,
        unpricedModelIds: ["unknown"],
        unpricedRecordCount: 2,
        additionalUnitsUnsupported: true,
      }}
    />,
  );
  expect(screen.getByText(/本机同步.*未记录/)).toBeVisible();
  expect(screen.getByText(/账户余额更新/)).toBeVisible();
  fireEvent.click(screen.getByText("来源同步状态"));
  expect(screen.getByText(/Claude Code.*失败/)).toBeVisible();
  expect(screen.getByText(/价格目录不可用/)).toBeVisible();
  expect(screen.queryByText(/private path/)).toBeNull();
});
