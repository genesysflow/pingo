//! The SMTP URL as hosting platforms hand it over.
//!
//! RxFerrix reads `RXFERRIX_SMTP_URL` from the environment and passes it to lettre. Two things
//! platforms do get in the way:
//!
//! - Platforms that keep secrets out of the environment (Docker and Kubernetes secrets, genesys)
//!   mount the URL as a file and name it in `RXFERRIX_SMTP_URL_FILE`.
//! - `smtp://host:587` means STARTTLS to most mail tools, but lettre speaks plain text on it
//!   unless the URL says `?tls=required`, and a submission server will not take a password
//!   without TLS.
//!
//! [`prepare_env`] settles both before RxFerrix reads the variable.

pub const SMTP_URL: &str = "RXFERRIX_SMTP_URL";
pub const SMTP_URL_FILE: &str = "RXFERRIX_SMTP_URL_FILE";

/// Port 587, the mail submission port, where the client starts TLS with STARTTLS (RFC 6409).
const SUBMISSION_PORT: u16 = 587;

/// Puts the SMTP URL where RxFerrix looks for it: from `RXFERRIX_SMTP_URL`, or else the file
/// named by `RXFERRIX_SMTP_URL_FILE`, with STARTTLS required on the submission port.
///
/// Call it at the top of `main`, before any thread starts: it sets a process environment variable.
pub fn prepare_env() -> Result<(), String> {
    let url = match std::env::var(SMTP_URL).ok().filter(|v| !v.is_empty()) {
        Some(url) => url,
        None => match std::env::var_os(SMTP_URL_FILE).filter(|v| !v.is_empty()) {
            Some(path) => std::fs::read_to_string(&path)
                .map_err(|e| format!("{SMTP_URL_FILE}: {}: {e}", path.to_string_lossy()))?
                .trim()
                .to_string(),
            None => return Ok(()),
        },
    };
    let url = require_starttls(&url);
    // SAFETY: called from `main` before RxFerrix builds its runtime, so no other thread reads
    // the environment yet.
    unsafe { std::env::set_var(SMTP_URL, url) };
    Ok(())
}

/// `smtp://…:587` without a `tls` parameter gets `tls=required`. Every other URL, including one
/// that asks for plain text with `tls=` of its own, stays as it is. The URL is appended to rather
/// than rebuilt, so its percent-encoded user and password reach lettre unchanged.
pub fn require_starttls(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return url.to_string();
    };
    let submission = parsed.scheme() == "smtp" && parsed.port() == Some(SUBMISSION_PORT);
    if !submission || parsed.query_pairs().any(|(k, _)| k == "tls") {
        return url.to_string();
    }
    let separator = match parsed.query() {
        None => "?",
        Some("") => "",
        Some(_) => "&",
    };
    format!("{url}{separator}tls=required")
}

#[cfg(test)]
mod tests {
    use super::require_starttls;

    #[test]
    fn submission_port_requires_starttls() {
        assert_eq!(
            require_starttls("smtp://noreply%40example.com:k3y%2Bx@mx.example.com:587"),
            "smtp://noreply%40example.com:k3y%2Bx@mx.example.com:587?tls=required"
        );
        assert_eq!(
            require_starttls("smtp://u:p@mx.example.com:587/client.example.com?x=1"),
            "smtp://u:p@mx.example.com:587/client.example.com?x=1&tls=required"
        );
        assert_eq!(
            require_starttls("smtp://u:p@mx.example.com:587?"),
            "smtp://u:p@mx.example.com:587?tls=required"
        );
    }

    #[test]
    fn other_urls_stay_as_they_are() {
        for url in [
            "smtps://u:p@smtp.example.com:465",
            "smtp://u:p@mx.example.com:587?tls=opportunistic",
            "smtp://localhost:1025",
            "smtp://u:p@mx.example.com",
            "not a url",
        ] {
            assert_eq!(require_starttls(url), url);
        }
    }
}
