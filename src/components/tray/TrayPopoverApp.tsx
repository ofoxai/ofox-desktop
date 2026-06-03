import { useEffect } from "react";
import ProfileHeader from "./ProfileHeader";
import BalanceWarningBanner from "./BalanceWarningBanner";
import AccountBalance from "./AccountBalance";
import ActionButtons from "./ActionButtons";
import StatsRow from "./StatsRow";
import ToolStatusList from "./ToolStatusList";
import BottomMenu from "./BottomMenu";

export default function TrayPopoverApp() {
  useEffect(() => {
    // 设置透明背景
    document.documentElement.classList.add("tray-popover");
    return () => {
      document.documentElement.classList.remove("tray-popover");
    };
  }, []);

  return (
    <div className="h-screen w-full bg-transparent p-4">
      <div
        className="flex h-full flex-col overflow-hidden rounded-xl border border-border/50 shadow-[0_4px_12px_rgba(0,0,0,0.12)] backdrop-blur-xl"
        style={{ backgroundColor: "hsl(var(--popover) / 0.92)" }}
      >
        {/* 固定区域：不滚动 */}
        <div className="shrink-0">
          <ProfileHeader />
          <BalanceWarningBanner />
          <AccountBalance />
          <ActionButtons />
          <StatsRow />
        </div>
        {/* 工具状态：标题固定，列表可滚动 */}
        <ToolStatusList />
        {/* 固定区域：底部菜单 */}
        <div className="shrink-0">
          <BottomMenu />
        </div>
      </div>
    </div>
  );
}
