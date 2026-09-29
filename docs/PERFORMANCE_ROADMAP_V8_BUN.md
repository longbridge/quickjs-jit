# 对齐 V8 / Bun 的 JIT 性能路线图（2026-09-27 分析）

本文件接续 [M3](M3.md)、[下一阶段目标](PERFORMANCE_NEXT.md)、
[边界优化记录](BUN_BOUNDARIES_20260909.md) 和
[semantic SSA 进展](SEMANTIC_SSA_PROGRESS.md)，基于 `82d3808`（0.12.9）
做一次**差距归因**，并给出分阶段、可验收的规划。

本文件**不包含任何运行时优化**，README 矩阵保持不变。文中所有实验
数值都是单机诊断（i7-13700KF，CPU 8，`powersave`，Bun 1.4.0，3–7 个
进程取中位数），不是 AGENTS.md 要求的发布级配对证据，不能当作优化
收益引用；每个阶段落地时仍需完整三引擎矩阵。

## 1. 结论摘要

1. **测量协议有系统偏差，当前矩阵高估了 quickjs-jit 相对 Bun 的位置。**
   Bun worker 用 `bun -e` 执行，第 64 个批次（恰好是协议唯一计时的
   那一批）会稳定出现一次 JSC 侧的一次性停顿（约 0.1–0.2 ms）。
   所有无 `workloadArgument` 的双参数场景的 Bun 延迟都被放大
   1.2x–32x。README 中 7 个"快于 Bun"的场景，按稳态批次口径**全部
   慢于 Bun**（见第 2 节）。先修协议，否则后续所有"对齐 Bun"的
   目标都建立在错误基线上。
2. **按稳态口径，quickjs-jit 在全部 30 个场景都慢于 Bun。** 最好的
   标量循环约为 Bun 的 0.16x–0.5x；调用、属性、数组约 0.1x；闭包、
   递归、集合、异常、多态对象约 0.01x–0.05x；最差的
   `generic-call-fallback` 为 0.0009x，而且只有解释器的 0.14x。
3. **最大的单点损耗是 JIT 自身的防御性开销，不是缺少优化。**
   - `qjsjit_validate_helper_frame_state`：每次 helper 调用都做
     ~30 项身份/ABI 校验，外加 `qjsjit_valid_pc` 的**线性字节码扫描**
     （回边之后从字节码开头重扫）。它在 `objects-polymorphic` 中占
     63% 自身时间，在真实的 `generic-call-fallback` 中占 61%。
     把 PC 校验变成 O(1) 的诊断实验分别带来 **1.71x** 和 **1.29x**，
     其余场景持平，checksum 一致。
   - 每次调用的簿记：`record_hot`/`acquire_entry`/`native_enter`/
     `native_exit` 四次跨 C/Rust 回调、两次 `Instant::now()`、哈希表
     查找、`record_benefit`，以及每 32 次调用一次的全量 `maintenance`。
     `mixed-quotes` 的排序回调中 `maintenance` 占 37.5%。
4. **覆盖面比 V8 窄一个数量级。** 约 255 个 opcode 中有 124 个被
   policy 拒绝，包括全部闭包变量访问、`push_this`、`throw`、`typeof`、
   `in`/`instanceof`、`for_of`/`for_in`、`fclosure`、try/catch、
   `arguments`。含其中任一 opcode 的函数整体停在解释器。V8 Sparkplug
   和 JSC Baseline 覆盖全部字节码。
5. **没有通用的原生到原生调用约定。** 除了受限的 Int32/Bool 叶函数
   直调和有界内联，所有调用都经过
   `JS_JitHelperCall → JS_Call → JS_CallInternal → 回调 → 原生入口`。
   递归因此被 policy 禁止进入原生层；强制进入 Tier 2 后
   `fibonacci-recursive` 反而比解释器慢 7.8x。
6. 在这些之后，才轮到 V8 TurboFan/Maglev、JSC DFG/FTL 级别的优化
   编译能力：冗余 guard 与检查消除、LICM、load/store 转发、边界检查
   消除、多态内联缓存、逃逸分析和分配下沉、通用内联。
   `property-heavy`、`call-heavy`、`int32array-traversal` 已经在原生层，
   但每次迭代仍比 Bun 慢约 10x，差距就在这里。

## 2. 测量协议缺陷与修正

`benchmarks/run.rs` 的 `bun_child` 用 `bun -e <wrapper> <script>` 执行：
先预热 64 个批次，再单独计时第 65 批。逐批打印 Bun 的延迟后发现
（numeric，单位 µs）：

```text
bun -e:   ... 61:6 62:6 63:8 64:187 65:16 66:15 67:16 68:530 69:11 ...
bun file: ... 61:4 62:5 63:5 64:5  65:4  66:6  67:5  68:5  69:5  ...
```

在 `-e` 模式下，第 64 个批次（从 0 开始计数，正好是计时批）稳定出现
一次约 0.1–0.2 ms 的一次性停顿，之后恢复。把最终计时移到同一个循环
体内也照样出现，说明它不是调用点形状的问题，而是在固定调用次数上
发生的一次性事件（JSC 分层编译或 GC）。以同一 `.mjs` 文件方式运行时
不出现。受影响的是没有 `workloadArgument` 的双参数场景：固定批次是
稳态的 1.2x–32x；三参数场景不受影响（1.0x）。

后果：README 中 7 个"快于 Bun"的场景（`scalar-control-flow` 1.67x、
`scalar-expressions` 1.83x、`host-compute` 3.52x、`quickjs-bitops`
2.46x、`numeric` 4.46x、`scalar-loop` 4.74x、`fibonacci-iterative`
1.07x），以及 `float64-dense` 的"统计持平"，按稳态口径分别只有
0.15x–0.51x。PERFORMANCE_NEXT.md 中"iterative Fibonacci 和 Float64
场景为 Bun 的 1.3x/1.2x"的历史读数同样不能再引用。

协议修正（P0，在任何新的 Bun 对齐目标之前完成）：

1. 每个进程计时 K 个连续批次（建议 K = 16），报告中位数，并保留
   全部批次原始值；固定批次读数作为冷态/抖动诊断单独列出。三个引擎
   使用同一规则。
2. Bun 改为执行落盘的 `.mjs` wrapper（记录其哈希），不用 `-e`；
   `-e` 结果只能作为对照。
3. 检测批次序列中的离群尖峰（例如 > 5 倍中位数），报告出现次数，
   不能静默丢弃。
4. 重跑完整 30 场景矩阵，替换 README 中受影响的数值，并把旧矩阵标注
   为"受 Bun 固定批次尖峰影响的历史数据"。

## 3. 稳态口径下的三引擎差距（诊断）

单位：ms / 10 次 workload 调用。"稳态"为每个进程最后 16 个预热批次
（第 48–63 批）的中位数，再取 3 个进程的中位数；"固定批次"为协议唯一
计时的第 65 批。三个引擎运行同一 worker 和同一 driver，固定在 CPU 8
上，按场景顺序依次运行。这是单机诊断，没有置信区间；发布级结论需要
按修正后的协议重跑完整配对矩阵。按 JIT / Bun 速度从低到高排序：

| 场景 | QuickJS ms | quickjs-jit ms | Bun ms | JIT / QuickJS 速度 | JIT / Bun 速度（稳态） | JIT / Bun 速度（固定批次） | Bun 固定批次放大倍数 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| generic-call-fallback | 1.2809 | 9.1241 | 0.0081 | 0.14x | 0.0009x | 0.0009x | 1.0x |
| objects-polymorphic | 6.1895 | 23.8293 | 0.1064 | 0.26x | 0.0045x | 0.0083x | 1.9x |
| float64array-traversal | 1.5369 | 1.1065 | 0.0083 | 1.39x | 0.0075x | 0.0073x | 1.0x |
| quickjs-fibonacci | 0.9467 | 0.3657 | 0.0044 | 2.59x | 0.0122x | 0.4080x | 32.3x |
| arrays-typed | 4.1375 | 2.9720 | 0.0447 | 1.39x | 0.0150x | 0.0467x | 3.1x |
| calls-recursion-closures | 6.4055 | 6.4065 | 0.1252 | 1.00x | 0.0195x | 0.0321x | 1.6x |
| mixed-quotes | 2.7642 | 2.4809 | 0.0597 | 1.11x | 0.0241x | 0.0586x | 2.4x |
| calls-closures | 3.3356 | 3.3101 | 0.0824 | 1.01x | 0.0249x | 0.0555x | 2.2x |
| collections | 1.6327 | 1.6444 | 0.0457 | 0.99x | 0.0278x | 0.0825x | 3.0x |
| fibonacci-recursive | 9.7966 | 9.8254 | 0.2741 | 1.00x | 0.0279x | 0.0549x | 2.0x |
| strings-json | 1.9159 | 2.0004 | 0.0697 | 0.96x | 0.0348x | 0.0827x | 2.4x |
| quickjs-int-arith | 5.7955 | 1.3431 | 0.0476 | 4.32x | 0.0355x | 0.0991x | 2.8x |
| adversarial | 0.9336 | 0.9525 | 0.0356 | 0.98x | 0.0374x | 0.1295x | 3.5x |
| map-set-bigint | 16.4096 | 17.5385 | 0.7952 | 0.94x | 0.0453x | 0.0544x | 1.2x |
| strings-regexp | 19.2642 | 19.8956 | 0.9736 | 0.97x | 0.0489x | 0.0650x | 1.3x |
| exceptions-promises-async | 2.3311 | 3.6857 | 0.1986 | 0.63x | 0.0539x | 0.0769x | 1.4x |
| property-heavy | 1.0664 | 0.0985 | 0.0079 | 10.83x | 0.0797x | 0.0798x | 1.0x |
| json-codec | 77.0292 | 78.9868 | 7.5310 | 0.98x | 0.0953x | 0.0974x | 1.0x |
| packed-array-traversal | 0.8262 | 0.0481 | 0.0049 | 17.18x | 0.1025x | 0.1040x | 1.0x |
| call-heavy | 1.2789 | 0.0594 | 0.0064 | 21.52x | 0.1072x | 0.1062x | 1.0x |
| int32array-traversal | 1.0350 | 0.0496 | 0.0057 | 20.86x | 0.1155x | 0.1163x | 1.0x |
| scalar-loop | 0.5375 | 0.0292 | 0.0045 | 18.41x | 0.1534x | 4.5244x | 29.6x |
| numeric | 0.5384 | 0.0293 | 0.0045 | 18.39x | 0.1540x | 4.4316x | 28.9x |
| generic-call-entry | 0.9613 | 0.0322 | 0.0050 | 29.86x | 0.1544x | 0.1524x | 1.0x |
| fibonacci-iterative | 35.5027 | 0.9943 | 0.1592 | 35.71x | 0.1601x | 0.9383x | 5.9x |
| scalar-expressions | 2.1598 | 0.0503 | 0.0093 | 42.93x | 0.1840x | 1.7863x | 9.7x |
| scalar-control-flow | 2.1338 | 0.0544 | 0.0103 | 39.21x | 0.1896x | 1.6627x | 8.8x |
| quickjs-bitops | 1.1930 | 0.0615 | 0.0140 | 19.39x | 0.2281x | 2.3919x | 10.5x |
| host-compute | 2.1641 | 0.0715 | 0.0169 | 30.27x | 0.2360x | 3.4974x | 14.2x |
| float64-dense | 2.2796 | 0.1292 | 0.0657 | 17.65x | 0.5087x | 1.1483x | 2.2x |

读法：

- quickjs-jit 和 QuickJS 的固定批次与稳态一致（比值在 0.955–1.014
  之间），偏差只出现在 Bun 一侧。
- 按稳态口径，**没有任何场景达到 Bun 速度**。最接近的是 `float64-dense`
  （0.51x），其次是一组标量内核（0.15x–0.24x）。
- 有 11 个场景 quickjs-jit 不快于解释器（速度 ≤ 1.01x）。其中
  `generic-call-fallback`（0.14x）、`objects-polymorphic`（0.26x）和
  `exceptions-promises-async`（0.63x）是净退步，其余属于没能进入原生
  代码、或原生收益被 helper 开销抵消。

原始数据：[`roadmap-steady-diagnostic-82d3808-x86_64.json`](../benchmarks/results/roadmap-steady-diagnostic-82d3808-x86_64.json)
（每个场景、每个引擎为 `[固定批次中位数, 稳态中位数]`，单位 ms）。

## 4. 瓶颈归因

采样方法：`perf_event_paranoid=2` 禁止 perf/samply，改用 gdb 信号采样
（每 20 ms 向 worker 发 `SIGUSR2`，gdb 停下取 `bt`）。JIT 帧无 unwind
信息，回溯穿过原生代码后的帧（如 `hash_one`）不可信，只使用自身帧
（SELF）和 JIT 帧以上的 C/Rust 调用链。样本数 60–480，属于定性诊断。

### B1 helper 帧校验：线性 PC 扫描（最高优先级）

每个 C helper（`Dup`/`Free`/`GetProperty`/`Call`/`InlineEnter`...）入口都会
调用 `qjsjit_validate_helper_frame_state`，逐项检查 runtime/API/ABI 结构
大小、cookie、generation、stack map、`arg_buf`/`var_buf`/栈边界，最后
调用 `qjsjit_valid_pc`。后者从 `jit_validated_pc_offset` 开始**按指令长度
线性扫描**字节码，直到命中 `frame->pc`；只能复用"不晚于当前 pc"的
缓存，因此循环回边之后的第一次 helper 调用会从字节码开头重扫。
每次 helper 调用的代价因此是 O(字节码长度)，而不是 O(1)。
`JS_JitInlineEnter` 在一次调用中还要额外校验 `call_pc` 和
`continuation_pc`，再加上一次 `calloc` 分配 inline 状态。

| 场景 | 自身时间占比 | 调用来源 |
| --- | ---: | --- |
| objects-polymorphic | 63.2% | `JS_JitHelperDup`（38%）、`SetProperty`、`NewObject` |
| generic-call-fallback（正式脚本） | 61.4% | `InlineEnter`/`InlineCheck`/`MaterializeOwner`/`Dup`/`Free` |
| float64array-traversal | 30–42% | `JS_JitHelperGetProperty`（`ints.length`） |
| property-heavy（M3 旧 profile） | 55.8% | 属性 helper |
| arrays-typed | 10–12% | 属性/调用 helper |

诊断实验 A：在 `qjsjit_valid_pc` 开头直接返回 true，即只保留边界检查
之外的其余校验。这个改法**不安全，只用于估算上限**。同一个二进制
用环境变量切换，7 次交错运行取中位数：

| 场景 | 基线 ms | 实验 A ms | 速度 |
| --- | ---: | ---: | ---: |
| objects-polymorphic | 24.482 | 14.345 | 1.71x |
| generic-call-fallback | 9.720 | 7.549 | 1.29x |
| property-heavy | 0.098 | 0.098 | 1.00x |
| arrays-typed | 3.071 | 3.058 | 1.00x |

正式方案应保持 fail-closed：编译期就知道每个 stack map 对应的 PC，
helper 只需比较 `frame->pc == stack_map[id].pc`；也可以在 artifact
安装时一次性生成指令边界位图，然后 O(1) 查询。剩下的 ~30 项身份
校验建议每次原生入口只做一次，在 helper 中改为 debug/sanitizer 断言，
或者合并成对单个 epoch/cookie 的比较。

### B2 每次调用的 JIT 簿记（调用与回调密集型场景）

解释器每次进入一个已登记函数时，C 侧依次执行：
`JS_SetUncatchableError` → `record_hot`（vtable 回调到 Rust：哈希集合
查询、`maintenance_if_due`、`tier_state`、feedback 观测）→
`acquire_entry` 回调 → `native_enter`（`FxHashMap` 查询、`Instant::now()`
入栈）→ 原生代码 → `native_exit`（`Instant::now()`、
`execution_profiles` 更新、`record_benefit`、再次 `maintenance_if_due`）。
`maintenance_if_due` 每 64 个 tick 做一次全量 `maintenance`。每次调用
有 2 个 tick，所以实际约每 32 次调用一次（gdb 断点计数：20,000 次调用
触发 636 次）。

- `mixed-quotes`：`Array.prototype.sort` 的比较器每次从 C 回调进入原生
  代码，`maintenance` 占 37.5%，`native_exit`/`clock_gettime`/`tier_state`/
  `publish_metrics`/SipHash 合计约 25%。这正是 gpui-shell 排序回调退步
  （[混合负载追踪](MIXED_REGRESSION_20260909.md)）的形态。
- 当 `maintenance` 有待评估的 Tier2 候选时（长循环变体中观测到），
  它会每次重新做 `FeedbackTable::snapshot`、`CompileSnapshot::verify`
  和 `BaselineIr::translate`，单次约 5–6 µs，可占总时间的 40%。在正式
  `generic-call-fallback` 脚本中候选列表为空，这条路径没有被触发
  （实验 B 缓存扫描结果，速度 1.00x）。它是潜在的调度风险，但不是
  该场景当前的主因。

V8 的对照做法是：字节码里对 feedback vector 的 interrupt budget 做
递减，预算耗尽时才调用一次运行时，平时每次调用只需几条指令，不做
计时，也不跨语言回调；分层决策依赖计数而不是逐次计时。JSC 类似，
使用 execution counter 和 `ExecutableBase` 中缓存的入口指针。

### B3 覆盖缺口：函数整体留在解释器

`policy_audit.rs` 中 124/255 个 opcode 的 policy 是 Reject。按原因归类：
闭包帧（`get_var_ref*`/`put_var_ref*`/`set_var_ref*`/`make_var_ref*`，共
20 个）、`UnsupportedOpcode`（`fclosure`、`push_this` 所在的 ExtendedFrame、
`throw`、`typeof*`、`in`、`instanceof`、`for_in_*`、`for_of_*`、
`iterator_*`、`apply`、`special_object`、`define_class`、`put_var`、`pow`、
`delete`...）、异常区（`catch`/`gosub`/`ret`/`nip_catch`）、async/generator、
`with`、`eval`。

受影响场景的稳态结果（解释器速度 ≈ JIT 速度）：`calls-closures`
1.01x、`calls-recursion-closures` 1.00x、`collections` 0.99x（`for_of`）、
`fibonacci-recursive` 1.00x（递归被 policy 挡住）、`adversarial` 0.98x、
`exceptions-promises-async` 0.63x（`catch`，另有回调开销）、
`map-set-bigint` 0.94x。采样显示 `calls-recursion-closures` 84% 的自身
时间在 `JS_CallInternalResume`（解释器）。

### B4 调用约定：没有通用的原生到原生调用

目前只有两条快速路径：受限的 Int32/Bool 无副作用叶函数直调，以及
有界的 effect-free/frame 内联。其余调用都走 generic bridge，每次调用
都带 B1 和 B2 的全部开销。强制 Tier 2 的 `fibonacci-recursive` 需要
76.9 ms，解释器只要 9.86 ms（慢 7.8x）。所以 `maintenance` 中
`generic_call_without_loop` 规则干脆把只调用、不含循环的函数留在
解释器里。V8/JSC 的 JS 到 JS 调用是原生栈帧加直接跳转：Sparkplug 和
Baseline JIT 调用 IC，优化层直接调用或内联，deopt 时才惰性物化帧。

### B5 typed array `length` 与非参数接收者

`float64array-traversal` 的循环条件 `i < ints.length` 中，`ints` 来自
`buffers.ints`（对象属性），不是函数参数。`length` 站点因此没有走
0020 typed-array guard，每次迭代都经过 `JS_JitHelperGetProperty` →
`JS_GetProperty` → getter（`JS_CallFree`），约 55 ns/迭代。对照组
`int32array-traversal` 直接以参数为接收者，稳态为解释器的 20.9x。

### B6 已原生化场景的剩余差距：优化编译器能力

| 场景 | quickjs-jit 每迭代 | Bun 每迭代 | 典型缺失能力 |
| --- | ---: | ---: | --- |
| property-heavy | ~4.9 ns | ~0.4 ns | 循环内 shape guard 冗余消除、load/store 转发、字段标量替换 |
| call-heavy | ~3.0 ns | ~0.3 ns | 通用内联和循环内 target guard 外提 |
| int32array-traversal | ~2.5 ns | ~0.29 ns | 边界检查消除、length/data 指针外提 |
| numeric / scalar-loop | ~1.5 ns | ~0.23 ns | 循环头 poll 与溢出检查开销、强度削减、寄存器分配 |

JSC FTL（B3 后端）和 V8 TurboFan/Turboshaft 在这些内核上都是"循环
体只剩几条机器指令"。Cranelift 本身能产出接近的代码，差距主要在
前端：guard 和 poll 的密度、帧同步，以及没做的 LICM 和 BCE。

### B7 运行时库：不是 JIT 能解决的部分

`json-codec`（Bun 快 10.5x）、`strings-regexp`（14x）、`strings-json`、
`map-set-bigint` 的时间主要花在 QuickJS 的 C 内建函数中：JSON
解析/序列化、libregexp 字节码解释器、字符串拼接与分配、Map 哈希、
BigInt。JSC 有 Yarr 正则 JIT、rope 字符串和高度优化的 JSON 快路径。
这部分应单独立项做库优化，不能记作 JIT 进展。

## 5. 能力对照：V8 / JSC(Bun) / quickjs-jit

| 能力 | V8 | JSC（Bun） | quickjs-jit（82d3808） | 差距与影响 |
| --- | --- | --- | --- | --- |
| 基线层覆盖 | Sparkplug 覆盖全部字节码 | Baseline JIT 覆盖全部字节码 | 124/255 个 opcode 被拒绝，含任一被拒 opcode 的函数整体解释执行 | 闭包、递归、for-of、try/catch 类场景 JIT ≈ 解释器（B3） |
| 分层触发 | feedback vector 上的 interrupt budget，只做计数 | execution counter，只做计数 | 每次调用 4 次 C↔Rust 回调、2 次计时、哈希查询，每 32 次调用一次全量 maintenance | 调用和回调密集型场景的固定开销（B2） |
| helper 边界 | 可信的运行时调用，不做校验 | 同左 | 每次 helper 做 ~30 项校验，加 O(字节码长度) 的 PC 扫描 | objects-polymorphic 中占 63%（B1） |
| 引用计数/GC | 追踪式 GC，存值时只有写屏障 | 同左（Riptide） | 引用计数；`Dup`/`Free` 多数是出线 helper 调用 | 对象密集代码中 helper 调用密度高 |
| 值表示 | 31 位 Smi + 指针压缩 | NaN-boxing，8 字节 | 16 字节 `JSValue`（tag + 值）；Tier 2 内部可以拆箱 | 帧与堆访问量翻倍，属于结构性上限 |
| JS→JS 调用 | 原生帧直接调用，按需物化帧 | 调用 IC 链接、原生帧 | 仅限受限叶函数直调和有界内联，其余走 generic bridge | 递归被禁用；强制 Tier 2 比解释器慢 7.8x（B4） |
| 属性 IC | 单态/多态/超多态 IC，优化层内联 map check | 同左，外加 structure watchpoint | Tier 2 的单态 shape+generation guard；多态直接回退到 helper | objects-polymorphic 0.0045x |
| 内联 | 按预算的通用内联 | DFG/FTL 通用内联 | effect-free 有界内联，加有副作用的 frame inline（每次 enter 都 `calloc`） | call-heavy 0.11x |
| 冗余检查消除/LICM | TurboFan/Turboshaft 的 LoadElimination、CheckElimination | DFG/FTL 的 CSE、LICM、整数范围分析 | 标量归纳变量已实现；shape/target guard 外提仅限入口 | property-heavy 每迭代 4.9 ns，Bun 0.4 ns |
| 边界检查消除 | 基于 range 分析 | 同左 | typed array guard 每次访问都检查，非参数接收者直接走 helper | 数组遍历约 0.1x，float64array 0.0075x（B5） |
| 逃逸分析/分配 | 逃逸分析，分配折叠 | 对象分配下沉 | 无；每个对象字面量都走 C helper 分配 | objects-polymorphic、mixed-quotes |
| 异常/async | 优化层支持 try/catch，async 可内联 | 同左 | catch 被拒；async 回到解释器 | exceptions 0.63x 解释器 |
| 库：正则/JSON/字符串 | Irregexp JIT、JSON 快路径、rope 字符串 | Yarr JIT、rope、JSON 快路径 | QuickJS C 实现（字节码正则、线性字符串） | json/regexp 约 0.05x–0.1x，与 JIT 无关（B7） |

## 6. 分阶段路线图

阶段顺序按"收益 ÷ 成本"和依赖关系排列。每个阶段都按 AGENTS.md
执行：完整三引擎矩阵、以旧 JIT 为回归基线、以同版本解释器为收益
基线、速度方向的置信区间，并更新 README。下面的阶段目标都在 P0
修正协议后的稳态口径下衡量。

### P0 测量协议修正（数天，前置条件）

- 按第 2 节修改 `bun_child` 和 worker：计时 K 个批次、落盘 wrapper、
  检测尖峰。
- 在新协议下重跑 30 场景完整矩阵，建立新基线 `S0`，替换 README，
  并标注历史数据。
- 把本文件第 3 节的诊断表升级为带配对置信区间的正式表。
- 验收：Bun 固定批次与稳态之比在所有场景的 0.8–1.25 之间，或者
  由多批次中位数替代固定批次。

### P1 去除自身开销：零成本边界（约 2–3 周，收益最确定）

1. **O(1) PC 校验**（B1）：stack map 在编译期记录 PC，helper 只做
   相等比较；非 stack-map helper 使用安装时生成的指令边界位图。
   诊断上限：objects-polymorphic 1.71x，generic-call-fallback 1.29x。
2. **分层校验**：完整身份/ABI 校验只在原生入口、OSR、resume 时执行
   一次；helper 内改为比较单个 cookie/epoch，其余项放进
   `debug_assertions`/sanitizer 构建。保留 fail-closed 语义：cookie
   不匹配时拒绝。
3. **内联 `Dup`/`Free`**：原生代码中直接做引用计数的加减，只有减到
   零时才调用 `__JS_FreeValueRT`。
4. **C 侧分层状态**：在 `JSFunctionBytecode` 上缓存"已安装入口 +
   epoch"，外加一个递减计数器。解释器调用已安装函数时直接跳进原生
   代码，不做 `record_hot`/`acquire_entry` 回调；计数器耗尽时才回调
   Rust。`native_enter/exit` 的计时只在 profitability 试验窗口内开启。
   `maintenance` 从调用路径移到 poll/中断检查点，并按输入版本缓存
   候选扫描结果（实验 B 的正式版）。
5. **inline 状态 arena**：`JS_JitInlineEnter` 改用运行时级的栈式
   arena，取代每次 `calloc`，并去掉重复的 `call_pc`/`continuation_pc`
   扫描。
6. **typed array 非参数接收者**（B5）：`length` 与元素访问允许任何
   已 guard 的 SSA 对象值作为接收者，不限于参数。

验收：所有场景 JIT / QuickJS 速度 CI 下界 ≥ 0.95x（消除净退步），
`generic-call-fallback` 与 `objects-polymorphic` ≥ 1.0x 解释器；
`mixed-quotes` 相对当前 main 速度 CI 下界 ≥ 1.5x；gpui-shell 真实
宿主 mixed/compute 不退步。

### P2 基线覆盖对齐 Sparkplug（约 4–6 周）

按场景影响排序，逐个开放 Tier 1 lowering（helper 形式即可，先求
覆盖，再求快）：

1. 闭包：`fclosure`/`fclosure8`、`get_var_ref*`/`put_var_ref*`/
   `set_var_ref*`、`make_var_ref*`、`close_loc`。
2. `push_this`、`special_object`（arguments/new.target）、`set_name`、
   `put_var`/`define_var` 系列。
3. `throw`/`throw_error`，以及 try/catch：给原生代码建立异常表，
   让 `catch`/`nip_catch`/`gosub`/`ret` 在原生帧内分派；做不到时，
   退回到在 catch 目标处恢复解释器执行。
4. `typeof*`、`in`、`instanceof`、`pow`、`delete`。
5. `for_of_*`/`for_in_*`/`iterator_*`：数组迭代器走快路径，其他
   情况调用 helper。
6. `apply`、class 定义、私有字段（以 helper 为主）。

验收：基准套件和 test262 的被拒函数比例下降到 < 5%；所有新覆盖
opcode 都有 opcode-case 清单和 deopt 回归测试；`calls-closures`、
`collections`、`calls-recursion-closures` 的 JIT / QuickJS 速度
≥ 1.5x。

### P3 原生调用约定（约 4–8 周，架构级）

- 定义 JIT 到 JIT 的调用 ABI：callee 在原生栈上分配帧，调用点带
  单态 target IC（函数身份 + generation + epoch），并直接跳到
  callee 的入口。
- 惰性物化帧：只有在 deopt、异常、GC 栈遍历、调试或回溯时，才从
  原生帧重建 `JSStackFrame`。这需要栈遍历器识别原生帧，是本阶段的
  核心风险，应先写设计文档并做独立审查。
- 开放自递归，移除 `generic_call_without_loop` 规则。
- 从 C 调用 JS 的快速入口（`Array.prototype.sort` 比较器、`map`/
  `forEach` 回调）直接使用 P1 缓存的入口。

验收：`fibonacci-recursive` ≥ 5x 解释器，且 ≥ 0.25x Bun；
`call-heavy`/`generic-call-entry` ≥ 0.3x Bun；`calls-*` ≥ 0.2x Bun；
强制 Tier 2 不再慢于解释器。

### P4 优化编译器能力（持续，按内核推进）

- 在 semantic SSA 上实现 shape/target/type guard 的冗余消除与外提
  （利用已有的 facts/effects，#29），并做 load/store 转发。
- 基于 `ir/ranges.rs` 做边界检查消除，把 length/data 指针外提到
  循环外。
- Tier 2 多态属性 IC：≤ 4 个 shape 的内联分派，超出则回退为
  megamorphic 探测缓存。
- 对象分配快路径：已知 shape 的字面量在原生代码中直接 bump 分配，
  然后做有限的逃逸分析与标量替换。
- 通用有预算内联（依赖 P3 的帧物化能力）。
- 降低 poll 和溢出检查的密度：将计数型循环的 poll 下沉为 64 次一次
  或 interrupt 标志检查；已证明范围的归纳变量省去溢出检查。

验收里程碑（稳态，按场景分别报告，不用几何平均掩盖）：
`property-heavy`/`call-heavy`/`*-traversal` ≥ 0.33x Bun；标量内核
≥ 0.5x Bun；`objects-polymorphic` ≥ 0.1x Bun。

### P5 运行时库（独立轨道，不计入 JIT 进展）

JSON 解析/序列化快路径、字符串 `+=` 的 builder 或 rope 利用、
`Map`/`Set` 哈希、正则（libregexp 的缓存或 JIT）。以纯库微基准对
Bun 单独立项。

### 分阶段对 Bun 的预期位置（稳态、粗估）

| 场景类 | 现状 | P1 后 | P2 后 | P3 后 | P4 后 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 标量循环 | 0.15–0.24x | 同左 | 同左 | 同左 | ≥ 0.5x |
| 属性/调用/数组（已原生） | 0.08–0.12x | 0.1–0.15x | 同左 | 0.2–0.3x | ≥ 0.33x |
| 闭包/递归/集合 | 0.02–0.03x | 同左 | 0.04–0.06x | 0.1–0.2x | 0.2–0.3x |
| 净退步类（fallback/多态/异常） | 0.001–0.05x | ≥ 解释器 | 0.05–0.1x | 同左 | 0.1–0.2x |
| 库类（JSON/regexp/Map） | 0.05–0.1x | 同左 | 同左 | 同左 | 取决于 P5 |

这些数字只是规划用的数量级估计，每个阶段完成后都要用实测矩阵替换。
第一优先级是消除"JIT 慢于解释器"，然后才是缩小与 Bun 的倍数差距。

## 7. 复现方法

```sh
cargo build --release --manifest-path benchmarks/Cargo.toml --bins
# 单个 worker（记录 fixed_metrics 与 64 个预热批次）
./target/release/jit-bench worker --mode automatic --script benchmarks/scripts/numeric.js
./target/release/jit-bench worker --mode bun --script benchmarks/scripts/numeric.js
```

- **稳态对比**：对每个场景、每个引擎，取 `protocol.warmup_batch_ns`
  最后 16 项的中位数，3 个进程再取中位数，`taskset -c 8`。
- **Bun 尖峰复现**：用 `bun -e` 和 `.mjs` 文件两种方式运行 harness
  wrapper，并逐批打印 `Bun.nanoseconds()` 的差值。
- **采样**：在 `perf_event_paranoid=2` 下，用 gdb 启动 worker，设置
  `handle SIGUSR2 stop print nopass`（注意 `noprint` 隐含 `nostop`），
  外部每 20 ms 发一次 `kill -USR2`，每次停下取 `bt 12`，最后汇总自身
  帧和包含帧。如果可以执行
  `echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid`，改用
  `samply record` 可以得到更高质量的 profile。
- **实验 A/B**：在独立 worktree 中修改 `0011-helper-pc-cursor.patch`
  和 `ProductionBackend::maintenance`，用环境变量切换；同时更新
  `sys/build_support/patch.rs` 中的补丁摘要和 `quickjs.c` 指纹。
  这些改动只用于诊断，没有进入仓库。
