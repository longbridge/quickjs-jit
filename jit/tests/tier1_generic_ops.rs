#![cfg(all(
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(all(target_os = "windows", target_arch = "aarch64"))
))]

//! Exact-semantics coverage for the `GENERIC_OP` helper family and the stack
//! shuffles admitted with it. Every run compares a forced Tier 1 execution
//! against the interpreter, including language exceptions, and executes the
//! helper under stress GC.

use rquickjs_jit::bytecode::{FallbackReason, HelperId};
use rquickjs_jit::test_support::{assert_tier1_rejected, differential};

const TRY: &str = "(()=>{try{return (";
const CATCH: &str = ")}catch(e){return e.name+':'+e.message}})()";

fn guarded(expression: &str) -> String {
    format!("{TRY}{expression}{CATCH}")
}

fn generic(definition: &str, expression: &str, opcode: &str) {
    differential(definition, expression)
        .force_baseline()
        .stress_gc()
        .expect_executed_opcode(opcode)
        .expect_helper(HelperId::GenericOp)
        .assert_same();
}

#[test]
fn push_this_boxes_sloppy_primitives_and_keeps_strict_receivers() {
    generic(
        "function f(o){ return typeof this + ':' + this.x + ':' + o }",
        "[f.call({x:40},2), f.call(7,1), f.call(undefined,1), f.call(null,1), f.call('s',1), f.call(true,1)]",
        "push_this",
    );
    generic(
        "function f(o){ 'use strict'; return typeof this + ':' + o }",
        "[f.call({x:40},2), f.call(7,1), f.call(undefined,1), f.call(null,1), f.call('s',1)]",
        "push_this",
    );
    generic(
        "function f(o){ return this.x + o }",
        &format!(
            "[{}, {}]",
            guarded("f.call({get x(){throw new RangeError('getter')}},1)"),
            guarded("f.call({x:1},2)")
        ),
        "push_this",
    );
}

#[test]
fn push_this_observes_method_constructor_and_recursive_receivers() {
    generic(
        "function f(n){ if(n>0) return this.tag + f.call({tag:n}, n-1); return this.tag }",
        "f.call({tag:'root'}, 3)",
        "push_this",
    );
    generic(
        "function f(v){ this.v = v; return this }",
        "[new f(4).v, new f('x').v, f.call({}, 5).v]",
        "push_this",
    );
}

#[test]
fn unmapped_arguments_new_target_this_function_and_null_proto_objects() {
    generic(
        "function f(){ 'use strict'; let a = arguments; a[0] = 9; return a.length + ':' + a[0] + ':' + Array.isArray(a) }",
        "[f(), f(1), f(1,2,3), f(...Array(20).fill(0))]",
        "special_object",
    );
    generic(
        "function f(a, b){ 'use strict'; arguments[0] = 'changed'; return a + ':' + arguments[0] + ':' + arguments.length }",
        "[f(1), f(1,2), f()]",
        "special_object",
    );
    generic(
        "function f(){ return new.target === undefined ? 'call' : new.target.name }",
        "[f(), new f() instanceof f, Reflect.construct(f, [], Array) instanceof Array]",
        "special_object",
    );
    generic(
        "var f = function g(n){ return n > 0 ? g(n - 1) + 1 : typeof g }",
        "[f(0), f(3)]",
        "special_object",
    );
    generic(
        "function f(v){ return {__proto__: null, v} }",
        "[Object.getPrototypeOf(f(1)), f(2).v, 'toString' in f(3)]",
        "special_object",
    );
}

#[test]
fn sloppy_mapped_arguments_stay_in_the_interpreter() {
    assert_tier1_rejected(
        "function f(a){ arguments[0] = 2; return a }",
        "f(1)",
        FallbackReason::ExtendedFrame,
    );
}

#[test]
fn global_reads_writes_and_deletes_match_the_interpreter() {
    generic(
        "function f(){ return typeof missingGlobal + ':' + typeof Math + ':' + typeof f }",
        "f()",
        "get_var_undef",
    );
    generic(
        "globalThis.writableGlobal = 0; function f(v){ writableGlobal = v; return writableGlobal }",
        "[f(1), f('x'), f({a:1}).a, writableGlobal.a]",
        "put_var",
    );
    generic(
        "Object.defineProperty(globalThis, 'frozenGlobal', {value: 1, writable: false}); \
         function f(v){ 'use strict'; frozenGlobal = v; return frozenGlobal }",
        &guarded("f(2)"),
        "put_var",
    );
    generic(
        "function f(v){ 'use strict'; undeclaredStrictGlobal = v; return v }",
        &guarded("f(2)"),
        "put_var",
    );
    generic(
        "globalThis.deletable = 1; Object.defineProperty(globalThis, 'pinned', {value: 1}); \
         function f(){ return [delete deletable, delete pinned, delete neverDefined] }",
        "[f(), typeof deletable, typeof pinned]",
        "delete_var",
    );
}

#[test]
fn typeof_family_matches_every_tag() {
    let values = "[1, 1.5, -0, NaN, 's', '', null, undefined, {}, [], f, class {}, 1n, 2n**80n, Symbol(), true, new Proxy(function(){}, {}), new Proxy({}, {})]";
    generic(
        "function f(x){ return typeof x }",
        &format!("{values}.map(f)"),
        "typeof",
    );
    generic(
        "function f(x){ return typeof x === 'undefined' }",
        &format!("{values}.map(f)"),
        "typeof_is_undefined",
    );
    generic(
        "function f(x){ return typeof x === 'function' }",
        &format!("{values}.map(f)"),
        "typeof_is_function",
    );
    generic(
        "function f(x){ let n = 0; for (let i = 0; i < 64; i++) { if (typeof x === 'undefined') n++; else n += 2 } return n }",
        "[f(), f(1), f('s'), f({})]",
        "typeof_is_undefined",
    );
}

#[test]
fn in_instanceof_and_delete_preserve_exceptions() {
    generic(
        "function f(k,o){ return k in o }",
        &format!(
            "[f('a',{{a:1}}), f('b',{{a:1}}), f(0,[1]), f(1,[1]), f(Symbol.iterator,[]), f('x', new Proxy({{}}, {{has(){{return true}}}})), {}, {}, {}]",
            guarded("f('a',1)"),
            guarded("f({toString(){throw new RangeError('key')}},{})"),
            guarded("f('a', new Proxy({}, {has(){throw new RangeError('trap')}}))"),
        ),
        "in",
    );
    generic(
        "function f(a,b){ return a instanceof b }",
        &format!(
            "[f([],Array), f({{}},Array), f(1,Number), f(Object.create(null),Object), f(new (class A {{}}), Object), {}, {}, {}]",
            guarded("f(1,1)"),
            guarded("f({}, {[Symbol.hasInstance](){throw new RangeError('has')}})"),
            guarded("f({}, () => 1)"),
        ),
        "instanceof",
    );
    generic(
        "function f(o,k){ return delete o[k] }",
        &format!(
            "[f({{a:1}},'a'), f(Object.freeze({{a:1}}),'a'), f([1,2],0), f({{}}, 'missing'), f('str', 0), {}]",
            guarded("f(null,'a')"),
        ),
        "delete",
    );
    generic(
        "function f(o,k){ 'use strict'; return delete o[k] }",
        &format!(
            "[f({{a:1}},'a'), {}, {}]",
            guarded("f(Object.freeze({a:1}),'a')"),
            guarded("f(new Proxy({}, {deleteProperty(){throw new RangeError('trap')}}),'a')"),
        ),
        "delete",
    );
}

#[test]
fn object_destructuring_and_compound_element_updates() {
    generic(
        "function f(o){ const {a, b} = o; return a + ':' + b }",
        &format!(
            "[f({{a:1,b:2}}), f('xy'), f(5), f(Object.create({{a:'proto'}})), {}, {}]",
            guarded("f(null)"),
            guarded("f(undefined)"),
        ),
        "to_object",
    );
    generic(
        "function f(o,k){ o[k] += 1; return o[k] }",
        &format!(
            "[f({{a:1}},'a'), f([5],0), f({{}}, {{toString(){{return 'z'}}}}), f({{}}, Symbol.iterator), f([1], 0.5), {}, {}, {}]",
            guarded("f(null,'a')"),
            guarded("f(undefined,{toString(){throw new RangeError('never')}})"),
            guarded("f({}, {toString(){throw new RangeError('key')}})"),
        ),
        "to_propkey2",
    );
}

#[test]
fn exponent_uses_the_exact_arithmetic_slow_path() {
    differential(
        "function f(a,b){ return a ** b }",
        &format!(
            "[f(2,10), f(2,0.5), f(-8,1/3), f(2,-1074), String(f(2n,3n)), f('3',2), f({{valueOf(){{return 3}}}},2), {}, {}]",
            guarded("f(2n,1)"),
            guarded("f({valueOf(){throw new RangeError('v')}},2)"),
        ),
    )
    .force_baseline()
    .stress_gc()
    .expect_executed_opcode("pow")
    .expect_helper(HelperId::BinaryArithSlow)
    .assert_same();
}

#[test]
fn destructuring_stack_shuffles_run_natively() {
    for (definition, expression, opcode) in [
        (
            "function f(s,k){ let x; ({[k]: x} = s); return x }",
            "[f({a:1},'a'), f({},'b'), f([7,8],1)]",
            "dup1",
        ),
        (
            "function f(o,k){ o[k] *= 2; return o[k] }",
            "[f({a:21},'a'), f([5],0), f({m:1.5}, 'm')]",
            "dup2",
        ),
        (
            "function f(a,i){ let v = a[i]++; return v + ':' + a[i] }",
            "[f([1,2],0), f({x:5},'x'), f([0,0,0,-1], 3)]",
            "perm4",
        ),
        (
            "function f(s,k,o){ ({[k]: o.p} = s); return o.p }",
            "[f({a:1},'a',{}), f({},'b',{p:3})]",
            "rot3r",
        ),
        (
            "function f(s,k,a,i){ ({[k]: a[i]} = s); return a[i] }",
            "[f({a:1},'a',[],0), f({b:2},'b',{},'x')]",
            "swap2",
        ),
        (
            "function f(s,a,i){ ({p: a[i]} = s); return a[i] }",
            "[f({p:4},[],1), f({},{q:1},'q')]",
            "rot3l",
        ),
    ] {
        differential(definition, expression)
            .force_baseline()
            .stress_gc()
            .expect_executed_opcode(opcode)
            .assert_same();
    }
}

#[test]
fn method_heavy_loop_with_this_typeof_and_in_stays_native() {
    generic(
        "function f(items){ let total = 0; for (let i = 0; i < items.length; i++) { \
           const item = items[i]; \
           if (typeof item === 'undefined') continue; \
           if ('w' in item && item instanceof Object) total += item.w ** 2; \
           else if (typeof item.v === 'number') total += item.v; \
         } if (this === undefined) return -total; return total }",
        "[f.call({}, [{w:2},{v:3},undefined,{v:'x'},{w:1}]), f([{w:3}])]",
        "typeof_is_undefined",
    );
}
