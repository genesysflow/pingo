use super::*;
use str0m::media::{Rid, Simulcast, SimulcastLayer};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Publication {
    Video(VideoPublishConfig),
    ScreenAudio,
}
#[derive(Default)]
pub(super) struct LocalPublication {
    pub mid: Option<Mid>,
    pub desired: Option<Publication>,
    pub offered: Option<Publication>,
    pub negotiated: Option<Publication>,
    configured: Option<Publication>,
    capture_epoch: Option<Duration>,
    time_offset: u64,
    last_time: u64,
    audio_timestamp: Option<u64>,
}
pub(super) fn source_kind(kind: TrackKind) -> Option<VideoSource> {
    match kind {
        TrackKind::Screen => Some(VideoSource::Screen),
        TrackKind::Camera => Some(VideoSource::Camera),
        _ => None,
    }
}
fn track_kind(source: VideoSource) -> TrackKind {
    match source {
        VideoSource::Screen => TrackKind::Screen,
        VideoSource::Camera => TrackKind::Camera,
    }
}
impl Session {
    pub fn set_video_published(
        &mut self,
        source: VideoSource,
        config: Option<VideoPublishConfig>,
    ) -> Result<()> {
        if let Some(config) = &config {
            if config.source != source {
                return Err(Error::Invalid("video source/config mismatch"));
            }
            config.layers().map_err(rtc_error)?;
        }
        let publication = self.publications.entry(track_kind(source)).or_default();
        let desired = config.map(Publication::Video);
        if desired.is_some() && desired != publication.desired {
            publication.capture_epoch = None;
            publication.time_offset = publication.last_time.saturating_add(9000);
        }
        publication.desired = desired;
        self.offer_audio_change()
    }
    pub fn set_screen_audio_published(&mut self, published: bool) -> Result<()> {
        self.publications
            .entry(TrackKind::ScreenAudio)
            .or_default()
            .desired = published.then_some(Publication::ScreenAudio);
        self.offer_audio_change()
    }
    pub fn local_video_mid(&self, source: VideoSource) -> Option<Mid> {
        self.publications
            .get(&track_kind(source))
            .and_then(|p| p.mid)
    }
    pub fn set_video_quality(&mut self, mid: String, max_height: u32) {
        self.outgoing
            .push_back(ClientMsg::SetVideoQuality { mid, max_height });
    }
    pub fn request_video_keyframe(&mut self, mid: &str) -> Result<()> {
        if let Some(mut writer) = self.rtc.writer(Mid::from(mid)) {
            writer
                .request_keyframe(None, KeyframeRequestKind::Pli)
                .map_err(rtc_error)?;
        }
        Ok(())
    }
    pub(super) fn published_tracks(&self) -> Vec<PublishedTrack> {
        let mut tracks: Vec<_> = self
            .audio_mid
            .filter(|_| self.audio_published)
            .into_iter()
            .map(|mid| PublishedTrack {
                mid: mid.to_string(),
                kind: TrackKind::Audio,
            })
            .collect();
        for kind in [TrackKind::Screen, TrackKind::Camera, TrackKind::ScreenAudio] {
            if let Some(publication) = self.publications.get(&kind)
                && publication.desired.is_some()
                && let Some(mid) = publication.mid
            {
                tracks.push(PublishedTrack {
                    mid: mid.to_string(),
                    kind,
                });
            }
        }
        tracks
    }
    pub(super) fn offer_audio_change(&mut self) -> Result<()> {
        if !self.joined || self.pending.is_some() {
            return Ok(());
        }
        let audio_changed = self.negotiated_audio != self.audio_published;
        if !audio_changed
            && self
                .publications
                .values()
                .all(|p| p.desired == p.negotiated)
        {
            return Ok(());
        }
        let mut changes = self.rtc.sdp_api();
        if audio_changed {
            changes.set_direction(
                self.audio_mid.ok_or(Error::Invalid("no audio mid"))?,
                if self.audio_published {
                    Direction::SendOnly
                } else {
                    Direction::Inactive
                },
            );
        }
        for kind in [TrackKind::Screen, TrackKind::Camera, TrackKind::ScreenAudio] {
            let Some(publication) = self.publications.get_mut(&kind) else {
                continue;
            };
            if publication.desired != publication.negotiated {
                if let Some(spec) = &publication.desired {
                    if publication.mid.is_none() || publication.configured.as_ref() != Some(spec) {
                        if let Some(mid) = publication.mid {
                            changes.set_direction(mid, Direction::Inactive);
                        }
                        let simulcast = match spec {
                            Publication::ScreenAudio => None,
                            Publication::Video(config) => {
                                let mut simulcast = Simulcast::new();
                                for layer in config.layers().map_err(rtc_error)? {
                                    simulcast.add_send_layer(
                                        SimulcastLayer::new_with_attributes(&layer.rid)
                                            .max_width(layer.width)
                                            .max_height(layer.height)
                                            .max_fps(config.fps)
                                            .max_br(layer.bitrate)
                                            .build(),
                                    );
                                }
                                Some(simulcast)
                            }
                        };
                        publication.mid = Some(changes.add_media(
                            if kind == TrackKind::ScreenAudio {
                                MediaKind::Audio
                            } else {
                                MediaKind::Video
                            },
                            Direction::SendOnly,
                            None,
                            None,
                            simulcast,
                        ));
                        publication.configured = Some(spec.clone());
                    } else if let Some(mid) = publication.mid {
                        changes.set_direction(mid, Direction::SendOnly);
                    }
                } else if let Some(mid) = publication.mid {
                    changes.set_direction(mid, Direction::Inactive);
                }
            }
            publication.offered = publication.desired.clone();
        }
        if let Some((offer, pending)) = changes.apply() {
            self.pending = Some(pending);
            self.offered_audio = self.audio_published;
            self.local_sdp = opus_sdp(offer.to_sdp_string());
            self.outgoing.push_back(ClientMsg::Offer {
                sdp: self.local_sdp.clone(),
            });
            self.outgoing.push_back(ClientMsg::PublishTracks {
                tracks: self.published_tracks(),
            });
        }
        let desired = self
            .publications
            .values()
            .filter_map(|p| match &p.desired {
                Some(Publication::Video(config)) => Some(
                    config
                        .layers()
                        .ok()?
                        .iter()
                        .map(|l| l.bitrate as u64)
                        .sum::<u64>(),
                ),
                _ => None,
            })
            .sum::<u64>()
            + 256_000;
        self.rtc.bwe().set_desired_bitrate(Bitrate::bps(desired));
        Ok(())
    }
    pub fn send_video(
        &mut self,
        source: VideoSource,
        frame: EncodedVideoFrame,
        now: Instant,
    ) -> Result<bool> {
        if !self.connected {
            return Ok(false);
        }
        let Some(publication) = self.publications.get_mut(&track_kind(source)) else {
            return Ok(false);
        };
        if publication.desired.is_none() || publication.desired != publication.negotiated {
            return Ok(false);
        }
        let Some(Publication::Video(config)) = &publication.negotiated else {
            return Ok(false);
        };
        if frame.codec != config.codec {
            return Err(Error::Invalid("video codec differs from publication"));
        }
        let mid = publication.mid.ok_or(Error::Invalid("no video mid"))?;
        let epoch = *publication.capture_epoch.get_or_insert(frame.timestamp);
        let media_time = publication.time_offset.saturating_add(
            (frame.timestamp.saturating_sub(epoch).as_secs_f64() * 90_000.0) as u64,
        );
        if media_time < publication.last_time {
            return Ok(false);
        }
        publication.last_time = media_time;
        let mut writer = self
            .rtc
            .writer(mid)
            .ok_or(Error::Invalid("no video sender"))?;
        let codec = match frame.codec {
            VideoCodec::H264 => Codec::H264,
            VideoCodec::Vp8 => Codec::Vp8,
        };
        let pt = writer
            .payload_params()
            .find(|p| {
                p.spec().codec == codec
                    && (codec != Codec::H264
                        || p.spec().format.packetization_mode == Some(1)
                            && p.spec()
                                .format
                                .profile_level_id
                                .is_some_and(|id| id >> 16 == 0x42))
            })
            .map(|p| p.pt())
            .ok_or(Error::Invalid(
                "remote did not accept requested video codec",
            ))?;
        let rid = frame
            .rid
            .as_deref()
            .ok_or(Error::Invalid("simulcast frame needs RID"))?;
        writer = writer.rid(Rid::from(rid));
        writer
            .write(pt, now, MediaTime::from_90khz(media_time), frame.data)
            .map_err(rtc_error)?;
        Ok(true)
    }
    pub fn send_screen_audio(&mut self, packet: EncodedPacket, now: Instant) -> Result<bool> {
        if !self.connected {
            return Ok(false);
        }
        let Some(publication) = self.publications.get_mut(&TrackKind::ScreenAudio) else {
            return Ok(false);
        };
        if publication.desired.is_none() || publication.desired != publication.negotiated {
            return Ok(false);
        }
        let mid = publication
            .mid
            .ok_or(Error::Invalid("no screen audio mid"))?;
        let timestamp = publication
            .audio_timestamp
            .map_or(packet.timestamp as u64, |high| {
                (high as i64 + packet.timestamp.wrapping_sub(high as u32) as i32 as i64).max(0)
                    as u64
            });
        publication.audio_timestamp = Some(timestamp);
        let writer = self
            .rtc
            .writer(mid)
            .ok_or(Error::Invalid("no screen audio sender"))?;
        let pt = writer
            .payload_params()
            .find(|p| p.spec().codec == Codec::Opus)
            .map(|p| p.pt())
            .ok_or(Error::Invalid("screen audio Opus not accepted"))?;
        writer
            .audio_level(-(packet.audio_level.min(127) as i8), packet.speaking)
            .write(
                pt,
                now,
                MediaTime::new(timestamp, Frequency::FORTY_EIGHT_KHZ),
                packet.payload,
            )
            .map_err(rtc_error)?;
        Ok(true)
    }
}
