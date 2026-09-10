# 下一阶段性能目标：以 Bun 为差距参照

更新：2026-09-10。下一阶段以 **quickjs-jit automatic 相对 Bun 默认配置的性能差距**选择优化点和定义阶段目标。旧版 JIT 是回归基线，QuickJS 解释器是判断原生执行是否有收益的基线；二者都不能代替 Bun 这一目标参照。

以下优先级最初以运行时 `07535b1` 制定。本轮调用/属性/数组实现与验证见
[边界优化记录](BUN_BOUNDARIES_20260909.md)。历史完整矩阵是 2026-09-06 的
[`47aeb11` 五模式、22 场景比较](../benchmarks/results/main-47aeb11-engines.md)，
早于 #24，且存在预热和计时边界差异。以下旧数据只用于选择优先调查的场景，
**不是当前版本的 Bun 成绩，也不是引擎峰值差距**。本轮本机 Bun 为 1.4.0。

当前 candidate4 的完整 24 场景比较已列入 [README](../README.md#experimental-jit-performance)。
三项重点场景相对旧 JIT 获益，但真正非直调和部分 fallback 场景仍慢于解释器；
最终 compute 宿主热重载也未通过 0.95x 预算，PR 保持草稿。下一步应优先隔离
这个宿主退步，再推进通用调用效率和数组循环检查消除，不把局部收益当作 M2 完成。

## 先统一协议，再给当前版本设数字目标

制定本目标时，旧 `benchmarks/run.rs` 和报告存在这些限制：Bun worker 默认
加 `--smol`、进程内只预热一次；QuickJS 使用 readiness/settling，计时内还
执行 Rust 侧 workload 查找、调用、checksum 转换与 polling，而 Bun 在
计时后生成 checksum。直接再跑一次原命令无法消除这些偏差。

本轮已加入共享 JS 批量 driver、固定等量预热和计时外 checksum，并移除
默认 `--smol`。新结果按 v2 协议单独记录；Tier1 和 Tier2 保留为诊断模式，
不用 forced Tier2 的最好成绩代表自动策略。固定预热矩阵不等于以下完整
长期协议目标全部完成：

- 分开新进程首次调用、固定等量预热、稳定热态。各引擎都记录连续批次趋势，
  同时给出固定预热结果和达到稳定条件后的结果；不能只让一个引擎充分预热。
- JS 内使用同一个批量循环、相同输入和结果消费，计时外校验；宿主调用成本
  单独测量。异步任务完成/排空边界一致，不把未完成 Promise 当作工作完成。
- 计时前后记录 native/Tier1/Tier2、compile/install、helper、deopt 增量；
  无可编译路径不能靠等待 2–3 秒 readiness 超时“稳定”。Bun 内部指标不可得
  时明确写 N/A，不能用 QuickJS 累计计数证明 Bun 的执行层级。
- 直接支持 Bun 默认 flags，记录版本、实际可执行文件和脚本哈希；历史
  `--smol` launcher 的处理见旧方法文件，不把 CLI 默认输出误当默认配置。
- 维持至少 5 个丢弃预热进程、30 个交错独立进程和 10 个吞吐窗口。吞吐
  测 JS 工作量/秒，进程创建吞吐另列。不能用一次短 smoke 的倍数设验收目标。

## 从 Bun 差距选择优化方向

表中速度均为 **automatic / Bun**，例如 0.10x 表示 Bun 速度的 10%。
延迟单位沿用历史协议的 ms/10 次 workload 批次；两列延迟用于识别绝对
耗时空间，不能解释为单次函数调用耗时。按差距、绝对耗时、宿主相关性、
实现成本共同排序，不只按最小速度比排序。

| 调查顺序 | 场景 | 历史 JIT / Bun 速度 | 历史 JIT / Bun 批次 ms | 要验证的优化点 |
| --- | --- | ---: | ---: | --- |
| P1：调用 | generic-call-entry | 0.002x | 7.985 / 0.019 | 通用 CALL、跨 C/Rust/native 边界、装箱、每次反馈/计时/metrics；保持真实 generic 路径 |
| P1：调用 | fibonacci-recursive；call-heavy | 0.023x；0.077x | 12.215 / 0.282；0.316 / 0.024 | 递归为何回退；direct call 与 generic call 的差距；选择性内联是否适用 |
| P1：属性与数组 | property-heavy；arrays-typed | 0.069x；0.063x | 5.153 / 0.357；7.011 / 0.442 | shape guard、ownership、helper；数组布局/边界检查；自动策略与真实 Tier2 的差距 |
| P2：对象与闭包覆盖 | objects-polymorphic；calls-closures | 0.090x；0.157x | 6.725 / 0.608；3.651 / 0.573 | 稳定多态属性、闭包变量和函数调用中有多少仍在解释器，何处适合新增 lowering |
| P2：库函数及分配归因 | json-codec；strings-regexp | 0.127x；0.095x | 78.931 / 10.050；19.943 / 1.887 | 绝对耗时空间很大；先拆 C 内建库、字符串分配/GC 和 JS 调度，不能直接归因于缺 opcode |
| P2：集合及异步 | map-set-bigint；exceptions-promises-async | 0.135x；0.263x | 16.384 / 2.210；2.848 / 0.750 | 库算法、对象分配与无原生执行反馈成本；保护 job/异常语义 |
| P3：数值补缺与控制 | quickjs-bitops；quickjs-fibonacci；scalar-loop | 0.086x；0.039x；0.508x | 0.209 / 0.018；0.372 / 0.014；0.026 / 0.013 | 先确认稳定层级和实际类型，区分 lowering 缺口与宿主边界；已快循环作为回归控制 |

调用、属性和数组既有明显 Bun 差距，也对应 gpui-shell 的排序/对象访问，
因此作为第一批 runtime 调查。JSON/RegExp 的绝对耗时空间更大，应尽早
完成热点归因；若新协议表明主要成本在 C 库而非 JIT，则单独提出库优化，
不能声称降低 JIT 回调成本就完成该目标。

历史 iterative Fibonacci 和 Float64 场景分别为 Bun 的 1.329x、1.246x
速度，但协议不对等，暂不视为峰值领先。统一协议后重新测量，保留为控制
场景；不以这两个点推出“整体追平 Bun”。

## 阶段目标如何验收

对每个选中场景记录三个速度比：

- **目标差距：** `S = Bun 延迟 / automatic 延迟`，即 automatic 相对 Bun 的速度。
- **自身进展：** 同协议 candidate / 当前 main 的配对速度。
- **原生收益：** automatic / 同版本解释器的配对速度。

用新的公平基线 `S0` 替换历史读数后，调用、属性、数组的首轮目标为：
相对当前 main 的配对速度 CI 下界 **≥1.50x**，同时在同批重测的 Bun
比较中把相对速度从 `S0` 提高到至少 `1.50 × S0`，报告该比值的联合配对
置信区间。两种比较都要支持收益，不能因为 Bun 在另一轮变慢而达标。
这只是缩小差距的首阶段，**不等于已接近 Bun**。后续按每个场景确认的
瓶颈与成本提出 0.10x、0.25x、0.50x、1.00x 等适合该场景的阶段里程碑，
不在新基线缺失时给全部场景套同一个绝对倍数承诺。

中间阶段也要明确报告 automatic 是否仍慢于解释器；原生效率方向长期
需达到解释器速度 CI 下界 ≥1.00x。数组另外检查 automatic 是否达到
有收益的强制层速度 CI 下界 ≥0.95x；无原生执行场景保护解释器速度下界
≥0.95x。回退可以改善生产性能，但不算完成原生路径效率或 Bun 覆盖目标。

所有比较保留绝对 ns/op 或 ms/批次、P99、启动/编译成本及内存。区间跨
1x 写“统计持平”及慢/快范围；不同输入规模分别报告，不用重复数值内核
组成的几何平均掩盖调用、对象和库场景的大差距。对照场景采用 candidate /
main 速度 CI 下界 ≥0.95x 的预算；涉及 GC、异常、ownership、invalidation
或 ABI 的修改继续执行对应语义及 sanitizer 验证。

## gpui-shell 回归是独立约束

[2026-09-09 混合负载追踪](MIXED_REGRESSION_20260909.md) 已独立复现当前
main 相对旧 `0.12.6` JIT 为 0.700x 速度，增加约 0.059 ms/次。新采样
把排序回调和短函数边界列为优先验证对象。这给调用/属性方向增加了真实
宿主证据，但**恢复旧版速度不是下一阶段优化的总目标**。

给 Bun 增加对应的纯 JS mixed 内核（quoteScore、对象数组、排序、结果
消费），与 gpui-shell 的完整 snapshot 测试分开。Bun 运行纯 JS 内核的
耗时不能与包含 GPUI snapshot 构造/校验的耗时直接相除。每个候选同时
报告 Bun 对齐内核成绩和真实 shell 的 mixed/panel/compute 稳态、尾延迟、
热重载，保留解释器控制；需要时用 2,000 次 render 检查长期稳定性。

既有 compute 5x、指定 kernel 10x、startup/热重载/P99 与 gpui-shell
门槛仍单独报告。冷路径首阶段相对同协议 main 速度下界 ≥1.25x 的目标
及最终速度下界 1/1.05 的约束也保留；不能用 Bun 差距缩小代替这些门槛。
本轮优先验证 `generic-call-entry`、`arrays-typed`、`property-heavy`，
并更新 README 的完整三引擎矩阵；实际成绩和未完成门槛以
[边界优化记录](BUN_BOUNDARIES_20260909.md) 为准。bitops 的 shift/XOR 已有
native lowering，不能把与 Bun 的差距直接归因于缺 `>>>`；wrapping
Fibonacci 与 bounded iterative Fibonacci 也不能仅凭倍数差认定为入口成本。
