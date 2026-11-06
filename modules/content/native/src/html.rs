// SPDX-License-Identifier: GPL-3.0-only
//! HTML 5 pages built from section templates.
//!
//! A page has a head, a header with a navigation list, a main element that
//! grows by one section at a time until the page reaches the size, and a
//! footer. Links point to the reserved `.test` domain or to paths on the same
//! host, and image sources point to paths that the page does not serve.

use crate::prose::{capitalise, lang};
use crate::rng::Rng;
use crate::words::{ADJECTIVES, NOUNS};

fn title(r: &mut Rng) -> String {
    capitalise(&format!("{} {}", r.pick(ADJECTIVES), r.pick(NOUNS)))
}

fn paragraphs(r: &mut Rng, s: &mut String) {
    let Some(en) = lang("en") else { return };
    s.push_str(&format!("<section>\n<h2>{}</h2>\n", title(r)));
    for _ in 0..r.range(2, 3) {
        let mut p = String::new();
        for _ in 0..r.range(2, 4) {
            en.sentence(r, &mut p);
            p.push(' ');
        }
        s.push_str(&format!("<p>{}</p>\n", p.trim_end()));
    }
    s.push_str("</section>\n");
}

fn list(r: &mut Rng, s: &mut String) {
    s.push_str(&format!(
        "<section>\n<h2>{}</h2>\n<ul class=\"items\">\n",
        title(r)
    ));
    for _ in 0..r.range(3, 6) {
        let (a, n) = (r.pick(ADJECTIVES), r.pick(NOUNS));
        s.push_str(&format!(
            "<li><a href=\"/{n}/{}\">{} {n}</a></li>\n",
            r.hex(6),
            capitalise(a)
        ));
    }
    s.push_str("</ul>\n</section>\n");
}

fn table(r: &mut Rng, s: &mut String) {
    s.push_str(&format!(
        "<section>\n<h2>{}</h2>\n<table>\n<thead><tr><th>Item</th><th>Count</th><th>Share</th></tr></thead>\n<tbody>\n",
        title(r)
    ));
    for _ in 0..r.range(3, 5) {
        s.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}.{}%</td></tr>\n",
            capitalise(r.pick(NOUNS)),
            r.below(5_000),
            r.below(100),
            r.below(10)
        ));
    }
    s.push_str("</tbody>\n</table>\n</section>\n");
}

fn figure(r: &mut Rng, s: &mut String) {
    let (w, h) = (r.range(2, 12) * 40, r.range(2, 9) * 40);
    s.push_str(&format!(
        "<figure>\n<img src=\"/img/{}.png\" width=\"{w}\" height=\"{h}\" alt=\"{} {}\">\n<figcaption>{}</figcaption>\n</figure>\n",
        r.hex(10),
        r.pick(ADJECTIVES),
        r.pick(NOUNS),
        title(r)
    ));
}

fn quote(r: &mut Rng, s: &mut String) {
    let Some(en) = lang("en") else { return };
    let mut q = String::new();
    en.sentence(r, &mut q);
    s.push_str(&format!(
        "<blockquote>\n<p>{q}</p>\n<footer><a href=\"https://www.example.test/{}\">{}</a></footer>\n</blockquote>\n",
        r.hex(8),
        title(r)
    ));
}

/// Returns a page of at least `size` bytes.
pub(crate) fn generate(r: &mut Rng, size: usize) -> Vec<u8> {
    let page_title = title(r);
    let mut s = String::with_capacity(size + 1024);
    s.push_str(&format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{page_title}</title>\n<link rel=\"stylesheet\" href=\"/assets/{}.css\">\n</head>\n<body class=\"page-{}\">\n<header>\n<h1>{page_title}</h1>\n<nav>\n<ul>\n",
        r.hex(8),
        r.pick(NOUNS)
    ));
    for _ in 0..r.range(3, 5) {
        let n = r.pick(NOUNS);
        s.push_str(&format!(
            "<li><a href=\"/{n}\">{}</a></li>\n",
            capitalise(n)
        ));
    }
    s.push_str("</ul>\n</nav>\n</header>\n<main>\n");
    let footer = format!(
        "</main>\n<footer>\n<p>Synthetic page {} for traffic tests. <a href=\"https://www.example.test/about\">About</a></p>\n</footer>\n</body>\n</html>\n",
        r.hex(8)
    );
    while s.len() + footer.len() < size {
        match r.below(10) {
            0..=3 => paragraphs(r, &mut s),
            4..=5 => list(r, &mut s),
            6..=7 => table(r, &mut s),
            8 => figure(r, &mut s),
            _ => quote(r, &mut s),
        }
    }
    s.push_str(&footer);
    s.into_bytes()
}
