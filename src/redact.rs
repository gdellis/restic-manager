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

/// Byte index just past the `scheme:` prefix of `token`, if it has one.
fn scheme_end(token: &str) -> Option<usize> {
    let colon = token.find(':')?;
    let scheme = &token[..colon];
    if scheme.is_empty() {
        return None;
    }
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return None;
    }
    Some(colon + 1)
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
