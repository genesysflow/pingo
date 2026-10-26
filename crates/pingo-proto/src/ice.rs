//! ICE configuration returned by admission. TURN credentials never travel over SFU signaling.
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::{fmt, net::Ipv6Addr};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
}
impl fmt::Debug for IceServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IceServer")
            .field("urls", &self.urls)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnTransport {
    Udp,
    Tcp,
    Tls,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnUrl {
    pub host: String,
    pub port: u16,
    pub transport: TurnTransport,
}

#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
#[error("expected turn:host[:port][?transport=udp|tcp] or turns:host[:port][?transport=tcp]")]
pub struct InvalidTurnUrl;

impl TurnUrl {
    pub fn parse(value: &str) -> Result<Self, InvalidTurnUrl> {
        if value.len() > 1024 {
            return Err(InvalidTurnUrl);
        }
        let (scheme, rest) = value.split_once(':').ok_or(InvalidTurnUrl)?;
        let tls = match scheme {
            "turn" => false,
            "turns" => true,
            _ => return Err(InvalidTurnUrl),
        };
        let (authority, transport) = rest.split_once('?').unwrap_or((rest, ""));
        let transport = match (tls, transport) {
            (true, "" | "transport=tcp") => TurnTransport::Tls,
            (false, "" | "transport=udp") => TurnTransport::Udp,
            (false, "transport=tcp") => TurnTransport::Tcp,
            _ => return Err(InvalidTurnUrl),
        };
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (host, suffix) = v6.split_once(']').ok_or(InvalidTurnUrl)?;
            host.parse::<Ipv6Addr>().map_err(|_| InvalidTurnUrl)?;
            (
                host,
                if suffix.is_empty() {
                    None
                } else {
                    Some(suffix.strip_prefix(':').ok_or(InvalidTurnUrl)?)
                },
            )
        } else {
            let (host, port) = authority
                .split_once(':')
                .map_or((authority, None), |(h, p)| (h, Some(p)));
            if host.is_empty()
                || !host
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
            {
                return Err(InvalidTurnUrl);
            }
            (host, port)
        };
        let port = match port {
            Some(p) if !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) => {
                p.parse::<u16>().map_err(|_| InvalidTurnUrl)?
            }
            Some(_) => return Err(InvalidTurnUrl),
            None => {
                if tls {
                    5349
                } else {
                    3478
                }
            }
        };
        if port == 0 {
            return Err(InvalidTurnUrl);
        }
        Ok(Self {
            host: host.into(),
            port,
            transport,
        })
    }
}

/// coturn's REST authentication requires standard base64 and HMAC-SHA1 (not media-token HMAC-SHA256).
pub fn turn_password(username: &str, secret: &[u8]) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(username.as_bytes());
    STANDARD.encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_turn_transports_and_ipv6() {
        assert_eq!(
            TurnUrl::parse("turn:relay.example.com").unwrap().transport,
            TurnTransport::Udp
        );
        assert_eq!(
            TurnUrl::parse("turn:relay.example.com:443?transport=tcp")
                .unwrap()
                .port,
            443
        );
        let tls = TurnUrl::parse("turns:[2001:db8::1]?transport=tcp").unwrap();
        assert_eq!(
            (tls.host.as_str(), tls.port, tls.transport),
            ("2001:db8::1", 5349, TurnTransport::Tls)
        );
        for bad in [
            "https://relay",
            "turn://relay",
            "turn:user:pass@relay",
            "turn:relay/path",
            "turn:relay:0",
            "turn:relay:+80",
            "turn:relay:",
            "turn:relay:65536",
            "turns:relay?transport=udp",
            "turn:relay?transport=tcp&secret=x",
            "turn:relay#x",
            "turn:[invalid]",
        ] {
            assert!(TurnUrl::parse(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn coturn_hmac_vector_and_redacted_debug() {
        assert_eq!(
            turn_password("The quick brown fox jumps over the lazy dog", b"key"),
            "3nybhbi3iqa8ino29wqQcBydtNk="
        );
        let server = IceServer {
            urls: vec!["turn:localhost".into()],
            username: "private-user".into(),
            credential: "private-password".into(),
        };
        let debug = format!("{server:?}");
        assert!(!debug.contains("private"));
    }
}
