// SPDX-License-Identifier: GPL-3.0-only
//! Scenario contract v0.3: the `profiles` root and the `dist` functions load,
//! check, and render through the public API.

use std::path::Path;

use fps_flow::function::NoFunctions;
use fps_flow::program::Op;
use fps_flow::template::{Filler, RenderCtx};
use fps_flow::{LoadError, Scenario, Template, Vars};
use rand_pcg::Pcg64Mcg;

const PAGE: &str = r#"
scenario: profiled
vars: { port: 8080 }
profiles:
  chrome:
    User-Agent: "Mozilla/5.0 (X11; Linux x86_64)"
    Accept-Language: en-US,en;q=0.9
    Upgrade-Insecure-Requests: 1
    DNT: true
flows:
  - id: page
    module: m
    client:
      - connect: { port: "{{ vars.port }}" }
      - send: "GET / HTTP/1.1\r\n{{ profiles.chrome }}\r\n{{ fill(dist.uniform(4, 4)) }}"
      - delay: { ms: "{{ dist.exponential(20.0, cap=30) }}" }
      - close: fin
    server:
      - accept: { port: "{{ vars.port }}" }
"#;

fn render(t: &Template, seed: u128) -> Vec<u8> {
    let (vars, overlay) = (Vars::new(), Vars::new());
    let filler = Filler::new(1);
    let mut rng = Pcg64Mcg::new(seed);
    let mut none = NoFunctions;
    let mut ctx = RenderCtx {
        vars: &vars,
        overlay: &overlay,
        session_id: 1,
        flow: "page",
        last: &[],
        rng: &mut rng,
        filler: &filler,
        functions: &mut none,
        clock: &fps_flow::SystemClock,
    };
    t.render_to_vec(&mut ctx).expect("render")
}

#[test]
fn the_specification_example_parses() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples/modules/http1/scenarios-pending/web_profile.yaml");
    let s = Scenario::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(s.name, "web_profile");
    let dump = s.dump();
    // The profile is folded into the text of the call argument.
    assert!(dump.contains("headers = text"), "{dump}");
    assert!(
        dump.contains(r"User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36\r\nAccept-Language: en-US,en;q=0.9\r\nAccept-Encoding: gzip, deflate\r\nUpgrade-Insecure-Requests: 1\r\n"),
        "{dump}"
    );
    assert!(dump.contains("dist.pareto"), "{dump}");
    assert!(dump.contains("dist.lognormal"), "{dump}");
    let warnings = Scenario::check_file(&path, None).unwrap_or_else(|e| panic!("{e}"));
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut s = s;
    s.fold_constants().expect("fold");
}

#[test]
fn a_loaded_scenario_renders_profiles_and_draws() {
    let s = Scenario::parse(PAGE).unwrap_or_else(|e| panic!("{e}"));
    let ops = &s.flows[0].client.ops;
    let Op::Send(send) = &ops[0] else {
        panic!("{:?}", ops[0])
    };
    let bytes = render(send, 7);
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        text.starts_with(concat!(
            "GET / HTTP/1.1\r\n",
            "User-Agent: Mozilla/5.0 (X11; Linux x86_64)\r\n",
            "Accept-Language: en-US,en;q=0.9\r\n",
            "Upgrade-Insecure-Requests: 1\r\n",
            "DNT: true\r\n",
            "\r\n"
        )),
        "{text:?}"
    );
    assert_eq!(text.len(), text.find("\r\n\r\n").unwrap() + 4 + 4);
    assert_eq!(bytes, render(send, 7));
    let Op::Delay(fps_flow::program::Delay::Fixed(ms)) = &ops[1] else {
        panic!("{:?}", ops[1])
    };
    let draws: Vec<i64> = (0..100)
        .map(|seed| {
            String::from_utf8(render(ms, seed))
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert!(draws.iter().all(|d| (0..=30).contains(d)));
    assert!(draws.contains(&30), "the cap should bind sometimes");
}

fn error_of(src: &str) -> String {
    match Scenario::parse(src) {
        Ok(_) => panic!("accepted:\n{src}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn the_loader_rejects_an_undeclared_profile() {
    let src = PAGE.replace("profiles.chrome", "profiles.firefox");
    let e = error_of(&src);
    assert!(e.contains("undefined profile 'firefox'"), "{e}");
    let e = error_of(
        &PAGE
            .replace("profiles:", "unused:")
            .replace("profiles.chrome", "profiles.x"),
    );
    assert!(e.contains("undefined profile 'x'"), "{e}");
}

#[test]
fn the_loader_rejects_bad_dist_calls() {
    for (call, needle) in [
        ("dist.exponential(0)", "must be positive"),
        ("dist.uniform(5, 1)", "low greater than high"),
        ("dist.zipf(1)", "unknown function 'dist.zipf'"),
        ("dist.pareto(1.2)", "misses required parameter 'scale'"),
    ] {
        let src = PAGE.replace("dist.uniform(4, 4)", call);
        let e = error_of(&src);
        assert!(e.contains(needle), "{call}: {e}");
    }
}

#[test]
fn the_loader_rejects_malformed_profiles() {
    for (entry, needle) in [
        ("X-Bad: \"a\\r\\nInjected: 1\"", "line break"),
        ("\"bad name\": 1", "bad entry name"),
        ("-lead: 1", "bad entry name"),
        ("nested: { a: 1 }", "unsupported value"),
    ] {
        let src = PAGE.replace("    DNT: true", &format!("    {entry}"));
        let e = error_of(&src);
        assert!(e.contains(needle), "{entry}: {e}");
    }
    let src = PAGE.replace(
        "  chrome:\n    User-Agent: \"Mozilla/5.0 (X11; Linux x86_64)\"\n    Accept-Language: en-US,en;q=0.9\n    Upgrade-Insecure-Requests: 1\n    DNT: true\n",
        "  chrome: {}\n",
    );
    assert!(error_of(&src).contains("at least one entry"));
}

#[test]
fn check_accepts_the_same_scenario() {
    let warnings = Scenario::check(PAGE, None).unwrap_or_else(|e: LoadError| panic!("{e}"));
    assert!(warnings.is_empty());
}
