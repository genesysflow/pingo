use std::collections::HashSet;
use std::time::{Duration, Instant};

use pingo_proto::{DataMsg, MediaClaims, TrackKind};
use str0m::media::{MediaKind, MediaTime, Rid, SimulcastLayer};

#[derive(Default)]
pub(crate) struct FrameClock {
    last: Option<(Option<Rid>, MediaTime, MediaTime, Instant)>,
}

impl FrameClock {
    /// Simulcast encoders can have different RTP epochs. Preserve a monotonic
    /// subscriber clock across a layer switch and discard late/duplicate frames.
    pub fn map(&mut self, rid: Option<Rid>, input: MediaTime, now: Instant) -> Option<MediaTime> {
        let output = if let Some((previous_rid, previous_input, previous_output, at)) = self.last {
            let delta = if previous_rid == rid {
                input.checked_sub(previous_input)?
            } else {
                MediaTime::from_micros(now.saturating_duration_since(at).as_micros().max(1) as u64)
            }
            .rebase(input.frequency());
            if delta.is_zero() {
                return None;
            }
            MediaTime::new(
                previous_output
                    .rebase(input.frequency())
                    .numer()
                    .saturating_add(delta.numer()),
                input.frequency(),
            )
        } else {
            input
        };
        self.last = Some((rid, input, output, now));
        Some(output)
    }
}

pub(crate) fn media_kind(kind: TrackKind) -> MediaKind {
    match kind {
        TrackKind::Audio | TrackKind::ScreenAudio => MediaKind::Audio,
        TrackKind::Camera | TrackKind::Screen => MediaKind::Video,
    }
}

pub(crate) fn allowed(claims: &MediaClaims, kind: TrackKind) -> bool {
    match kind {
        TrackKind::Audio => claims.can_speak,
        TrackKind::ScreenAudio => claims.can_speak && claims.can_share,
        TrackKind::Camera => claims.can_video,
        TrackKind::Screen => claims.can_share,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Level {
    pub db: i8,
    pub at: Instant,
}

impl Level {
    pub fn active(self, now: Instant) -> bool {
        now.duration_since(self.at) < Duration::from_millis(600) && self.db > -60
    }

    pub fn normalized(self) -> f32 {
        ((f32::from(self.db) + 60.0) / 60.0).clamp(0.0, 1.0)
    }
}

#[derive(Default)]
pub(crate) struct Speakers {
    pub selected: HashSet<String>,
}

impl Speakers {
    /// An incumbent gets a 6 dB advantage; a challenger must be audibly louder.
    pub fn select(
        &mut self,
        levels: impl Iterator<Item = (String, Level)>,
        n: usize,
        now: Instant,
    ) {
        let mut ranked: Vec<_> = levels.filter(|(_, level)| level.active(now)).collect();
        ranked.sort_by(|(a, la), (b, lb)| {
            let score = |id: &str, l: Level| {
                i16::from(l.db) + if self.selected.contains(id) { 6 } else { 0 }
            };
            score(b, *lb).cmp(&score(a, *la)).then_with(|| a.cmp(b))
        });
        self.selected = ranked.into_iter().take(n).map(|(id, _)| id).collect();
    }
}

pub(crate) struct RateLimit {
    tokens: f64,
    at: Instant,
}

impl RateLimit {
    pub fn new(now: Instant) -> Self {
        Self {
            tokens: 60.0,
            at: now,
        }
    }

    pub fn allow(&mut self, now: Instant) -> bool {
        self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * 60.0).min(60.0);
        self.at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

/// Speaking is server-only. Validate bounded annotations before room fan-out.
pub(crate) fn stamp(mut message: DataMsg, id: &str) -> Option<DataMsg> {
    let valid_pos = |p: &(f32, f32)| (0.0..=1.0).contains(&p.0) && (0.0..=1.0).contains(&p.1);
    let participant_id = match &mut message {
        DataMsg::Speaking { .. } => return None,
        DataMsg::Reaction {
            participant_id,
            emoji,
        } if emoji.len() <= 128 => participant_id,
        DataMsg::RaiseHand { participant_id, .. } => participant_id,
        DataMsg::Pointer {
            participant_id,
            pos,
            share_mid,
        } if share_mid.len() <= 16 && pos.as_ref().is_none_or(valid_pos) => participant_id,
        DataMsg::Stroke {
            participant_id,
            points,
            color,
            share_mid,
        } if share_mid.len() <= 16
            && points.len() <= 256
            && color.len() <= 32
            && points.iter().all(valid_pos) =>
        {
            participant_id
        }
        _ => return None,
    };
    *participant_id = id.to_owned();
    Some(message)
}

#[derive(Clone, Debug)]
pub(crate) struct Layer {
    pub rid: Rid,
    pub height: u32,
    pub bitrate: u64,
}

pub(crate) fn layers(layers: &[SimulcastLayer], screen: bool) -> Vec<Layer> {
    let browser_rids = layers.iter().any(|l| l.rid == Rid::from("f"));
    let named_rids = layers.iter().any(|l| matches!(&*l.rid, "l" | "m"));
    let mut result: Vec<_> = layers
        .iter()
        .enumerate()
        .map(|(i, layer)| {
            let attr = |name| {
                layer
                    .attributes
                    .as_ref()?
                    .iter()
                    .find(|(key, _)| key == name)?
                    .1
                    .parse::<u32>()
                    .ok()
            };
            let index = match &*layer.rid {
                "q" if browser_rids => 0,
                "h" if browser_rids => 1,
                "f" if browser_rids => 2,
                "l" if named_rids => 0,
                "m" if named_rids => 1,
                "h" if named_rids => 2,
                _ => i,
            };
            let height = attr("max-height")
                .unwrap_or(if screen {
                    360 * 3u32.pow(index.min(2) as u32)
                } else {
                    180 * 2u32.pow(index.min(3) as u32)
                })
                .clamp(1, 16_384);
            Layer {
                rid: layer.rid,
                height,
                // Screen content spends its budget on detail at a lower frame rate.
                bitrate: u64::from(
                    attr("max-br").unwrap_or(height * if screen { 800 } else { 1800 }),
                ),
            }
        })
        .collect();
    result.sort_by_key(|l| l.height);
    result
}

pub(crate) fn choose_layer(layers: &[Layer], max_height: u32, budget: u64) -> Option<Rid> {
    if max_height == 0 {
        return None;
    }
    layers
        .iter()
        // A 600px view needs the 720p layer, rather than an upscaled 360p layer.
        .find(|l| l.height >= max_height && l.bitrate <= budget)
        .or_else(|| layers.iter().rev().find(|l| l.bitrate <= budget))
        .or_else(|| layers.first())
        .map(|l| l.rid)
}

/// Include the same transport reserve deducted when selecting layers, so full quality is reachable.
pub(crate) fn video_transport_budget(video_bitrate: u64) -> u64 {
    video_bitrate
        .saturating_mul(10)
        .div_ceil(8)
        .saturating_add(128_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_clock_survives_different_layer_epochs_and_late_frames() {
        let now = Instant::now();
        let mut clock = FrameClock::default();
        let a = clock
            .map(Some("q".into()), MediaTime::from_90khz(900_000), now)
            .unwrap();
        let b = clock
            .map(
                Some("h".into()),
                MediaTime::from_90khz(0),
                now + Duration::from_millis(33),
            )
            .unwrap();
        assert!(b > a);
        let c = clock
            .map(
                Some("h".into()),
                MediaTime::from_90khz(3000),
                now + Duration::from_millis(66),
            )
            .unwrap();
        assert!(c > b);
        assert!(
            clock
                .map(
                    Some("h".into()),
                    MediaTime::from_90khz(0),
                    now + Duration::from_millis(70)
                )
                .is_none()
        );
        assert!(
            clock
                .map(
                    Some("h".into()),
                    MediaTime::from_90khz(3000),
                    now + Duration::from_millis(75)
                )
                .is_none()
        );
    }

    #[test]
    fn speakers_require_margin_and_expire() {
        let now = Instant::now();
        let mut speakers = Speakers::default();
        let levels = |a, b, t| {
            vec![
                ("a".into(), Level { db: a, at: t }),
                ("b".into(), Level { db: b, at: t }),
            ]
            .into_iter()
        };
        speakers.select(levels(-20, -30, now), 1, now);
        assert!(speakers.selected.contains("a"));
        speakers.select(levels(-20, -16, now), 1, now);
        assert!(speakers.selected.contains("a"));
        speakers.select(levels(-20, -10, now), 1, now);
        assert!(speakers.selected.contains("b"));
        speakers.select(levels(-20, -10, now), 1, now + Duration::from_secs(1));
        assert!(speakers.selected.is_empty());
        speakers.select(levels(-20, -10, now), 0, now);
        assert!(speakers.selected.is_empty());
    }

    #[test]
    fn rate_limit_refills_without_unbounded_bursts() {
        let now = Instant::now();
        let mut rate = RateLimit::new(now);
        for _ in 0..60 {
            assert!(rate.allow(now));
        }
        assert!(!rate.allow(now));
        assert!(rate.allow(now + Duration::from_millis(20)));
        assert!(!rate.allow(now + Duration::from_millis(20)));
        let later = now + Duration::from_secs(20);
        for _ in 0..60 {
            assert!(rate.allow(later));
        }
        assert!(!rate.allow(later));
    }

    #[test]
    fn stamps_every_client_message_and_rejects_speaking_and_invalid_points() {
        let messages = [
            DataMsg::Reaction {
                participant_id: "forged".into(),
                emoji: "👍".into(),
            },
            DataMsg::RaiseHand {
                participant_id: "forged".into(),
                raised: true,
            },
            DataMsg::Pointer {
                participant_id: "forged".into(),
                share_mid: "0".into(),
                pos: Some((0.5, 0.7)),
            },
            DataMsg::Stroke {
                participant_id: "forged".into(),
                share_mid: "0".into(),
                points: vec![(0.1, 0.2)],
                color: "red".into(),
            },
        ];
        for message in messages {
            let stamped = serde_json::to_value(stamp(message, "real").unwrap()).unwrap();
            assert_eq!(stamped["participantId"], "real");
        }
        assert!(
            stamp(
                DataMsg::Speaking {
                    participant_id: "x".into(),
                    level: 1.0
                },
                "real"
            )
            .is_none()
        );
        assert!(
            stamp(
                DataMsg::Pointer {
                    participant_id: "x".into(),
                    share_mid: "0".into(),
                    pos: Some((f32::NAN, 2.0))
                },
                "real"
            )
            .is_none()
        );
    }

    #[test]
    fn layer_selection_respects_height_budget_and_screen_detail() {
        let source = vec![
            SimulcastLayer::new_with_attributes("q")
                .max_height(180)
                .max_br(100_000)
                .build(),
            SimulcastLayer::new_with_attributes("h")
                .max_height(720)
                .max_br(700_000)
                .build(),
        ];
        let layers = layers(&source, false);
        assert_eq!(choose_layer(&layers, 720, 1_000_000), Some("h".into()));
        assert_eq!(choose_layer(&layers, 360, 1_000_000), Some("h".into()));
        assert_eq!(choose_layer(&layers, 180, 1_000_000), Some("q".into()));
        assert_eq!(choose_layer(&layers, 1080, 1_000_000), Some("h".into()));
        assert_eq!(choose_layer(&layers, 720, 200_000), Some("q".into()));
        assert_eq!(choose_layer(&layers, 0, 1_000_000), None);
        assert!(super::layers(&[SimulcastLayer::new("q")], true)[0].bitrate < 360 * 1800);
        let named = super::layers(
            &[
                SimulcastLayer::new("h"),
                SimulcastLayer::new("m"),
                SimulcastLayer::new("l"),
            ],
            false,
        );
        assert_eq!(named[0].rid, Rid::from("l"));
        assert_eq!(named[2].rid, Rid::from("h"));
        let large = super::layers(
            &[SimulcastLayer::new_with_attributes("x")
                .max_height(u32::MAX)
                .build()],
            false,
        );
        assert_eq!(large[0].height, 16_384);
    }

    #[test]
    fn advertised_browser_screen_layers_can_reach_full_quality() {
        let layers = layers(
            &[
                SimulcastLayer::new_with_attributes("q")
                    .max_height(540)
                    .max_br(3_000_000)
                    .build(),
                SimulcastLayer::new_with_attributes("f")
                    .max_height(1080)
                    .max_br(12_000_000)
                    .build(),
            ],
            true,
        );
        let transport = video_transport_budget(12_000_000);
        let budget = transport.saturating_sub(128_000) * 8 / 10;
        assert_eq!(choose_layer(&layers, 800, budget), Some("f".into()));
        assert_eq!(choose_layer(&layers, 800, 4_000_000), Some("q".into()));
        assert_eq!(choose_layer(&layers, 0, budget), None);
        assert_eq!(choose_layer(&layers, 540, budget), Some("q".into()));
    }
}
