//! Lossless csv for the columns that hold a list.
//!
//! `order_versions.payment_methods` is a csv column (`docs/SPEC.md` §4), but
//! the values it holds are free text an instance chose: the captured corpus
//! already contains methods like
//! `Sabadell Instant Money. Códigos al teléfono del comprador.`, and `pm`
//! carries them as separate tag values, so nothing stops one from containing a
//! comma.
//!
//! A plain `join(",")` would make that comma indistinguishable from the
//! separator, and the read back would report one method as two — corrupting
//! both the stored version and the payment-method statistics of SPEC §6.4.
//! So this is csv in the RFC 4180 sense rather than "text with commas in it":
//! a value that would otherwise be ambiguous is quoted, and a quote inside a
//! quoted value is doubled.

/// Encodes `values` as one csv field per value.
///
/// A value is quoted when it contains a comma or a quote, and also when it has
/// leading or trailing whitespace — the parser keeps such a value verbatim
/// (only blank ones are rejected), so the padding is part of what an instance
/// published and unquoted csv would silently trim it away.
pub(crate) fn join(values: &[String]) -> String {
    values
        .iter()
        .map(|value| {
            if needs_quoting(value) {
                format!("\"{}\"", value.replace('"', "\"\""))
            } else {
                value.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Decodes what [`join`] wrote.
///
/// Blank fields are dropped rather than kept as empty values: an empty column
/// is no methods at all, and `"".split(',')` would otherwise yield a single
/// method named after nothing.
pub(crate) fn split(csv: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut inside_quotes = false;
    let mut characters = csv.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '"' if inside_quotes => {
                // A doubled quote is one literal quote; a lone one closes the
                // field.
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    inside_quotes = false;
                }
            }
            // A quote only opens a field at its start; anywhere else it is
            // just a character an instance happened to publish.
            '"' if field.is_empty() && !quoted => {
                quoted = true;
                inside_quotes = true;
            }
            ',' if !inside_quotes => {
                push(&mut values, std::mem::take(&mut field), quoted);
                quoted = false;
            }
            _ => field.push(character),
        }
    }
    push(&mut values, field, quoted);

    values
}

/// Whether writing `value` unquoted would change what reading it back yields.
fn needs_quoting(value: &str) -> bool {
    value.contains(',') || value.contains('"') || value.trim() != value
}

/// Adds one decoded field, unless it carries nothing.
///
/// Only an unquoted field is trimmed: quoting is what says the padding was
/// deliberate.
fn push(values: &mut Vec<String>, field: String, quoted: bool) {
    let value = if quoted {
        field
    } else {
        field.trim().to_string()
    };

    if !value.is_empty() {
        values.push(value);
    }
}

#[cfg(test)]
mod tests;
