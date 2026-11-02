//! Compatibility between servers and desktop apps, which are released and updated independently.
//!
//! The browser UI always comes from the server it talks to, so it can't drift. The desktop app ships its own
//! UI and native media engine, so it can meet a server that is newer or older than itself. Each side
//! publishes the protocol it speaks and the oldest peer protocol it still supports; they work together when
//! each one's protocol is within the other's range.
//!
//! Bump [`PROTOCOL`] only for a change one side can't tolerate: a function in `docs/API.md` removed or changed
//! incompatibly, or a signaling message the other side would reject. Raise [`MIN_PROTOCOL`] when dropping the
//! compatibility code for an older peer.

use serde::{Deserialize, Serialize};

/// The protocol this build speaks.
pub const PROTOCOL: u32 = 1;
/// The oldest peer protocol this build still works with.
pub const MIN_PROTOCOL: u32 = 1;
/// Where a server publishes its [`VersionInfo`]. Outside `/api` because RxFerrix owns that prefix.
pub const VERSION_PATH: &str = "/.well-known/pingo";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// Release version, for messages only. Compatibility uses the protocol numbers.
    pub version: String,
    pub protocol: u32,
    pub min_protocol: u32,
}

impl VersionInfo {
    /// This build, with its package version.
    pub fn current(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            protocol: PROTOCOL,
            min_protocol: MIN_PROTOCOL,
        }
    }

    /// A server from before [`VERSION_PATH`] existed. Those all speak protocol 1.
    pub fn legacy() -> Self {
        Self {
            version: "unknown".into(),
            protocol: 1,
            min_protocol: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Compatibility {
    Compatible,
    /// The server no longer supports this app: update the app.
    ClientTooOld,
    /// This app no longer supports the server: the server's admin must update it.
    ServerTooOld,
}

pub fn compatibility(client: &VersionInfo, server: &VersionInfo) -> Compatibility {
    if client.protocol < server.min_protocol {
        Compatibility::ClientTooOld
    } else if server.protocol < client.min_protocol {
        Compatibility::ServerTooOld
    } else {
        Compatibility::Compatible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(protocol: u32, min_protocol: u32) -> VersionInfo {
        VersionInfo {
            version: format!("p{protocol}"),
            protocol,
            min_protocol,
        }
    }

    #[test]
    fn peers_within_each_others_range_are_compatible() {
        assert_eq!(
            compatibility(&side(1, 1), &side(1, 1)),
            Compatibility::Compatible
        );
        // A newer server that still supports protocol 1 clients, and the reverse.
        assert_eq!(
            compatibility(&side(1, 1), &side(3, 1)),
            Compatibility::Compatible
        );
        assert_eq!(
            compatibility(&side(3, 1), &side(1, 1)),
            Compatibility::Compatible
        );
    }

    #[test]
    fn the_side_that_fell_behind_is_named() {
        assert_eq!(
            compatibility(&side(1, 1), &side(3, 2)),
            Compatibility::ClientTooOld
        );
        assert_eq!(
            compatibility(&side(3, 2), &side(1, 1)),
            Compatibility::ServerTooOld
        );
    }

    #[test]
    fn current_build_accepts_servers_that_predate_the_endpoint() {
        let client = VersionInfo::current("0.1.0");
        assert_eq!(
            compatibility(&client, &VersionInfo::legacy()),
            Compatibility::Compatible
        );
    }

    #[test]
    fn wire_format_is_camel_case() {
        let json = serde_json::to_value(VersionInfo::current("1.2.3")).unwrap();
        assert_eq!(json["minProtocol"], MIN_PROTOCOL);
        assert_eq!(json["protocol"], PROTOCOL);
        assert_eq!(json["version"], "1.2.3");
        assert_eq!(
            serde_json::to_value(Compatibility::ClientTooOld).unwrap(),
            "clientTooOld"
        );
    }
}
