# 用量修复验证记录（2026-10-18）

范围：从 `67cbfba5` 继续的共享 Codex 原生投影、历史有效视图、工具活动、覆盖说明及四个消费入口。参考 Magpie 固定提交 `bfaedf18301d02a423bd5422d22e41432d231ab8`。设计和保留限制见[本机统计重建契约](usage-native-rebuild.md)。最终应用摘要、提交 SHA、截图及平台验收状态记录在本轮交付清单；本文件不把合成测试称为真实资源消费或 Windows 验收。

## 冻结输入对账

运行独立 Go overlay 参考和 Rust 四消费者回归，使用完全相同的四个虚构 JSONL 来源、来源 SHA-256、UTC 窗口及价格。源参考 checkout 前后保持干净。严格比较结果为 `matched`，差异数组为空。

| 指标                      | Go 固定参考 |       Rust |
| ------------------------- | ----------: | ---------: |
| 消耗线程 / 去重调用       |       3 / 8 |      3 / 8 |
| 新输入 / 输出             |   573 / 114 |  573 / 114 |
| 缓存读取 / 写入           |    150 / 27 |   150 / 27 |
| 主 Tokens                 |         687 |        687 |
| nearest-rank 中位数 / p90 |   193 / 395 |  193 / 395 |
| 工具活动                  |   189900 ms |  189900 ms |
| 固定测试价估算            |  $0.0021765 | $0.0021765 |

每日和逐线程四桶／估价、跨午夜后事件归属、严格五分钟边界、时钟回退、DST 回拨以及子来源活动排除都参与比较。概览、有效明细／CSV、会话分析和列表使用同一冻结投影核对；不制造实测 HTTP、耗时或历史保存金额。参考字段别名支持及缺失 cache-write 策略是单独验证的差异，不加入共同格式基线。生产 Ofox 价格与 Magpie 目录的差异不通过修改账本配平。

复现：

```sh
python3 scripts/usage-parity/test_compare.py
python3 scripts/usage-parity/run_reference.py --magpie /path/to/pinned/magpie --output /tmp/new-reference-report
OFOX_USAGE_PARITY_OUTPUT=/tmp/new-ofox-report.json cargo test --manifest-path src-tauri/Cargo.toml --lib native_parity_frozen_four_sources_match_magpie_and_all_consumers -- --nocapture
python3 scripts/usage-parity/compare.py --reference /tmp/new-reference-report/reference.json --actual /tmp/new-ofox-report.json --output /tmp/new-comparison.json
```

输出路径须为新路径。参考运行器和比较器的详细边界见[说明](../scripts/usage-parity/README.md)。沙箱回归先核验四个冻结来源完整读入，再记录与生产协调器一致的双通道健康回执；不以此升级生产中的未知／失败来源。

## 回归与独立复审

先取得失败回归，再修复的关键问题包括：

- canonical 头部与共享计费身份混用；坏 ordinal 导致复制父头抢占；复制历史、分页、压缩调用与新旧事件重复。
- 非末响应的迟到计数修订、后续累计 checkpoint、跨 turn／epoch 的 COUNT 重放及已确认运行重置后的旧缺口隔离。
- 缺少 cache-read 时的新输入不可证明；缺少 cache-write 时保留已知部分，不宣称测量零。
- 初次无法重建保留原视图；来源／roster／route／epoch 变化和取消不发布半套来源；事务失败回滚。
- epoch 变化时 warm 来源重新发布；路由切走再返回后恢复可用性与定位。
- 同线程局部冲突仅排除相关事件，精确传播线程／模型覆盖；干净兄弟、真实零、0＋及未知价格分别保护。
- 退休旧导入写入路径；Codex 原始 ID、Tokens、五个保存金额字段不被补价或清理改变。
- 后台失败不抢焦点，原始审计保留 query／basis／apex，区域或快照不匹配不能接收首个异步回复。

解析器和共享投影由未参与对应实现的 reviewer 复核；发现的 P2 均补回归修复并复审。最终指定范围未发现剩余可复现 P1/P2。计价辅助函数的抽取逐字核对原函数体，OpenCode 原有计价行为未变。

## 门禁与性能范围

- 前端：160 文件、1375 项单测通过；typecheck、format 通过。
- Rust：20 个 suite，共 3273 项通过、13 项预设忽略；fmt 与 all-targets Clippy `-D warnings` 通过。平台或需外部环境的忽略不冒充已验收。
- 比较器：8 项正反例通过，包括未知／零、过期输入、重复身份和精度差异拒绝。
- 640 个物理 Codex 来源：两次暖刷新均 `parsed=0`，事实不变；该次暖刷新合计约 618ms。这是来源验证测试，不是单次查询 p95。
- release 10k 混合来源性能使用既有可选择基准，实际 p50／p95 及运行平台随交付日志记录：

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml --lib session_analytics_release_eight_native_bridges_query_p95 -- --ignored --nocapture
```

该基准包含缓存／价格／健康读取、去重聚合和排行快照保留，排除冷解析、真实历史完整性、IPC 和渲染。不能与单独的合成渲染时间相加后称为端到端分位数。

三页在 900px 下完成中英日、浅深色和长名称检查，无横向裁切。合成界面分别验证未知、真实零、部分价格、旧后端说明、原始审计返回、重建取消保留、缺失 HTTP／耗时、Enter／Escape 与明细焦点返回。18 次全新页面首载中，合成 mock 请求到首次模型标记画面的测量约 20–65ms；不包含后端或真实 IPC。截图明确带合成测试标识，实际安装窗口截图另存。

本轮没有发起付费模型或资源消费验收。已安装 Codex CLI 版本核实为 0.162.0；版本检查不能证明原生资源消费。Windows 主机、公证凭据及真实资源消费继续独立待验；本机静态 Windows 配置测试不代替原生 Windows 结果。
