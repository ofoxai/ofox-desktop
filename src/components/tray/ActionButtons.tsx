export default function ActionButtons() {
  return (
    <div className="flex gap-2 px-3.5 pb-3">
      <button className="flex-1 rounded-lg bg-orange-500 py-1.5 text-[12px] font-medium text-white hover:bg-orange-600">
        充值
      </button>
      <button className="flex-1 rounded-lg border border-border py-1.5 text-[12px] font-medium text-foreground hover:bg-accent">
        用量详情
      </button>
    </div>
  );
}
