//! TOML to [`CircuitFile`], with positions.

use crate::{CircuitError, CircuitFile, FORMAT_VERSION};

/// Parse a circuit file and run the structural checks
/// ([`diagnostics`](crate::diagnostics)); returns the first problem in file order.
pub fn parse(src: &str) -> Result<CircuitFile, CircuitError> {
    let file = parse_unchecked(src)?;
    crate::validate::validate(src, &file)?;
    Ok(file)
}

/// Parse the TOML and check the format version only. Use this to repair a
/// file (for example [`update_hashes`](crate::library::update_hashes) on a
/// placeholder hash) before the structural checks run.
pub fn parse_unchecked(src: &str) -> Result<CircuitFile, CircuitError> {
    let file: CircuitFile = toml::from_str(src).map_err(|e| {
        let span = e.span().unwrap_or(0..0);
        CircuitError::at(src, span, e.message().trim_end().to_owned())
    })?;
    let v = *file.format.get_ref();
    if v == 0 || v > FORMAT_VERSION {
        return Err(CircuitError::at(
            src,
            file.format.span(),
            format!("format {v} is not supported (this Ohmforge supports {FORMAT_VERSION})"),
        ));
    }
    Ok(file)
}
