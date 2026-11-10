# SPDX-License-Identifier: GPL-3.0-only
"""Profile extraction (CAP-04): mix shares, arrival models, and size models."""

from __future__ import annotations

import json
import math
import random
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from capture.cli import extract, main
from capture.profile import (
    ProfileError,
    arrival_model,
    build_profile,
    classify,
    fit_distribution,
    parse_rules,
    size_model,
)
from capture.scrub import scrub_document, sha256_bytes

from .pcapgen import Rec, write_pcap
from .scenario_helpers import REPO, T0, tcp_session, udp_session

FIXTURES = REPO / "modules"
CONTRACT = json.loads((REPO / "contracts/schemas/test-config.schema.json").read_text())
CONSENT = "owner approved the lab capture, ticket 4711"


def profile_schema() -> dict[str, Any]:
    """The ``profile`` object of the test configuration, with the definitions it needs."""
    return {
        "$schema": CONTRACT["$schema"],
        **CONTRACT["properties"]["profile"],
        "$defs": CONTRACT["$defs"],
    }


def fixture_profile(module: str, *rules: str, **kwargs: Any) -> dict[str, Any]:
    path = FIXTURES / module / "fixtures" / f"{module}.pcapng"
    return build_profile(extract(path), app_rules=rules, **kwargs)


def weights(doc: dict[str, Any]) -> dict[str, float]:
    return {e["scenario"]: e["weight"] for e in doc["profile"]["mix"]}


def counts(doc: dict[str, Any]) -> dict[str, int]:
    return {a["name"]: a["sessions"] for a in doc["applications"]}


# ---------------------------------------------------------------- committed fixtures


def test_http1_fixture_mix_by_sessions() -> None:
    """The harness ran 120 plain HTTP sessions on 8080 and 80 TLS sessions on 8443 to 8446."""
    doc = fixture_profile("http1", "8443-8446=https")
    assert counts(doc) == {"http": 120, "https": 80}
    assert weights(doc) == {"http": 60.0, "https": 40.0}
    assert [e["scenario"] for e in doc["profile"]["mix"]] == ["http", "https"]
    assert doc["unclassified"]["sessions"] == 0
    assert doc["source"]["sessions"] == 200
    assert doc["source"]["file"] == "http1.pcapng"


def test_http1_fixture_mix_by_bytes() -> None:
    """Distinct payload bytes: 447869 for http and 490812 for https, summed by hand."""
    doc = fixture_profile("http1", "8443-8446=https", basis="bytes")
    by_app = {a["name"]: a["payload_bytes"] for a in doc["applications"]}
    assert by_app == {"http": 447869, "https": 490812}
    assert weights(doc) == {"https": 52.2874, "http": 47.7126}
    assert [e["scenario"] for e in doc["profile"]["mix"]] == ["https", "http"]
    assert doc["profile"]["basis"] == "bytes"


def test_http1_fixture_without_rule_leaves_the_tls_ports_out() -> None:
    """Only 8443 is a well known port. 8444 to 8446 stay unclassified and say so."""
    doc = fixture_profile("http1")
    assert counts(doc) == {"http": 120, "https": 20}
    assert doc["unclassified"]["sessions"] == 60
    assert doc["unclassified"]["server_ports"] == [
        {"protocol": "tcp", "port": 8444, "sessions": 20},
        {"protocol": "tcp", "port": 8445, "sessions": 20},
        {"protocol": "tcp", "port": 8446, "sessions": 20},
    ]
    assert weights(doc) == {"http": 85.7143, "https": 14.2857}


def test_ftp_fixture_counts_control_connections_only() -> None:
    """The 40 control connections are FTP. The 80 passive data connections are not named."""
    doc = fixture_profile("ftp")
    assert counts(doc) == {"ftp": 40}
    assert weights(doc) == {"ftp": 100.0}
    assert doc["unclassified"]["sessions"] == 80
    assert doc["unclassified"]["payload_bytes"] > 0
    named = fixture_profile("ftp", "20000-20999=ftp_data")
    assert counts(named) == {"ftp": 40, "ftp_data": 80}
    assert weights(named) == {"ftp": 33.3333, "ftp_data": 66.6667}


def test_dns_fixture_joins_udp_and_tcp_under_one_name() -> None:
    doc = fixture_profile("dns")
    assert counts(doc) == {"dns": 160}
    assert doc["unclassified"]["sessions"] == 0
    # 140 UDP sessions send one datagram per direction, and 20 TCP sessions one stream
    ev = doc["applications"][0]["size"]
    assert ev["request"]["samples"] == 160
    assert ev["response"]["samples"] == 160


def test_smtp_and_imap_fixtures_with_nonstandard_ports() -> None:
    smtp = fixture_profile("smtp")
    assert counts(smtp) == {"smtp": 48}
    assert smtp["unclassified"]["sessions"] == 8
    assert counts(fixture_profile("smtp", "5587=smtp")) == {"smtp": 56}
    imap = fixture_profile("imap")
    assert counts(imap) == {"imap": 40}
    assert counts(fixture_profile("imap", "1143=imap")) == {"imap": 48}
    assert counts(fixture_profile("pop3")) == {"pop3": 40}


def test_fixture_profile_validates_against_the_contract() -> None:
    for module, rules in [("http1", ("8443-8446=https",)), ("dns", ()), ("smtp", ("5587=smtp",))]:
        doc = fixture_profile(module, *rules)
        jsonschema.validate(json.loads(json.dumps(doc["profile"])), profile_schema())
        for app in doc["applications"]:
            assert app["arrival"]["model"] is not None, (module, app["name"])


def test_http1_fixture_models_have_the_contract_shape() -> None:
    doc = fixture_profile("http1", "8443-8446=https")
    for entry in doc["profile"]["mix"]:
        assert entry["arrival"]["dist"] in {"exponential", "uniform", "lognormal", "pareto"}
        assert set(entry["size"]) == {"request", "response"}
    https = doc["applications"][1]
    assert https["size"]["request"]["samples"] == 80
    assert https["size"]["response"]["samples"] == 80
    # the TLS responses are about 4.5 times as large as the plain ones in this fixture
    assert https["size"]["response"]["model"]["mu"] == pytest.approx(8.42286, abs=1e-5)
    assert https["size"]["response"]["model"]["dist"] == "lognormal"


# ---------------------------------------------------------------- generated runs


def generated_run(
    tmp_path: Path, seed: int, mix: dict[str, int], per_app: int = 0, gap_s: float = 0.05
) -> tuple[dict[str, Any], dict[str, int]]:
    """Write a capture of a run whose mix is configured, and return its session document.

    ``mix`` gives the number of sessions of http (TCP 8080), smtp (TCP 25) and dns (UDP 53).
    The sessions are shuffled and start at exponential gaps, and the sizes are lognormal,
    so the run has the temporal structure of a real one. The second result is the number of
    payload bytes that the generator wrote per application, which the extractor must match."""
    rng = random.Random(seed)
    order = [app for app, n in mix.items() for _ in range(n)]
    rng.shuffle(order)
    recs: list[Rec] = []
    written = dict.fromkeys(mix, 0)
    t = T0
    for i, app in enumerate(order):
        t += rng.expovariate(1.0 / gap_s)
        sport = 10000 + i
        if app == "dns":
            q = max(12, round(rng.lognormvariate(3.5, 0.2)))
            a = max(12, round(rng.lognormvariate(4.6, 0.4)))
            recs += udp_session([("c", b"q" * q), ("s", b"a" * a)], t, sport=sport, dport=53)
        else:
            up = max(1, round(rng.lognormvariate(5.0, 0.5)))
            down = max(1, round(rng.lognormvariate(7.5 if app == "http" else 6.0, 0.8)))
            port = 8080 if app == "http" else 25
            recs += tcp_session([("c", b"u" * up), ("s", b"d" * down)], t, sport=sport, dport=port)
        written[app] += up + down if app != "dns" else q + a
    path = tmp_path / "run.pcap"
    path.write_bytes(write_pcap(sorted(recs, key=lambda r: r.t)))
    return extract(path), written


def test_generated_run_reproduces_its_configured_mix_within_two_points(tmp_path: Path) -> None:
    configured = {"http": 300, "smtp": 180, "dns": 120}
    doc, _ = generated_run(tmp_path, seed=7, mix=configured)
    profile = build_profile(doc)
    target = {"http": 50.0, "smtp": 30.0, "dns": 20.0}
    got = weights(profile)
    assert set(got) == set(target)
    for app, pct in target.items():
        assert abs(got[app] - pct) <= 2.0, (app, got[app])
    # this run is exact, since every configured session shows up as one session
    assert got == target
    assert sum(got.values()) == pytest.approx(100.0)


def test_generated_run_reproduces_its_byte_mix_within_two_points(tmp_path: Path) -> None:
    doc, written = generated_run(tmp_path, seed=11, mix={"http": 300, "smtp": 180, "dns": 120})
    profile = build_profile(doc, basis="bytes")
    total = sum(written.values())
    got = weights(profile)
    for app, nbytes in written.items():
        assert abs(got[app] - 100.0 * nbytes / total) <= 0.001, app
    assert {a["name"]: a["payload_bytes"] for a in profile["applications"]} == written


def test_generated_run_recovers_the_arrival_and_size_models(tmp_path: Path) -> None:
    configured = {"http": 600, "smtp": 0, "dns": 0}
    doc, _ = generated_run(tmp_path, seed=3, mix=configured, gap_s=0.05)
    profile = build_profile(doc)
    (entry,) = profile["profile"]["mix"]
    # Exponential gaps of mean 50 ms. The generator draws them for the whole run, and the
    # run is one application, so the application and the overall model agree.
    assert entry["arrival"]["dist"] == "exponential"
    assert entry["arrival"]["mean_ms"] == pytest.approx(50.0, rel=0.1)
    assert profile["profile"]["arrival"] == entry["arrival"]
    # Lognormal sizes with mu 5.0, sigma 0.5 up and mu 7.5, sigma 0.8 down
    up, down = entry["size"]["request"], entry["size"]["response"]
    assert up["dist"] == "lognormal"
    assert up["mu"] == pytest.approx(5.0, abs=0.1)
    assert up["sigma"] == pytest.approx(0.5, abs=0.05)
    assert down["dist"] == "lognormal"
    assert down["mu"] == pytest.approx(7.5, abs=0.15)
    assert down["sigma"] == pytest.approx(0.8, abs=0.08)


def test_mix_by_application_has_its_own_models(tmp_path: Path) -> None:
    doc, _ = generated_run(tmp_path, seed=5, mix={"http": 300, "smtp": 180, "dns": 120})
    by_name = {e["scenario"]: e for e in build_profile(doc)["profile"]["mix"]}
    # DNS datagrams are one unit each, so the sizes follow the datagrams, around 33 and 100
    assert by_name["dns"]["size"]["request"]["dist"] == "lognormal"
    assert by_name["dns"]["size"]["request"]["mu"] == pytest.approx(3.5, abs=0.1)
    assert by_name["http"]["size"]["response"]["mu"] == pytest.approx(7.5, abs=0.15)
    assert by_name["smtp"]["size"]["response"]["mu"] == pytest.approx(6.0, abs=0.15)


# ---------------------------------------------------------------- fitting


def test_fit_recognizes_each_family() -> None:
    rng = random.Random(1)
    n = 800
    cases = {
        "exponential": [rng.expovariate(1 / 120.0) for _ in range(n)],
        "uniform": [rng.uniform(20.0, 80.0) for _ in range(n)],
        "lognormal": [rng.lognormvariate(4.0, 0.7) for _ in range(n)],
        "pareto": [30.0 * rng.paretovariate(1.6) for _ in range(n)],
    }
    for family, sample in cases.items():
        got, _, d, others = fit_distribution(sample)
        assert got == family, (family, others)
        assert d < 0.05
        assert set(others) == {"exponential", "uniform", "lognormal", "pareto"}
    _, p, _, _ = fit_distribution(cases["pareto"])
    assert p["alpha"] == pytest.approx(1.6, rel=0.1)
    assert p["scale"] == pytest.approx(30.0, rel=0.02)
    _, p, _, _ = fit_distribution(cases["uniform"])
    assert (p["min"], p["max"]) == (min(cases["uniform"]), max(cases["uniform"]))


def test_fit_constant_and_its_tolerance() -> None:
    name, p, d, _ = fit_distribution([100.0, 100.4, 99.8, 100.0, 100.2])
    assert name == "constant"
    assert p == {"value": 100.0, "tolerance": pytest.approx(0.4)}
    assert d == 0.0
    spec, _, _ = arrival_model([250.0] * 10)
    assert spec == {"dist": "constant", "ms": 250.0, "tolerance_ms": 0.0}
    spec, _, _ = size_model([512.0] * 10)
    assert spec == {"dist": "constant", "value": 512.0, "tolerance": 0.0}


def test_arrival_model_uses_the_contract_names_and_seconds_for_lognormal() -> None:
    rng = random.Random(2)
    gaps_ms = [math.exp(rng.gauss(6.0, 0.5)) for _ in range(1000)]  # median about 403 ms
    spec, _, _ = arrival_model(gaps_ms)
    assert spec["dist"] == "lognormal"
    assert spec["unit"] == "s"
    assert spec["mu"] == pytest.approx(6.0 - math.log(1000.0), abs=0.05)
    pareto, _, _ = arrival_model([40.0 * rng.paretovariate(1.5) for _ in range(1000)])
    assert set(pareto) == {"dist", "alpha", "scale_ms"}


def test_zero_gaps_skip_the_families_that_need_positive_values() -> None:
    sample = [0.0] * 5 + [float(x) for x in range(1, 60)]
    name, _, _, others = fit_distribution(sample)
    assert set(others) == {"exponential", "uniform"}
    assert name in others


def test_few_observations_give_no_model(tmp_path: Path) -> None:
    recs = tcp_session([("c", b"x" * 10), ("s", b"y" * 20)], T0, sport=40000)
    recs += tcp_session([("c", b"x" * 10), ("s", b"y" * 20)], T0 + 1, sport=40001)
    path = tmp_path / "two.pcap"
    path.write_bytes(write_pcap(sorted(recs, key=lambda r: r.t)))
    doc = build_profile(extract(path))
    assert doc["profile"]["mix"] == [{"scenario": "http", "weight": 100.0}]
    assert "arrival" not in doc["profile"]
    assert doc["applications"][0]["arrival"] == {
        "samples": 1,
        "model": None,
        "reason": "insufficient_data",
    }
    lowered = build_profile(extract(path), min_samples=2)
    assert lowered["profile"]["mix"][0]["size"]["request"] == {
        "dist": "constant",
        "value": 10.0,
        "tolerance": 0.0,
    }


# ---------------------------------------------------------------- classification and input


def test_rules_parse_and_take_precedence_over_the_table() -> None:
    assert parse_rules(["8443-8446=https", "udp:5353=mdns", "9000=web/browse"]) == [
        (None, 8443, 8446, "https"),
        ("udp", 5353, 5353, "mdns"),
        (None, 9000, 9000, "web/browse"),
    ]
    for bad in ["80", "80=", "0=x", "70000=x", "9-3=x", "80=Web", "tcp:80-=x", "80=a b"]:
        with pytest.raises(ProfileError):
            parse_rules([bad])


def test_user_rule_renames_a_well_known_port(tmp_path: Path) -> None:
    recs: list[Rec] = []
    for i in range(40):
        recs += tcp_session([("c", b"a" * 50), ("s", b"b" * 500)], T0 + i, sport=40000 + i)
    path = tmp_path / "web.pcap"
    path.write_bytes(write_pcap(sorted(recs, key=lambda r: r.t)))
    doc = extract(path)
    assert weights(build_profile(doc)) == {"http": 100.0}
    assert weights(build_profile(doc, app_rules=["8080=http1/single_fetch"])) == {
        "http1/single_fetch": 100.0
    }
    with pytest.raises(ProfileError, match="no session matched"):
        build_profile(doc, defaults=False)


def test_unknown_roles_fall_back_to_the_client_port() -> None:
    sess: dict[str, Any] = {
        "protocol": "tcp",
        "server": {"ip": "10.0.0.1", "port": 51000},
        "client": {"ip": "10.0.0.2", "port": 25},
        "roles": {"client_known": False, "basis": "first_packet"},
    }
    assert classify(sess, [], True) == "smtp"
    sess["roles"]["client_known"] = True
    assert classify(sess, [], True) is None


def test_udp_idle_split_is_not_a_new_session(tmp_path: Path) -> None:
    recs = udp_session([("c", b"q" * 20), ("s", b"a" * 40)], T0, sport=50000, dport=53)
    recs += udp_session([("c", b"q" * 20), ("s", b"a" * 40)], T0 + 100, sport=50000, dport=53)
    recs += udp_session([("c", b"q" * 20), ("s", b"a" * 40)], T0 + 5, sport=50001, dport=53)
    path = tmp_path / "udp.pcap"
    path.write_bytes(write_pcap(sorted(recs, key=lambda r: r.t)))
    doc = extract(path)
    assert doc["summary"]["sessions"] == 3
    out = build_profile(doc)
    assert counts(out) == {"dns": 2}
    assert out["applications"][0]["payload_bytes"] == 180
    assert any("continue an earlier flow" in n for n in out["notes"])


def test_bad_inputs() -> None:
    with pytest.raises(ProfileError, match="schema version"):
        build_profile({"schema_version": "2.0", "sessions": []})
    doc = extract(FIXTURES / "pop3" / "fixtures" / "pop3.pcapng")
    with pytest.raises(ProfileError, match="basis"):
        build_profile(doc, basis="flows")
    with pytest.raises(ProfileError, match="min samples"):
        build_profile(doc, min_samples=1)


# ---------------------------------------------------------------- scrub record


def scrubbed(doc: dict[str, Any], **kwargs: Any) -> dict[str, Any]:
    return scrub_document(
        doc,
        consent=CONSENT,
        input_sha256=sha256_bytes(json.dumps(doc).encode()),
        date="2026-11-10",
        seed=1,
        hostnames=False,
        extra_hosts=[],
        literals=[],
        drop_ports=kwargs.get("drop_ports", []),
    )


def test_profile_of_a_scrubbed_document_carries_the_record() -> None:
    path = FIXTURES / "http1" / "fixtures" / "http1.pcapng"
    raw = extract(path, payloads=True)
    clean = scrubbed(raw)
    out = build_profile(clean, app_rules=["8443-8446=https"])
    assert out["scrub"]["state"] == "intact"
    assert out["scrub"]["date"] == "2026-11-10"
    assert out["scrub"]["dropped_sessions"] == 0
    assert out["scrub"]["content_sha256"] == clean["scrub"]["content_sha256"]
    assert out["source"]["sha256"] == raw["source"]["sha256"]
    # the scrub changes payload bytes, so it may change sizes but never the mix
    assert weights(out) == {"http": 60.0, "https": 40.0}
    assert build_profile(raw)["scrub"] == {"state": "none"}


def test_dropped_sessions_and_edited_records_are_reported() -> None:
    path = FIXTURES / "http1" / "fixtures" / "http1.pcapng"
    raw = extract(path, payloads=True)
    out = build_profile(scrubbed(raw, drop_ports=[8444]), app_rules=["8443-8446=https"])
    assert out["scrub"]["dropped_sessions"] == 20
    assert counts(out) == {"http": 120, "https": 60}
    assert weights(out) == {"http": 66.6667, "https": 33.3333}
    assert any("scrub dropped 20 sessions" in n for n in out["notes"])
    tampered = scrubbed(raw)
    tampered["sessions"][0]["server"]["port"] = 9
    bad = build_profile(tampered, app_rules=["8443-8446=https"])
    assert bad["scrub"]["state"] == "invalid"
    assert any("not intact" in n for n in bad["notes"])


def test_profile_holds_no_payload_or_address(tmp_path: Path) -> None:
    raw = extract(FIXTURES / "smtp" / "fixtures" / "smtp.pcapng", payloads=True)
    text = json.dumps(build_profile(raw))
    for session in raw["sessions"][:20]:
        assert session["client"]["ip"] not in text
        assert session["server"]["ip"] not in text
    assert "data_b64" not in text


# ---------------------------------------------------------------- command line


def run_cli(*argv: str) -> int:
    return main(["profile", *argv])


def test_command_from_a_capture(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    out = tmp_path / "profile.json"
    pcap = FIXTURES / "http1" / "fixtures" / "http1.pcapng"
    assert run_cli("--pcap", str(pcap), "--app", "8443-8446=https", "--out", str(out)) == 0
    doc = json.loads(out.read_text())
    assert doc["tool"] == "capture profile"
    assert weights(doc) == {"http": 60.0, "https": 40.0}
    err = capsys.readouterr().err
    assert "profile on sessions: http 60, https 40" in err
    assert "warning" not in err


def test_command_from_sessions_profile_only(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    sessions = tmp_path / "s.json"
    sessions.write_text(json.dumps(extract(FIXTURES / "ftp" / "fixtures" / "ftp.pcapng")))
    assert run_cli("--sessions", str(sessions), "--basis", "bytes", "--profile-only") == 0
    cap = capsys.readouterr()
    prof = json.loads(cap.out)
    assert set(prof) == {"basis", "arrival", "mix"}
    assert prof["basis"] == "bytes"
    assert "80 sessions match no application" in cap.err


def test_command_errors(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    missing = tmp_path / "none.json"
    assert run_cli("--sessions", str(missing)) == 1
    assert "cannot read" in capsys.readouterr().err
    pcap = FIXTURES / "pop3" / "fixtures" / "pop3.pcapng"
    assert run_cli("--pcap", str(pcap), "--no-default-apps") == 1
    assert "no session matched" in capsys.readouterr().err
    assert run_cli("--pcap", str(pcap), "--app", "bad") == 1
    assert "PORT[-PORT]=NAME" in capsys.readouterr().err
    assert run_cli("--pcap", str(pcap), "--udp-idle-timeout", "0") == 1
    assert run_cli("--pcap", str(tmp_path / "x.pcap")) == 1
    with pytest.raises(SystemExit):
        run_cli("--pcap", str(pcap), "--sessions", str(missing))
