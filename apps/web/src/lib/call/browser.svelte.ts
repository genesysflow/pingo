import { CallState } from './controller.svelte';
import type { AudioSettings, CallController } from './controller.svelte';
import type { AudioProfile } from '../data/types';
import type { ClientMsg, ServerMsg, DataMsg, TrackInfo, TrackKind, ParticipantInfo } from '../proto';
import { configureSdp, qualityLabel, ReconnectPolicy, screenConstraints, gatheredDescription, iceRenewalDelay, type Admission } from './media';
import { Microphone } from './microphone';
import { errorMessage } from '../data/errors';

type AudioRoute = { source: MediaStreamAudioSourceNode; gain: GainNode; pan: StereoPannerNode };

/** RxFerrix supplies admission; only the SFU owns durable participant state. */
export class BrowserCallController extends CallState implements CallController {
  private pc?: RTCPeerConnection;
  private socket?: WebSocket;
  private channel?: RTCDataChannel;
  private microphone?: Microphone;
  private display?: MediaStream;
  private webcam?: MediaStream;
  private context?: AudioContext;
  private output?: HTMLAudioElement;
  private destination?: MediaStreamAudioDestinationNode;
  private sources = new Map<string, AudioRoute>();
  private tracks = new Map<string, TrackInfo>();
  private received = new Map<string, MediaStreamTrack>();
  private published = new Map<RTCRtpTransceiver, TrackKind>();
  private participants = new Map<string, ParticipantInfo>();
  private self = '';
  private generation = 0;
  private retry = new ReconnectPolicy();
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private disconnectTimer?: ReturnType<typeof setTimeout>;
  private renewalTimer?: ReturnType<typeof setTimeout>;
  private statsTimer?: ReturnType<typeof setInterval>;
  private profile: AudioProfile = 'voice';
  screenMid = $state('');
  private queue: Promise<void> = Promise.resolve();
  private localQueue: Promise<void> = Promise.resolve();
  private previousPackets = { received: 0, lost: 0 };
  private manualVolume: Record<string, number> = {};
  private spatialMix: Record<string, { volume: number; pan: number }> = {};
  constructor(
    private admit: (room: string) => Promise<Admission>,
    private describe: (room: string) => Promise<{ audioProfile: AudioProfile }>,
    private identity: () => string,
  ) {
    super(
      async () => {},
      () => {},
      false,
    );
    this.userId = '';
    try {
      const saved = JSON.parse(localStorage.getItem('pingo-audio') ?? 'null');
      if (saved) this.audio = saved;
    } catch {
      /* use defaults */
    }
  }

  override async join(room: string, name: string) {
    this.leave();
    this.userId = this.identity();
    this.room = room;
    this.roomName = name;
    this.startedAt = Date.now();
    this.connection = 'connecting';
    this.error = '';
    this.muted = false;
    this.deafened = false;
    const generation = ++this.generation;
    try {
      await this.connect(room, generation);
    } catch (error) {
      if (generation === this.generation) {
        this.cleanup();
        this.room = null;
        this.connection = 'failed';
        this.error = errorMessage(error);
      }
      throw error;
    }
  }
  private async connect(room: string, generation: number) {
    const [admission, details] = await Promise.all([this.admit(room), this.describe(room)]);
    if (generation !== this.generation) throw new Error('Call was cancelled.');
    this.profile = details.audioProfile;
    this.context = new AudioContext();
    await this.context.resume();
    this.destination = this.context.createMediaStreamDestination();
    this.output = new Audio();
    this.output.autoplay = true;
    this.output.srcObject = this.destination.stream;
    await this.selectSpeaker();
    const microphone = await Microphone.open({
      ...this.audio,
      studio: this.profile === 'studio' || this.audio.studio,
    });
    if (generation !== this.generation) {
      microphone.stop();
      throw new Error('Call was cancelled.');
    }
    this.microphone = microphone;
    this.noiseEngine = microphone.engine;
    const pc = (this.pc = new RTCPeerConnection({
      bundlePolicy: 'max-bundle',
      iceServers: admission.iceServers ?? [],
      iceTransportPolicy: localStorage.getItem('pingo-relay-only') === 'true' ? 'relay' : 'all',
    }));
    if (this.display?.getVideoTracks().some(t => t.readyState === 'live')) this.addVideo(this.display, 'screen');
    if (this.webcam?.getVideoTracks().some(t => t.readyState === 'live')) this.addVideo(this.webcam, 'camera');
    const renewIn = iceRenewalDelay(admission.iceExpiresAt);
    if (renewIn !== null) this.renewalTimer = setTimeout(() => this.reconnect(generation), renewIn);
    microphone.enabled = !this.muted;
    this.published.set(
      pc.addTransceiver(microphone.track, { direction: 'sendonly', streams: [microphone.stream] }),
      'audio',
    );
    this.channel = pc.createDataChannel('pingo');
    this.channel.onmessage = (event) => {
      try {
        this.receive(JSON.parse(event.data) as DataMsg);
      } catch {
        /* malformed ephemeral packet */
      }
    };
    pc.ontrack = (event) => {
      const mid = event.transceiver.mid;
      if (mid === null) return;
      this.received.set(mid, event.track);
      this.attach(mid);
      event.track.onended = () => this.detach(mid);
    };
    pc.onconnectionstatechange = () => {
      if (generation !== this.generation) return;
      if (pc.connectionState === 'connected') {
        this.connection = 'connected';
        this.error = '';
        this.retry.connected();
        clearTimeout(this.disconnectTimer);
        void this.output?.play().catch(() => {
          this.error = 'Your browser paused call audio. Click the call to resume it.';
        });
      } else if (pc.connectionState === 'failed') this.reconnect(generation);
      else if (pc.connectionState === 'disconnected') {
        clearTimeout(this.disconnectTimer);
        this.disconnectTimer = setTimeout(() => this.reconnect(generation), 4000);
      }
    };
    const url = new URL(admission.url, location.origin);
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = (this.socket = new WebSocket(url));
    await new Promise<void>((resolve, reject) => {
      let admitted = false;
      const timeout = setTimeout(() => reject(new Error('The call server did not answer.')), 12_000);
      const fail = (error: unknown) => {
        clearTimeout(timeout);
        reject(error);
      };
      socket.onopen = () => this.signal({ type: 'join', token: admission.token });
      socket.onerror = () => fail(new Error('Could not reach the call server.'));
      socket.onclose = () => {
        fail(new Error('The call connection closed.'));
        if (admitted) this.reconnect(generation);
      };
      socket.onmessage = (event) => {
        this.queue = this.queue
          .then(async () => {
            if (generation !== this.generation) return;
            const msg = JSON.parse(event.data) as ServerMsg;
            if (msg.type === 'joined') {
              this.self = msg.participantId;
              this.participants = new Map(msg.participants.map((p) => [p.participantId, p]));
              admitted = true;
              clearTimeout(timeout);
              resolve();
            } else await this.handle(msg, pc);
          })
          .catch((error) => {
            this.error = errorMessage(error);
            fail(error);
            if (admitted) this.reconnect(generation);
          });
      };
    });
    if (generation !== this.generation) throw new Error('Call was cancelled.');
    await this.offer();
    this.state();
    this.statsTimer = setInterval(() => {
      this.signal({ type: 'ping' });
      void this.sampleStats(pc).catch(() => {});
    }, 2000);
  }
  private signal(msg: ClientMsg) {
    if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify(msg));
  }
  private async handle(msg: ServerMsg, pc: RTCPeerConnection) {
    switch (msg.type) {
      case 'offer':
        // The SFU rolls back its own colliding offer. Our outstanding offer wins.
        if (pc.signalingState === 'have-local-offer') return;
        await pc.setRemoteDescription({ type: 'offer', sdp: msg.sdp });
        await pc.setLocalDescription({
          type: 'answer',
          sdp: configureSdp((await pc.createAnswer()).sdp!, this.audio.studio ? 'studio' : this.profile),
        });
        this.signal({ type: 'answer', sdp: await gatheredDescription(pc) });
        break;
      case 'answer':
        if (pc.signalingState === 'have-local-offer')
          await pc.setRemoteDescription({
            type: 'answer',
            sdp: configureSdp(msg.sdp, this.audio.studio ? 'studio' : this.profile),
          });
        break;
      case 'participantJoined':
      case 'participantUpdated':
        this.participants.set(msg.participant.participantId, msg.participant);
        if (msg.participant.muted) this.levels[msg.participant.user] = 0;
        break;
      case 'participantLeft': {
        const user = this.participants.get(msg.participantId)?.user;
        if (user) {
          delete this.levels[user];
          this.raised = this.raised.filter((u) => u !== user);
        }
        this.participants.delete(msg.participantId);
        for (const [mid, track] of this.tracks)
          if (track.participantId === msg.participantId) this.detach(mid);
        break;
      }
      case 'trackPublished':
        this.tracks.set(msg.track.mid, msg.track);
        this.attach(msg.track.mid);
        break;
      case 'trackUnpublished':
        this.detach(msg.mid);
        this.tracks.delete(msg.mid);
        break;
      case 'error':
        throw new Error(msg.message);
    }
  }
  private async offer() {
    const generation = this.generation;
    const pc = this.pc;
    if (!pc || pc.signalingState === 'closed') return;
    // Local controls serialize offers and wait for an outstanding answer/server exchange.
    if (pc.signalingState !== 'stable')
      await new Promise<void>((resolve, reject) => {
        const check = () => {
          if (pc.signalingState === 'stable') {
            cleanup();
            resolve();
          } else if (pc.signalingState === 'closed') {
            cleanup();
            reject(new Error('Call closed.'));
          }
        };
        const timeout = setTimeout(() => {
          cleanup();
          reject(new Error('Call negotiation timed out.'));
        }, 12_000);
        const cleanup = () => {
          clearTimeout(timeout);
          pc.removeEventListener('signalingstatechange', check);
        };
        pc.addEventListener('signalingstatechange', check);
        check();
      });
    await pc.setLocalDescription({
      type: 'offer',
      sdp: configureSdp((await pc.createOffer()).sdp!, this.audio.studio ? 'studio' : this.profile),
    });
    const sdp = await gatheredDescription(pc);
    if (generation !== this.generation) return;
    this.signal({ type: 'offer', sdp });
    this.screenMid = [...this.published].find(([, kind]) => kind === 'screen')?.[0].mid ?? '';
    this.signal({
      type: 'publishTracks',
      tracks: [...this.published].flatMap(([transceiver, kind]) =>
        transceiver.mid !== null && transceiver.sender.track ? [{ mid: transceiver.mid, kind }] : [],
      ),
    });
  }
  private attach(mid: string) {
    const info = this.tracks.get(mid),
      track = this.received.get(mid);
    if (!info || !track || this.media.some((m) => m.mid === mid)) return;
    const user = this.participants.get(info.participantId)?.user ?? info.participantId;
    const stream = new MediaStream([track]);
    this.media.push({ mid, user, kind: info.kind, stream });
    if (track.kind === 'audio' && this.context && this.destination) {
      const source = this.context.createMediaStreamSource(stream);
      const gain = this.context.createGain(),
        pan = this.context.createStereoPanner();
      source.connect(gain).connect(pan).connect(this.destination);
      this.sources.set(mid, { source, gain, pan });
      this.mix(user);
    }
  }
  private detach(mid: string) {
    const route = this.sources.get(mid);
    route?.source.disconnect();
    route?.gain.disconnect();
    route?.pan.disconnect();
    this.sources.delete(mid);
    this.media = this.media.filter((m) => m.mid !== mid);
  }
  private mix(user: string) {
    const spatial = this.spatialMix[user] ?? { volume: 1, pan: 0 };
    for (const media of this.media) {
      if (media.user !== user) continue;
      const route = this.sources.get(media.mid);
      if (!route) continue;
      route.gain.gain.value = this.deafened ? 0 : spatial.volume * (this.manualVolume[user] ?? 1);
      route.pan.pan.value = spatial.pan;
    }
  }
  override setSpatialAudio(user: string, mix: { volume: number; pan: number }) {
    this.spatialMix[user] = mix;
    super.setSpatialAudio(user, { volume: this.manualVolume[user] ?? 1, pan: mix.pan });
    this.mix(user);
  }
  override setParticipantVolume(user: string, volume: number) {
    this.manualVolume[user] = Math.max(0, Math.min(1.5, volume));
    super.setParticipantVolume(user, this.manualVolume[user]);
    this.mix(user);
  }
  override resumeAudio() {
    void (async () => {
      await this.context?.resume();
      await this.output?.play();
      this.error = '';
    })().catch((error) => (this.error = errorMessage(error)));
  }
  private state() {
    this.signal({ type: 'setState', muted: this.muted, deafened: this.deafened });
  }
  override toggleMute() {
    this.muted = !this.muted;
    if (this.microphone) this.microphone.enabled = !this.muted;
    if (this.muted) this.levels[this.userId] = 0;
    this.state();
  }
  override toggleDeafen() {
    this.deafened = !this.deafened;
    for (const user of new Set(this.media.map((m) => m.user))) this.mix(user);
    this.state();
    void this.context?.resume();
    void this.output?.play().catch(() => {});
  }
  private receive(data: DataMsg) {
    if (!data || typeof data !== 'object') return;
    const user =
      data.participantId === this.self ? this.userId : this.participants.get(data.participantId)?.user;
    if (user) super.send({ ...data, participantId: user });
  }
  override send(data: DataMsg) {
    if (data.type === 'speaking') return;
    const message = { ...data, participantId: this.self };
    if (this.channel?.readyState === 'open') {
      this.channel.send(JSON.stringify(message));
      super.send({ ...message, participantId: this.userId });
    }
  }
  override setVideoQuality(mid: string, maxHeight: number) {
    this.signal({ type: 'setVideoQuality', mid, maxHeight: Math.max(0, Math.round(maxHeight)) });
  }
  private control(action: () => Promise<void>, cancelled: () => void = () => {}) {
    const generation = this.generation;
    this.localQueue = this.localQueue
      .then(async () => {
        if (generation === this.generation) await action();
        else cancelled();
      })
      .catch((error) => {
        this.error = errorMessage(error);
      });
  }
  private addVideo(stream: MediaStream, kind: 'screen' | 'camera') {
    const pc = this.pc;
    if (!pc) {
      stream.getTracks().forEach((t) => t.stop());
      return;
    }
    for (const track of stream.getTracks()) {
      const trackKind = track.kind === 'audio' ? 'screenAudio' : kind;
      let transceiver: RTCRtpTransceiver;
      const init: RTCRtpTransceiverInit = { direction: 'sendonly', streams: [stream] };
      if (track.kind === 'video')
        init.sendEncodings = [
          { rid: 'q', scaleResolutionDownBy: 2, maxBitrate: 350_000 },
          { rid: 'f', scaleResolutionDownBy: 1, maxBitrate: kind === 'screen' ? 5_000_000 : 2_000_000 },
        ];
      try {
        transceiver = pc.addTransceiver(track, init);
      } catch {
        transceiver = pc.addTransceiver(track, { direction: 'sendonly', streams: [stream] });
      }
      this.published.set(transceiver, trackKind);
      if (track.kind === 'video')
        track.onended = () => {
          if (kind === 'screen' && this.sharing) this.toggleShare();
          if (kind === 'camera' && this.camera) this.toggleCamera();
        };
    }
  }
  private async removeMedia(kinds: TrackKind[]) {
    for (const [transceiver, kind] of this.published)
      if (kinds.includes(kind)) {
        transceiver.sender.track?.stop();
        await transceiver.sender.replaceTrack(null);
        transceiver.direction = 'inactive';
        this.published.delete(transceiver);
      }
  }
  override toggleShare() {
    if (!this.room || !this.pc) return;
    // Capture is requested during the user's gesture, before waiting on a negotiation queue.
    if (!this.sharing) {
      const generation = this.generation;
      const capture = navigator.mediaDevices.getDisplayMedia({
        video: screenConstraints(this.shareMode),
        audio: true,
      });
      void capture
        .then((stream) => {
          if (generation !== this.generation || !this.room) {
            stream.getTracks().forEach((t) => t.stop());
            return;
          }
          this.control(
            async () => {
              if (generation !== this.generation) {
                stream.getTracks().forEach((t) => t.stop());
                return;
              }
              this.display = stream;
              this.localScreen = stream;
              this.sharing = true;
              stream
                .getVideoTracks()
                .forEach((t) => (t.contentHint = this.shareMode === 'Text' ? 'detail' : 'motion'));
              this.addVideo(stream, 'screen');
              await this.offer();
            },
            () => stream.getTracks().forEach((t) => t.stop()),
          );
        })
        .catch((error) => {
          if (generation === this.generation) this.error = errorMessage(error);
        });
    } else
      this.control(async () => {
        this.sharing = false;
        this.localScreen = null;
        this.display?.getTracks().forEach((t) => t.stop());
        this.display = undefined;
        await this.removeMedia(['screen', 'screenAudio']);
        await this.offer();
      });
  }
  override toggleCamera() {
    if (!this.room || !this.pc) return;
    if (!this.camera) {
      const generation = this.generation;
      const capture = navigator.mediaDevices.getUserMedia({
        video: { width: 1280, height: 720 },
        audio: false,
      });
      void capture
        .then((stream) => {
          if (generation !== this.generation || !this.room) {
            stream.getTracks().forEach((t) => t.stop());
            return;
          }
          this.control(
            async () => {
              if (generation !== this.generation) {
                stream.getTracks().forEach((t) => t.stop());
                return;
              }
              this.webcam = stream;
              this.camera = true;
              this.addVideo(stream, 'camera');
              await this.offer();
            },
            () => stream.getTracks().forEach((t) => t.stop()),
          );
        })
        .catch((error) => {
          if (generation === this.generation) this.error = errorMessage(error);
        });
    } else
      this.control(async () => {
        this.camera = false;
        this.webcam?.getTracks().forEach((t) => t.stop());
        this.webcam = undefined;
        await this.removeMedia(['camera']);
        await this.offer();
      });
  }
  override setShareMode(mode: 'Text' | 'Motion') {
    super.setShareMode(mode);
    this.display?.getVideoTracks().forEach((t) => {
      t.contentHint = mode === 'Text' ? 'detail' : 'motion';
      void t.applyConstraints(screenConstraints(mode)).catch((e) => (this.error = errorMessage(e)));
    });
  }
  private async selectSpeaker() {
    if (this.output && 'setSinkId' in this.output)
      await this.output.setSinkId(this.audio.speaker === 'default' ? '' : this.audio.speaker);
  }
  override configureAudio(settings: AudioSettings) {
    super.configureAudio(settings);
    localStorage.setItem('pingo-audio', JSON.stringify(settings));
    this.control(async () => {
      await this.selectSpeaker();
      if (!this.pc) return;
      const generation = this.generation;
      const options = { ...settings, studio: this.profile === 'studio' || settings.studio };
      if (this.microphone?.retune(options)) return;
      const microphone = await Microphone.open(options);
      if (generation !== this.generation) {
        microphone.stop();
        return;
      }
      microphone.enabled = !this.muted;
      for (const [transceiver, kind] of this.published)
        if (kind === 'audio') await transceiver.sender.replaceTrack(microphone.track);
      this.microphone?.stop();
      this.microphone = microphone;
      this.noiseEngine = microphone.engine;
      await this.offer();
    });
  }
  private async sampleStats(pc: RTCPeerConnection) {
    const stats = await pc.getStats();
    let received = 0,
      lost = 0,
      jitter = 0,
      rtt = 0;
    stats.forEach((report) => {
      if (report.type === 'inbound-rtp' && report.kind === 'audio') {
        received += report.packetsReceived ?? 0;
        lost += report.packetsLost ?? 0;
        jitter = Math.max(jitter, report.jitter ?? 0);
      }
      if (report.type === 'candidate-pair' && report.state === 'succeeded')
        rtt = Math.max(rtt, report.currentRoundTripTime ?? 0);
    });
    this.quality = qualityLabel({
      received: Math.max(0, received - this.previousPackets.received),
      lost: Math.max(0, lost - this.previousPackets.lost),
      jitter,
      rtt,
    });
    this.previousPackets = { received, lost };
  }
  private reconnect(generation: number) {
    if (generation !== this.generation || !this.room || this.reconnectTimer) return;
    const delay = this.retry.nextDelay();
    this.cleanup(delay !== null);
    if (delay === null) {
      this.connection = 'failed';
      this.error = 'Could not reconnect. Leave the call and join again.';
      return;
    }
    this.connection = 'reconnecting';
    const room = this.room,
      next = ++this.generation;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      void this.connect(room, next).catch((error) => {
        if (next !== this.generation) return;
        this.error = errorMessage(error);
        this.reconnect(next);
      });
    }, delay);
  }
  private cleanup(preserveCapture = false) {
    clearTimeout(this.renewalTimer);
    clearInterval(this.statsTimer);
    clearTimeout(this.disconnectTimer);
    if (this.socket) {
      this.socket.onopen = null;
      this.socket.onclose = null;
      this.socket.onmessage = null;
      this.socket.onerror = null;
      this.socket.close();
    }
    this.socket = undefined;
    this.pc?.close();
    this.pc = undefined;
    this.channel = undefined;
    for (const mid of this.sources.keys()) this.detach(mid);
    this.tracks.clear();
    this.received.clear();
    this.published.clear();
    this.participants.clear();
    this.microphone?.stop();
    this.microphone = undefined;
    if (!preserveCapture) {
      for (const stream of [this.display, this.webcam]) stream?.getTracks().forEach((t) => t.stop());
      this.display = this.webcam = undefined;
      this.localScreen = null;
      this.sharing = false;
      this.camera = false;
    }
    if (this.output) {
      this.output.pause();
      this.output.srcObject = null;
      this.output = undefined;
    }
    void this.context?.close();
    this.context = undefined;
    this.destination = undefined;
    this.media = [];
    this.levels = {};
    this.previousPackets = { received: 0, lost: 0 };
    this.queue = Promise.resolve();
    this.localQueue = Promise.resolve();
  }
  override leave() {
    ++this.generation;
    clearTimeout(this.reconnectTimer);
    this.reconnectTimer = undefined;
    this.cleanup();
    this.spatialMix = {};
    this.manualVolume = {};
    this.volumes = {};
    this.retry.connected();
    super.leave();
  }
}
