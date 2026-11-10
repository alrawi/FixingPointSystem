// SPDX-License-Identifier: GPL-3.0-only
//! Human readable dump of compiled templates for golden tests.
//!
//! The format lists one segment per line. Arguments follow their call on
//! deeper lines, core function arguments under their parameter names and
//! module call arguments as `#<index>` or `<keyword>`.
//!
//! ```text
//! dynamic, 2 segments
//! [0] lit 6 "Host: "
//! [1] call http1.response
//!     #0 = int 200
//!     body = fill
//!         n = vars.size
//! ```

use std::fmt::Write;

use super::{Const, Expr, Seg, Template};

const STEP: usize = 4;

impl Template {
    /// Returns the dump of the template, ending with a newline.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.write_dump(&mut out, 0);
        out
    }

    /// Appends the dump to `out`, with every line indented by `indent` spaces.
    pub fn write_dump(&self, out: &mut String, indent: usize) {
        let kind = if self.is_static() {
            "constant"
        } else {
            "dynamic"
        };
        let n = self.segs.len();
        let plural = if n == 1 { "" } else { "s" };
        line(out, indent, format_args!("{kind}, {n} segment{plural}"));
        for (i, seg) in self.segs.iter().enumerate() {
            match seg {
                Seg::Lit(b) => line(
                    out,
                    indent,
                    format_args!("[{i}] lit {} \"{}\"", b.len(), b.escape_ascii()),
                ),
                Seg::Expr(e) => write_expr(out, indent, &format!("[{i}] "), e),
            }
        }
    }
}

fn line(out: &mut String, indent: usize, args: std::fmt::Arguments<'_>) {
    // Writing into a String cannot fail.
    let _ = writeln!(out, "{:indent$}{args}", "");
}

fn write_expr(out: &mut String, indent: usize, prefix: &str, e: &Expr) {
    let child = indent + STEP;
    match e {
        Expr::Const(c) => line(out, indent, format_args!("{prefix}{}", constant(c))),
        Expr::Var(n) => line(out, indent, format_args!("{prefix}vars.{n}")),
        Expr::SessionId => line(out, indent, format_args!("{prefix}session.id")),
        Expr::SessionFlow => line(out, indent, format_args!("{prefix}session.flow")),
        Expr::Last => line(out, indent, format_args!("{prefix}last")),
        Expr::Fill(n) => {
            line(out, indent, format_args!("{prefix}fill"));
            write_expr(out, child, "n = ", n);
        }
        Expr::Hex(s) => {
            line(out, indent, format_args!("{prefix}hex"));
            write_expr(out, child, "s = ", s);
        }
        Expr::RandInt(a, b) => {
            line(out, indent, format_args!("{prefix}rand_int"));
            write_expr(out, child, "a = ", a);
            write_expr(out, child, "b = ", b);
        }
        Expr::IntBytes(value, width, order) => {
            line(out, indent, format_args!("{prefix}int_bytes"));
            write_expr(out, child, "value = ", value);
            write_expr(out, child, "width = ", width);
            write_expr(out, child, "endianness = ", order);
        }
        Expr::Now(format) => {
            line(out, indent, format_args!("{prefix}now"));
            write_expr(out, child, "format = ", format);
        }
        Expr::Profile(n) => line(out, indent, format_args!("{prefix}profiles.{n}")),
        Expr::Dist(d, args) => {
            line(out, indent, format_args!("{prefix}{}", d.qualified()));
            for (param, arg) in d.params().iter().zip(args) {
                write_expr(out, child, &format!("{} = ", param.name), arg);
            }
        }
        Expr::Call(c) => {
            line(
                out,
                indent,
                format_args!("{prefix}call {}.{}", c.module, c.function),
            );
            for (i, arg) in c.positional.iter().enumerate() {
                write_expr(out, child, &format!("#{i} = "), arg);
            }
            for (k, arg) in &c.named {
                write_expr(out, child, &format!("{k} = "), arg);
            }
        }
    }
}

fn constant(c: &Const) -> String {
    match c {
        Const::Int(i) => format!("int {i}"),
        Const::Float(f) => format!("float {f:?}"),
        Const::Bool(b) => format!("boolean {b}"),
        Const::Text(s) => format!("text \"{}\"", s.as_bytes().escape_ascii()),
        Const::Bytes(b) => {
            let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
            format!("bytes {} {hex}", b.len())
        }
    }
}
