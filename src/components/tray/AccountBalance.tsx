export default function AccountBalance() {
  return (
    <div className="px-3.5 pt-3 pb-2">
      <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        OFOX 账户余额
      </div>
      <div className="mt-0.5 text-2xl font-bold text-foreground">$8.40</div>
      <div className="mt-0.5 text-[11px] text-muted-foreground">
        今日 <span className="text-red-500">-$3.82</span> &middot; 本月{" "}
        <span className="text-muted-foreground">-$47.20</span>
      </div>
    </div>
  );
}
