# gpui-shell 混合负载退步与下一批 JIT 优化点

后续调用/属性/数组优化及相同依赖锁的宿主复测见
[2026-09-09–10 边界优化记录](BUN_BOUNDARIES_20260909.md)。本页保留
`07535b1` 对旧 `0.12.6` 的历史实验，不代表新候选版本的结果。

日期：2026-09-09。状态：**已复现，未修复；排序回调路径是首要调查对象，尚未完成单变量因果验证。**

本记录单独追踪 Linux x86_64 上 crates.io `0.12.6` 到当前 `main
07535b14bf290c65f052bff19c49b89b316c0914` 的混合负载退步。它与 PR #24
在 Apple M3 上相对 upstream 解释器的结果使用不同基线，不能互相替代。
本轮仅增加测量、分析与原始证据，没有修改运行时代码。
整体下一阶段以 [Bun 差距目标](PERFORMANCE_NEXT.md) 为准；下面的 mixed
调查用于定位宿主回归及验证相关调用/属性优化，不替代 Bun 对比目标。

## 独立复测

沿用原回测的两个 release 二进制，并重新核对 SHA-256。宿主为 Intel
Core i7-13700KF / Linux x86_64，绑 CPU 2，共享桌面，未独占核心。
每个配置先丢弃 5 个进程，再保留 30 个交错独立进程；每进程 64 次预热、
200 次计时 snapshot 构造及 debug-tree 校验。这不是 FPS 或绘制耗时。
速度使用配对延迟比的几何平均及 10,000 次配对 bootstrap 的 95% 区间。

| 混合负载比较 | 基线中位 ms/次 | 新版中位 ms/次 | 相对基线速度 | 95% 区间 |
| --- | ---: | ---: | ---: | ---: |
| 新 JIT / 旧 JIT，稳态 | 0.136251 | 0.195065 | 0.700x | 0.694–0.707x |
| 新 JIT / 旧 JIT，P99 | 0.153517 | 0.199984 | 0.752x | 0.725–0.771x |
| 新解释器 / 旧解释器，稳态 | 0.318734 | 0.265009 | 1.203x | 1.202–1.204x |
| 新 JIT / 新解释器，稳态 | 0.265009 | 0.195065 | 1.365x | 1.356–1.376x |
| 新 JIT / 旧 JIT，热重载 | 0.357190 | 0.305627 | 1.167x | 1.159–1.175x |

新版相对旧 JIT **速度降低约 30%**，中位延迟增加 **0.058814 ms/次**。
速度降低 30% 不等于延迟增加 30%；这里中位延迟增加约 43.2%。
原回测为 0.701x（0.698–0.704x）、增加约 0.0575 ms，独立复测方向一致。
新版仍比自身解释器快约 36.5%；问题不能描述为所有执行模式一并变慢。

120 个保留样本的 checksum、snapshot SHA-256 和 270 次 script render
总数全部一致；native entries/exits 配平，deopt 和 fallback 均为零。
原回测的 panel 稳态统计持平（新版速度范围为旧版的 0.989–1.001x，即慢
1.1% 到快 0.1%）；compute 稳态也统计持平（慢 4.8% 到快 22.5%）。
本轮只重新计时 mixed，未把原有控制场景当作本轮重跑结果。

## 原生覆盖变化比 deopt 更值得先查

| 每进程累计计数 | 旧 JIT | 新 JIT |
| --- | ---: | ---: |
| native entries | 24,237–24,574 | 164,781–175,449 |
| installed | 3 | 4 |
| invalid artifacts | 4 | 0 |
| compile failures | 13 | 9 |
| deopts / fallbacks | 0 / 0 | 0 / 0 |

这些计数在计时结束后、热重载前读取，**包括首次执行和预热，不是计时段增量**。
新版多装了一个产物、入口增加约七倍，同时消除了 invalid-artifact 拒绝。
这与“短函数获得原生覆盖，但收益不足以抵消边界成本”相符；不能从累计
计数断言该产物的函数身份或层级，更不能通过恢复旧版 artifact 拒绝来修复性能。

## 新采样把调查范围缩小到排序回调

用 gprofng 以 0.5 ms 周期分别采集两版各 30 个交错进程。以下是聚合
**全进程 CPU 时间采样占比**，包含启动、编译、预热、200 次计时、reload
及销毁；与上面的未插桩稳态计时分开。JIT 机器码可能显示为 Unknown，
也可能截断栈，因此不能据此算出某项优化的加速上限。

| 采样函数 | 旧版占比 | 新版占比 | 解释 |
| --- | ---: | ---: | --- |
| `rqsort.constprop.3`，inclusive | 9.04% | 32.90% | 排序及其回调调用链明显扩大 |
| `native_exit`，inclusive | 7.32% | 13.82% | 含返回反馈、计时与收益记账，不能与子项相加 |
| `publish_metrics`，self | 0.91% | 3.09% | 更多短调用放大每次发布成本 |
| `qjsjit_validate_helper_frame`，self | 未列出 | 3.17% | 属性原生路径的校验成本值得重新测量 |
| `JS_JitHelperShapeGuard`，self | 未列出 | 2.34% | 属性 guard 本身仍走 helper |

排序的 callers/callees 报告显示增量沿 `js_array_sort → rqsort →
js_array_cmp_generic / JS_Call` 展开。宿主脚本中 `quotes.sort(compareQuotes)`
的 comparator 只做 `right.score - left.score`，每次 render 排序 96 个对象；
计算函数 `quoteScore` 则每 render 调用 96 次、每次迭代 32 次。
入口增加、排序栈扩大和一个新增产物共同支持优先调查 comparator，
但还需要按 FunctionKey/name 和 tier 的计时段计数确认。
旧版函数采样表中未列出上述两个属性 helper；这不证明该函数从未执行。

旧版采样中的 `(u64, u64)` RandomState hash 和 DefaultHasher::write 分别
占 5.81% 和 5.54% self；新版热点已转移。不能沿用旧报告，把哈希查询
继续当作当前混合负载最主要的问题，也不能用不同原生覆盖的百分比证明
某个查询优化单独贡献了多少收益。

## 接下来的优化点与验证方式

1. **短排序回调的原生调用成本与自动策略。** 先在诊断构建中分函数记录
   warmup 前后、计时前后的 Tier1/Tier2 entries、安装和返回耗时，确认
   `compareQuotes` 是否是新增产物。再用同一 runtime/harness 分别只禁止
   comparator 的编译、只保留 comparator、只保留 quoteScore 做 A/B，
   定位增量来源。禁止某函数的诊断开关不是生产修复；最终需减少边界
   成本或通过真实解释器收益比较决定是否原生执行。现有
   `ProductionBackend::native_exit` 将优化执行与历史 **Tier1** 平均耗时
   比较来记录 benefit，并非与解释器比较，故“有 benefit”不证明比解释器快。

2. **每次原生返回的计时、收益和 metrics 成本。** `native_enter` 每次
   `Instant::now()`，`native_exit` 每次查 execution profile 并计时；收益
   更新最终仍查 code-cache artifact，更新 benefit 与 last_used。
   `maintenance_if_due(true)` 在非维护周期仍锁住并刷新完整外部 metrics
   字段。先分别测计时、收益、发布的调用次数和独占成本，再评估缩小
   发布字段、复用安全的 artifact 访问或有界计时采样。必须保留精确入口/
   出口统计、reentrant metrics 可见性、退役/失效语义以及缓存淘汰收益语义；
   不能简单删计数或不再更新 last_used。优先用 mixed、generic-call-entry
   验证收益，用 direct-call/scalar/recursion 和 panel 做控制。

3. **属性 guard 的固定成本。** 当前 `emit_property` 已有 rooted receiver
   借用路径，但仍写回参数、局部变量和活跃栈、更新 PC，并调用 ShapeGuard
   helper。先用只有两次属性读取和一次减法的 comparator 隔离这个固定
   成本；再判断是否能消除重复 spill/验证。必须证明完整 live-value
   provenance、GC rooting、shape identity/generation、prototype 依赖和
   精确 deopt；原有 OwnedSlot/Unknown 回退不能省略。不能因 comparator
   当前简单，就把 leaf guard 的假设扩大到可能分配或抛异常的所有 helper。

4. **无原生执行的附加成本，作为独立控制。** 原 panel 数据两版均为零
   native entries，新 JIT 相对自身解释器速度 0.991x（0.983–0.995x）。
   它是约 1% 级开销的调查线索，优先级低于 mixed；先重新测量，再拆分
   feedback、maintenance/polling 与宿主 snapshot 成本。历史 broad matrix
   的数组/async 数字早于 #24，不能直接当作当前优化空间。

每个候选都需独立 A/B、相同输入和计时边界、30 个交错进程及足量预热；
比较长期收益时补 2,000-render 批次和计时段计数，避免把编译完成时机误当
执行速度。性能修改完成后再跑相应语义测试、ownership/GC/deopt 测试和
涉及 C/ABI 的 sanitizer。这里没有 runtime 修改，不声称已通过新的运行时
测试，也不声称退步已解决或整体性能验收通过。

## 证据与重算

- [独立复测 summary](../benchmarks/results/linux-x64-mixed-20260909-summary.json)
- [环境、二进制身份及限制](../benchmarks/results/linux-x64-mixed-20260909-metadata.json)
- [旧版 profile](../benchmarks/results/linux-x64-mixed-20260909-baseline-profile.txt)、[新版 profile](../benchmarks/results/linux-x64-mixed-20260909-candidate-profile.txt)
- [旧版排序调用链](../benchmarks/results/linux-x64-mixed-20260909-baseline-sort-callers.txt)、[新版排序调用链](../benchmarks/results/linux-x64-mixed-20260909-candidate-sort-callers.txt)
- [原始证据归档](../benchmarks/results/linux-x64-mixed-20260909-evidence.tar.gz)、[SHA-256 清单](../benchmarks/results/linux-x64-mixed-20260909-sha256.json)

归档 `original/` 保留原 420 个样本、原汇总和脚本、Cargo lockfiles/diff、
构建及测试日志；`recheck/` 保留本轮 140 个样本（含 20 个丢弃预热样本）、
60 个 profiler 实验及输出、collect.py 和 summarize.py。
解包后运行 `python3 recheck/summarize.py` 会校验 120 个保留样本并重算
summary。二进制不放进 Git；`collect.py` 使用原临时目录的两份二进制，
启动前按原 metadata 校验哈希，重建环境见原 build metadata/config/lockfiles。

注意原 build 的 Cargo resolution 除五个 QuickJS 包外还变更了部分已有
平台/构建依赖和 tempfile/getrandom。它是这次二进制比较的明确限制，
所以最终归因需要同一依赖锁定的 runtime 单变量实验，不能称为已完成
源代码级 bisect。当前重新使用原二进制，未重新引入 Cargo resolution 变化。
