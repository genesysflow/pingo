export type PresenceStatus = 'online' | 'idle' | 'dnd' | 'offline';
export type RoomKind = 'chat' | 'lounge' | 'stage' | 'dm';
export type AudioProfile = 'voice' | 'studio';
export interface Profile {
  user: string;
  name: string;
  color: string;
  avatarUrl?: string;
  status: PresenceStatus;
  statusText: string;
}
export interface Member extends Omit<Profile, 'statusText'> {
  roles: string[];
}
export interface SpaceSummary {
  _id: string;
  name: string;
  slug: string;
  accent: string;
  iconUrl?: string;
  unread: boolean;
  mentions: number;
}
export interface Role {
  _id: string;
  space: string;
  name: string;
  color: string;
  perms: number;
  position: number;
}
export interface SpaceInfo extends SpaceSummary {
  owner: string;
  myPerms: number;
  memberCount: number;
  roles: Role[];
}
export interface VoiceParticipant {
  user: string;
  name: string;
  color: string;
  muted: boolean;
  deafened: boolean;
  sharingScreen: boolean;
  video: boolean;
}
export interface RoomSummary {
  _id: string;
  space?: string | null;
  name: string;
  topic: string;
  kind: RoomKind;
  private: boolean;
  position: number;
  unread: number;
  mentions: number;
  voice: VoiceParticipant[];
}
export interface Room extends RoomSummary {
  audioProfile: AudioProfile;
  myPerms: number;
}
export interface Attachment {
  id: string;
  url: string;
  name: string;
  contentType: string;
  size: number;
  width?: number;
  height?: number;
}
export interface MessageView {
  _id: string;
  _creationTime: number;
  room: string;
  author: Pick<Profile, 'user' | 'name' | 'color' | 'avatarUrl'>;
  body: string;
  threadRoot?: string;
  replyCount: number;
  lastReplyAt?: number;
  attachments: Attachment[];
  reactions: { emoji: string; count: number; mine: boolean }[];
  editedAt?: number;
  deleted: boolean;
}
export interface InboxItem {
  _id: string;
  kind: 'mention' | 'reply' | 'dm';
  message: MessageView;
  room: Pick<Room, '_id' | 'name' | 'kind'>;
  space?: { _id: string; name: string };
  done: boolean;
}
export interface TypingUser {
  user: string;
  name: string;
}
export interface Dm {
  room: string;
  other: Member;
  unread: number;
  lastMessageAt: number;
}
