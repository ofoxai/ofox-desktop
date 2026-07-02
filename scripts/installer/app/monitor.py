"""
轮询监控：等待条件满足或超时
仅使用 Python 3.9 标准库
"""
from __future__ import annotations

import time
from typing import Callable, Optional


def wait_for_condition(
    condition_fn: Callable[[], bool],
    timeout: int,
    interval: float = 3.0,
    progress_callback: Optional[Callable[[int, int], None]] = None,
) -> bool:
    """
    每 interval 秒调用 condition_fn()，满足则返回 True，超时返回 False。
    每次轮询回调 progress_callback(elapsed_seconds, timeout_seconds) 更新显示。
    """
    start = time.monotonic()

    while True:
        elapsed = int(time.monotonic() - start)

        if condition_fn():
            return True

        if elapsed >= timeout:
            return False

        if progress_callback is not None:
            progress_callback(elapsed, timeout)

        # 等待 interval 秒，但在最后一次等待时不超过 timeout
        remaining = timeout - elapsed
        sleep_time = min(interval, remaining)
        if sleep_time > 0:
            time.sleep(sleep_time)
