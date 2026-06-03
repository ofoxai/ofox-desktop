export default function BottomMenu() {
  const items = [
    { label: "打开 Ofox 主窗口", shortcut: "\u2318O" },
    { label: "重新扫描工具", shortcut: undefined },
    { label: "偏好设置...", shortcut: undefined },
  ];

  return (
    <div className="border-t border-border">
      {items.map((item) => (
        <button
          key={item.label}
          className="flex w-full items-center justify-between px-3.5 py-2 text-[12px] text-foreground hover:bg-accent/50"
        >
          <span>{item.label}</span>
          {item.shortcut && (
            <span className="text-[11px] text-muted-foreground">
              {item.shortcut}
            </span>
          )}
        </button>
      ))}
      <button className="flex w-full items-center px-3.5 py-2 text-[12px] text-muted-foreground hover:bg-accent/50">
        退出 Ofox
      </button>
    </div>
  );
}
