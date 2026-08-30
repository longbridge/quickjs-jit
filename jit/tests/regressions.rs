use rquickjs::{Context, Runtime};
use rquickjs_jit::correctness::canonical_plain_data_observation_source;
use serde::Deserialize;

#[derive(Deserialize)]
struct Regression {
    seed: u64,
    source: String,
    invocation: String,
    expected: String,
    required_tier: String,
}

#[test]
fn replay_checked_in_regressions() {
    let source = include_str!("regressions/cases/special-values.json");
    let case: Regression = serde_json::from_str(source).unwrap();
    assert_eq!(case.seed, 0);
    assert_eq!(case.required_tier, "interpreter-or-automatic");
    let runtime = Runtime::new().unwrap();
    let observation = Context::full(&runtime).unwrap().with(|ctx| {
        ctx.eval::<(), _>(case.source.as_str()).unwrap();
        ctx.eval::<String, _>(canonical_plain_data_observation_source(&case.invocation))
            .unwrap()
    });
    assert_eq!(observation, case.expected);
}
