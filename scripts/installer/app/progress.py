"""
结构化进度输出：让主进程能还原安装进度

display.py 的输出是给人看的（带 ANSI 颜色、用 \r 原地覆写进度条）。那套格式主进程
解析不了：Rust 侧 install_tool.rs 的 spawn_stream_pump 用 BufReader::lines() 按 \n
切行，\r 覆写的中间态根本不会成为独立的一行，会一直憋到下一个 \n 才整块冒出来。

所以这里单独走一条机器可读的通路——每条进度打一行独立 JSON，前端按 type 字段认领。
和 display.py 并存而不是取代它：终端里的人类可读输出保持原样。

仅使用 Python 3.9 标准库。
"""
from __future__ import annotations

import json
from typing import Callable, Optional

#: 前端用它从混杂的日志行里认出进度事件。改这个值要同步改 useToolInstall.ts。
PROGRESS_TYPE = "ofox-install-progress"


def emit_progress(
    step: int,
    total: int,
    name: str,
    phase: str,
    elapsed: Optional[int] = None,
    timeout: Optional[int] = None,
) -> None:
    """
    打印一行进度 JSON 到 stdout。

    phase 取值：
      - "start"   开始执行某一步
      - "waiting" 轮询等待中（带 elapsed/timeout，用于估算等待条）
      - "done"    该步成功
      - "skipped" 该步跳过（已安装等）
      - "failed"  该步失败

    elapsed/timeout 只在 waiting 阶段有意义，其余阶段省略而不是填 0——省略让前端能
    区分"没有这个信息"和"真的是 0 秒"。
    """
    payload = {
        "type": PROGRESS_TYPE,
        "step": step,
        "total": total,
        "name": name,
        "phase": phase,
    }
    if elapsed is not None:
        payload["elapsed"] = elapsed
    if timeout is not None:
        payload["timeout"] = timeout

    # ensure_ascii=False 让中文步骤名保持原样，便于 dev 时直接读日志。
    # flush=True 是关键：安装器 stdout 走管道时默认全缓冲，不 flush 的话进度会积压
    # 到进程结束才一次性吐出来，前端就完全看不到"正在进行"。
    print(json.dumps(payload, ensure_ascii=False), flush=True)


def make_poll_callback(
    step: int,
    total: int,
    name: str,
    tick_fn: Optional[Callable[[int, int], None]] = None,
) -> Callable[[int, int], None]:
    """
    造一个给 `monitor.wait_for_condition(progress_callback=...)` 用的回调。

    wait_for_condition 只会把 (elapsed, timeout) 传给回调，不知道"这是第几步"。
    所以把步骤上下文闭包进来，让每次轮询都能发出一条完整的 waiting 进度。

    tick_fn 是原有的人类可读输出（display.print_poll_tick）。这里用注入而不是直接
    import display，是为了让 progress 模块不反向依赖展示层——两条输出通路各走各的。
    """

    def _callback(elapsed: int, timeout: int) -> None:
        if tick_fn is not None:
            tick_fn(elapsed, timeout)
            # 关键：tick 用 \r 原地覆写进度条且不换行，光标停在行尾。不补这个
            # 换行的话，下面的进度 JSON 会被追加到同一行 —— Rust 侧按 \n 切行，
            # 前端又要求行首是 '{'，这样的混合行会被整条丢弃，进度传不到 UI。
            #
            # 代价是终端里的进度条从"原地刷新"变成逐行滚动。在 ofox-desktop 里
            # 无所谓：init.py 的 stdout 被 Rust 管道接走，没有人直接看它，\r 的
            # 视觉效果本来就不存在。
            print(flush=True)
        emit_progress(
            step=step,
            total=total,
            name=name,
            phase="waiting",
            elapsed=elapsed,
            timeout=timeout,
        )

    return _callback
