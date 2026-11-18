// SPDX-License-Identifier: GPL-3.0-only
//! The OS profiles of the own stack against the table "Profile files" of
//! `docs/wiki/Transport.md` (TRN-28). The table is the one that the kernel
//! backend implements through socket options and sysctls, so a profile of the
//! own stack that agrees with it agrees with the kernel backend, except for
//! the option order that the kernel cannot set.

use std::path::Path;

use fps_tcp_own::profile::BUILTIN_PROFILES;
use fps_tcp_own::OsProfile;

/// The cells of the row of the table whose first cell is `field`, one per
/// profile in the order of the header (windows11, linux6, macos14).
fn row(page: &str, field: &str) -> Vec<String> {
    let key = format!("| `{field}` |");
    let line = page
        .lines()
        .find(|l| l.starts_with(&key))
        .unwrap_or_else(|| panic!("the Transport page has no row for {field}"));
    line.split('|')
        .skip(2)
        .take(3)
        .map(|c| c.trim().to_owned())
        .collect()
}

fn transport_page() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/wiki/Transport.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn on(cell: &str) -> bool {
    match cell {
        "on" => true,
        "off" => false,
        other => panic!("expected on or off, found {other}"),
    }
}

#[test]
fn the_table_lists_the_profiles_in_the_order_these_tests_read_them() {
    let page = transport_page();
    let header = page
        .lines()
        .find(|l| l.starts_with("| Field | windows11 |"))
        .expect("the header of the profile table");
    let names: Vec<&str> = header.split('|').skip(2).take(3).map(str::trim).collect();
    assert_eq!(names, ["windows11", "linux6", "macos14"]);
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, BUILTIN_PROFILES);
}

#[test]
fn every_profile_has_the_values_of_the_table() {
    let page = transport_page();
    let order = ["windows11", "linux6", "macos14"];
    let ttl = row(&page, "socket.ttl");
    let mss = row(&page, "socket.mss");
    let sack = row(&page, "system.sack");
    let timestamps = row(&page, "system.timestamps");
    let scaling = row(&page, "system.window_scaling");
    let scale = row(&page, "reference.window_scale");
    let window = row(&page, "reference.syn_window");
    let options = row(&page, "reference.options");
    for (i, name) in order.iter().enumerate() {
        let p = OsProfile::builtin(name).expect("a built in profile");
        assert_eq!(p.name, *name);
        assert_eq!(p.ttl.to_string(), ttl[i], "{name} socket.ttl");
        assert_eq!(p.mss.to_string(), mss[i], "{name} socket.mss");
        assert_eq!(p.sack, on(&sack[i]), "{name} system.sack");
        assert_eq!(p.timestamps, on(&timestamps[i]), "{name} system.timestamps");
        assert_eq!(
            p.window_scaling,
            on(&scaling[i]),
            "{name} system.window_scaling"
        );
        assert_eq!(
            p.window_scale.to_string(),
            scale[i],
            "{name} reference.window_scale"
        );
        assert_eq!(
            p.syn_window.to_string(),
            window[i],
            "{name} reference.syn_window"
        );
        assert_eq!(
            p.option_names().join(", "),
            options[i],
            "{name} reference.options"
        );
    }
}

#[test]
fn the_options_of_a_profile_agree_with_what_it_offers() {
    // The option list names an option exactly when the host offers it.
    for name in BUILTIN_PROFILES {
        let p = OsProfile::builtin(name).expect("a built in profile");
        let names = p.option_names();
        assert_eq!(names.contains(&"ts"), p.timestamps, "{name}");
        assert_eq!(names.contains(&"sok"), p.sack, "{name}");
        assert_eq!(names.contains(&"ws"), p.window_scaling, "{name}");
        assert!(names.contains(&"mss"), "{name}");
    }
}
