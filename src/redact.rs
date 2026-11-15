//! Redaction of credentials embedded in repository URLs.
//!
//! Restic accepts repository URLs that carry credentials inline, e.g.
//! `sftp:user:password@host:/path` or
//! `s3:https://ACCESSKEY:SECRETKEY@endpoint/bucket`. When such a
//! repository is unreachable, restic echoes the *full* URL - including
//! the password - in its stderr, and that stderr is both written to the
//! logs and forwarded to the Telegram chat.
//!
//! `redact_credentials` rewrites every URL-ish token in a piece of text
//! so that the userinfo component (everything between the scheme and
//! the `@`) is replaced by `***`, while the rest of the text is left
//! untouched.

/// Replacement used in place of a URL's userinfo component.
const REDACTED: &str = "***";

/// Text that is guaranteed to have had any embedded repository
/// credentials redacted.
///
/// The only way to build one is [`RedactedText::new`], which redacts on
/// the way in. Holding this type instead of a `String` means a password
/// cannot reach an error, a log line, or a notification by accident:
/// there is exactly one place where redaction happens, and it happens
/// before the text can be shown to anyone.
pub struct RedactedText(String);

impl RedactedText {
    /// Redact any credentials embedded in `text`.
    pub fn new(text: &str) -> Self {
        Self(redact_credentials(text))
    }

    /// The redacted text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RedactedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// `Debug` prints the same redacted value as `Display`, so formatting an
// error with `{:?}` cannot reveal what `{}` would hide.
impl std::fmt::Debug for RedactedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

/// Returns true if `text` contains a URL with an embedded userinfo
/// component (i.e. `scheme:[//]something@host`).
pub fn contains_credentials(text: &str) -> bool {
    token_ranges(text).iter().any(|&(start, end)| {
        redact_token(&text[start..end])
            .map(|redacted| redacted != text[start..end])
            .unwrap_or(false)
    })
}

/// Redact credentials from every URL found in `text`.
///
/// Text that isn't a URL is returned unchanged. Within a URL, only the
/// userinfo component is replaced - the scheme, host and path are kept
/// so the message stays useful for debugging.
pub fn redact_credentials(text: &str) -> String {
    let ranges = token_ranges(text);
    if ranges.is_empty() {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for &(start, end) in &ranges {
        out.push_str(&text[cursor..start]);
        let token = &text[start..end];
        match redact_token(token) {
            Some(redacted) => out.push_str(&redacted),
            None => out.push_str(token),
        }
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Byte ranges of the whitespace-separated tokens in `text`.
fn token_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                ranges.push((s, idx));
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(s) = start {
        ranges.push((s, text.len()));
    }
    ranges
}

/// Redact the userinfo component of a single URL-ish token.
///
/// Returns `None` if the token doesn't look like a URL with credentials,
/// in which case it should be used verbatim.
fn redact_token(token: &str) -> Option<String> {
    let scheme_end = scheme_end(token)?;
    let mut userinfo_start = scheme_end;

    let rest = &token[userinfo_start..];
    if let Some(after_slashes) = rest.strip_prefix("//") {
        userinfo_start += 2;
        let _ = after_slashes;
    } else if let Some(idx) = rest.find("://") {
        // e.g. `rest:https://user:pass@host/repo`
        userinfo_start += idx + 3;
    }

    let at = token[userinfo_start..].find('@')? + userinfo_start;

    // A `/` before the `@` means the `@` belongs to a path segment,
    // not to a userinfo component - leave the token alone.
    if token[userinfo_start..at].contains('/') {
        return None;
    }

    // An empty userinfo (`scheme:@host`) carries no credentials.
    if at == userinfo_start {
        return None;
    }

    let mut out = String::with_capacity(token.len());
    out.push_str(&token[..userinfo_start]);
    out.push_str(REDACTED);
    out.push_str(&token[at..]);
    Some(out)
}

/// Byte index just past a `scheme:` prefix within `token`, if it has one.
///
/// The scheme does not have to start the token: restic and the tools
/// around it may quote the URL (`"sftp:user:pass@host"`) or prefix it
/// with a key (`url=sftp:user:pass@host`). Those shapes still carry the
/// credentials, so the token is scanned for the first colon whose
/// preceding characters form a valid scheme.
fn scheme_end(token: &str) -> Option<usize> {
    for (colon, ch) in token.char_indices() {
        if ch != ':' {
            continue;
        }
        // Walk back over the run of scheme characters immediately
        // before the colon; whatever precedes that run ends the scheme.
        let start = token[..colon]
            .char_indices()
            .rev()
            .find(|(_, c)| !is_scheme_char(*c))
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);

        let scheme = &token[start..colon];
        if scheme.is_empty() {
            continue;
        }
        let mut chars = scheme.chars();
        let first = chars.next()?;
        if !first.is_ascii_alphabetic() {
            continue;
        }
        if !chars.all(is_scheme_char) {
            continue;
        }
        return Some(colon + 1);
    }
    None
}

/// Whether `c` may appear in a URI scheme (RFC 3986: alphanumeric, `+`,
/// `-`, `.`).
fn is_scheme_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redacts_sftp_password() {
        let text = "Fatal: unable to open repository at sftp:user:SECRETPASS@host.invalid:/x";
        let redacted = redact_credentials(text);
        assert!(!redacted.contains("SECRETPASS"), "{redacted}");
        assert!(redacted.contains("host.invalid"), "{redacted}");
    }

    #[test]
    fn test_redacts_https_userinfo() {
        let text = "failed for https://user:hunter2@example.com/repo";
        let redacted = redact_credentials(text);
        assert!(!redacted.contains("hunter2"), "{redacted}");
        assert!(redacted.contains("example.com"), "{redacted}");
    }

    #[test]
    fn test_redacts_nested_scheme() {
        let text = "repo rest:https://AK:SECRET@endpoint/bucket failed";
        let redacted = redact_credentials(text);
        assert!(!redacted.contains("SECRET"), "{redacted}");
        assert!(redacted.contains("endpoint/bucket"), "{redacted}");
    }

    #[test]
    fn test_leaves_plain_text_unchanged() {
        let text = "Fatal: unable to start the sftp session";
        assert_eq!(redact_credentials(text), text);
    }

    #[test]
    fn test_redacts_quoted_url() {
        // A quoted URL is still a URL: the password must not survive
        // just because the token starts with a quote character.
        let text = r#"could not open "sftp:user:SECRETPASS@host.invalid:/x""#;
        let redacted = redact_credentials(text);
        assert!(!redacted.contains("SECRETPASS"), "{redacted}");
        assert!(redacted.contains("host.invalid"), "{redacted}");
    }

    #[test]
    fn test_redacts_key_value_prefixed_url() {
        // Likewise for a `key=value` shaped token.
        let text = "url=sftp:user:SECRETPASS@host.invalid:/x";
        let redacted = redact_credentials(text);
        assert!(!redacted.contains("SECRETPASS"), "{redacted}");
        assert!(redacted.contains("host.invalid"), "{redacted}");
    }

    #[test]
    fn test_redacted_text_hides_credentials() {
        let text = RedactedText::new("sftp:user:SECRETPASS@host.invalid:/x");
        assert!(!text.to_string().contains("SECRETPASS"), "{text}");
        assert!(!format!("{text:?}").contains("SECRETPASS"), "{text:?}");
        assert!(text.as_str().contains("host.invalid"), "{text}");
    }

    #[test]
    fn test_redacted_text_of_plain_text_is_unchanged() {
        let text = RedactedText::new("nothing to redact here");
        assert_eq!(text.as_str(), "nothing to redact here");
    }

    #[test]
    fn test_leaves_local_path_unchanged() {
        let text = "/var/lib/restic/repository";
        assert_eq!(redact_credentials(text), text);
    }

    #[test]
    fn test_leaves_email_like_token_unchanged() {
        // No scheme, so the `@` isn't a userinfo marker.
        let text = "contact someone@example.com";
        assert_eq!(redact_credentials(text), text);
    }

    #[test]
    fn test_contains_credentials_detection() {
        assert!(contains_credentials("sftp:user:SECRETPASS@host.invalid:/x"));
        assert!(!contains_credentials("/var/lib/restic/repository"));
        assert!(!contains_credentials("plain error message"));
    }
}
