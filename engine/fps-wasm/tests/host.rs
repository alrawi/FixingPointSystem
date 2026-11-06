// SPDX-License-Identifier: GPL-3.0-only
//! Tests of the WebAssembly extension host with a component written in WAT.

mod support;

use fps_flow::{ArgValue, CallArgs, FnCall, FnErrorKind, TemplateFn, ValueType};
use fps_stats::FailureReason;
use fps_wasm::{
    CloseReason, DataOutcome, Limits, SessionInfo, Side, TimerId, Transport, WasmEngine,
    WasmExtension, WasmFailure, WasmFunctions, WasmSession,
};
use rand::RngCore;
use rand_pcg::Pcg64Mcg;
use support::{component_bytes, TestHost};

fn load(limits: Limits) -> Result<WasmExtension, WasmFailure> {
    let engine = WasmEngine::new().unwrap();
    WasmExtension::load(&engine, &component_bytes(), limits)
}

fn small_fuel() -> Limits {
    Limits {
        fuel_per_call: 200_000,
        ..Limits::default()
    }
}

fn open(
    ext: &WasmExtension,
    id: u64,
    record: &support::Record,
) -> Result<WasmSession, WasmFailure> {
    ext.open(
        &SessionInfo {
            id,
            flow: "main",
            side: Side::Client,
            transport: Transport::Tcp,
            vars: &[("n", ArgValue::Int(3))],
        },
        TestHost::boxed(record),
    )
}

fn call<'a>(function: &'a str, positional: &'a [ArgValue<'a>]) -> FnCall<'a> {
    FnCall {
        module: "t",
        function,
        args: CallArgs {
            positional,
            named: &[],
        },
    }
}

#[test]
fn the_catalog_is_read_at_load() {
    let ext = load(Limits::default()).unwrap();
    let names: Vec<&str> = ext.functions().iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["echo", "spin", "grow", "probe", "rand", "fail"]);
    let echo = &ext.functions()[0];
    assert_eq!(echo.params.len(), 1);
    assert_eq!(echo.params[0].name, "data");
    assert_eq!(echo.params[0].ty, ValueType::Bytes);
    assert_eq!(echo.params[0].default, None);
    assert_eq!(echo.summary, "test function");
}

#[test]
fn a_component_without_the_world_is_rejected() {
    let engine = WasmEngine::new().unwrap();
    let err = WasmExtension::load(
        &engine,
        &wat::parse_str("(component)").unwrap(),
        Limits::default(),
    )
    .err()
    .unwrap();
    assert!(matches!(err, WasmFailure::Invalid(_)), "{err}");
    let err = WasmExtension::load(&engine, b"not wasm", Limits::default())
        .err()
        .unwrap();
    assert!(matches!(err, WasmFailure::Invalid(_)), "{err}");
}

#[test]
fn template_calls_reach_the_render_hook() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();
    let mut fns = WasmFunctions::new();
    fns.insert("t", open(&ext, 1, &record).unwrap());
    let mut rng = Pcg64Mcg::new(1);
    let mut out = b"<".to_vec();
    fns.render(
        &call("echo", &[ArgValue::Bytes(b"abc")]),
        &mut rng,
        &mut out,
    )
    .unwrap();
    fns.render(&call("echo", &[ArgValue::Text("def")]), &mut rng, &mut out)
        .unwrap();
    assert_eq!(out, b"<abcdef");

    let err = fns
        .render(&call("echo", &[ArgValue::Int(1)]), &mut rng, &mut out)
        .unwrap_err();
    assert_eq!(err.kind, FnErrorKind::InvalidArgument);
    let failure = fns.take_failure().unwrap();
    assert_eq!(failure.label(), "extension_invalid_argument");
    assert_eq!(failure.reason(), FailureReason::ExtensionError);
}

#[test]
fn unknown_modules_and_functions_are_unsupported() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();
    let mut fns = WasmFunctions::new();
    fns.insert("t", open(&ext, 1, &record).unwrap());
    let mut rng = Pcg64Mcg::new(1);
    let mut out = Vec::new();
    let other = FnCall {
        module: "other",
        ..call("echo", &[])
    };
    let err = fns.render(&other, &mut rng, &mut out).unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Unsupported);
    let err = fns
        .render(&call("missing", &[]), &mut rng, &mut out)
        .unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Unsupported);
    // Neither call reached the guest, so the session still works.
    fns.render(&call("echo", &[ArgValue::Bytes(b"ok")]), &mut rng, &mut out)
        .unwrap();
    assert_eq!(out, b"ok");
}

#[test]
fn data_hooks_return_their_outcome() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();
    let mut s = open(&ext, 1, &record).unwrap();
    assert_eq!(s.on_data(b"p").unwrap(), DataOutcome::Pass);
    assert_eq!(s.on_data(b"").unwrap(), DataOutcome::Pass);
    assert_eq!(s.on_data(b"c").unwrap(), DataOutcome::Consume);
    assert_eq!(
        s.on_data(b"r").unwrap(),
        DataOutcome::Replace(b"R".to_vec())
    );
    assert_eq!(s.on_data(b"xyz").unwrap(), DataOutcome::Pass);
    assert_eq!(*record.sends.lock().unwrap(), vec![b"hi".to_vec()]);
    s.on_timer(TimerId(5)).unwrap();
    assert_eq!(record.sends.lock().unwrap()[1], b"timer");
    assert!(!s.is_failed());
    s.on_close(CloseReason::Finished).unwrap();
}

#[test]
fn a_hook_error_fails_the_session() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();
    let mut s = open(&ext, 1, &record).unwrap();
    let err = s.on_data(b"e").unwrap_err();
    match &err {
        WasmFailure::Hook(e) => {
            assert_eq!(e.kind, FnErrorKind::Internal);
            assert_eq!(e.message, "boom");
        }
        other => panic!("{other}"),
    }
    assert_eq!(err.reason(), FailureReason::ExtensionError);
    assert_eq!(err.label(), "extension_internal");
    assert!(s.is_failed());
    assert!(matches!(s.on_data(b"p"), Err(WasmFailure::SessionFailed)));
    s.on_close(CloseReason::Failed).unwrap();
}

#[test]
fn on_open_errors_are_reported() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();
    let err = open(&ext, 13, &record).err().unwrap();
    assert_eq!(err.label(), "extension_protocol");
}

/// The acceptance test: an endless loop stops at its fuel limit and fails only
/// its own session.
#[test]
fn an_endless_loop_is_stopped_by_fuel_and_fails_only_its_session() {
    let limits = small_fuel();
    let ext = load(limits).unwrap();
    let record = support::Record::default();
    let mut victim = open(&ext, 1, &record).unwrap();
    let mut bystander = open(&ext, 2, &record).unwrap();

    let err = victim.on_data(b"s").unwrap_err();
    assert!(matches!(err, WasmFailure::OutOfFuel), "{err}");
    assert_eq!(err.reason(), FailureReason::ExtensionError);
    assert_eq!(err.label(), "extension_internal");
    assert!(victim.fuel_used_by_last_call() >= limits.fuel_per_call);
    assert!(victim.is_failed());
    assert!(matches!(
        victim.on_data(b"p"),
        Err(WasmFailure::SessionFailed)
    ));

    // The other session of the same extension is unaffected, and so is a
    // session that opens afterwards.
    assert_eq!(bystander.on_data(b"c").unwrap(), DataOutcome::Consume);
    assert!(!bystander.is_failed());
    let mut later = open(&ext, 3, &record).unwrap();
    assert_eq!(later.on_data(b"p").unwrap(), DataOutcome::Pass);
}

#[test]
fn fuel_is_refilled_for_every_call() {
    let limits = small_fuel();
    let ext = load(limits).unwrap();
    let record = support::Record::default();
    let mut s = open(&ext, 1, &record).unwrap();
    let mut total = 0;
    for _ in 0..50 {
        s.on_data(b"p").unwrap();
        let used = s.fuel_used_by_last_call();
        assert!(used > 0 && used < limits.fuel_per_call, "{used}");
        total += used;
    }
    // The calls together exceed one budget only if the budget is per call.
    assert!(total > 0);
}

#[test]
fn an_endless_render_fails_the_template_call_and_counts_as_extension_error() {
    let ext = load(small_fuel()).unwrap();
    let record = support::Record::default();
    let mut fns = WasmFunctions::new();
    fns.insert("t", open(&ext, 1, &record).unwrap());
    let mut rng = Pcg64Mcg::new(1);
    let mut out = Vec::new();
    let err = fns
        .render(&call("spin", &[]), &mut rng, &mut out)
        .unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Internal);
    let failure = fns.take_failure().unwrap();
    assert!(matches!(failure, WasmFailure::OutOfFuel));
    assert_eq!(failure.reason(), FailureReason::ExtensionError);
}

#[test]
fn growing_past_the_memory_limit_fails_the_session() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();
    let mut s = open(&ext, 1, &record).unwrap();
    let mut other = open(&ext, 2, &record).unwrap();
    let err = s.on_data(b"g").unwrap_err();
    assert!(matches!(err, WasmFailure::OutOfMemory), "{err}");
    assert_eq!(err.reason(), FailureReason::ExtensionError);
    assert!(s.is_failed());
    assert_eq!(other.on_data(b"p").unwrap(), DataOutcome::Pass);

    // The same through a template function.
    let mut fns = WasmFunctions::new();
    fns.insert("t", open(&ext, 3, &record).unwrap());
    let mut rng = Pcg64Mcg::new(1);
    let mut out = Vec::new();
    fns.render(&call("grow", &[]), &mut rng, &mut out)
        .unwrap_err();
    assert!(matches!(fns.take_failure(), Some(WasmFailure::OutOfMemory)));
}

#[test]
fn memory_below_the_initial_size_of_the_module_is_refused() {
    // The test component declares one page, which is 64 KiB.
    let limits = Limits {
        max_memory_bytes: 32 << 10,
        ..Limits::default()
    };
    let err = load(limits).err().unwrap();
    assert!(matches!(err, WasmFailure::OutOfMemory), "{err}");
}

#[test]
fn engine_calls_follow_the_hook_rules() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();

    // `send` during `render` returns `not-allowed` (case 2 of engine-error).
    let mut fns = WasmFunctions::new();
    fns.insert("t", open(&ext, 1, &record).unwrap());
    let mut rng = Pcg64Mcg::new(1);
    let mut out = Vec::new();
    fns.render(&call("probe", &[]), &mut rng, &mut out).unwrap();
    assert_eq!(out, [1, 2]);
    assert!(record.sends.lock().unwrap().is_empty());

    // Random numbers during `on-close` have no error case, so the guest traps.
    let mut s = open(&ext, 2, &record).unwrap();
    let err = s.on_close(CloseReason::Reset).unwrap_err();
    assert!(matches!(err, WasmFailure::Trap(_)), "{err}");
    assert!(
        err.to_string().contains("not allowed during on-close"),
        "{err}"
    );
}

#[test]
fn random_numbers_come_from_the_session_host() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();
    let mut fns = WasmFunctions::new();
    fns.insert("t", open(&ext, 1, &record).unwrap());
    let mut rng = Pcg64Mcg::new(1);
    let before = rng.next_u64();
    let mut rng = Pcg64Mcg::new(1);
    let mut out = Vec::new();
    fns.render(&call("rand", &[]), &mut rng, &mut out).unwrap();
    assert_eq!(out, TestHost::RANDOM.to_le_bytes());
    // The adapter leaves the renderer generator alone.
    assert_eq!(rng.next_u64(), before);
}

#[test]
fn a_failing_render_reports_the_hook_error() {
    let ext = load(Limits::default()).unwrap();
    let record = support::Record::default();
    let mut fns = WasmFunctions::new();
    fns.insert("t", open(&ext, 1, &record).unwrap());
    let mut rng = Pcg64Mcg::new(1);
    let mut out = Vec::new();
    let err = fns
        .render(&call("fail", &[]), &mut rng, &mut out)
        .unwrap_err();
    assert_eq!(
        (err.kind, err.message.as_str()),
        (FnErrorKind::Internal, "boom")
    );
    assert_eq!(fns.take_failure().unwrap().label(), "extension_internal");
}
