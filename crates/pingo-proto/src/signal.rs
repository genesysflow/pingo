//! Signaling over the `/rtc` WebSocket (JSON text frames) and ephemeral messages over the WebRTC data channel.
//!
//! Flow:
//! 1. client opens `wss://host/rtc`, sends `Join { token }`
//! 2. server replies `Joined { participant_id, participants, tracks }`
//! 3. client sends `Offer { sdp }` with its send tracks (audio, optionally camera/screen) and a data channel
//!    named `"pingo"`; server replies `Answer { sdp }`
//! 4. whenever the set of forwarded tracks changes, the SERVER sends `Offer` and the client replies `Answer`
//!    (only one offer in flight per side; if both collide the client's offer wins and the server retries after)
//! 5. `TrackPublished` / `TrackUnpublished` tell the client which participant and kind each `mid` belongs to
//!
//! ICE candidates are carried in the SDP (no trickle) to keep this simple.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackKind {
    /// Microphone (Opus).
    Audio,
    Camera,
    Screen,
    /// System/app audio captured with a screen share (stereo Opus).
    ScreenAudio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantInfo {
    pub participant_id: String,
    pub user: String,
    pub name: String,
    pub color: String,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub participant_id: String,
    /// SDP media id of the track in the receiving client's session.
    pub mid: String,
    pub kind: TrackKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ClientMsg {
    Join { token: String },
    Offer { sdp: String },
    Answer { sdp: String },
    /// Declares what the client's send-side mids carry (sent right after its Offer).
    PublishTracks { tracks: Vec<PublishedTrack> },
    SetState { muted: bool, deafened: bool },
    /// Subscriber preference for a video track: the SFU picks the simulcast layer that fits.
    SetVideoQuality { mid: String, max_height: u32 },
    Ping,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedTrack {
    pub mid: String,
    pub kind: TrackKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerMsg {
    Joined { participant_id: String, participants: Vec<ParticipantInfo> },
    Offer { sdp: String },
    Answer { sdp: String },
    ParticipantJoined { participant: ParticipantInfo },
    ParticipantLeft { participant_id: String },
    ParticipantUpdated { participant: ParticipantInfo },
    TrackPublished { track: TrackInfo },
    TrackUnpublished { participant_id: String, mid: String },
    Error { code: String, message: String },
    Pong,
}

/// Ephemeral, high-frequency messages on the `"pingo"` data channel. Never stored in the database.
/// Clients send these to the SFU, which stamps `participant_id` and relays them to everyone else in the room.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DataMsg {
    /// Sent by the SFU (from RFC 6464 audio levels) about 5x per second for active speakers. level: 0.0..=1.0
    Speaking { participant_id: String, level: f32 },
    Reaction { participant_id: String, emoji: String },
    RaiseHand { participant_id: String, raised: bool },
    /// Pointer on a screen share, normalized 0..1 coordinates. `None` hides it.
    Pointer { participant_id: String, share_mid: String, pos: Option<(f32, f32)> },
    /// Freehand annotation stroke on a screen share, normalized points; fades out client-side.
    Stroke { participant_id: String, share_mid: String, points: Vec<(f32, f32)>, color: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_is_tagged_camel_case() {
        let m = ClientMsg::SetVideoQuality { mid: "2".into(), max_height: 720 };
        assert_eq!(serde_json::to_string(&m).unwrap(), r#"{"type":"setVideoQuality","mid":"2","maxHeight":720}"#);
        let s: ServerMsg = serde_json::from_str(r#"{"type":"participantLeft","participantId":"p1"}"#).unwrap();
        assert_eq!(s, ServerMsg::ParticipantLeft { participant_id: "p1".into() });
    }
}
