//! Feed-text hygiene. Feed strings reach the TUI with two hazards that corrupt
//! the list render; both are fixed once, at ingest, via [`clean_feed_text`], so
//! the stored string is clean and every render site draws it as-is.
//!
//! 1. **Entities.** Feed strings arrive with HTML entities the XML layer does not
//!    resolve: numeric (`&#8217;`, `&#x2019;`) and named (`&amp;`, `&rsquo;`).
//!    Left alone they render as literal text. [`decode_entities`] resolves them;
//!    decoding is lossless (the entity *is* the character).
//!
//! 2. **Emoji width.** A ZWJ emoji sequence (e.g. the rainbow flag,
//!    `U+1F3F3 U+FE0F U+200D U+1F308`) has a rendered width the terminal and
//!    `unicode-width` disagree about, so ratatui's cell math drifts and stale
//!    glyph fragments ghost on the list rows. No terminal renders such a cluster
//!    uncorrupted, so [`sanitize_display`] drops only the joiners (ZWJ +
//!    variation selectors + skin-tone modifiers), leaving width-honest base
//!    scalars. Ordinary UTF-8 (accents, CJK, curly quotes) is never touched.
//!
//! [`truncate_display`] is the render-side companion: a grapheme- and
//! column-aware clip used where a fixed-width preview is built (the episode-card
//! description snippet).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Zero-width joiner: glues emoji scalars into one cluster.
const ZWJ: char = '\u{200D}';

/// Decode HTML entities in feed text, leaving anything unrecognized untouched.
///
/// Handles numeric decimal (`&#8217;`), numeric hex (`&#x2019;` / `&#X2019;`),
/// and a table of the named entities that actually show up in podcast feeds.
/// Unlike a strict XML unescape, an unknown entity (`&weird;`) is passed through
/// verbatim rather than failing the whole string; real feeds are messy, and a
/// bare `&` or a stray `&foo;` must not wipe out decoding for the rest of the
/// text.
pub fn decode_entities(input: &str) -> String {
    // Fast path: no entities at all.
    if !input.contains('&') {
        return input.to_string();
    }

    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            // Copy this whole UTF-8 char (may be multibyte).
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        // An entity body is `[A-Za-z0-9#]+` terminated by `;`. Scan only those
        // chars: a bare `&` (or `&` followed by anything non-body) is literal and
        // must not swallow a `;` from a later real entity (e.g. `X & Y &mdash;`).
        let rest = &input[i + 1..];
        let body_len = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'#')
            .count();
        if body_len > 0 && rest.as_bytes().get(body_len) == Some(&b';') {
            let body = &rest[..body_len];
            match decode_one(body) {
                Some(decoded) => out.push_str(&decoded),
                None => {
                    // Well-formed but unknown entity: keep it literal.
                    out.push('&');
                    out.push_str(body);
                    out.push(';');
                }
            }
            i += 1 + body_len + 1;
        } else {
            out.push('&');
            i += 1;
        }
    }
    out
}

/// Resolve the inside of an entity (`amp`, `#8217`, `#x2019`) to its string.
fn decode_one(body: &str) -> Option<String> {
    if let Some(num) = body.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        return char::from_u32(code).map(String::from);
    }
    named_entity(body).map(String::from)
}

/// The named HTML entities common in podcast titles/notes. Numeric refs cover the
/// long tail; this is just the handful of names feeds reach for by word.
fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{00A0}",
        "rsquo" => "\u{2019}",
        "lsquo" => "\u{2018}",
        "rdquo" => "\u{201D}",
        "ldquo" => "\u{201C}",
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "hellip" => "\u{2026}",
        "copy" => "\u{00A9}",
        "reg" => "\u{00AE}",
        "trade" => "\u{2122}",
        "deg" => "\u{00B0}",
        "eacute" => "\u{00E9}",
        "egrave" => "\u{00E8}",
        "agrave" => "\u{00E0}",
        "uuml" => "\u{00FC}",
        "ouml" => "\u{00F6}",
        "auml" => "\u{00E4}",
        "ccedil" => "\u{00E7}",
        "ntilde" => "\u{00F1}",
        "middot" => "\u{00B7}",
        "bull" => "\u{2022}",
        _ => return None,
    })
}

/// Normalize a feed string for storage: decode entities, then strip the emoji
/// joiners that break terminal width math. Applied once at ingest so every
/// render site draws clean, width-honest text without per-site handling. Both
/// steps are lossless for ordinary UTF-8; the only thing dropped is a ZWJ
/// emoji's exact glyph, which no terminal renders uncorrupted anyway.
pub fn clean_feed_text(input: &str) -> String {
    sanitize_display(&decode_entities(input))
}

/// Strip only the emoji joiners that make cluster width disagree with the
/// terminal: ZWJ, variation selectors (`U+FE00..=U+FE0F`), and skin-tone
/// modifiers (`U+1F3FB..=U+1F3FF`). Every other code point, including ordinary
/// emoji, accents, CJK, and curly quotes, is preserved. Used as the second half
/// of [`clean_feed_text`] at ingest.
pub fn sanitize_display(input: &str) -> String {
    if input.is_ascii() {
        return input.to_string();
    }
    input
        .chars()
        .filter(|&c| {
            c != ZWJ
                && !('\u{FE00}'..='\u{FE0F}').contains(&c)
                && !('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
        })
        .collect()
}

/// Truncate to at most `max_cols` display columns, appending an ellipsis when
/// clipped. Grapheme-aware (never splits a cluster) and column-aware (counts
/// terminal cells via `unicode-width`, not `char`s), so a wide glyph costs its
/// real width and a multi-scalar cluster is kept or dropped whole.
pub fn truncate_display(input: &str, max_cols: usize) -> String {
    if input.width() <= max_cols {
        return input.to_string();
    }
    // Reserve one column for the ellipsis.
    let budget = max_cols.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0usize;
    for g in input.graphemes(true) {
        let w = g.width();
        if used + w > budget {
            break;
        }
        out.push_str(g);
        used += w;
    }
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_numeric_decimal_and_hex() {
        assert_eq!(decode_entities("it&#8217;s"), "it\u{2019}s");
        assert_eq!(decode_entities("it&#x2019;s"), "it\u{2019}s");
        assert_eq!(decode_entities("it&#X2019;s"), "it\u{2019}s");
    }

    #[test]
    fn decodes_named_entities() {
        assert_eq!(decode_entities("Q&amp;A"), "Q&A");
        assert_eq!(decode_entities("a &mdash; b"), "a \u{2014} b");
        assert_eq!(decode_entities("caf&eacute;"), "caf\u{00E9}");
    }

    #[test]
    fn unknown_entity_is_left_literal() {
        // A stray entity-like token must not wipe the rest of the string.
        assert_eq!(decode_entities("a &weird; b &amp; c"), "a &weird; b & c");
    }

    #[test]
    fn bare_ampersand_survives() {
        assert_eq!(decode_entities("rock & roll"), "rock & roll");
        assert_eq!(decode_entities("A&B and &"), "A&B and &");
    }

    #[test]
    fn no_entities_is_identity() {
        assert_eq!(decode_entities("plain text"), "plain text");
        assert_eq!(
            decode_entities("caf\u{00E9} \u{65E5}\u{672C}"),
            "caf\u{00E9} \u{65E5}\u{672C}"
        );
    }

    #[test]
    fn sanitize_strips_zwj_and_variation_selectors() {
        // Rainbow flag: white flag + VS16 + ZWJ + rainbow. Joiners gone, the two
        // base emoji remain (each width-honest on its own).
        let flag = "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}";
        assert_eq!(sanitize_display(flag), "\u{1F3F3}\u{1F308}");
    }

    #[test]
    fn sanitize_strips_skin_tone_modifiers() {
        let wave = "\u{1F44B}\u{1F3FD}"; // waving hand + medium skin tone
        assert_eq!(sanitize_display(wave), "\u{1F44B}");
    }

    #[test]
    fn sanitize_preserves_ordinary_utf8() {
        let s = "caf\u{00E9} \u{2014} \u{65E5}\u{672C}\u{8A9E} \u{2018}curly\u{2019}";
        assert_eq!(sanitize_display(s), s);
    }

    #[test]
    fn truncate_is_column_and_grapheme_aware() {
        assert_eq!(truncate_display("hello", 10), "hello");
        // 8 columns total = 7 content columns + the ellipsis glyph.
        assert_eq!(truncate_display("hello world", 8), "hello w\u{2026}");
        // A wide CJK glyph costs two columns.
        let cjk = "\u{65E5}\u{672C}\u{8A9E}"; // 6 columns
        assert_eq!(truncate_display(cjk, 4).width(), 3); // 2 + ellipsis
    }

    #[test]
    fn truncate_keeps_short_multibyte_intact() {
        assert_eq!(truncate_display("caf\u{00E9}", 10), "caf\u{00E9}");
    }

    #[test]
    fn clean_feed_text_decodes_then_strips_joiners() {
        // Entity decode + ZWJ strip in one pass; plain text is untouched.
        let flag = "Pride \u{1F3F3}\u{FE0F}\u{200D}\u{1F308} &amp; joy";
        assert_eq!(clean_feed_text(flag), "Pride \u{1F3F3}\u{1F308} & joy");
        assert_eq!(clean_feed_text("plain"), "plain");
    }
}
