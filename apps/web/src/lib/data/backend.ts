import type { PaginationOpts, PaginationResult } from '@rxferrix/client';
import { createSeed, participant } from './seed';
import type { Attachment, MessageView, Profile, Room } from './types';
import { expandEmoji, parseMentions } from '../logic/chat';

/** Opaque boundary cursors stay stable when live messages are inserted before the next page. */
export function paginateMessages(messages: MessageView[], room: string, opts: PaginationOpts): PaginationResult<MessageView> {
  const sorted = messages.filter(m => m.room === room && !m.threadRoot).sort((a, b) => b._creationTime - a._creationTime || b._id.localeCompare(a._id));
  const start = opts.cursor ? sorted.findIndex(m => m._id === opts.cursor) + 1 : 0;
  const end = opts.endCursor ? sorted.findIndex(m => m._id === opts.endCursor) + 1 : start + Math.max(1, opts.numItems);
  const page = sorted.slice(start, Math.max(start, end));
  return { page, continueCursor: page.at(-1)?._id ?? opts.cursor ?? '', isDone: end >= sorted.length };
}
type Args = Record<string, unknown>;
const str = (a: Args, key: string) => typeof a[key] === 'string' ? a[key] as string : '';
const name = (value: string) => { const n = value.trim(); if (!n || n.length > 32) throw new Error('Use a name between 1 and 32 characters.'); return n; };
export class MockBackend {
  state = createSeed();
  authenticated = true;
  typing: { room: string; user: string; name: string }[] = [];
  private listeners = new Set<() => void>();
  subscribe = (fn: () => void) => { this.listeners.add(fn); return () => { this.listeners.delete(fn); }; };
  private cache = new Map<string, unknown>();
  notify() { this.cache.clear(); for (const fn of this.listeners) fn(); }
  private room(id: string) { const r = this.state.rooms.find(r => r._id === id); if (!r) throw new Error('Room not found.'); return r; }
  private message(id: string) { const m = this.state.messages.find(m => m._id === id); if (!m) throw new Error('Message not found.'); return m; }
  query(path: string, a: Args = {}): unknown {
    const key = `${path}:${JSON.stringify(a)}`;
    if (!this.cache.has(key)) this.cache.set(key, structuredClone(this.queryRaw(path, a)));
    return this.cache.get(key);
  }
  private queryRaw(path: string, a: Args = {}): unknown {
    const s = this.state;
    if (path === 'profiles:me') return this.authenticated ? s.people[0] : null;
    if (path === 'invites:preview') return str(a, 'code') && str(a, 'code') !== 'expired' ? { spaceName: s.spaces[0].name, accent: s.spaces[0].accent, memberCount: 8 } : null;
    if (!this.authenticated) throw new Error('Please sign in to continue.');
    switch (path) {
      case 'spaces:list': return s.spaces;
      case 'spaces:get': return s.spaces.find(x => x._id === a.space);
      case 'spaces:members': return s.people.map(p => ({ ...p, roles: [p.user === 'you' ? 'maker' : 'everyone'] }));
      case 'rooms:list': return s.rooms.filter(r => r.space === a.space && r.kind !== 'dm').sort((x, y) => x.position - y.position);
      case 'rooms:get': return this.room(str(a, 'room'));
      case 'dms:list': return s.rooms.filter(r => r.kind === 'dm').map(r => ({ room: r._id, other: { ...s.people.find(p => p.name === r.name)!, roles: [] }, unread: r._id === 'dm-maya' ? 1 : 0, lastMessageAt: s.messages.filter(m => m.room === r._id).at(-1)?._creationTime ?? 0 }));
      case 'messages:list': return paginateMessages(s.messages, str(a, 'room'), a.paginationOpts as PaginationOpts);
      case 'messages:thread': return { root: this.message(str(a, 'root')), replies: s.messages.filter(m => m.threadRoot === a.root).sort((x, y) => x._creationTime - y._creationTime) };
      case 'messages:search': { const roomIds = new Set(s.rooms.filter(r => r.space === a.space).map(r => r._id)); return str(a, 'text').trim() ? s.messages.filter(m => roomIds.has(m.room) && !m.deleted && m.body.toLowerCase().includes(str(a, 'text').toLowerCase())).slice(-50).reverse() : []; }
      case 'typing:list': return this.typing.filter(t => t.room === a.room && t.user !== 'you');
      case 'inbox:list': return s.inbox.filter(i => a.includeDone || !i.done).slice(0, 100);
      case 'inbox:count': return s.inbox.filter(i => !i.done).length;
      case 'voice:participants': return this.room(str(a, 'room')).voice;
      default: throw new Error(`Unknown mock query: ${path}`);
    }
  }
  mutate(path: string, a: Args = {}): unknown {
    if (!this.authenticated) throw new Error('Please sign in to continue.');
    const s = this.state;
    let result: unknown;
    switch (path) {
      case 'profiles:update': { const me = s.people[0]; if (a.name !== undefined) me.name = name(str(a, 'name')); if (a.color) me.color = str(a, 'color'); break; }
      case 'spaces:create': { const n = name(str(a, 'name')); const id = crypto.randomUUID(); s.spaces.push({ _id: id, name: n, slug: n.toLowerCase().replace(/\s+/g, '-'), accent: str(a, 'accent') || '#8064cc', unread: false, mentions: 0, owner: 'you', myPerms: 1023, memberCount: 1, roles: [] }); for (const [i, kind] of (['chat', 'lounge'] as const).entries()) s.rooms.push({ _id: `${id}-${kind}`, space: id, name: kind === 'chat' ? 'general' : 'Lounge', topic: 'Make yourself at home.', kind, position: i, private: false, voice: [], unread: 0, mentions: 0, audioProfile: 'voice', myPerms: 1023 }); result = id; break; }
      case 'spaces:update': { const space = s.spaces.find(x => x._id === a.space); if (!space) throw new Error('Space not found.'); if (a.name !== undefined) space.name = name(str(a, 'name')); if (a.accent) space.accent = str(a, 'accent'); break; }
      case 'spaces:leave': s.spaces = s.spaces.filter(x => x._id !== a.space); break;
      case 'invites:create': result = 'fieldwork-friends'; break;
      case 'invites:accept': if (!a.code || a.code === 'expired') throw new Error('This invite has expired. Ask for a fresh one.'); result = 'fieldwork'; break;
      case 'rooms:create': { const id = crypto.randomUUID(); s.rooms.push({ _id: id, space: str(a, 'space'), name: name(str(a, 'name')), topic: str(a, 'topic'), kind: a.kind as Room['kind'], private: !!a.private, position: s.rooms.length, voice: [], unread: 0, mentions: 0, audioProfile: 'voice', myPerms: 1023 }); result = id; break; }
      case 'rooms:update': { const r = this.room(str(a, 'room')); if (a.name !== undefined) r.name = name(str(a, 'name')); if (a.topic !== undefined) r.topic = str(a, 'topic'); if (a.audioProfile) r.audioProfile = a.audioProfile as Room['audioProfile']; if (typeof a.position === 'number') r.position = a.position; break; }
      case 'rooms:remove': s.rooms = s.rooms.filter(r => r._id !== a.room); break;
      case 'rooms:markRead': { const r = this.room(str(a, 'room')); if (!r.unread && !r.mentions) return; r.unread = 0; r.mentions = 0; break; }
      case 'dms:open': { let r = s.rooms.find(r => r._id === `dm-${a.user}`); if (!r) { const p = s.people.find(p => p.user === a.user); if (!p) throw new Error('Person not found.'); r = { ...s.rooms[0], _id: `dm-${a.user}`, name: p.name, space: undefined, kind: 'dm', voice: [], unread: 0, mentions: 0 }; s.rooms.push(r); } result = r._id; break; }
      case 'messages:send': {
        const body = expandEmoji(str(a, 'body').trim());
        const attachments = (a.attachments as string[] | undefined) ?? [];
        if ((!body && !attachments.length) || body.length > 4000 || attachments.length > 10) throw new Error('Write up to 4,000 characters and attach up to 10 files.');
        this.room(str(a, 'room'));
        const root = a.threadRoot ? this.message(str(a, 'threadRoot')) : undefined;
        if (root && root.room !== a.room) throw new Error('Reply in the original room.');
        const msg: MessageView = { _id: crypto.randomUUID(), _creationTime: Date.now(), room: str(a, 'room'), author: { ...s.people[0] }, body, threadRoot: root?._id, replyCount: 0, reactions: [], attachments: attachments.map(id => s.attachments.find(f => f.id === id)).filter((f): f is Attachment => !!f), deleted: false };
        s.messages.push(msg); if (root) { root.replyCount++; root.lastReplyAt = msg._creationTime; }
        parseMentions(body); this.typing = this.typing.filter(t => t.user !== 'you'); result = msg._id; break;
      }
      case 'messages:edit': { const m = this.message(str(a, 'message')); if (m.author.user !== 'you') throw new Error('You can only edit your own messages.'); if (!str(a, 'body').trim() || str(a, 'body').length > 4000) throw new Error('Write between 1 and 4,000 characters.'); m.body = str(a, 'body'); m.editedAt = Date.now(); break; }
      case 'messages:remove': { const m = this.message(str(a, 'message')); if (m.author.user !== 'you') throw new Error('You can only delete your own messages in this demo.'); m.deleted = true; m.body = ''; m.attachments = []; m.reactions = []; break; }
      case 'files:generateUploadUrl': result = 'mock://upload'; break;
      case 'reactions:toggle': { const m = this.message(str(a, 'message')), e = str(a, 'emoji'); const r = m.reactions.find(r => r.emoji === e); if (r) { r.count += r.mine ? -1 : 1; r.mine = !r.mine; m.reactions = m.reactions.filter(r => r.count > 0); } else { if (m.reactions.length >= 20) throw new Error('This message already has 20 reactions.'); m.reactions.push({ emoji: e, count: 1, mine: true }); } break; }
      case 'typing:start': case 'presence:heartbeat': return;
      case 'presence:setStatus': s.people[0].status = a.status as Profile['status']; if (a.text !== undefined) s.people[0].statusText = str(a, 'text'); break;
      case 'inbox:markDone': { const i = s.inbox.find(i => i._id === a.item); if (i) i.done = true; break; }
      case 'inbox:markAllDone': s.inbox.forEach(i => { i.done = true; }); break;
      case 'voice:join': result = { token: 'demo-media-token', url: '/rtc' }; break;
      default: throw new Error(`Unknown mock mutation: ${path}`);
    }
    this.notify(); return result;
  }
  updateCall(room: string, state: { muted: boolean; deafened: boolean; sharingScreen: boolean } | null) {
    // This simulates SFU-owned durable voice state. UI components never call internal mutations.
    for (const r of this.state.rooms) r.voice = r.voice.filter(p => p.user !== 'you');
    if (state) this.room(room).voice.push({ ...participant(this.state.people[0]), ...state });
    this.notify();
  }
}
