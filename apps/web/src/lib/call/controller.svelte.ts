import type { TrackKind } from '../proto';
import type { DataMsg } from '../proto';
import type { Point } from '../logic/lounge';
export interface AudioSettings {
  microphone: string;
  speaker: string;
  suppression: 'Off' | 'Gentle' | 'Strong';
  studio: boolean;
  echoCancel?: boolean;
  pushToTalk?: boolean;
}
/** Which suppressor is actually cleaning the microphone. */
export type NoiseEngine = 'DeepFilterNet' | 'Browser' | 'Off';
export interface CallMedia {
  mid: string;
  user: string;
  kind: TrackKind;
  stream: MediaStream | null;
  native?: boolean;
}
export interface CallController {
  readonly userId: string;
  readonly screenMid: string;
  readonly connection: 'idle' | 'connecting' | 'connected' | 'reconnecting' | 'failed';
  readonly error: string;
  readonly media: CallMedia[];
  readonly localScreen: MediaStream | null;
  readonly localCamera: MediaStream | null;
  readonly camera: boolean;
  readonly cameraBusy: boolean;
  readonly backgroundBlur: boolean;
  readonly backgroundBlurSupported: boolean;
  setBackgroundBlur(enabled: boolean): void;
  attachVideo?(mid: string, canvas: HTMLCanvasElement): () => void;
  toggleCamera(): void;
  resumeAudio(): void;
  setVideoQuality(mid: string, maxHeight: number): void;
  readonly room: string | null;
  readonly roomName: string;
  readonly startedAt: number;
  readonly clock: number;
  readonly muted: boolean;
  readonly deafened: boolean;
  readonly sharing: boolean;
  readonly quality: 'Great' | 'OK' | 'Poor';
  readonly levels: Record<string, number>;
  readonly raised: string[];
  readonly reactions: { id: number; user: string; emoji: string; expires: number }[];
  readonly pointers: Record<string, { pos: Point; expires: number; shareMid: string }>;
  readonly strokes: {
    id: number;
    user: string;
    shareMid: string;
    points: [number, number][];
    color: string;
    expires: number;
  }[];
  readonly volumes: Record<string, { volume: number; pan: number }>;
  readonly audio: AudioSettings;
  readonly noiseEngine: NoiseEngine;
  readonly shareMode: 'Text' | 'Motion';
  join(room: string, name: string): Promise<void>;
  leave(): void;
  toggleMute(): void;
  toggleDeafen(): void;
  toggleShare(): void;
  setShareMode(mode: 'Text' | 'Motion'): void;
  configureAudio(settings: AudioSettings): void;
  send(data: DataMsg): void;
  setSpatialAudio(user: string, mix: { volume: number; pan: number }): void;
  setParticipantVolume(user: string, volume: number): void;
  destroy(): void;
}
export class CallState implements CallController {
  userId = 'you';
  screenMid = 'demo-screen';
  connection = $state<CallController['connection']>('idle');
  error = $state('');
  media = $state<CallMedia[]>([]);
  localScreen = $state<MediaStream | null>(null);
  localCamera = $state<MediaStream | null>(null);
  camera = $state(false);
  cameraBusy = $state(false);
  backgroundBlur = $state(false);
  backgroundBlurSupported = true;
  setBackgroundBlur(enabled: boolean) {
    this.backgroundBlur = enabled;
    try { localStorage.setItem('pingo-background-blur', String(enabled)); } catch {}
  }
  toggleCamera() {
    this.camera = !this.camera;
  }
  resumeAudio() {}
  setVideoQuality(_mid: string, _maxHeight: number) {}
  room = $state<string | null>(null);
  roomName = $state('');
  startedAt = $state(0);
  clock = $state(Date.now());
  muted = $state(false);
  deafened = $state(false);
  sharing = $state(false);
  quality = $state<'Great' | 'OK' | 'Poor'>('Great');
  levels = $state<Record<string, number>>({});
  raised = $state<string[]>([]);
  reactions = $state<{ id: number; user: string; emoji: string; expires: number }[]>([]);
  pointers = $state<Record<string, { pos: Point; expires: number; shareMid: string }>>({});
  strokes = $state<
    {
      id: number;
      user: string;
      shareMid: string;
      points: [number, number][];
      color: string;
      expires: number;
    }[]
  >([]);
  volumes = $state<Record<string, { volume: number; pan: number }>>({});
  audio = $state<AudioSettings>({
    microphone: 'default',
    speaker: 'default',
    suppression: 'Gentle',
    studio: false,
  });
  noiseEngine = $state<NoiseEngine>('DeepFilterNet');
  shareMode = $state<'Text' | 'Motion'>('Text');
  private timer: ReturnType<typeof setInterval>;
  private seq = 0;
  constructor(
    private requestJoin: (room: string) => Promise<unknown>,
    private durable: (
      room: string,
      state: { muted: boolean; deafened: boolean; sharingScreen: boolean } | null,
    ) => void,
    simulate = true,
  ) {
    this.timer = setInterval(() => {
      this.clock = Date.now();
      if (simulate)
        for (const [i, user] of ['maya', 'leo', 'sofia', 'noah', 'jules', 'you'].entries())
          this.levels[user] =
            user === 'you' && this.muted ? 0 : Math.max(0, Math.sin(this.clock / 900 + i * 2.8)) * 0.8;
      this.reactions = this.reactions.filter((r) => r.expires > this.clock);
      this.strokes = this.strokes.filter((s) => s.expires > this.clock);
      for (const [user, p] of Object.entries(this.pointers))
        if (p.expires < this.clock) delete this.pointers[user];
    }, 200);
  }
  private sync() {
    if (this.room)
      this.durable(this.room, { muted: this.muted, deafened: this.deafened, sharingScreen: this.sharing });
  }
  async join(room: string, name: string) {
    await this.requestJoin(room);
    this.connection = 'connected';
    this.room = room;
    this.roomName = name;
    this.startedAt = Date.now();
    this.muted = false;
    this.deafened = false;
    this.sharing = false;
    this.raised = [];
    this.sync();
  }
  leave() {
    if (this.room) this.durable(this.room, null);
    this.connection = 'idle';
    this.room = null;
    this.sharing = false;
    this.raised = [];
    this.reactions = [];
    this.pointers = {};
    this.strokes = [];
  }
  toggleMute() {
    this.muted = !this.muted;
    this.sync();
  }
  toggleDeafen() {
    this.deafened = !this.deafened;
    if (this.deafened) this.muted = true;
    this.sync();
  }
  toggleShare() {
    this.sharing = !this.sharing;
    this.sync();
  }
  setShareMode(mode: 'Text' | 'Motion') {
    this.shareMode = mode;
  }
  configureAudio(settings: AudioSettings) {
    this.audio = { ...settings };
  }
  setSpatialAudio(user: string, mix: { volume: number; pan: number }) {
    this.volumes[user] = mix;
  }
  setParticipantVolume(user: string, volume: number) {
    this.volumes[user] = { volume, pan: this.volumes[user]?.pan ?? 0 };
  }
  send(data: DataMsg) {
    const user = data.participantId;
    switch (data.type) {
      case 'speaking':
        this.levels[user] = data.level;
        break;
      case 'reaction':
        this.reactions.push({ id: ++this.seq, user, emoji: data.emoji, expires: Date.now() + 3500 });
        break;
      case 'raiseHand':
        this.raised = data.raised
          ? [...new Set([...this.raised, user])]
          : this.raised.filter((u) => u !== user);
        break;
      case 'pointer':
        if (data.pos)
          this.pointers[user] = {
            pos: { x: data.pos[0], y: data.pos[1] },
            shareMid: data.shareMid,
            expires: Date.now() + 2500,
          };
        else delete this.pointers[user];
        break;
      case 'stroke':
        this.strokes.push({
          id: ++this.seq,
          user,
          shareMid: data.shareMid,
          points: data.points,
          color: data.color,
          expires: Date.now() + 5000,
        });
        break;
    }
  }
  destroy() {
    this.leave();
    clearInterval(this.timer);
  }
}

/** Design-only controller; live calls reuse only the shared reactive state and event reducer. */
export class MockCallController extends CallState {}
