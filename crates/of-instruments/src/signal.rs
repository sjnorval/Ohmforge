//! Signal names (RFC 0004): anything the measurement service can sample.
//!
//! The grammar is the `[[probe]] signal` string of a circuit file:
//!
//! | Form | Meaning |
//! |---|---|
//! | `v(net)` | voltage of `net` to the circuit ground |
//! | `v(a,b)` | `V(a) − V(b)` |
//! | `i(R1)` | current into pin 1 of a two-pin component (from pin 1 to pin 2 through it) |
//! | `i(U1.3V3)` | current into the named pin of a component |
//! | `r(PSU1.volts)` | a named reading of a component (`Component::readings`) |
//! | `d(U1.GPIO8)` | the digital state of a component pin, from its own engine |
//!
//! Spaces around names are ignored; the canonical form ([`std::fmt::Display`])
//! has none, so a parse and print round trip is stable.

use std::fmt;
use std::str::FromStr;

/// Anything measurable, by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Signal {
    /// `v(net)`: net voltage to the circuit ground (V).
    NetVolts(String),
    /// `v(a,b)`: voltage between two nets, `V(a) − V(b)` (V).
    DiffVolts(String, String),
    /// `i(R1)` or `i(U1.3V3)`: current into a component pin (A). `pin: None`
    /// means the first pin of a two-pin part.
    PinCurrent {
        /// Component reference designator.
        refdes: String,
        /// Pin name, or `None` for a two-pin part.
        pin: Option<String>,
    },
    /// `r(PSU1.volts)`: a named component reading.
    Reading {
        /// Component reference designator.
        refdes: String,
        /// Reading name as the component reports it.
        name: String,
    },
    /// `d(U1.GPIO8)`: digital state of a component pin (0 or 1).
    Digital {
        /// Component reference designator.
        refdes: String,
        /// Pin name.
        pin: String,
    },
}

/// Why a signal string did not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseSignalError {
    /// Not of the form `x(...)`.
    #[error("expected v(..), i(..), r(..) or d(..), got `{0}`")]
    Shape(String),
    /// The prefix letter is not one of `v`, `i`, `r`, `d`.
    #[error("unknown signal kind `{0}` (expected v, i, r or d)")]
    Kind(String),
    /// A name inside the parentheses is empty or malformed.
    #[error("bad name in `{0}`")]
    Name(String),
}

impl FromStr for Signal {
    type Err = ParseSignalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let shape = || ParseSignalError::Shape(s.trim().to_owned());
        let bad = || ParseSignalError::Name(s.trim().to_owned());
        let t = s.trim();
        let open = t.find('(').ok_or_else(shape)?;
        let inner = t[open + 1..].strip_suffix(')').ok_or_else(shape)?;
        let kind = t[..open].trim().to_ascii_lowercase();
        let name = |x: &str| {
            let x = x.trim();
            (!x.is_empty() && !x.contains(['(', ')', ',', '.']))
                .then(|| x.to_owned())
                .ok_or_else(bad)
        };
        let dotted = |x: &str| -> Result<(String, String), ParseSignalError> {
            let (a, b) = x.split_once('.').ok_or_else(bad)?;
            Ok((name(a)?, name(b)?))
        };
        match kind.as_str() {
            "v" => match inner.split_once(',') {
                None => Ok(Self::NetVolts(name(inner)?)),
                Some((a, b)) => Ok(Self::DiffVolts(name(a)?, name(b)?)),
            },
            "i" if inner.contains('.') => {
                let (refdes, pin) = dotted(inner)?;
                Ok(Self::PinCurrent { refdes, pin: Some(pin) })
            }
            "i" => Ok(Self::PinCurrent {
                refdes: name(inner)?,
                pin: None,
            }),
            "r" => {
                let (refdes, name) = dotted(inner)?;
                Ok(Self::Reading { refdes, name })
            }
            "d" => {
                let (refdes, pin) = dotted(inner)?;
                Ok(Self::Digital { refdes, pin })
            }
            _ => Err(ParseSignalError::Kind(t[..open].trim().to_owned())),
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NetVolts(n) => write!(f, "v({n})"),
            Self::DiffVolts(a, b) => write!(f, "v({a},{b})"),
            Self::PinCurrent { refdes, pin: None } => write!(f, "i({refdes})"),
            Self::PinCurrent { refdes, pin: Some(p) } => write!(f, "i({refdes}.{p})"),
            Self::Reading { refdes, name } => write!(f, "r({refdes}.{name})"),
            Self::Digital { refdes, pin } => write!(f, "d({refdes}.{pin})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Signal {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn every_form_parses() {
        assert_eq!(p("v(vdd)"), Signal::NetVolts("vdd".into()));
        assert_eq!(p("v(gpio2, 0)"), Signal::DiffVolts("gpio2".into(), "0".into()));
        assert_eq!(
            p("i(R1)"),
            Signal::PinCurrent {
                refdes: "R1".into(),
                pin: None
            }
        );
        assert_eq!(
            p("i(U1.3V3)"),
            Signal::PinCurrent {
                refdes: "U1".into(),
                pin: Some("3V3".into())
            }
        );
        assert_eq!(
            p("r(PSU1.regulated_amps)"),
            Signal::Reading {
                refdes: "PSU1".into(),
                name: "regulated_amps".into()
            }
        );
        assert_eq!(
            p("d(U1.GPIO8)"),
            Signal::Digital {
                refdes: "U1".into(),
                pin: "GPIO8".into()
            }
        );
        assert_eq!(p(" V( vdd ) "), Signal::NetVolts("vdd".into()));
    }

    #[test]
    fn printing_is_canonical_and_round_trips() {
        for s in [
            "v(vdd)",
            "v(gpio2,0)",
            "i(R1)",
            "i(U1.3V3)",
            "r(PSU1.volts)",
            "d(U1.GPIO8)",
        ] {
            assert_eq!(p(s).to_string(), s);
            assert_eq!(p(&p(s).to_string()), p(s));
        }
        assert_eq!(p("v( a , b )").to_string(), "v(a,b)");
    }

    #[test]
    fn the_shipped_demo_probes_parse() {
        // The `[[probe]] signal` strings in circuits/demos (E4).
        assert!(matches!(p("i(R1)"), Signal::PinCurrent { .. }));
        assert!(matches!(p("i(R2)"), Signal::PinCurrent { .. }));
    }

    #[test]
    fn malformed_signals_are_rejected() {
        let e = |s: &str| s.parse::<Signal>().unwrap_err();
        assert!(matches!(e("vdd"), ParseSignalError::Shape(_)));
        assert!(matches!(e("v(vdd"), ParseSignalError::Shape(_)));
        assert!(matches!(e("x(vdd)"), ParseSignalError::Kind(_)));
        assert!(matches!(e("v()"), ParseSignalError::Name(_)));
        assert!(matches!(e("v(a,b,c)"), ParseSignalError::Name(_)));
        assert!(matches!(e("v(a,)"), ParseSignalError::Name(_)));
        assert!(matches!(e("i(R1,R2)"), ParseSignalError::Name(_)));
        assert!(matches!(e("i(U1.)"), ParseSignalError::Name(_)));
        assert!(matches!(e("r(PSU1)"), ParseSignalError::Name(_)));
        assert!(matches!(e("d(U1)"), ParseSignalError::Name(_)));
        assert!(matches!(e("d(U1.a.b)"), ParseSignalError::Name(_)));
    }
}
