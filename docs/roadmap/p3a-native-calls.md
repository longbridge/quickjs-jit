# P3a 原生调用约定：设计与第一切片

分支 `perf/p3a-native-calls`，基于 `82d3808`。对应
[性能路线图](../PERFORMANCE_ROADMAP_V8_BUN.md) 的 B4/P3。本文先给出完整的
JIT→JIT 原生调用约定设计，再说明本分支实际落地的第一个 fail-closed 切片、
它刻意没有做的部分，以及后续工作。

## 1. 问题

目前 compiled→compiled 调用只有两条快速路径：

1. 受限的 Int32/Bool **无副作用叶函数**直调（`lower_direct_call_machine`、
   `lower_direct_bool_leaf`）：`(out*, 标量参数...) -> status`，status 非零时
   调用方在 CALL 字节码处 deopt，由解释器重新执行这次调用。
2. 有界的 effect-free / frame 内联。

其余调用都走 `JS_JitHelperCall → JS_Call → JS_CallInternal → 回调 →
原生入口`，每次调用都要经过 helper 帧校验（B1）和 JIT 簿记回调（B2）。
递归函数因此被 `maintenance` 的 `generic_call_without_loop` 规则留在
解释器里；强制 Tier 2 时 `fibonacci-recursive` 比解释器慢约 7.8x。

## 2. 完整设计（目标状态）

### 2.1 调用 ABI

每个可原生调用的 Tier 2 artifact 额外发布一个 **native call entry**：

```text
status = entry(nc: *mut JSJitNativeCallContext, out: *mut Scalar, args...)
```

- `nc` 是按"最外层原生调用"分配在调用方栈上的上下文：栈下限、剩余中断
  预算、状态标志，以及（后续切片）惰性帧链的头指针。所有嵌套原生调用
  共享同一个 `nc`，因此一次原生调用链只需要一次进入/退出簿记。
- 参数按 feedback 签名拆箱：Int32/Bool 为 `i32`，Float64 为 `f64`，借用的
  堆引用为对象指针（调用方保证在调用期间持有根）。结果写入 `out`。
- `status`：`0` 成功；`1` 重试（被调方保证尚未产生任何可观察副作用，调用方
  在 CALL 处 deopt，由解释器按原语义重新执行）；`2` 异常已挂在
  `ctx->rt->current_exception` 上，调用方按异常出口展开（后续切片）。

### 2.2 调用点 IC

调用点使用单态 target IC：

- **identity**：函数对象指针（被 feedback 根住）和 `JSObject::u.func.function_bytecode`；
- **generation**：被调方 `FunctionKey` 的 generation，写入调用方 artifact 的依赖；
- **epoch**：被调方 artifact 的发布 pin。调用方持有被调方 native entry 的
  `PublishedBaselineCode` 克隆，被调方被替换或淘汰时，调用方 artifact 随依赖
  一起失效，旧代码在所有 pin 释放前保持可执行。

未命中 IC 时走原来的 generic CALL 路径（或在 CALL 处 deopt），绝不猜测。

第一切片的实现：运行时守卫只比较对象指针和字节码指针（没有独立的
generation/epoch 比较字）。generation 通过依赖实现：每个链接到其他函数
native entry 的调用点都把被调方 `FunctionKey` 加入调用方 artifact 的
`ArtifactDependency`，被调方被回收（retire）时调用方 artifact 随之失效，
因此被释放的地址被另一个同名函数复用时不会命中旧的原始指针守卫（ABA）。
epoch 就是上面的发布 pin。

### 2.3 原生帧与惰性 `JSStackFrame` 物化

原生 callee 不压 `JSStackFrame`。它的帧由 Cranelift 管理，并为每个可能
离开原生代码的点（helper 调用、异常、deopt）记录一个 frame-state：函数身份、
字节码 pc、参数/局部/操作数栈到机器位置的映射。只有下列事件才需要把原生
帧物化成真实的 `JSStackFrame`：

| 事件 | 处理 |
| --- | --- |
| deopt | 从最内层原生帧开始，按 frame-state 为链上每一层重建解释器帧（与 `0018-inline-frame-recovery` 的影子帧同构），然后在最内层 pc 恢复解释器。 |
| 异常 | 构造 `Error.stack` 之前，C 侧 backtrace 遍历器先询问 `nc` 链上的原生帧，按 frame-state 输出函数名和 pc；没有 try/catch 的原生帧直接返回 status 2 展开。 |
| GC 栈扫描 | QuickJS 使用引用计数加环检测；原生帧只持有借用引用（调用方的根保证存活）或被 frame-state 标注为拥有的值。拥有值在 helper 调用前溢出到 `nc` 管理的影子槽，`mark_children` 通过运行时上的原生帧链把它们当作根。 |
| backtrace / 调试 | 同异常。 |

在做到帧物化之前，原生 callee 必须满足"失败时无副作用"，这正是第一切片
的约束（见第 3 节）。

### 2.4 与 C 栈遍历器、中断和栈溢出的交互

- **栈遍历器**：`rt->current_stack_frame` 链不包含原生帧；目标状态下运行时
  新增 `jit_native_frames` 链头，遍历器（backtrace、`Function.caller` 类、
  GC）在遇到 JIT 入口帧时插入原生帧。第一切片的原生帧从不对外可见：没有
  helper、没有异常、没有分配，因此无需遍历。
- **中断**：解释器在每次 `JS_CallInternal` 入口递减
  `ctx->interrupt_counter`。原生入口同样按调用递减 `nc->budget`（从
  `ctx->interrupt_counter` 读入，退出时写回）。预算耗尽时：没有安装
  interrupt handler 就只重置预算（与 `__js_poll_interrupts` 相同）；安装了
  handler 则调用 handler，返回 0 时继续原生执行，返回非零时在运行时上记录
  `rt->jit_interrupt_pending`、把 `ctx->interrupt_counter` 置 0 并返回
  status 1。解释器重试这次调用，它的下一个 poll（重试 CALL 的入口 poll）
  看到 pending 后直接抛出不可捕获的 `interrupted`，**不会再次调用 handler**，
  所以 handler 的一次 `true` 不会丢失（只返回一次 `true` 的 handler 也能
  中断）。pending 期间 `Begin` 拒绝新链。
- **栈溢出**：解释器用 `js_check_stack_overflow(rt, alloca_size)` 比较栈指针
  和 `rt->stack_limit`。原生入口在序言里做同样的比较，并且按"解释器帧的
  保守开销"累计虚拟栈深度（见 3.4），保证原生递归不会在解释器会抛
  `RangeError` 的深度之后才失败。失败返回 status 1，由解释器以真实帧
  重新执行并在同样的检查处抛出 `RangeError`。为避免每一层解释器帧重复一次
  原生递归（二次复杂度），失败时在运行时上记录栈水位
  `rt->jit_native_floor`；严格更深的位置不再尝试原生调用。水位在以下任一
  情况清除：重试到达真实栈限并抛出 stack overflow `RangeError`
  （`JS_ThrowStackOverflow`）；某条链在水位或更浅处开始；或者拒绝次数超过
  按剩余栈估计的上限（`(sp - stack_limit) / 256 + 64`）。
- **GC 根**：原生参数只有标量和借用引用；借用引用的根由最外层调用方持有
  （参数/局部或 owned 栈槽）。第一切片不传堆参数。

## 3. 第一切片（本分支）

### 3.1 范围

- **native call entry**（`jit/src/compiler/native_call.rs`）：Tier 2 编译一个
  函数时，如果它的 `bounded_specialization` 签名是 Int32/Float64 参数、
  Int32/Float64 结果，且函数体只包含下列字节码，就在同一个 artifact 里
  额外发布一个 native entry：
  - `get_arg*`、整数常量（`push_*`）、常量池里的 Int32/Float64
    （`push_const`/`push_const8`）；
  - Int32 的 `add`/`sub`/`mul`/`inc`/`dec`（溢出和 `-0` 失败），Float64 的
    `add`/`sub`/`mul`/`div`/`inc`/`dec`（精确 IEEE-754，Int32 操作数先精确
    扩宽），比较 `lt`/`lte`/`gt`/`gte`/`eq`/`neq`/`strict_eq`/`strict_neq`
    （Float64 用有序比较，NaN 结果与解释器一致）、`lnot`、`drop`、`nop`；
  - 前向 `if_true`/`if_false`/`goto`（汇合点的操作数栈按 Cranelift block
    参数传递，类型必须一致），`return`；
  - 对自身的调用：`get_var <atom>` + `call0..3`/`call`/`tail_call`，实参个数
    等于形参个数，所有 `get_var` 必须是同一个 atom。

  任何其他字节码、异常表、闭包变量、局部变量、回边（循环）、Bool/堆引用
  签名都让 native entry 不发布（fail-closed），函数照旧编译。
- **Tier 2 调用点**（`emit_opt_native_call`）：函数自己的自递归调用点，以及
  调用另一个已发布 native entry 的单态调用点（coordinator 在排队时把
  被调方的 native entry 作为 `NativeCallTarget` 交给请求，并保持其发布 pin），
  按以下顺序执行：保护 callee 对象身份和字节码身份 → 保护实参 tag 与签名
  一致 → `qjsjit_native_call_begin`（Rust，包一层线程局部栈水位后调用补丁
  0029 的 `JS_JitNativeCallBegin`）→ 调用 native entry → `qjsjit_native_call_end`
  → 释放 `get_var` 得到的函数引用（`FREE` helper）。任一步失败都在 CALL 处
  deopt，由解释器重新执行这次调用（与现有叶直调的失败路径相同）。
  有 native target 的调用点不再参与 pure/frame 内联（内联一层递归函数只会
  让内层调用走 generic bridge）。
- **分层策略**（`jit/src/lib.rs` 的 `maintenance`）：
  - 自递归调用点被新路径接纳的函数，不再被 `generic_call_without_loop` 挡在
    解释器；调用一个已就绪 native entry 的无循环函数同样豁免。
  - 进入原生代码仍有几百纳秒的簿记开销（B2），约等于十次解释器调用，
    所以只接纳"每次外部进入平均至少 16 次调用"的递归：baseline 在
    `native_enter` 统计外部进入次数（父执行不是同一函数），与 baseline
    执行次数相比。数据不足时保持 baseline 等待（最多等到 64 次外部进入），
    之后仍然太浅就按原规则拒绝。`calls-recursion-closures` 的
    `recursiveSum(i & 7)` 因此继续留在解释器，没有退步。
  - 这类函数获得一次有界 Tier 2 试验；它们的 Tier 2 单次调用覆盖整棵
    递归树，不能与逐次调用的 baseline 样本比较，所以不参加
    `classify_tier2_trial` 的单次时间比较。deopt 统计和已有的 side-exit
    预算仍然有效。
  - 调用方在单态被调方的 native entry 发布之前保持 baseline（与等待标量
    直调 entry 相同），避免把调用点永久编译成 generic CALL。

### 3.2 为什么"重试"是精确的

原生 entry 的函数体没有任何副作用：不调用 helper（中断轮询除外，见 3.4）、
不分配、不读写堆。自调用的 `get_var <self>` 在原生代码里**不执行**，而是在
进入原生调用链前由 `JS_JitNativeCallBegin` 一次性证明：

1. 被调对象是 `JS_CLASS_BYTECODE_FUNCTION`、`func_kind == JS_FUNC_NORMAL`，
   其 realm 就是调用方的 context（调用点已经保护了对象和字节码身份）；
2. 按 `JS_GetGlobalVar` 的查找顺序，名字解析为普通数据属性
   （`global_var_obj` 中的词法绑定，或 `global_obj` 自有的
   `JS_PROP_NORMAL` 属性），值就是这个函数对象本身。未初始化的词法绑定
   （值不是该对象）、访问器、auto-init、var-ref 都拒绝。

链中不会运行任何 JS 代码，所以这个绑定在整条链执行期间不变，每一次内部
`get_var` 都会得到同一个值、不会调用 getter、不会抛异常。任何检查失败
（Int32 溢出、`-0`、栈深、中断请求）都只丢弃纯计算，然后在最外层 CALL 处
deopt，由解释器重新执行，结果、异常类型、消息和 backtrace 都来自解释器。

测试（`jit/tests/native_calls.rs`）覆盖：自动分层和强制 Tier 2 下的
native 递归（稳态无 generic CALL helper、每次求值只有一次 QuickJS 入口）；
深处 Int32 溢出、非 Int32 实参、`-0`；运行中重绑定全局函数、改成访问器
（getter 调用次数与解释器一致）、被全局 `let` 遮蔽；栈耗尽抛出
`RangeError` 且不出现二次复杂度、解释器会抛错的深度原生代码也抛错；
interrupt handler 被原生链轮询并能中断；单态调用方链接到 native entry、
重绑定后回退；浅递归被拒绝；Float64 递归与签名保护。

### 3.3 发布与生命周期

- native entry 与主代码在同一个 artifact 中编译：`publish_relocatable`
  先发布 native entry，再发布主代码，主代码里的 `func_addr` 符号重定位
  （`qjsjit.native_call_entry`）解析为 native entry 地址；主代码（及其 OSR
  代码）的发布分配持有 native entry 的发布 pin，释放顺序是先主代码后 entry。
- native entry 调用自身使用 colocated 用户名（`NATIVE_SELF_NAMESPACE`），
  最终重定位解析为自身基址（x86_64 `X86CallPCRel4`，aarch64 `Arm64Call`）。
- 跨函数调用方把被调方 native entry 的发布 pin 放入
  `direct_call_dependencies`，与现有叶直调一致；同时把被调方
  `FunctionKey` 加入 artifact 依赖（r1），被调方 retire 时调用方失效。native entry 只依赖字节码，
  被调方 artifact 被替换后旧 entry 仍然正确，调用点的身份保护负责选择。
- native entry 有 unwind 信息，但没有 stack map 或 frame state；它从不在
  helper、GC 或异常中出现在栈遍历器面前。

### 3.4 中断、栈深度和 GC

- **中断**：`Begin` 从 `ctx->interrupt_counter` 读入预算（没有 handler 时
  为 `INT32_MAX`，与 `__js_poll_interrupts` 只重置计数器等价）。每次原生
  调用递减一次（对应 `JS_CallInternal` 入口的 poll）。预算耗尽时调用
  `JS_JitNativeCallPoll`：handler 返回 0 就重置预算继续；返回非零则标记
  `INTERRUPT_DUE` 并让整条链重试，`End` 把 `ctx->interrupt_counter` 置 0，
  并设置 `rt->jit_interrupt_pending`。重试的 CALL 进入 `JS_CallInternal`
  时立即 poll，`__js_poll_interrupts` 看到 pending 就清除它并抛出不可捕获
  的 `interrupted`，不再调用 handler。r1 之前重试会再次调用 handler，
  只返回一次 `true` 的 handler 的中断因此丢失（审查发现，已有回归测试）。
  仍然存在的差异只涉及 handler 的调用时机：原生链每次调用轮询一次，而
  解释器还在每个 `goto`/`if_true`/`if_false` 上轮询；因其他原因
  （溢出、`-0`、栈）重试的链会让解释器重新消耗预算，handler 可能比纯解释器
  多被调用（返回 0 的调用）。这些只影响中断延迟和 handler 调用次数，不会
  丢失或重复一次中断。
- **栈深度**：native entry 的第三个参数是"虚拟栈指针"，最外层取调用点的
  真实栈指针，每层减去 `frame_charge`：`16 × (参数 + 局部 + 栈 + 4)` 加上
  **本构建实测**的解释器单层开销（`interpreter_frame_bytes`：在一个无 JIT 的
  私有 runtime 里，比较宿主探针在递归深度 0 和 64 时的栈指针，再加 25%
  余量；x86_64 release 实测约 1984 字节/层，取 2480）。检查条件等价于
  解释器的 `js_check_stack_overflow`（`vsp - charge < rt->stack_limit`）。
  因为每层至少按解释器开销计费，原生链总在解释器会抛 `RangeError` 的深度
  之前失败，然后由解释器以真实帧复现。失败时 `End` 在运行时
  （`rt->jit_native_floor`）记录最外层调用点的栈地址；严格更深处不再尝试
  原生链（否则每一层解释器帧都会重新跑一次接近栈限的原生递归，变成二次
  复杂度）。重试抛出 stack overflow `RangeError`、同一高度或更浅处开始新链、
  或拒绝次数超过剩余栈对应的层数上限时清除，因此一次被捕获的
  `RangeError` 不会让同一调用高度永久失去原生调用（r1 之前的线程局部水位
  只在严格更浅处清除，而同一个包装函数换了执行层级后帧高度会变化，导致
  永久拒绝并反复 deopt，最终被降级）。
- **GC**：链中没有分配，也没有堆值；借用的函数对象由调用方 owned 栈槽和
  全局绑定共同持有。

### 3.5 本切片不做（后续工作）

- 堆引用参数/结果、Bool 参数/结果、局部变量、循环、helper 调用、属性访问、
  闭包变量、`this`、`arguments`；
- 原生 entry 内部调用**其他**函数（需要对每个全局名做同样的绑定证明，或者
  依赖 2.3 的帧物化）；
- status 2（原生帧内的异常）和惰性帧物化、栈遍历器集成、GC 根扫描——这是
  放开"失败时无副作用"约束的前提；
- Tier 1 调用方和 C→JS 回调（`sort`/`map`）使用 native entry；
- 用 P1 的 C 侧入口缓存降低 QuickJS→native 的进入成本后，再降低
  "每次外部进入至少 16 次调用"的门槛（`calls-recursion-closures`）。

## 4. 噪声诊断（非发布数据）

主机 i7-13700KF，24 核 / 31 GB，同时运行约 16 个 agent；未绑核，P/E 核
导致明显的双峰分布。每个场景交替运行基线（`82d3808` 的
`target/release/jit-bench`）和本分支二进制，取每次运行最后 16 个预热批次的
中位数，再取各次运行的中位数；速度 = 基线 / 新版本（>1 更快）。单位
ms / 10 次 workload 调用。这不是 AGENTS.md 要求的发布级配对证据，README
矩阵未修改。

| 场景（automatic） | 运行次数 | 基线 ms | 本分支 ms | 速度 | 备注 |
| --- | ---: | ---: | ---: | ---: | --- |
| fibonacci-recursive | 9 | 9.8984 | 0.3892 | 25.4x | 同一二进制解释器 9.82 ms → 本分支约 25x 解释器（目标 ≥ 5x） |
| calls-recursion-closures | 9 | 9.0645 | 6.4681 | 1.40x（噪声） | 两边各次最小值 6.44 / 6.45，实为持平；`recursiveSum(i&7)` 被浅递归门槛拒绝，留在解释器 |
| call-heavy | 9 | 0.1140 | 0.1118 | 1.02x | 持平（已走叶直调，本切片不改变） |
| mixed-quotes | 7 | 2.9061 | 2.6447 | 1.10x（噪声） | 双峰，持平 |
| generic-call-entry | 7 | 0.0576 | 0.0548 | 1.05x（噪声） | 持平 |
| quickjs-fibonacci | 7 | 0.5326 | 0.3709 | 1.44x（噪声） | 两边最小值 0.370 / 0.359，持平 |
| numeric | 7 | 0.0298 | 0.0293 | 1.02x | 持平 |
| property-heavy | 7 | 0.0988 | 0.0985 | 1.00x | 持平 |

强制 Tier 2 诊断：`fibonacci-recursive` 从 74.6 ms 降到 0.368 ms（此前比
解释器慢 7.6x，现在快约 27x）；`calls-recursion-closures` 从 85.7 ms 降到
44.2 ms（仍慢于解释器 6.4 ms，闭包路径不在本切片范围内）。

全部 30 个场景在 interpreter / tier1 / automatic / tier2 四种模式下的
checksum 与解释器一致。按第 2 节的稳态口径，Bun 的 `fibonacci-recursive`
约 0.274 ms，本分支约为 Bun 速度的 0.7x（噪声诊断，待集成者的三引擎矩阵
确认）。

## 5. 已知的既有问题（在 `82d3808` 上同样复现）

- 不可捕获的 `interrupted` 异常展开 Tier 2 帧后，如果宿主没有调用
  `ctx.catch()` 清除异常，下一次求值中的任何 JIT helper 都会因
  `JS_HasException` 而抛出 "invalid JIT helper state"（解释器不受影响）。
  本分支的中断测试在中断后清除异常。
- 强制 Tier 2 的 `call-heavy` 和 `generic-call-entry` 多数运行约 5.6 ms
  （偶尔 0.06 ms），基线二进制同样如此。
- frame 内联一个递归被调方时，调用方每次求值 deopt 一次；有 native target
  的调用点现在不再内联，因此不受影响。

## 6. 审查修复（`perf/p3a-native-calls-r1`）

阻塞项：

1. **中断丢失**（已修复）。patch 0029 在 `JSRuntime` 上新增
   `jit_interrupt_pending`：`JS_JitNativeCallPoll` 在 handler 返回非零时设置它，
   `__js_poll_interrupts` 先检查它并直接抛出 `interrupted`，`Begin` 在
   pending 时拒绝。回归测试
   `native_recursion_delivers_a_one_shot_interrupt`（handler 只返回一次
   `true`，自动分层和强制 Tier 2 都必须得到 `interrupted`）；修复前失败。
2. **README 三引擎矩阵与发布级证据**：按本次工作流的环境规则（16 个 agent
   共享机器，"do NOT run the full benchmark matrix and do NOT edit the README
   matrix"），发布级 QuickJS / Bun / quickjs-jit 矩阵由集成者在合并前统一
   测量并写入 README。本分支只提供第 4 节和最终报告里的噪声诊断，
   因此在集成者刷新矩阵之前，这个优化**不算完成**（AGENTS.md）。

非阻塞项中已修复的：

- 粘性栈水位：改为运行时字段，并增加 stack overflow 清除和拒绝预算
  （回归测试 `native_recursion_resumes_after_a_caught_range_error`，修复前
  失败）。
- 非自调用 native 调用点缺少被调方依赖（ABA）：已加入
  `ArtifactDependency`（测试 `a_replaced_callee_invalidates_its_linked_caller`
  是尽力而为的回归测试，地址复用无法被确定性地构造）。
- 设计文档与实现的 IC 描述不一致：2.2 已更新。

新增测试覆盖：tail call 自调用（`native_tail_self_calls_return_the_callee_result`）、
Float64 比较中的 NaN / `-0`（`native_float64_comparisons_match_the_interpreter`）、
Int32 `lnot`（`native_int32_lnot_matches_the_interpreter`）、调用方通过别名
（参数）调用被调方而全局名已重绑定（`a_linked_caller_follows_an_aliased_callee_binding`）、
get_var 取得的被调方在成功和各 deopt 边上的引用计数（用 `WeakRef` 检查被
替换的函数确实被回收，`native_call_sites_release_the_global_callee_on_every_edge`）。

仍未处理（记录为后续工作）：

- 进入 `native_recursive` 后 `tier2_trial_decided` 被强制为 true：每次都重试
  的链（例如 Int32 反馈但实际溢出）不会被度量或降级。应改为按链重试率
  （native fallback / Tier 2 entry）降级。
- 调用方在 native_recursive 被调方处于 Cold/Queued/Compiling/Ready 时一直
  等待；被调方如果因回退永远停留在 Cold，调用方就不会升级。应给等待加上
  次数或时间上限。维护逻辑也把 `native_call_ready` 的调用点都算作已覆盖，
  但 `obj.fib(n)` 等带 `this` 的形状仍走 generic call。
- 三元表达式把 Float64 值留在操作数栈上再调用自身（例如
  `(x>1.5?0.5:0.75)+cmp(x-1.125)`）时 Tier 2 编译返回 `InvalidArtifact`，
  失败是关闭的（留在低层级），但这个形状没有得到 native 调用。
- 第 3.5 节列出的范围（Bool / 堆参数、链内调用其他函数、惰性帧物化）以及
  `calls-recursion-closures` 的 `recursiveSum` 和 `call-heavy` 目标仍然未完成。

r1 噪声诊断（同一台负载机器，交替运行 7 次，最后 16 个预热批次中位数的
中位数，automatic；速度 = `82d3808` / r1，>1 更快；非发布数据）：
`fibonacci-recursive` 9.839 → 0.360 ms（27.3x；同一基线二进制的解释器
9.830 ms）；`calls-recursion-closures` 6.426 → 6.436 ms（1.00x，持平）；
`call-heavy` 0.0598 → 0.0603 ms（0.99x，持平）；`quickjs-fibonacci`
0.365 → 0.364 ms（1.00x，持平）。四个场景的 checksum 都与解释器一致。
