// SPDX-License-Identifier: GPL-3.0-only
//! The bitrate ladder, which is a list of rungs, and the plans that pick a
//! rung for each segment.

/// The most rungs that a ladder may have.
pub const MAX_RUNGS: usize = 16;

/// One rendition of the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rung {
    /// Media bit rate in bits per second, which is the rate of the whole transport stream.
    pub bits_per_second: u64,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
}

/// Parses `bits_per_second@WIDTHxHEIGHT,...`.
pub fn parse(text: &str) -> Result<Vec<Rung>, String> {
    let mut rungs = Vec::new();
    for item in text.split(',') {
        let item = item.trim();
        let shape = || format!("rung '{item}' must look like 800000@960x540");
        let (rate, size) = item.split_once('@').ok_or_else(shape)?;
        let (w, h) = size.split_once('x').ok_or_else(shape)?;
        let number = |s: &str| {
            s.parse::<u64>()
                .map_err(|_| format!("rung '{item}' has the non-number '{s}'"))
        };
        let bits_per_second = number(rate)?;
        let width = number(w)?;
        let height = number(h)?;
        if !(8_000..=200_000_000).contains(&bits_per_second) {
            return Err(format!(
                "rung '{item}' must have a bit rate between 8000 and 200000000"
            ));
        }
        if !(16..=8192).contains(&width) || !(16..=8192).contains(&height) {
            return Err(format!(
                "rung '{item}' must have a width and a height between 16 and 8192"
            ));
        }
        rungs.push(Rung {
            bits_per_second,
            width: width as u32,
            height: height as u32,
        });
        if rungs.len() > MAX_RUNGS {
            return Err(format!("a ladder has at most {MAX_RUNGS} rungs"));
        }
    }
    Ok(rungs)
}

/// Returns the rung with the index `variant`.
pub fn rung(rungs: &[Rung], variant: i64) -> Result<Rung, String> {
    usize::try_from(variant)
        .ok()
        .and_then(|i| rungs.get(i).copied())
        .ok_or_else(|| {
            format!(
                "variant {variant} is not a rung of a ladder with {} rungs",
                rungs.len()
            )
        })
}

/// The rung that the plan `fixed:I` or `ramp:K` picks for segment `n`.
pub fn pick(rungs: &[Rung], plan: &str, n: i64) -> Result<usize, String> {
    if n < 0 {
        return Err(format!("segment number {n} is negative"));
    }
    let top = rungs.len().saturating_sub(1);
    let shape = || format!("plan '{plan}' must be fixed:RUNG or ramp:SEGMENTS");
    let (kind, arg) = plan.split_once(':').ok_or_else(shape)?;
    let arg: usize = arg.parse().map_err(|_| shape())?;
    match kind {
        "fixed" if arg <= top => Ok(arg),
        "fixed" => Err(format!(
            "plan '{plan}' names rung {arg} of a ladder with {} rungs",
            rungs.len()
        )),
        "ramp" if arg >= 1 => Ok((n as usize / arg).min(top)),
        "ramp" => Err(format!("plan '{plan}' needs at least one segment per rung")),
        _ => Err(shape()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_ladder() {
        let r = parse("400000@640x360, 800000@960x540").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].bits_per_second, 800_000);
        assert_eq!((r[1].width, r[1].height), (960, 540));
    }

    #[test]
    fn rejects_bad_ladders() {
        for bad in [
            "",
            "800000",
            "800000@960",
            "x@1x1",
            "100@640x360",
            "800000@2x2",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let long = vec!["800000@640x360"; MAX_RUNGS + 1].join(",");
        assert!(parse(&long).is_err());
    }

    #[test]
    fn plans_pick_rungs() {
        let r = parse("200000@426x240,400000@640x360,800000@960x540").unwrap();
        assert_eq!(pick(&r, "fixed:1", 99).unwrap(), 1);
        let ramp: Vec<usize> = (0..8).map(|n| pick(&r, "ramp:3", n).unwrap()).collect();
        assert_eq!(ramp, [0, 0, 0, 1, 1, 1, 2, 2]);
        assert!(pick(&r, "fixed:3", 0).is_err());
        assert!(pick(&r, "ramp:0", 0).is_err());
        assert!(pick(&r, "walk", 0).is_err());
        assert!(pick(&r, "ramp:2", -1).is_err());
        assert!(rung(&r, 3).is_err());
        assert!(rung(&r, -1).is_err());
    }
}
