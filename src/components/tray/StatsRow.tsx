export default function StatsRow() {
  const stats = [
    { label: "今日请求", value: "1,284" },
    { label: "TOKENS", value: "2.3M" },
    { label: "连接工具", value: "3/5" },
  ];

  return (
    <div className="flex border-t border-border">
      {stats.map((stat, i) => (
        <div
          key={stat.label}
          className={`flex flex-1 flex-col items-center py-2 ${
            i < stats.length - 1 ? "border-r border-border" : ""
          }`}
        >
          <div className="text-[10px] text-muted-foreground">{stat.label}</div>
          <div className="text-[15px] font-bold text-foreground">
            {stat.value}
          </div>
        </div>
      ))}
    </div>
  );
}
