// Public names and shapes mirror docs/API.md. These are actual RxFerrix function references.
import { makeFunctionReference, type PaginationOpts, type PaginationResult } from '@rxferrix/client';
import type {
  Profile,
  SpaceSummary,
  SpaceInfo,
  RoomSummary,
  Room,
  Member,
  MessageView,
  InboxItem,
  TypingUser,
  VoiceParticipant,
  Dm,
  RoomKind,
  AudioProfile,
  PresenceStatus,
  AccessStatus,
} from './types';
const q = <A, R>(path: string) => makeFunctionReference<'query', A, R>('query', path);
const m = <A, R = void>(path: string) => makeFunctionReference<'mutation', A, R>('mutation', path);
type Empty = Record<string, never>;
export const api = {
  access: {
    status: q<Empty, AccessStatus>('access:status'),
  },
  profiles: {
    me: q<Empty, Profile | null>('profiles:me'),
    update: m<{ name?: string; color?: string; avatar?: string }>('profiles:update'),
  },
  spaces: {
    list: q<Empty, SpaceSummary[]>('spaces:list'),
    get: q<{ space: string }, SpaceInfo>('spaces:get'),
    create: m<{ name: string; accent?: string }, string>('spaces:create'),
    update: m<{ space: string; name?: string; accent?: string; icon?: string }>('spaces:update'),
    members: q<{ space: string }, Member[]>('spaces:members'),
    leave: m<{ space: string }>('spaces:leave'),
  },
  invites: {
    create: m<{ space: string; maxUses?: number; expiresInHours?: number }, string>('invites:create'),
    preview: q<{ code: string }, { spaceName: string; accent: string; memberCount: number } | null>(
      'invites:preview',
    ),
    accept: m<{ code: string }, string>('invites:accept'),
  },
  rooms: {
    list: q<{ space: string }, RoomSummary[]>('rooms:list'),
    get: q<{ room: string }, Room>('rooms:get'),
    create: m<{ space: string; name: string; topic?: string; kind: RoomKind; private: boolean }, string>(
      'rooms:create',
    ),
    update: m<{
      room: string;
      name?: string;
      topic?: string;
      audioProfile?: AudioProfile;
      position?: number;
    }>('rooms:update'),
    remove: m<{ room: string }>('rooms:remove'),
    markRead: m<{ room: string }>('rooms:markRead'),
  },
  dms: { open: m<{ user: string }, string>('dms:open'), list: q<Empty, Dm[]>('dms:list') },
  messages: {
    list: q<{ room: string; paginationOpts: PaginationOpts }, PaginationResult<MessageView>>('messages:list'),
    thread: q<{ root: string }, { root: MessageView; replies: MessageView[] }>('messages:thread'),
    send: m<{ room: string; body: string; threadRoot?: string; attachments?: string[] }, string>(
      'messages:send',
    ),
    edit: m<{ message: string; body: string }>('messages:edit'),
    remove: m<{ message: string }>('messages:remove'),
    search: q<{ space: string; text: string }, MessageView[]>('messages:search'),
  },
  files: { generateUploadUrl: m<Empty, string>('files:generateUploadUrl') },
  reactions: { toggle: m<{ message: string; emoji: string }>('reactions:toggle') },
  typing: {
    start: m<{ room: string }>('typing:start'),
    list: q<{ room: string }, TypingUser[]>('typing:list'),
  },
  presence: {
    heartbeat: m<Empty>('presence:heartbeat'),
    setStatus: m<{ status: PresenceStatus; text?: string }>('presence:setStatus'),
  },
  inbox: {
    list: q<{ includeDone?: boolean }, InboxItem[]>('inbox:list'),
    count: q<Empty, number>('inbox:count'),
    markDone: m<{ item: string }>('inbox:markDone'),
    markAllDone: m<Empty>('inbox:markAllDone'),
  },
  voice: {
    join: m<{ room: string }, { token: string; url: string }>('voice:join'),
    participants: q<{ room: string }, VoiceParticipant[]>('voice:participants'),
  },
};
