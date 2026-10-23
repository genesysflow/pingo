import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { CallState, type AudioSettings } from './controller.svelte';
import type { AudioProfile } from '../data/types';
import type { DataMsg, ParticipantInfo, TrackInfo } from '../proto';
import { ReconnectPolicy } from './media';
import { nativeVideo } from '../desktop/video';

export interface NativeSettings extends AudioSettings {
  echoCancel: boolean; muted: boolean; deafened: boolean; pushToTalk: boolean; pttPressed: boolean;
  volumes: Record<string, { volume: number; pan: number }>;
}
export function nativeSettings(audio: AudioSettings): NativeSettings {
  return { ...audio, echoCancel: audio.echoCancel ?? true, pushToTalk: audio.pushToTalk ?? false, muted: false, deafened: false, pttPressed: false, volumes: {} };
}
export interface NativeEvent {
  sessionId: string;
  event: {
    type: string; participantId?: string; participants?: ParticipantInfo[]; participant?: ParticipantInfo;
    data?: DataMsg; quality?: 'Great' | 'OK' | 'Poor'; message?: string; code?: string;
    track?: TrackInfo; mid?: string; receivedPackets?: number;
  };
}

/** The webview owns reactive presentation and admission; Rust owns devices, DSP and RTC. */
export class TauriCallController extends CallState {
  participants = $state<ParticipantInfo[]>([]);
  receivedPackets = $state(0);
  private participantId = '';
  private sessionId = '';
  private generation = 0;
  private disposed = false;
  private unlisten: UnlistenFn[] = [];
  private ready: Promise<void>;
  private retry = new ReconnectPolicy();
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private connectTimer?: ReturnType<typeof setTimeout>;
  private operations: Promise<unknown> = Promise.resolve();
  private profile: AudioProfile = 'voice';
  private manual: Record<string, number> = {};
  private spatial: Record<string, { volume: number; pan: number }> = {};
  constructor(
    private admit: (room: string) => Promise<{ token: string; url: string }>,
    private describe: (room: string) => Promise<{ audioProfile: AudioProfile }>,
    private identity: () => string,
  ) {
    super(async () => {}, () => {}, false);
    this.userId = '';
    try { this.audio = { ...this.audio, ...JSON.parse(localStorage.getItem('pingo-audio') ?? '{}') }; } catch { /* defaults */ }
    this.ready = Promise.all([
      listen<NativeEvent>('native-call', e => this.receive(e.payload)),
      listen<boolean>('desktop-mute', e => { this.muted = e.payload; }),
    ]).then(stops => { if (this.disposed) stops.forEach(stop => stop()); else this.unlisten.push(...stops); });
  }
  private command<T = void>(name: string, args: Record<string, unknown>): Promise<T> {
    const next = this.operations.then(() => invoke<T>(name, args));
    this.operations = next.catch(() => {});
    return next;
  }
  override async join(room: string, name: string) {
    this.leave();
    const generation = ++this.generation;
    this.userId = this.identity(); this.room = room; this.roomName = name;
    this.startedAt = Date.now(); this.connection = 'connecting'; this.error = '';
    this.muted = false; this.deafened = false;
    try { await this.connect(room, generation); }
    catch (error) {
      if (generation !== this.generation) return;
      this.error = String(error); this.connection = 'failed'; this.room = null;
      this.stopNative();
      throw error;
    }
  }
  private async connect(room: string, generation: number) {
    await this.ready;
    const [admission, details] = await Promise.all([this.admit(room), this.describe(room)]);
    if (this.disposed || generation !== this.generation) return;
    this.profile = details.audioProfile;
    this.participants = []; this.media = []; this.receivedPackets = 0;
    this.sessionId = crypto.randomUUID();
    await this.command('call_join', { sessionId: this.sessionId, ...admission, settings: this.settings() });
    if (generation !== this.generation) return;
    clearTimeout(this.connectTimer);
    if (this.connection !== 'connected') this.connectTimer = setTimeout(() => this.reconnect('Native call connection timed out.'), 15000);
  }
  private settings(): NativeSettings {
    const volumes: NativeSettings['volumes'] = {};
    for (const p of this.participants) {
      const spatial = this.spatial[p.user] ?? { volume: 1, pan: 0 };
      volumes[p.participantId] = { volume: spatial.volume * (this.manual[p.user] ?? 1), pan: spatial.pan };
    }
    return { ...nativeSettings(this.audio), studio: this.profile === 'studio' || this.audio.studio, muted: this.muted, deafened: this.deafened, volumes };
  }
  private configure() {
    if (!this.sessionId) return;
    void this.command('call_configure', { sessionId: this.sessionId, settings: this.settings() }).catch(e => { this.error = String(e); });
  }
  private receive({ sessionId, event }: NativeEvent) {
    if (this.disposed || sessionId !== this.sessionId) return;
    switch (event.type) {
      case 'joined':
        this.participantId = event.participantId!;
        this.participants = event.participants!; this.configure(); break;
      case 'participantJoined': case 'participantUpdated':
        this.participants = [...this.participants.filter(p => p.participantId !== event.participant!.participantId), event.participant!];
        this.configure(); break;
      case 'participantLeft': {
        const p = this.participants.find(p => p.participantId === event.participantId);
        if (p) {
          delete this.levels[p.user]; this.raised = this.raised.filter(u => u !== p.user);
          this.media = this.media.filter(m => m.user !== p.user);
        }
        this.participants = this.participants.filter(p => p.participantId !== event.participantId);
        break;
      }
      case 'trackPublished': {
        const track = event.track!;
        if (track.kind === 'screen' || track.kind === 'camera') {
          const user = this.participants.find(p => p.participantId === track.participantId)?.user;
          if (user) this.media = [...this.media.filter(m => m.mid !== track.mid), { mid: track.mid, user, kind: track.kind, native: true, stream: null }];
        }
        break;
      }
      case 'trackUnpublished': this.media = this.media.filter(m => m.mid !== event.mid); break;
      case 'connected':
        clearTimeout(this.connectTimer); this.connection = 'connected'; this.error = ''; this.retry.connected(); break;
      case 'disconnected': this.reconnect(); break;
      case 'error': this.reconnect(event.message); break;
      case 'deviceError': this.error = `Audio device: ${event.message}`; break;
      case 'quality': this.quality = event.quality!; break;
      case 'playout': this.receivedPackets = event.receivedPackets ?? 0; break;
      case 'data': {
        const data = event.data!;
        const user = data.participantId === this.participantId ? this.userId : this.participants.find(p => p.participantId === data.participantId)?.user;
        if (user) super.send({ ...data, participantId: user });
        break;
      }
    }
  }
  private reconnect(message = 'Connection interrupted. Rejoining…') {
    if (!this.room || this.reconnectTimer || this.disposed) return;
    this.stopNative();
    const delay = this.retry.nextDelay();
    this.error = message; this.levels = {}; this.media = []; this.participants = [];
    if (delay === null) { this.connection = 'failed'; return; }
    this.connection = 'reconnecting';
    const generation = this.generation, room = this.room;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      if (generation !== this.generation) return;
      void this.connect(room, generation).catch(e => this.reconnect(String(e)));
    }, delay);
  }
  private stopNative() {
    clearTimeout(this.connectTimer);
    if (this.sessionId) void this.command('call_leave', { sessionId: this.sessionId }).catch(() => {});
    this.sessionId = '';
  }
  override leave() {
    ++this.generation; clearTimeout(this.reconnectTimer); this.reconnectTimer = undefined;
    this.stopNative(); this.retry.connected(); super.leave();
    this.participants = []; this.media = []; this.levels = {}; this.receivedPackets = 0;
  }
  override toggleMute() { this.muted = !this.muted; this.configure(); }
  override toggleDeafen() { this.deafened = !this.deafened; this.configure(); }
  override configureAudio(settings: AudioSettings) {
    this.audio = { ...settings }; localStorage.setItem('pingo-audio', JSON.stringify(this.audio)); this.configure();
  }
  pushToTalk(pressed: boolean) {
    if (!this.room || !this.audio.pushToTalk) return;
    void invoke('call_ptt', { pressed }).catch(e => { this.error = String(e); });
  }
  override send(data: DataMsg) {
    if (!this.sessionId || data.type === 'speaking') return;
    void this.command('call_data', { sessionId: this.sessionId, data }).then(() => super.send({ ...data, participantId: this.userId })).catch(e => { this.error = String(e); });
  }
  override setSpatialAudio(user: string, mix: { volume: number; pan: number }) {
    this.spatial[user] = { volume: Math.max(0, Math.min(1, mix.volume)), pan: Math.max(-1, Math.min(1, mix.pan)) };
    super.setSpatialAudio(user, { volume: this.manual[user] ?? 1, pan: this.spatial[user].pan }); this.configure();
  }
  override setParticipantVolume(user: string, volume: number) {
    this.manual[user] = Math.max(0, Math.min(1.5, volume)); super.setParticipantVolume(user, this.manual[user]); this.configure();
  }
  attachVideo(mid: string, canvas: HTMLCanvasElement) { return nativeVideo(this.sessionId, mid, canvas, e => { this.error = String(e); }); }
  override setVideoQuality(mid: string, maxHeight: number) {
    if (!this.sessionId) return;
    void this.command('call_video_quality', { sessionId: this.sessionId, mid, maxHeight: Math.max(0, Math.min(4320, Math.round(maxHeight))) }).catch(e => { this.error = String(e); });
  }
  override toggleShare() { this.error = 'Native screen sending is not available in this build.'; }
  override toggleCamera() { this.error = 'Native camera sending is not available in this build.'; }
  override destroy() {
    this.disposed = true; this.unlisten.splice(0).forEach(stop => stop()); super.destroy();
  }
}
