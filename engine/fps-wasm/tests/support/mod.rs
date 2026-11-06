// SPDX-License-Identifier: GPL-3.0-only
//! A test extension written as WAT text, and a recording host.
//!
//! The component implements the `extension` world of
//! `contracts/extension/fps-extension.wit` by hand, so the tests need no
//! WebAssembly toolchain. Its behavior depends on the first byte of the data,
//! the function name, or the timer id, which the table below lists.
//!
//! | Hook | Input | Behavior |
//! |---|---|---|
//! | `on-open` | session id 13 | returns a protocol error |
//! | `on-data` | `p` or empty | pass |
//! | | `c` | consume |
//! | | `r` | replace with `R` |
//! | | `s` | loops forever |
//! | | `g` | grows memory by 1000 pages |
//! | | `e` | returns an internal error |
//! | | `x` | calls `send("hi")`, then passes |
//! | `on-timer` | 5 | calls `send("timer")` |
//! | | 99 | loops forever |
//! | `on-close` | `reset` | calls `random-u64`, which the contract forbids there |
//! | `render` | `echo(bytes or text)` | returns the argument |
//! | | `spin()` | loops forever |
//! | | `grow()` | grows memory by 1000 pages |
//! | | `probe()` | calls `send` and returns the result as two bytes |
//! | | `rand()` | returns `random-u64` as 8 little endian bytes |
//! | | `fail()` | returns an internal error |

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use fps_wasm::{EngineError, Host, LogLevel, TimerId};

/// Static data of the test component, which starts at `BASE` in memory.
struct Image {
    bytes: Vec<u8>,
}

const BASE: u32 = 1024;

impl Image {
    fn here(&self) -> u32 {
        BASE + self.bytes.len() as u32
    }
    fn align(&mut self, a: usize) {
        let pad = (a - self.bytes.len() % a) % a;
        self.bytes.resize(self.bytes.len() + pad, 0);
    }
    fn str(&mut self, s: &str) -> u32 {
        let p = self.here();
        self.bytes.extend_from_slice(s.as_bytes());
        p
    }
    fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
}

/// Builds the WAT text of the test component.
pub fn component_wat() -> String {
    let mut img = Image { bytes: Vec::new() };
    let refused = img.str("refused");
    let boom = img.str("boom");
    let r = img.str("R");
    let hi = img.str("hi");
    let timer = img.str("timer");
    let probe = img.str("p");
    let names = ["echo", "spin", "grow", "probe", "rand", "fail"];
    let summary = img.str("test function");
    let name_ptrs: Vec<u32> = names.iter().map(|n| img.str(n)).collect();
    let param_name = img.str("data");
    img.align(8);
    // One param-desc: name, type `bytes` (case 4), no default. 40 bytes.
    let param = img.here();
    img.u32(param_name);
    img.u32(4);
    img.bytes.push(4);
    img.bytes.extend_from_slice(&[0u8; 31]);
    img.align(8);
    // Six function-desc records of 24 bytes: name, params, summary.
    let catalog = img.here();
    for (i, p) in name_ptrs.iter().enumerate() {
        img.u32(*p);
        img.u32(names[i].len() as u32);
        if i == 0 {
            img.u32(param);
            img.u32(1);
        } else {
            img.u32(0);
            img.u32(0);
        }
        img.u32(summary);
        img.u32(13);
    }
    let escaped: String = img.bytes.iter().map(|b| format!("\\{b:02x}")).collect();
    TEMPLATE
        .replace("@BASE@", &BASE.to_string())
        .replace("@IMAGE@", &escaped)
        .replace("@CATALOG@", &catalog.to_string())
        .replace("@REFUSED@", &refused.to_string())
        .replace("@BOOM@", &boom.to_string())
        .replace("@R@", &r.to_string())
        .replace("@HI@", &hi.to_string())
        .replace("@TIMER@", &timer.to_string())
        .replace("@PROBE@", &probe.to_string())
}

const TEMPLATE: &str = r#"
(module
  (import "fps:extension/engine@0.1.0" "send" (func $send (param i32 i32 i32)))
  (import "fps:extension/engine@0.1.0" "random-u64" (func $random (result i64)))
  (import "[export]fps:extension/hooks@0.1.0" "[resource-new]session"
    (func $resource_new (param i32) (result i32)))

  (memory (export "memory") 1)
  (global $bump (mut i32) (i32.const 8192))
  (data (i32.const @BASE@) "@IMAGE@")

  (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
    (local $p i32)
    (local.set $p
      (i32.and
        (i32.add (global.get $bump) (i32.sub (local.get 2) (i32.const 1)))
        (i32.sub (i32.const 0) (local.get 2))))
    (global.set $bump (i32.add (local.get $p) (local.get 3)))
    (local.get $p))

  (func $err (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    (i32.store8 (i32.const 0) (i32.const 1))
    (i32.store8 (i32.const 4) (local.get $kind))
    (i32.store (i32.const 8) (local.get $ptr))
    (i32.store (i32.const 12) (local.get $len))
    (i32.const 0))
  (func $ok_unit (result i32)
    (i32.store8 (i32.const 0) (i32.const 0))
    (i32.const 0))
  (func $ok_list (param $ptr i32) (param $len i32) (result i32)
    (i32.store8 (i32.const 0) (i32.const 0))
    (i32.store (i32.const 4) (local.get $ptr))
    (i32.store (i32.const 8) (local.get $len))
    (i32.const 0))
  (func $outcome (param $d i32) (result i32)
    (i32.store8 (i32.const 0) (i32.const 0))
    (i32.store8 (i32.const 4) (local.get $d))
    (i32.const 0))
  (func $spin (loop $l (br $l)))

  (func (export "fps:extension/template@0.1.0#functions") (result i32)
    (i32.store (i32.const 0) (i32.const @CATALOG@))
    (i32.store (i32.const 4) (i32.const 6))
    (i32.const 0))

  (func (export "fps:extension/hooks@0.1.0#on-open")
    (param i64 i32 i32 i32 i32 i32 i32) (result i32)
    (if (i64.eq (local.get 0) (i64.const 13))
      (then (return (call $err (i32.const 1) (i32.const @REFUSED@) (i32.const 7)))))
    (i32.store8 (i32.const 0) (i32.const 0))
    (i32.store (i32.const 4) (call $resource_new (i32.const 1)))
    (i32.const 0))

  (func (export "fps:extension/hooks@0.1.0#[method]session.on-data")
    (param i32 i32 i32) (result i32)
    (local $b i32)
    (if (i32.eqz (local.get 2)) (then (return (call $outcome (i32.const 0)))))
    (local.set $b (i32.load8_u (local.get 1)))
    (if (i32.eq (local.get $b) (i32.const 99))
      (then (return (call $outcome (i32.const 1)))))
    (if (i32.eq (local.get $b) (i32.const 114))
      (then
        (i32.store8 (i32.const 0) (i32.const 0))
        (i32.store8 (i32.const 4) (i32.const 2))
        (i32.store (i32.const 8) (i32.const @R@))
        (i32.store (i32.const 12) (i32.const 1))
        (return (i32.const 0))))
    (if (i32.eq (local.get $b) (i32.const 115)) (then (call $spin)))
    (if (i32.eq (local.get $b) (i32.const 103))
      (then (drop (memory.grow (i32.const 1000)))))
    (if (i32.eq (local.get $b) (i32.const 101))
      (then (return (call $err (i32.const 3) (i32.const @BOOM@) (i32.const 4)))))
    (if (i32.eq (local.get $b) (i32.const 120))
      (then (call $send (i32.const @HI@) (i32.const 2) (i32.const 32))))
    (call $outcome (i32.const 0)))

  (func (export "fps:extension/hooks@0.1.0#[method]session.on-timer")
    (param i32 i32) (result i32)
    (if (i32.eq (local.get 1) (i32.const 99)) (then (call $spin)))
    (if (i32.eq (local.get 1) (i32.const 5))
      (then (call $send (i32.const @TIMER@) (i32.const 5) (i32.const 32))))
    (call $ok_unit))

  (func (export "fps:extension/hooks@0.1.0#[method]session.on-close")
    (param i32 i32)
    (if (i32.eq (local.get 1) (i32.const 1)) (then (drop (call $random)))))

  (func (export "fps:extension/hooks@0.1.0#[method]session.render")
    (param i32 i32 i32 i32 i32 i32 i32) (result i32)
    (local $c i32)
    (local.set $c (i32.load8_u (local.get 1)))
    ;; echo
    (if (i32.eq (local.get $c) (i32.const 101))
      (then
        (if (i32.eqz (local.get 4))
          (then (return (call $err (i32.const 0) (i32.const @BOOM@) (i32.const 4)))))
        (if (i32.or
              (i32.eq (i32.load8_u (local.get 3)) (i32.const 3))
              (i32.eq (i32.load8_u (local.get 3)) (i32.const 4)))
          (then
            (return (call $ok_list
              (i32.load (i32.add (local.get 3) (i32.const 8)))
              (i32.load (i32.add (local.get 3) (i32.const 12)))))))
        (return (call $err (i32.const 0) (i32.const @BOOM@) (i32.const 4)))))
    ;; spin
    (if (i32.eq (local.get $c) (i32.const 115)) (then (call $spin)))
    ;; grow
    (if (i32.eq (local.get $c) (i32.const 103))
      (then (drop (memory.grow (i32.const 1000)))))
    ;; probe
    (if (i32.eq (local.get $c) (i32.const 112))
      (then
        (call $send (i32.const @PROBE@) (i32.const 1) (i32.const 32))
        (i32.store8 (i32.const 48) (i32.load8_u (i32.const 32)))
        (i32.store8 (i32.const 49) (i32.load8_u (i32.const 33)))
        (return (call $ok_list (i32.const 48) (i32.const 2)))))
    ;; rand
    (if (i32.eq (local.get $c) (i32.const 114))
      (then
        (i64.store (i32.const 48) (call $random))
        (return (call $ok_list (i32.const 48) (i32.const 8)))))
    ;; fail
    (if (i32.eq (local.get $c) (i32.const 102))
      (then (return (call $err (i32.const 3) (i32.const @BOOM@) (i32.const 4)))))
    (call $ok_list (i32.const 0) (i32.const 0))))
"#;

/// Encodes the core module as a component of the `extension` world, with the
/// WIT file of the contract, so the types match the contract by construction.
pub fn component_bytes() -> Vec<u8> {
    let mut core = wat::parse_str(component_wat()).unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/extension");
    let (pkg, _) = resolve.push_path(dir).unwrap();
    let world = resolve.select_world(&[pkg], Some("extension")).unwrap();
    wit_component::embed_component_metadata(
        &mut core,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    wit_component::ComponentEncoder::default()
        .module(&core)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

/// What the recording host saw.
#[derive(Clone, Default)]
pub struct Record {
    pub sends: Arc<Mutex<Vec<Vec<u8>>>>,
    pub logs: Arc<Mutex<Vec<String>>>,
}

/// A host that records `send` and `log` and returns a fixed random value.
pub struct TestHost {
    pub record: Record,
}

impl TestHost {
    pub const RANDOM: u64 = 0x0102_0304_0506_0708;

    pub fn boxed(record: &Record) -> Box<dyn Host> {
        Box::new(TestHost {
            record: record.clone(),
        })
    }
}

impl Host for TestHost {
    fn send(&mut self, data: &[u8]) -> Result<(), EngineError> {
        self.record.sends.lock().unwrap().push(data.to_vec());
        Ok(())
    }
    fn set_timer(&mut self, _delay_ms: u32) -> Result<TimerId, EngineError> {
        Ok(TimerId(1))
    }
    fn cancel_timer(&mut self, _timer: TimerId) {}
    fn log(&mut self, level: LogLevel, message: &str, _fields: &[(&str, &str)]) {
        self.record
            .logs
            .lock()
            .unwrap()
            .push(format!("{level:?} {message}"));
    }
    fn random_u64(&mut self) -> u64 {
        Self::RANDOM
    }
    fn random_bytes(&mut self, out: &mut [u8]) {
        out.fill(7);
    }
}
