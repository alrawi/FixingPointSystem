//! End to end runs of the example scenarios over loopback TCP.

use std::path::PathBuf;
use std::time::Duration;

use fps_spike_engine::engine::{run, BenchConfig, Mode};
use fps_spike_engine::program::Scenario;
use fps_spike_engine::template::Value;

fn scenario(name: &str, port: i64) -> Scenario {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples/modules/http1/scenarios")
        .join(name);
    let mut s = Scenario::load(&path).unwrap();
    s.override_vars(&[("server_port".into(), Value::Int(port))])
        .unwrap();
    s
}

fn cfg(mode: Mode, concurrency: usize) -> BenchConfig {
    BenchConfig {
        mode,
        concurrency,
        duration: Duration::from_secs(2),
        ..BenchConfig::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn web_get_interp_has_no_failures() {
    let r = run(scenario("web_get.yaml", 18080), cfg(Mode::Interp, 8), 2)
        .await
        .unwrap();
    assert!(r.sessions_ok > 0, "{r:?}");
    assert_eq!(r.sessions_failed, 0, "{r:?}");
    assert_eq!(r.server_sessions_failed, 0, "{r:?}");
    // 4 requests of 4,096 byte bodies per session, plus headers.
    let per_session = r.l7_bits_per_s * r.duration_s / 8.0 / r.sessions_ok as f64;
    assert!(
        per_session > 4.0 * 4096.0 && per_session < 4.0 * 4096.0 + 2048.0,
        "{per_session}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn web_get_baseline_has_no_failures() {
    let r = run(scenario("web_get.yaml", 18081), cfg(Mode::Baseline, 8), 2)
        .await
        .unwrap();
    assert!(r.sessions_ok > 0, "{r:?}");
    assert_eq!(r.sessions_failed, 0, "{r:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn web_browse_with_think_time() {
    let r = run(scenario("web_browse.yaml", 18082), cfg(Mode::Interp, 32), 2)
        .await
        .unwrap();
    assert!(r.sessions_ok > 0, "{r:?}");
    assert_eq!(r.sessions_failed, 0, "{r:?}");
    // The lognormal think time (median about 20 ms) dominates session latency.
    assert!(r.session_latency_us.p50 > 5_000, "{r:?}");
}
