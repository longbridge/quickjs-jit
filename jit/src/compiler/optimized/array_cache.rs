//! Profile-selected, lazily guarded array metadata carried across SSA blocks.
//! A plan authorizes a guard, never a heap fact. Metadata is invalidated before
//! every operation that can reenter, finalize a reference or alter storage.

use crate::ir::{
    IntegerRangeAnalysis, KnownFacts, LoopAnalysis, OptimizedIr, ScalarHeapEffect,
    ScalarHeapOperation, ScalarValueId,
};
use crate::runtime::{ArrayAccess, ArrayFeedbackSnapshot, ArrayMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ArrayCandidate {
    pub argument: u16,
    pub mode: ArrayMode,
    /// Typed length requires the metadata query to guard observable lookup.
    pub needs_length: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ArrayAccessPlan {
    pub candidate: usize,
    pub access: ArrayAccess,
    pub index: Option<ScalarValueId>,
    pub result: Option<ScalarValueId>,
    /// Lowering must check an Int32 key, non-negativity, and unsigned
    /// `index < dense_count` before using cached storage. The plan never turns
    /// an out-of-bounds access into `undefined` by itself.
    pub requires_index_guard: bool,
    /// This access is dominated by a packed-array metadata guard in the
    /// natural-loop preheader, and integer range analysis proved its key is
    /// below that guard's exact logical length. Lowering may omit only the
    /// per-access count comparison; tag, hole/value and address checks remain.
    pub bounds_covered_by_hoist: bool,
    pub loop_header: u32,
}

/// Metadata that may be materialized exactly once on the loop-entry edge.
///
/// Packed arrays require `logical_length == dense_count` in this guard. That
/// equality makes the SSA `bound` used by range analysis an exact storage
/// bound and rejects sparse/extended arrays before entering native loop code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ArrayLoopHoist {
    pub candidate: usize,
    pub preheader: u32,
    pub header: u32,
    pub bound: ScalarValueId,
    pub length_node: u32,
}

#[derive(Debug, Default)]
pub(super) struct ArrayPlan {
    candidates: Vec<ArrayCandidate>,
    accesses: Vec<Option<ArrayAccessPlan>>,
    hoists: Vec<ArrayLoopHoist>,
    invalidations: Vec<bool>,
}

impl ArrayPlan {
    const MAX_CANDIDATES: usize = 16;

    #[allow(clippy::too_many_arguments)]
    pub(super) fn analyze(
        ir: &OptimizedIr,
        facts: &KnownFacts,
        loops: &LoopAnalysis,
        feedback_len: usize,
        feedback_at: impl Fn(u32) -> Option<ArrayFeedbackSnapshot>,
        typed_length_guard: bool,
        non_reentrant: impl Fn(u32) -> bool,
        mut work: usize,
    ) -> Self {
        if !loops.is_complete() {
            return Self::default();
        }
        let graph = ir.scalar_graph();
        let nodes = ir.nodes();
        // Charge all output-sized allocations before making them. Partial
        // analysis is never published when the caller's finite budget runs out.
        let Some(initial_cost) = nodes
            .len()
            .checked_mul(8)
            .and_then(|cost| {
                feedback_len
                    .checked_mul(4)
                    .and_then(|feedback| cost.checked_add(feedback))
            })
            .and_then(|cost| cost.checked_add(64))
        else {
            return Self::default();
        };
        if spend(&mut work, initial_cost).is_none() {
            return Self::default();
        }

        #[derive(Clone, Copy)]
        struct Proposed {
            node: u32,
            argument: u16,
            mode: ArrayMode,
            access: ArrayAccess,
            index: Option<ScalarValueId>,
            result: Option<ScalarValueId>,
            loop_header: u32,
        }

        let mut proposed = Vec::with_capacity(nodes.len().min(work));
        for node in nodes {
            if spend(&mut work, 1).is_none() {
                return Self::default();
            }
            let Some(operation) = graph.heap_operation(node.id()).copied() else {
                continue;
            };
            let (base, access, index, result) = match operation {
                ScalarHeapOperation::GetElement {
                    base,
                    index,
                    result,
                    ..
                } => (base, ArrayAccess::Load, Some(index), Some(result)),
                ScalarHeapOperation::PutElement { base, index, .. } => {
                    (base, ArrayAccess::Store, Some(index), None)
                }
                ScalarHeapOperation::GetProperty {
                    base, atom, result, ..
                } if atom == rquickjs_core::qjs::JS_ATOM_length => {
                    (base, ArrayAccess::Length, None, Some(result))
                }
                _ => continue,
            };
            let Some(site) = feedback_at(node.pc()) else {
                continue;
            };
            if site.access() != access || !site.can_specialize() {
                continue;
            }
            let mut modes = site.modes();
            let Some(mode) = modes.next() else { continue };
            if modes.next().is_some() || mode == ArrayMode::Generic {
                continue;
            }
            // Only the typed store lowering has the leaf contract: an exact
            // fixed, attached, nonshared writable view, in-bounds Int32 key,
            // and numeric value whose conversion cannot run script. Packed
            // stores may replace an owner or grow storage and stay generic.
            if access == ArrayAccess::Store
                && !matches!(mode, ArrayMode::Int32 | ArrayMode::Float64)
            {
                continue;
            }
            if access == ArrayAccess::Length && mode != ArrayMode::Packed && !typed_length_guard {
                continue;
            }
            let Some(argument) = facts.entry_argument(base) else {
                continue;
            };
            let Some(block) = loops.block_for_node(node.id()) else {
                continue;
            };
            let containing_loop = loops
                .loops()
                .iter()
                .filter(|natural_loop| natural_loop.contains_block(block))
                .min_by_key(|natural_loop| natural_loop.members().len());
            // A canonical traversal often snapshots `a.length` immediately
            // before the loop. Associate that dominating read with the loop;
            // element accesses themselves must still be inside it.
            // A typed snapshot read in an earlier block is not the same SSA
            // value as the count returned by the later leaf query, so it must
            // not authorize bounds-check elimination. Packed Array length is
            // the engine-owned, non-overridable logical length and its guard
            // additionally proves logical_length == dense_count.
            let dominating_loop = (access == ArrayAccess::Length && mode == ArrayMode::Packed)
                .then(|| {
                    loops
                        .loops()
                        .iter()
                        .filter(|natural_loop| {
                            natural_loop
                                .preheader()
                                .is_some_and(|preheader| loops.dominates(block, preheader))
                        })
                        .min_by_key(|natural_loop| natural_loop.members().len())
                })
                .flatten();
            let loop_header = if let Some(natural_loop) = containing_loop.or(dominating_loop) {
                natural_loop.header()
            } else if access != ArrayAccess::Length {
                // A standalone load still has a concrete guarded lowering and
                // must be represented in the final capability set. It cannot
                // hoist metadata or eliminate its index guard.
                block
            } else {
                continue;
            };
            proposed.push(Proposed {
                node: node.id(),
                argument,
                mode,
                access,
                index,
                result,
                loop_header,
            });
        }

        // Mixed modes for one entry identity cannot share one metadata tuple.
        // Drop that identity as a unit rather than publishing a partial plan.
        let Some(conflict_work) = proposed.len().checked_mul(proposed.len()) else {
            return Self::default();
        };
        if spend(&mut work, conflict_work).is_none() {
            return Self::default();
        }
        let mut conflicting_arguments = Vec::new();
        for left in &proposed {
            if proposed
                .iter()
                .any(|right| right.argument == left.argument && right.mode != left.mode)
                && !conflicting_arguments.contains(&left.argument)
            {
                conflicting_arguments.push(left.argument);
            }
        }
        proposed.retain(|access| !conflicting_arguments.contains(&access.argument));

        // A reentrant operation in the natural loop can replace storage, detach
        // a buffer, or run arbitrary script. Feedback never authorizes carrying
        // metadata through it. A lowering may explicitly certify a node only
        // when its emitted guarded path is non-reentrant.
        let Some(effect_work) = proposed.len().checked_mul(nodes.len()) else {
            return Self::default();
        };
        if spend(&mut work, effect_work).is_none() {
            return Self::default();
        }
        let mut proposed_nodes = vec![false; nodes.len()];
        for access in &proposed {
            proposed_nodes[access.node as usize] = true;
        }
        proposed.retain(|candidate| {
            let Some(block) = loops.block_for_node(candidate.node) else {
                return false;
            };
            let Some(natural_loop) = loops
                .loops()
                .iter()
                .find(|natural_loop| natural_loop.header() == candidate.loop_header)
            else {
                return candidate.access != ArrayAccess::Length && candidate.loop_header == block;
            };
            if candidate.access != ArrayAccess::Length && !natural_loop.contains_block(block) {
                return false;
            }
            if candidate.access == ArrayAccess::Length
                && !natural_loop.contains_block(block)
                && !natural_loop
                    .preheader()
                    .is_some_and(|preheader| loops.dominates(block, preheader))
            {
                return false;
            }
            for node in nodes {
                if !loops
                    .block_for_node(node.id())
                    .is_some_and(|owner| natural_loop.contains_block(owner))
                {
                    continue;
                }
                if graph.effect_for_node(node.id()) != ScalarHeapEffect::Reentrant
                    || non_reentrant(node.id())
                    || proposed_nodes[node.id() as usize]
                {
                    continue;
                }
                return false;
            }
            true
        });

        let mut candidates: Vec<ArrayCandidate> = Vec::new();
        let mut accesses: Vec<Option<ArrayAccessPlan>> = vec![None; nodes.len()];
        for proposed in proposed {
            let candidate = if let Some(index) = candidates.iter().position(|candidate| {
                candidate.argument == proposed.argument && candidate.mode == proposed.mode
            }) {
                index
            } else {
                if candidates.len() == Self::MAX_CANDIDATES {
                    return Self::default();
                }
                candidates.push(ArrayCandidate {
                    argument: proposed.argument,
                    mode: proposed.mode,
                    needs_length: false,
                });
                candidates.len() - 1
            };
            if proposed.access == ArrayAccess::Length {
                candidates[candidate].needs_length = true;
            }
            accesses[proposed.node as usize] = Some(ArrayAccessPlan {
                candidate,
                access: proposed.access,
                index: proposed.index,
                result: proposed.result,
                requires_index_guard: proposed.access != ArrayAccess::Length,
                bounds_covered_by_hoist: false,
                loop_header: proposed.loop_header,
            });
        }

        // JSC's integer-range phase only removes a check when the compared SSA
        // bound is the same value guarded at the storage boundary. For packed
        // arrays we establish that identity by guarding logical_length ==
        // dense_count. For typed arrays the versioned leaf API additionally
        // guards the exact intrinsic getter and fixed backing before returning
        // the same live count; `typed_length_guard` is the capability bit that
        // makes their length access eligible above.
        let ranges = IntegerRangeAnalysis::analyze_with_guarded_heap(
            graph,
            ir.blocks(),
            nodes,
            true,
            |id| accesses.get(id as usize).is_some_and(Option::is_some) || non_reentrant(id),
            work,
        );
        let mut hoists = Vec::new();
        for candidate in 0..candidates.len() {
            for natural_loop in loops.loops() {
                let Some(preheader) = natural_loop.preheader() else {
                    continue;
                };
                let mut lengths = accesses.iter().enumerate().filter_map(|(node, access)| {
                    let access = access.as_ref()?;
                    (access.candidate == candidate
                        && access.loop_header == natural_loop.header()
                        && access.access == ArrayAccess::Length)
                        .then_some((node as u32, access.result?))
                });
                let Some((length_node, bound)) = lengths.next() else {
                    continue;
                };
                // More than one observable length read does not identify one
                // exact bound for every load. Keep all checks in that case.
                if lengths.next().is_some() {
                    continue;
                }
                let mut covered_any = false;
                for (node, access) in accesses.iter_mut().enumerate() {
                    let Some(access) = access.as_mut() else {
                        continue;
                    };
                    if access.candidate != candidate
                        || access.loop_header != natural_loop.header()
                        || access.access != ArrayAccess::Load
                    {
                        continue;
                    }
                    let Some(index) = access.index else { continue };
                    if ranges.proves_in_bounds_at(index, bound, node as u32) {
                        access.requires_index_guard = false;
                        access.bounds_covered_by_hoist = true;
                        covered_any = true;
                    }
                }
                if covered_any {
                    hoists.push(ArrayLoopHoist {
                        candidate,
                        preheader,
                        header: natural_loop.header(),
                        bound,
                        length_node,
                    });
                }
            }
        }
        let mut invalidations = Vec::with_capacity(nodes.len());
        for node in nodes {
            let planned = accesses
                .get(node.id() as usize)
                .is_some_and(Option::is_some);
            let invalidates = match graph.effect_for_node(node.id()) {
                ScalarHeapEffect::Pure => false,
                // Planned operations have a lowering contract: loads retain
                // metadata only after all mode/index/storage guards succeed;
                // packed logical length never replaces the dense count.
                ScalarHeapEffect::Reentrant if planned => false,
                // Consumers may certify another emitted guarded fast path
                // (for example checked Int32 arithmetic). The slow/miss edge
                // deopts before returning to the cached path.
                ScalarHeapEffect::Reentrant if non_reentrant(node.id()) => false,
                // Local identity is part of the metadata-cache key. Even a
                // primitive/unowned move can rebind that local to a different
                // entry array, so every frame write is a provenance barrier.
                ScalarHeapEffect::FrameWrite(_) => true,
                // Even a non-reentrant store can grow/reallocate dense storage,
                // so writes and every other unknown boundary invalidate.
                _ => true,
            };
            invalidations.push(invalidates);
        }
        Self {
            candidates,
            accesses,
            hoists,
            invalidations,
        }
    }

    pub(super) fn candidates(&self) -> &[ArrayCandidate] {
        &self.candidates
    }
    pub(super) fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
    pub(super) fn access(&self, node: u32) -> Option<&ArrayAccessPlan> {
        self.accesses.get(node as usize)?.as_ref()
    }

    pub(super) fn guarded_leaf(&self, node: u32) -> bool {
        self.access(node).is_some_and(|access| match access.access {
            ArrayAccess::Load | ArrayAccess::Store => true,
            ArrayAccess::Length => self.hoists.iter().any(|hoist| hoist.length_node == node),
        })
    }
    pub(super) fn hoists(&self) -> &[ArrayLoopHoist] {
        &self.hoists
    }
    pub(super) fn invalidates_before(&self, node: u32) -> bool {
        self.invalidations
            .get(node as usize)
            .copied()
            .unwrap_or(true)
    }

    /// Conservative retained-plus-temporary allocation bound for compilation
    /// control. Work accounting independently bounds all quadratic scans.
    pub(super) fn bytes_upper_bound(ir: &OptimizedIr) -> usize {
        ir.nodes()
            .len()
            .saturating_mul(160)
            .saturating_add(Self::MAX_CANDIDATES.saturating_mul(64))
    }
}

fn spend(work: &mut usize, cost: usize) -> Option<()> {
    *work = work.checked_sub(cost)?;
    Some(())
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::{
        bytecode::VerifyLimits,
        ir::{OptimizedNodeKind, ScalarHeapOperation, ScalarNumericMode},
        runtime::{ArrayHazards, FeedbackSnapshot, FeedbackTable, FunctionKey},
        test_support::SnapshotFixture,
    };

    fn fixture(source: &str, mode: ArrayMode) -> (OptimizedIr, FeedbackSnapshot) {
        let compiled = SnapshotFixture::compile(source);
        let verified = compiled.snapshot().verify(VerifyLimits::default()).unwrap();
        let numeric_modes = verified
            .instructions()
            .iter()
            .filter(|instruction| {
                matches!(
                    instruction.opcode().name(),
                    "add"
                        | "sub"
                        | "mul"
                        | "div"
                        | "or"
                        | "lt"
                        | "lte"
                        | "gt"
                        | "gte"
                        | "inc"
                        | "dec"
                        | "post_inc"
                        | "post_dec"
                        | "inc_loc"
                        | "dec_loc"
                )
            })
            .map(|instruction| (instruction.pc(), ScalarNumericMode::Int32))
            .collect();
        let ir = OptimizedIr::translate_with_numeric_modes(&verified, 900, &numeric_modes).unwrap();
        let mut feedback = FeedbackTable::new(100, 3);
        for node in ir.nodes() {
            let access = match ir.scalar_graph().heap_operation(node.id()) {
                Some(ScalarHeapOperation::GetElement { .. }) => ArrayAccess::Load,
                Some(ScalarHeapOperation::PutElement { .. }) => ArrayAccess::Store,
                Some(ScalarHeapOperation::GetProperty { atom, .. })
                    if *atom == rquickjs_core::qjs::JS_ATOM_length =>
                {
                    ArrayAccess::Length
                }
                _ => continue,
            };
            feedback.observe_array(
                FunctionKey::new(900, 1),
                node.pc(),
                access,
                mode,
                ArrayHazards::NONE,
            );
        }
        (ir, feedback.snapshot(1))
    }

    fn plan(
        ir: &OptimizedIr,
        feedback: &FeedbackSnapshot,
        typed_length: bool,
        work: usize,
    ) -> ArrayPlan {
        let facts = KnownFacts::analyze_with_preserved_frame_reads(
            ir.scalar_graph(),
            ir.nodes(),
            |id| {
                ir.scalar_graph().heap_operation(id).is_some()
                    || matches!(ir.nodes()[id as usize].kind(),
                        OptimizedNodeKind::Bytecode { opcode } if opcode.as_ref() == "to_propkey")
                    || matches!(
                        ir.nodes()[id as usize].kind(),
                        OptimizedNodeKind::GuardNumeric { .. }
                    )
            },
            100_000,
        );
        let loops = LoopAnalysis::analyze(ir, 100_000);
        assert!(loops.is_complete());
        ArrayPlan::analyze(
            ir,
            &facts,
            &loops,
            feedback.arrays().len(),
            |pc| feedback.array_at(FunctionKey::new(900, 1), pc).copied(),
            typed_length,
            |id| {
                matches!(ir.nodes()[id as usize].kind(),
                OptimizedNodeKind::Bytecode { opcode } if opcode.as_ref() == "to_propkey")
            },
            work,
        )
    }

    #[test]
    fn typed_stores_publish_leaf_capabilities_but_packed_stores_do_not() {
        for mode in [ArrayMode::Int32, ArrayMode::Float64, ArrayMode::Packed] {
            let (ir, feedback) = fixture("(function(a,i,v){a[i]=v;return a[i]})", mode);
            let plan = plan(&ir, &feedback, true, 100_000);
            let store = ir
                .nodes()
                .iter()
                .find(|node| {
                    matches!(
                        ir.scalar_graph().heap_operation(node.id()),
                        Some(ScalarHeapOperation::PutElement { .. })
                    )
                })
                .unwrap();
            assert_eq!(plan.guarded_leaf(store.id()), mode != ArrayMode::Packed);
            if mode != ArrayMode::Packed {
                assert!(!plan.invalidates_before(store.id()));
                assert!(plan.access(store.id()).unwrap().requires_index_guard);
                assert!(ir
                    .nodes()
                    .iter()
                    .filter(|node| matches!(
                        ir.scalar_graph().heap_operation(node.id()),
                        Some(ScalarHeapOperation::GetElement { .. })
                    ))
                    .all(|node| plan.guarded_leaf(node.id())));
            }
        }
    }

    #[test]
    fn nested_counted_loop_uses_one_argument_identity_and_separate_length_access() {
        let (ir, feedback) = fixture("(function(a,n){let b=a,s=0;for(let j=0;j<n;j++){for(let i=0;i<b.length;i++){s+=b[i]}}return s})", ArrayMode::Packed);
        let plan = plan(&ir, &feedback, false, 100_000);
        assert_eq!(
            plan.candidates(),
            &[ArrayCandidate {
                argument: 0,
                mode: ArrayMode::Packed,
                needs_length: true
            }]
        );
        let accesses: Vec<_> = ir
            .nodes()
            .iter()
            .filter_map(|n| plan.access(n.id()))
            .collect();
        assert_eq!(accesses.len(), 2);
        assert!(accesses
            .iter()
            .any(|a| a.access == ArrayAccess::Length && a.index.is_none()));
        assert!(accesses
            .iter()
            .any(|a| a.access == ArrayAccess::Load && a.index.is_some()));
        assert!(accesses
            .iter()
            .filter(|access| access.access == ArrayAccess::Load)
            .all(|access| access.requires_index_guard ^ access.bounds_covered_by_hoist));
        for node in ir.nodes() {
            if matches!(
                node.kind(),
                OptimizedNodeKind::GuardNumeric { mid_loop: true, .. }
            ) {
                assert!(plan.invalidates_before(node.id()));
            }
        }
    }

    #[test]
    fn unknown_reentry_and_changed_entry_argument_do_not_retain_receiver_identity() {
        let source = "(function(a,n,f){for(let i=0;i<n;i++){f.length;a[i]}return 0})";
        let compiled = SnapshotFixture::compile(source);
        let verified = compiled.snapshot().verify(VerifyLimits::default()).unwrap();
        let ir = OptimizedIr::translate(&verified, 900).unwrap();
        let mut feedback = FeedbackTable::new(100, 3);
        for node in ir.nodes() {
            if matches!(
                ir.scalar_graph().heap_operation(node.id()),
                Some(ScalarHeapOperation::GetElement { .. })
            ) {
                feedback.observe_array(
                    FunctionKey::new(900, 1),
                    node.pc(),
                    ArrayAccess::Load,
                    ArrayMode::Int32,
                    ArrayHazards::NONE,
                );
            }
        }
        assert!(plan(&ir, &feedback.snapshot(1), false, 100_000)
            .candidates()
            .is_empty());

        let source = "(function(a,n){for(let i=0;i<n;i++){a=i;a[i]}return 0})";
        let (ir, feedback) = fixture(source, ArrayMode::Int32);
        assert!(plan(&ir, &feedback, false, 100_000).candidates().is_empty());
    }

    #[test]
    fn typed_length_requires_lookup_capability_and_budget_exhaustion_is_empty() {
        let (ir, feedback) = fixture(
            "(function(a){let s=0;for(let i=0;i<a.length;i++)s+=a[i];return s})",
            ArrayMode::Float64,
        );
        let without = plan(&ir, &feedback, false, 100_000);
        assert!(ir.nodes().iter().all(|n| without
            .access(n.id())
            .is_none_or(|a| a.access != ArrayAccess::Length)));
        let with = plan(&ir, &feedback, true, 100_000);
        assert_eq!(with.candidates().len(), 1);
        assert!(with.candidates()[0].needs_length);
        let [hoist] = with.hoists() else {
            panic!("an intrinsic typed-length guard must authorize one loop hoist: {with:#?}");
        };
        assert_ne!(hoist.preheader, hoist.header);
        assert!(ir
            .nodes()
            .iter()
            .filter_map(|node| with.access(node.id()))
            .filter(|access| access.access == ArrayAccess::Load)
            .all(|access| access.bounds_covered_by_hoist && !access.requires_index_guard));
        for work in [0, 1, 16] {
            assert!(plan(&ir, &feedback, true, work).candidates().is_empty());
        }
    }

    #[test]
    fn packed_counted_loop_records_preheader_and_covers_only_proven_load_bounds() {
        let (ir, feedback) = fixture(
            "(function(a){let n=a.length,s=0;for(let i=0;i<n;i++)s=(s+a[i])|0;return s})",
            ArrayMode::Packed,
        );
        let plan = plan(&ir, &feedback, false, 100_000);
        let [hoist] = plan.hoists() else {
            panic!("canonical packed traversal must have one legal metadata hoist: {plan:#?}");
        };
        assert_ne!(hoist.preheader, hoist.header);
        let length = plan.access(hoist.length_node).unwrap();
        assert_eq!(length.access, ArrayAccess::Length);
        assert_eq!(length.result, Some(hoist.bound));
        let covered = ir
            .nodes()
            .iter()
            .filter_map(|node| plan.access(node.id()))
            .filter(|access| access.access == ArrayAccess::Load)
            .collect::<Vec<_>>();
        assert_eq!(covered.len(), 1);
        assert!(covered[0].bounds_covered_by_hoist);
        assert!(!covered[0].requires_index_guard);
    }

    #[test]
    fn wrong_range_shape_never_publishes_a_hoist() {
        for source in [
            "(function(a){let n=a.length,s=0;for(let i=-1;i<n;i++)s+=a[i];return s})",
            "(function(a){let n=a.length,s=0;for(let i=0;i>=n;i++)s+=a[i];return s})",
            "(function(a){let n=a.length,s=0;for(let i=0;i<n;i=(i+1)|0)s+=a[i];return s})",
            "(function(a,f){let n=a.length;f();let s=0;for(let i=0;i<n;i++)s+=a[i];return s})",
        ] {
            let (ir, feedback) = fixture(source, ArrayMode::Packed);
            let plan = plan(&ir, &feedback, false, 100_000);
            assert!(plan.hoists().is_empty(), "{source}");
            assert!(
                ir.nodes().iter().all(|node| {
                    plan.access(node.id()).is_none_or(|access| {
                        access.access != ArrayAccess::Length || !plan.guarded_leaf(node.id())
                    })
                }),
                "an unhoisted observable length became a leaf capability: {source}"
            );
        }
    }

    #[test]
    fn alias_writes_and_unknown_reentry_never_publish_stale_bounds() {
        // These are the two ways a loop-local metadata tuple can become stale:
        // an alias mutates dense storage directly, or arbitrary script does so
        // across a reentrant boundary.  As in JSC's clobberize model, either
        // effect must prevent the preheader guard from being consumed at all.
        for source in [
            "(function(a){let n=a.length,s=0,b=a;for(let i=0;i<n;i++){if(i===0)b.length=1;s=(s+(a[i]|0))|0}return s})",
            "(function(a,f){let n=a.length,s=0;for(let i=0;i<n;i++){if(i===0)f(a);s=(s+(a[i]|0))|0}return s})",
            "(function(a){let n=a.length;a.length=1;let s=0;for(let i=0;i<n;i++)s=(s+(a[i]|0))|0;return s})",
            "(function(a,f){let n=a.length;f(a);let s=0;for(let i=0;i<n;i++)s=(s+(a[i]|0))|0;return s})",
        ] {
            let (ir, feedback) = fixture(source, ArrayMode::Packed);
            let plan = plan(&ir, &feedback, false, 100_000);
            assert!(
                plan.hoists().is_empty(),
                "effectful loop published stale packed metadata: {source}; {plan:#?}"
            );
            assert!(
                ir.nodes()
                    .iter()
                    .filter_map(|node| plan.access(node.id()))
                    .filter(|access| access.access == ArrayAccess::Load)
                    .all(|access| access.requires_index_guard
                        && !access.bounds_covered_by_hoist),
                "effectful loop removed a per-access bounds guard: {source}; {plan:#?}"
            );
        }
    }
}
