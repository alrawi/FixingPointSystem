# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of the JA4 computation against the values of the FoxIO specification."""

from __future__ import annotations

import hashlib

import pytest

from realism import ja4
from realism.ja4 import ClientHello, ClientHelloError, compute, explain_difference, parts
from realism.tests.tlsgen import build_client_hello

# The ClientHello of the worked example of the JA4 specification, in the wire order
# of a Chrome of 2023, with the GREASE values that Chrome adds. The specification
# names t13d1516h2_8daaf6152771_e5627efa2ab1 as its fingerprint.
CHROME_CIPHERS = [
    0x0A0A, 0x1301, 0x1302, 0x1303, 0xC02B, 0xC02F, 0xC02C, 0xC030,
    0xCCA9, 0xCCA8, 0xC013, 0xC014, 0x009C, 0x009D, 0x002F, 0x0035,
]  # fmt: skip
CHROME_EXTENSIONS = [
    0x1A1A, 0x0000, 0x0017, 0xFF01, 0x000A, 0x000B, 0x0023, 0x0010, 0x0005, 0x000D,
    0x0012, 0x0033, 0x002D, 0x002B, 0x001B, 0x4469, 0x0015, 0xFAFA,
]  # fmt: skip
CHROME_SIGNATURE_ALGORITHMS = [0x0403, 0x0804, 0x0401, 0x0503, 0x0805, 0x0501, 0x0806, 0x0601]
CHROME = ClientHello(
    version=0x0303,
    cipher_suites=CHROME_CIPHERS,
    extensions=CHROME_EXTENSIONS,
    alpn=[b"h2", b"http/1.1"],
    signature_algorithms=CHROME_SIGNATURE_ALGORITHMS,
    supported_versions=[0x3A3A, 0x0304, 0x0303],
)
CHROME_JA4 = "t13d1516h2_8daaf6152771_e5627efa2ab1"


def _sha12(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()[:12]


def test_the_specification_example_gives_its_published_fingerprint() -> None:
    assert compute(CHROME) == CHROME_JA4


def test_the_parts_of_the_example_are_the_published_ones() -> None:
    assert parts(CHROME) == ("t13d1516h2", "8daaf6152771", "e5627efa2ab1")


def test_the_same_hello_read_from_bytes_gives_the_same_fingerprint() -> None:
    data = build_client_hello(CHROME)
    parsed = ja4.parse_client_hello(data)
    assert parsed.server_name == "tls.example.test"
    assert list(parsed.extensions) == CHROME_EXTENSIONS
    assert compute(parsed) == CHROME_JA4


def test_a_hello_split_over_two_records_reads_the_same() -> None:
    data = build_client_hello(CHROME)
    payload = data[5:]
    cut = 100
    split = (
        b"\x16\x03\x01" + cut.to_bytes(2, "big") + payload[:cut]
        + b"\x16\x03\x01" + (len(payload) - cut).to_bytes(2, "big") + payload[cut:]
    )  # fmt: skip
    assert compute(ja4.parse_client_hello(split)) == CHROME_JA4


def test_grease_values_change_nothing() -> None:
    plain = ClientHello(
        version=0x0303,
        cipher_suites=[c for c in CHROME_CIPHERS if not ja4.is_grease(c)],
        extensions=[e for e in CHROME_EXTENSIONS if not ja4.is_grease(e)],
        alpn=[b"h2", b"http/1.1"],
        signature_algorithms=CHROME_SIGNATURE_ALGORITHMS,
        supported_versions=[0x0304, 0x0303],
    )
    assert compute(plain) == CHROME_JA4


def test_is_grease_accepts_exactly_the_sixteen_rfc_8701_values() -> None:
    found = [v for v in range(0x10000) if ja4.is_grease(v)]
    assert found == [n * 0x1010 + 0x0A0A for n in range(16)]


def test_the_order_of_ciphers_and_extensions_does_not_change_the_fingerprint() -> None:
    shuffled = ClientHello(
        version=0x0303,
        cipher_suites=list(reversed(CHROME_CIPHERS)),
        extensions=list(reversed(CHROME_EXTENSIONS)),
        alpn=[b"h2", b"http/1.1"],
        signature_algorithms=CHROME_SIGNATURE_ALGORITHMS,
        supported_versions=[0x0304, 0x0303],
    )
    assert compute(shuffled) == CHROME_JA4


def test_the_order_of_signature_algorithms_does_change_it() -> None:
    reordered = ClientHello(
        version=0x0303,
        cipher_suites=CHROME_CIPHERS,
        extensions=CHROME_EXTENSIONS,
        alpn=[b"h2"],
        signature_algorithms=list(reversed(CHROME_SIGNATURE_ALGORITHMS)),
        supported_versions=[0x0304],
    )
    assert parts(reordered)[2] != "e5627efa2ab1"
    assert parts(reordered)[:2] == ("t13d1516h2", "8daaf6152771")


def test_the_cipher_hash_is_the_truncated_hash_of_the_sorted_hex_list() -> None:
    expected = "002f,0035,009c,009d,1301,1302,1303,c013,c014,c02b,c02c,c02f,c030,cca8,cca9"
    assert parts(CHROME)[1] == _sha12(expected) == "8daaf6152771"


def test_without_sni_and_alpn_the_flags_read_i_and_00() -> None:
    hello = ClientHello(version=0x0303, cipher_suites=[0xC02F], extensions=[0x000A, 0x000B])
    a, b, c = parts(hello)
    assert a == "t12i010200"
    assert b == _sha12("c02f")
    assert c == _sha12("000a,000b")


def test_without_signature_algorithms_the_extension_string_has_no_underscore() -> None:
    hello = ClientHello(version=0x0303, cipher_suites=[0xC02F], extensions=[0x0000, 0x0017])
    assert parts(hello)[2] == _sha12("0017")


def test_an_empty_cipher_or_extension_list_hashes_to_twelve_zeros() -> None:
    assert parts(ClientHello(version=0x0303, cipher_suites=[], extensions=[])) == (
        "t12i000000",
        "000000000000",
        "000000000000",
    )
    only_sni = ClientHello(version=0x0303, cipher_suites=[0x1301], extensions=[0x0000, 0x0010])
    assert parts(only_sni)[2] == "000000000000"
    assert parts(only_sni)[0] == "t12d010200"


def test_the_version_comes_from_the_highest_supported_version_or_the_legacy_field() -> None:
    def version_of(legacy: int, supported: list[int]) -> str:
        hello = ClientHello(
            version=legacy, cipher_suites=[1], extensions=[0x002B], supported_versions=supported
        )
        return parts(hello)[0][1:3]

    assert version_of(0x0303, [0x0304, 0x0303, 0x0302]) == "13"
    assert version_of(0x0303, [0x0302, 0x0304]) == "13"
    assert version_of(0x0303, [0x0A0A, 0x0303]) == "12"
    assert version_of(0x0301, []) == "10"
    assert version_of(0x0300, []) == "s3"
    assert version_of(0x1234, []) == "00"
    assert parts(ClientHello(0xFEFD, [1], [], transport="d"))[0].startswith("d")


def test_alpn_uses_the_first_value_and_hex_for_non_alphanumeric_ends() -> None:
    def alpn_of(*protocols: bytes) -> str:
        hello = ClientHello(
            version=0x0303, cipher_suites=[1], extensions=[0x0010], alpn=list(protocols)
        )
        return parts(hello)[0][-2:]

    assert alpn_of(b"h2", b"http/1.1") == "h2"
    assert alpn_of(b"http/1.1") == "h1"
    assert alpn_of(b"\xab") == "ab"
    assert alpn_of(b"\xab\xcd") == "ad"
    assert alpn_of(b"") == "00"
    assert alpn_of() == "00"


def test_counts_stop_at_99() -> None:
    hello = ClientHello(
        version=0x0303, cipher_suites=list(range(1, 150)), extensions=list(range(100, 300))
    )
    assert parts(hello)[0] == "t12i9999" + "00"


def test_explain_difference_names_each_part_that_differs() -> None:
    same = "t13d1516h2_8daaf6152771_e5627efa2ab1"
    assert explain_difference(same, same) == "the fingerprints are equal"
    text = explain_difference("t13d1517h2_000000000000_e5627efa2ab1", same)
    assert (
        "part a" in text and "cipher suite hash" in text and "extension and signature" not in text
    )
    assert "extension and signature algorithm hash" in explain_difference(
        "t13d1516h2_8daaf6152771_000000000000", same
    )
    assert ja4.split("not a ja4") is None
    assert "is not" in explain_difference("garbage", same)


@pytest.mark.parametrize(
    "data",
    [
        b"",
        b"\x17\x03\x03\x00\x01\x00",
        b"\x16\x03\x01\x00\x10" + b"\x01\x00\x00\x40",
        b"\x16\x03\x01\x00\x04\x02\x00\x00\x00",
    ],
)
def test_bytes_that_are_not_a_client_hello_raise(data: bytes) -> None:
    with pytest.raises(ClientHelloError):
        ja4.parse_client_hello(data)
