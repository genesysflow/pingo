import type { Profile, SpaceInfo, Room, MessageView, VoiceParticipant, InboxItem, Attachment } from './types';
export const people: Profile[] = [
  { user: 'you', name: 'Alex Morgan', color: '#b9a2e7', status: 'online', statusText: 'Making room for good ideas' },
  { user: 'maya', name: 'Maya Chen', color: '#edb18e', status: 'online', statusText: 'A little deep work' },
  { user: 'leo', name: 'Leo Park', color: '#a0c4af', status: 'online', statusText: 'Building the good stuff' },
  { user: 'sofia', name: 'Sofia Reyes', color: '#e6a4ba', status: 'online', statusText: 'Let’s make it feel right' },
  { user: 'noah', name: 'Noah Williams', color: '#9cbddc', status: 'idle', statusText: 'Coffee, probably' },
  { user: 'jules', name: 'Jules Laurent', color: '#d1b674', status: 'online', statusText: 'In the flow' },
  { user: 'ava', name: 'Ava Thompson', color: '#b1b2db', status: 'dnd', statusText: 'Heads down until 3' },
  { user: 'sam', name: 'Sam Rivera', color: '#a5c9c5', status: 'offline', statusText: 'See you tomorrow' },
];
export const participant = (p: Profile): VoiceParticipant => ({ user: p.user, name: p.name, color: p.color, muted: false, deafened: false, sharingScreen: false, video: false });
export function createSeed(now = Date.now()) {
  const spaces: SpaceInfo[] = [
    { _id: 'fieldwork', name: 'Fieldwork', slug: 'fieldwork', accent: '#8064cc', unread: true, mentions: 3, owner: 'you', myPerms: 1023, memberCount: 8, roles: [{ _id: 'everyone', space: 'fieldwork', name: 'Everyone', color: '#8064cc', perms: 459, position: 0 }, { _id: 'maker', space: 'fieldwork', name: 'Maker', color: '#4b9472', perms: 1023, position: 1 }] },
    { _id: 'afterhours', name: 'After hours', slug: 'after-hours', accent: '#be7850', unread: true, mentions: 0, owner: 'maya', myPerms: 459, memberCount: 8, roles: [] },
    { _id: 'opensource', name: 'Open garden', slug: 'open-garden', accent: '#4b9472', unread: false, mentions: 0, owner: 'you', myPerms: 1023, memberCount: 8, roles: [] },
  ];
  const room = (id: string, name: string, topic: string, kind: Room['kind'], position: number, voice: VoiceParticipant[] = [], space = 'fieldwork'): Room => ({ _id: id, name, topic, kind, position, voice, space, private: false, audioProfile: 'voice', unread: 0, mentions: 0, myPerms: 1023 });
  const rooms: Room[] = [room('general', 'general', 'A little bit of everything. A lot of good people.', 'chat', 0), room('design', 'design-studio', 'Small details. Big feelings. Things we’re making together.', 'chat', 1, people.slice(1, 4).map(participant)), room('build', 'the-build', 'From “what if” to shipped.', 'chat', 2, [participant(people[5])]), room('inspiration', 'inspiration', 'Good things from all around the internet.', 'chat', 3), room('wins', 'little-wins', 'Big or small, it all counts.', 'chat', 4), room('lounge', 'The greenhouse', 'Pull up a chair. See where the conversation goes.', 'lounge', 5, [participant(people[2]), participant(people[4]), participant(people[5])]), room('stage', 'Show & tell', 'Something worth sharing, every Friday.', 'stage', 6), room('music', 'on-repeat', 'What’s in your headphones?', 'chat', 0, [], 'afterhours'), room('weekend', 'weekend-plans', 'A little time away from the screen.', 'chat', 1, [], 'afterhours'), room('garden', 'general', 'Build in the open. Grow together.', 'chat', 0, [], 'opensource'), room('dm-maya', 'Maya Chen', 'A conversation between you and Maya', 'dm', 0), room('dm-leo', 'Leo Park', 'A conversation between you and Leo', 'dm', 0), room('dm-sofia', 'Sofia Reyes', 'A conversation between you and Sofia', 'dm', 0)];
  rooms.find(r => r._id === 'design')!.unread = 6;
  rooms.find(r => r._id === 'build')!.unread = 12;
  rooms.find(r => r._id === 'inspiration')!.unread = 3;
  rooms.find(r => r._id === 'design')!.mentions = 2;
  const messages: MessageView[] = [];
  let serial = 0;
  const add = (roomId: string, who: number, body: string, time: number, options: Partial<MessageView> = {}) => {
    const msg: MessageView = { _id: `m-${++serial}`, _creationTime: time, room: roomId, author: people[who], body, replyCount: 0, attachments: [], reactions: [], deleted: false, ...options };
    messages.push(msg); return msg;
  };
  const history = ['The next iteration is looking really good. Thanks for all the thoughtful feedback.', 'Quick update: the little details are coming together. ✨', 'I left a few notes in the file. Nothing blocking — just ideas!', 'Love where this is heading. The simpler version feels right.', 'Pushed a small fix. Ready whenever you are.', 'A reminder to step outside for a bit. The work will still be here. 🌱', 'Does anyone have a good reference for this? I’m collecting ideas.', 'Yes! Let’s try it and see how it feels.'];
  for (const r of rooms) for (let i = 0; i < (r._id === 'design' ? 1250 : 65); i++) add(r._id, 1 + i % 7, history[i % history.length], now - (r._id === 'design' ? 1250 - i : 65 - i) * 360_000 - 3_600_000);
  const t = now - 19 * 60_000;
  add('design', 1, 'Morning, good people ☀️\nI spent a little time exploring where Fieldwork could go next. Less noise, more room to think.', t);
  const root = add('design', 1, 'Here’s a first peek at the new direction. A softer palette, a little more breathing room, and a home for all our half-formed ideas.\n\nWould love your eyes on this, @Alex 👀', t + 60_000, { attachments: [{ id: 'direction', name: 'A little room to grow.fig', url: '/direction.svg', contentType: 'image/svg+xml', size: 245000, width: 960, height: 480 }], reactions: [{ emoji: '💜', count: 6, mine: true }, { emoji: '🌱', count: 3, mine: false }, { emoji: '✨', count: 4, mine: false }], replyCount: 3 });
  add('design', 2, 'This feels like taking a deep breath. The green is so good.', t + 120_000, { threadRoot: root._id });
  add('design', 3, 'Yes to more breathing room! I have a few ideas for the empty states.', t + 180_000, { threadRoot: root._id });
  add('design', 0, 'The direction feels right. Let’s keep the little moments of warmth.', t + 240_000, { threadRoot: root._id });
  add('design', 2, 'Oh, this is lovely. The kind of app you actually want to open on a Monday.', t + 6 * 60_000, { reactions: [{ emoji: '🙌', count: 3, mine: false }] });
  add('design', 3, 'The tiny interactions are everything. Can we make the call feel like pulling up a chair, rather than joining a meeting?', t + 8 * 60_000, { replyCount: 0 });
  add('design', 0, 'Exactly this. Less “you are on mute”, more “hey, you’re here”. ✨', t + 9 * 60_000, { reactions: [{ emoji: '✨', count: 2, mine: false }] });
  add('design', 1, 'We’re hanging out in here if anyone wants to talk it through. No agenda, just a few good ideas. ☕', t + 12 * 60_000);
  add('build', 2, 'The connection indicator is in. It just says **Great**, **OK**, or **Poor**. Humans shouldn’t have to decode jitter stats.\n\n```ts\nconst quality = connection.health;\n// A little clarity goes a long way.\n```', now - 2_400_000, { reactions: [{ emoji: '👍', count: 4, mine: false }] });
  add('dm-maya', 1, 'Hey @Alex! Have you had a chance to look at the new direction? No rush, just excited to hear what you think. 🌱', now - 300_000);
  const inbox: InboxItem[] = [
    { _id: 'i1', kind: 'mention', message: root, room: rooms[1], space: spaces[0], done: false },
    { _id: 'i2', kind: 'reply', message: messages.find(m => m.threadRoot === root._id && m.author.user === 'sofia')!, room: rooms[1], space: spaces[0], done: false },
    { _id: 'i3', kind: 'dm', message: messages.at(-1)!, room: rooms.find(r => r._id === 'dm-maya')!, done: false },
    { _id: 'i4', kind: 'mention', message: add('build', 2, '@Alex the new audio controls are ready for a look when you have a moment. The prototype is at https://example.com', now - 720_000), room: rooms[2], space: spaces[0], done: false },
  ];
  return { people: structuredClone(people), spaces, rooms, messages, inbox, attachments: [] as Attachment[] };
}
