use rquickjs::{Context, Runtime};
use rquickjs_core::runtime::JitBackend;
use rquickjs_jit::bytecode::SlotKind;
use rquickjs_jit::runtime::{FunctionKey, OsrKey, OsrMap};
use rquickjs_jit::test_support::SnapshotFixture;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[test]
fn osr_map_retains_exact_function_pc_depth_and_slot_kinds() {
    let key = OsrKey::new(FunctionKey::new(9, 3), 17);
    let map = OsrMap::new(key, 41, 2, vec![SlotKind::Tagged, SlotKind::Int32]);
    assert_eq!(map.key(), key);
    assert_eq!(map.entry_offset(), 41);
    assert_eq!(map.stack_depth(), 2);
    assert_eq!(map.live_slots(), [SlotKind::Tagged, SlotKind::Int32]);
    assert!(map.matches(
        FunctionKey::new(9, 3),
        17,
        2,
        &[SlotKind::Tagged, SlotKind::Int32]
    ));
    assert!(!map.matches(
        FunctionKey::new(9, 4),
        17,
        2,
        &[SlotKind::Tagged, SlotKind::Int32]
    ));
    assert!(!map.matches(
        FunctionKey::new(9, 3),
        18,
        2,
        &[SlotKind::Tagged, SlotKind::Int32]
    ));
    assert!(!map.matches(
        FunctionKey::new(9, 3),
        17,
        1,
        &[SlotKind::Tagged, SlotKind::Int32]
    ));
}

#[test]
fn verifier_derives_osr_maps_for_each_real_loop_header() {
    let fixture = SnapshotFixture::compile(
        "(function f(n, zero){let s=zero;for(let i=zero;i<n;i++)s+=i;return s})",
    );
    let verified = fixture.snapshot().verify(Default::default()).unwrap();
    assert!(!verified.osr_points().is_empty());
    for point in verified.osr_points() {
        assert!(verified.control_flow_graph().is_loop_header(point.pc()));
        assert!(
            point.live_slots().len()
                >= verified.snapshot().arg_count() as usize
                    + verified.snapshot().local_count() as usize
        );
    }
}

#[test]
fn osr_maps_are_built_only_from_verified_headers_and_independent_entries() {
    let fixture = SnapshotFixture::compile(
        "(function f(n, zero){let s=zero;for(let i=zero;i<n;i++)s+=i;return s})",
    );
    let verified = fixture.snapshot().verify(Default::default()).unwrap();
    let point = &verified.osr_points()[0];
    let map = OsrMap::from_verified(&verified, point.pc(), 123).unwrap();
    assert_eq!(map.entry_offset(), 123);
    assert_eq!(map.key().pc(), point.pc());
    assert_eq!(map.live_slots(), point.live_slots());
    assert!(OsrMap::from_verified(&verified, point.pc().saturating_add(1), 123).is_none());
    assert!(
        OsrMap::from_verified(&verified, point.pc(), 0).is_none(),
        "function entry may not masquerade as OSR"
    );
}

#[derive(Default)]
struct EventState {
    calls: HashMap<(u64, u64), u32>,
    loops: u32,
    snapshots: u32,
    loop_pcs: Vec<u32>,
    hotness: HashMap<(u64, u64), rquickjs_jit::runtime::HotnessState>,
}

struct EventBackend(Arc<Mutex<EventState>>);

unsafe impl JitBackend for EventBackend {
    fn record_hot(&mut self, event: &rquickjs_core::qjs::JSJitHotEvent) -> u32 {
        let mut state = self.0.lock().unwrap();
        let key = (event.function.id, event.function.generation);
        if event.kind == rquickjs_core::qjs::JSJitHotKind_JS_JIT_HOT_CALL {
            *state.calls.entry(key).or_default() = state
                .calls
                .get(&key)
                .copied()
                .unwrap_or(0)
                .saturating_add(event.count);
            return u32::from(matches!(
                state
                    .hotness
                    .entry(key)
                    .or_default()
                    .record_call_event(event.count),
                rquickjs_jit::runtime::HotDecision::Queue(_)
            ));
        }
        state.loops = state.loops.saturating_add(event.count);
        state.loop_pcs.push(event.pc);
        u32::from(matches!(
            state
                .hotness
                .entry(key)
                .or_default()
                .record_loop_event(event.count),
            rquickjs_jit::runtime::HotDecision::Queue(_)
        ))
    }

    fn submit_snapshot(&mut self, snapshot: *mut rquickjs_core::qjs::JSJitFunctionSnapshot) {
        self.0.lock().unwrap().snapshots += 1;
        unsafe { rquickjs_core::qjs::JS_JitFreeSnapshot(snapshot) };
    }
}

#[test]
fn taken_backedges_report_after_interrupt_and_request_one_snapshot() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let state = Arc::new(Mutex::new(EventState::default()));
    let _guard = runtime
        .attach_jit_backend(EventBackend(Arc::clone(&state)))
        .unwrap();
    context.with(|ctx| {
        assert_eq!(
            ctx.eval::<i32, _>("function f(){let s=0;for(let i=0;i<80;i++)s+=i;return s} f()")
                .unwrap(),
            3160
        );
    });
    let state = state.lock().unwrap();
    assert!(state.calls.values().any(|calls| *calls == 1));
    assert!(state.loops >= 56);
    assert_eq!(state.snapshots, 1);
    assert!(state.loop_pcs.iter().all(|pc| *pc != 0));
}

#[test]
fn untaken_loop_reports_no_backedge() {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    let state = Arc::new(Mutex::new(EventState::default()));
    let _guard = runtime
        .attach_jit_backend(EventBackend(Arc::clone(&state)))
        .unwrap();
    context.with(|ctx| {
        ctx.eval::<(), _>("function f(){for(let i=0;i<0;i++){} } f()")
            .unwrap()
    });
    assert_eq!(state.lock().unwrap().loops, 0);
}
