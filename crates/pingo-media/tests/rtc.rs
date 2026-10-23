use pingo_media::{
    audio::codec::{EncodedPacket, Encoder, PacketDuration, Profile},
    pipeline::AudioSettings,
    rtc::{RtcEvent, Session},
};
use pingo_proto::{ClientMsg, DataMsg, ServerMsg, TrackInfo, TrackKind};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    time::{Duration, Instant},
};
use str0m::{
    Candidate, Event, Input, Output, Rtc,
    change::{SdpAnswer, SdpOffer},
    media::{Direction, MediaKind},
    net::{Protocol, Receive},
    rtp::{ExtensionValues, RtpWrite},
};

struct Datagram {
    to_peer: bool,
    source: SocketAddr,
    destination: SocketAddr,
    bytes: Vec<u8>,
}
fn drain_client(client: &mut Session, queue: &mut VecDeque<Datagram>) {
    loop {
        match client.poll_output().unwrap() {
            Output::Timeout(_) => break,
            Output::Transmit(tx) => queue.push_back(Datagram {
                to_peer: true,
                source: tx.source,
                destination: tx.destination,
                bytes: tx.contents.to_vec(),
            }),
            Output::Event(_) => unreachable!(),
        }
    }
}
fn drain_peer(peer: &mut Rtc, queue: &mut VecDeque<Datagram>, events: &mut Vec<Event>) {
    loop {
        match peer.poll_output().unwrap() {
            Output::Timeout(_) => break,
            Output::Transmit(tx) => queue.push_back(Datagram {
                to_peer: false,
                source: tx.source,
                destination: tx.destination,
                bytes: tx.contents.to_vec(),
            }),
            Output::Event(event) => events.push(event),
        }
    }
}
fn exchange(client: &mut Session, peer: &mut Rtc, now: Instant, events: &mut Vec<Event>) {
    let mut queue = VecDeque::new();
    drain_client(client, &mut queue);
    drain_peer(peer, &mut queue, events);
    for _ in 0..1000 {
        let Some(packet) = queue.pop_front() else {
            return;
        };
        if packet.to_peer {
            peer.handle_input(Input::Receive(
                now,
                Receive::new(
                    Protocol::Udp,
                    packet.source,
                    packet.destination,
                    &packet.bytes,
                )
                .unwrap(),
            ))
            .unwrap();
            drain_peer(peer, &mut queue, events);
        } else {
            client
                .receive(now, packet.source, packet.destination, &packet.bytes)
                .unwrap();
            drain_client(client, &mut queue);
        }
    }
    panic!("unbounded packet exchange");
}

#[test]
fn in_memory_answerer_negotiates_audio_levels_data_and_server_reoffer() {
    let now = Instant::now();
    let a: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let b: SocketAddr = "127.0.0.1:60000".parse().unwrap();
    let mut client = Session::new(now, &[a], AudioSettings::default()).unwrap();
    let mut peer = Rtc::builder()
        .set_rtp_mode(true)
        .clear_codecs()
        .enable_opus(true, false)
        .build(now);
    peer.add_local_candidate(Candidate::host(b, "udp").unwrap());
    let mut queue = VecDeque::new();
    let mut peer_events = Vec::new();
    drain_peer(&mut peer, &mut queue, &mut peer_events);
    client
        .signal(ServerMsg::Joined {
            participant_id: "self".into(),
            participants: vec![],
        })
        .unwrap();
    drain_client(&mut client, &mut queue);
    let Some(ClientMsg::Offer { sdp }) = client.next_signal() else {
        panic!("no offer");
    };
    assert!(
        sdp.contains("ssrc-audio-level")
            && sdp.contains("stereo=1")
            && sdp.contains("useinbandfec=1")
    );
    assert!(
        matches!(client.next_signal(),Some(ClientMsg::PublishTracks{tracks}) if tracks.len()==1 && tracks[0].kind==TrackKind::Audio)
    );
    // An offer arriving during our pending offer is ignored: client wins glare.
    client
        .signal(ServerMsg::Offer {
            sdp: "glare must not be parsed".into(),
        })
        .unwrap();
    assert!(client.next_signal().is_none());
    let answer = peer
        .sdp_api()
        .accept_offer(SdpOffer::from_sdp_string(&sdp).unwrap())
        .unwrap();
    drain_peer(&mut peer, &mut queue, &mut peer_events);
    client
        .signal(ServerMsg::Answer {
            sdp: answer.to_sdp_string(),
        })
        .unwrap();
    let mut clock = now;
    for step in 1..=600 {
        clock = now + Duration::from_millis(step * 5);
        exchange(&mut client, &mut peer, clock, &mut peer_events);
        client.timeout(clock).unwrap();
        exchange(&mut client, &mut peer, clock, &mut peer_events);
        peer.handle_input(Input::Timeout(clock)).unwrap();
        exchange(&mut client, &mut peer, clock, &mut peer_events);
    }
    assert!(client.connected);
    let cid = peer_events
        .iter()
        .find_map(|event| {
            if let Event::ChannelOpen(id, name) = event {
                assert_eq!(name, "pingo");
                Some(*id)
            } else {
                None
            }
        })
        .expect("data channel open");
    let mut encoder = Encoder::new(Profile::Voice, PacketDuration::Default20).unwrap();
    let payload = encoder.encode(&[0.1; 960]).unwrap();
    client
        .send_audio(
            EncodedPacket {
                sequence: 0,
                timestamp: 0,
                duration: PacketDuration::Default20,
                payload: payload.clone(),
                audio_level: 42,
                speaking: true,
            },
            clock,
        )
        .unwrap();
    exchange(&mut client, &mut peer, clock, &mut peer_events);
    for i in 1..20 {
        let t = clock + Duration::from_millis(i * 5);
        client.timeout(t).unwrap();
        exchange(&mut client, &mut peer, t, &mut peer_events);
        peer.handle_input(Input::Timeout(t)).unwrap();
        exchange(&mut client, &mut peer, t, &mut peer_events);
    }
    let received = peer_events
        .iter()
        .find_map(|e| {
            if let Event::RtpPacket(p) = e {
                Some(p)
            } else {
                None
            }
        })
        .expect("audio RTP received");
    assert_eq!(received.header.ext_vals.audio_level, Some(-42));
    assert_eq!(received.header.ext_vals.voice_activity, Some(true));
    assert_eq!(received.payload.as_ref(), payload);
    let data = DataMsg::RaiseHand {
        participant_id: "self".into(),
        raised: true,
    };
    assert!(client.send_data(&data).unwrap());
    exchange(
        &mut client,
        &mut peer,
        clock + Duration::from_millis(100),
        &mut peer_events,
    );
    assert!(peer_events.iter().any(|e|matches!(e,Event::ChannelData(d) if serde_json::from_slice::<DataMsg>(&d.data).ok()==Some(data.clone()))));
    let response = DataMsg::Speaking {
        participant_id: "remote".into(),
        level: 0.6,
    };
    peer.channel(cid)
        .unwrap()
        .write(false, &serde_json::to_vec(&response).unwrap())
        .unwrap();
    exchange(
        &mut client,
        &mut peer,
        clock + Duration::from_millis(100),
        &mut peer_events,
    );
    let mut seen = false;
    while let Some(event) = client.next_event() {
        if matches!(event,RtcEvent::Data(ref d) if *d==response) {
            seen = true;
        }
    }
    assert!(seen);

    let mut changes = peer.sdp_api();
    let remote_mid = changes.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
    let (offer, pending) = changes.apply().unwrap();
    exchange(
        &mut client,
        &mut peer,
        clock + Duration::from_millis(100),
        &mut peer_events,
    );
    client
        .signal(ServerMsg::TrackPublished {
            track: TrackInfo {
                participant_id: "remote".into(),
                mid: remote_mid.to_string(),
                kind: TrackKind::Audio,
            },
        })
        .unwrap();
    client
        .signal(ServerMsg::Offer {
            sdp: offer.to_sdp_string(),
        })
        .unwrap();
    exchange(
        &mut client,
        &mut peer,
        clock + Duration::from_millis(100),
        &mut peer_events,
    );
    let Some(ClientMsg::Answer { sdp }) = client.next_signal() else {
        panic!("no answer to reoffer");
    };
    peer.sdp_api()
        .accept_answer(pending, SdpAnswer::from_sdp_string(&sdp).unwrap())
        .unwrap();
    exchange(
        &mut client,
        &mut peer,
        clock + Duration::from_millis(100),
        &mut peer_events,
    );
    let pt = peer.media(remote_mid).unwrap().remote_pts()[0];
    peer.direct_api()
        .stream_tx_by_mid(remote_mid, None)
        .unwrap()
        .write_rtp(
            RtpWrite::new(pt, 0.into(), 0, clock + Duration::from_millis(100), payload)
                .ext_vals(ExtensionValues::default()),
        );
    exchange(
        &mut client,
        &mut peer,
        clock + Duration::from_millis(100),
        &mut peer_events,
    );
    for i in 21..60 {
        let t = clock + Duration::from_millis(i * 5);
        client.timeout(t).unwrap();
        exchange(&mut client, &mut peer, t, &mut peer_events);
        peer.handle_input(Input::Timeout(t)).unwrap();
        exchange(&mut client, &mut peer, t, &mut peer_events);
    }
    assert!(
        client.receiver.stats().values().any(|s| s.received > 0),
        "TrackPublished must route RTP to receive pipeline"
    );
    client
        .signal(ServerMsg::ParticipantLeft {
            participant_id: "remote".into(),
        })
        .unwrap();
    assert!(client.receiver.stats().is_empty());
}

#[test]
fn signaling_rejects_unsolicited_answers_and_tracks_cleanup() {
    let mut session = Session::new(Instant::now(), &[], AudioSettings::default()).unwrap();
    assert!(
        session
            .signal(ServerMsg::Answer { sdp: "bad".into() })
            .is_err()
    );
    for mid in ["1", "2"] {
        session
            .signal(ServerMsg::TrackPublished {
                track: TrackInfo {
                    participant_id: "remote".into(),
                    mid: mid.into(),
                    kind: TrackKind::Audio,
                },
            })
            .unwrap();
    }
    assert_eq!(session.receiver.stats().len(), 2);
    session
        .signal(ServerMsg::TrackUnpublished {
            participant_id: "remote".into(),
            mid: "1".into(),
        })
        .unwrap();
    assert_eq!(session.receiver.stats().len(), 1);
    session
        .signal(ServerMsg::ParticipantLeft {
            participant_id: "remote".into(),
        })
        .unwrap();
    assert!(session.receiver.stats().is_empty());
    let msg = ClientMsg::Join {
        token: "test-token".into(),
    };
    assert_eq!(
        serde_json::to_string(&msg).unwrap(),
        r#"{"type":"join","token":"test-token"}"#
    );
}
