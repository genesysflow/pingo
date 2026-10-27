# Backend API contract (pingo-server → generated `api`)

This is the contract between `crates/pingo-server` (RxFerrix functions) and `apps/web` (UI). The generated
TypeScript `api` object mirrors it as `api.<module>.<function>`. Rust names are snake_case; JSON/TS fields are
camelCase (RxFerrix does that automatically). Ids are strings in TS (`Id<"rooms">` etc.).

If the backend needs to deviate, update this file in the same change.

Every function requires a signed-in user unless noted, and that user must be admitted by the registration policy
(`access.rs`, see below): RxFerrix's public-function guard enforces admission centrally, otherwise failing with
`invite_required`, `email_not_allowed` or `email_not_verified`. Authorization goes through one helper,
`require_perm(ctx, space, Perm::X)`. Errors use `user_error(code, message)` with the codes listed below.

## Tables (see PLAN.md §4.1)

`spaces`, `space_members`, `roles`, `rooms`, `messages`, `reactions`, `read_states`, `typing`, `presence`,
`voice_states`, `mentions`, `invites`, `profiles`.

- `RoomKind` = `"chat" | "lounge" | "stage" | "dm"`
- `AudioProfile` = `"voice" | "studio"`
- `PresenceStatus` = `"online" | "idle" | "dnd" | "offline"`
- Permissions are a `u64` bitflag on roles: `VIEW, SEND, MANAGE_MESSAGES, MANAGE_ROOMS, MANAGE_SPACE, INVITE, SPEAK,
  VIDEO, SHARE_SCREEN, MODERATE_VOICE`. Space owners have every permission. A default `@everyone` role is created
  with each space: `VIEW|SEND|INVITE|SPEAK|VIDEO|SHARE_SCREEN`.

## Shapes returned to the client

```ts
Profile        { user, name, color, avatarUrl?: string, status: PresenceStatus, statusText: string }
SpaceSummary   { _id, name, slug, accent, iconUrl?: string, unread: boolean, mentions: number }
SpaceInfo      { ...Space, myPerms: number, memberCount: number, roles: Role[] }
RoomSummary    { _id, name, topic, kind, private, position, unread: number /* capped 100 */, mentions: number,
                 voice: VoiceParticipant[] /* who's in the room's call right now */ }
VoiceParticipant { user, name, color, muted, deafened, sharingScreen, video }
MessageView    { _id, _creationTime, room, author: { user, name, color, avatarUrl? }, body, threadRoot?,
                 replyCount, lastReplyAt?, attachments: { id, url, name, contentType, size, width?, height? }[],
                 reactions: { emoji, count, mine: boolean }[], editedAt?, deleted }
InboxItem      { _id, kind: "mention" | "reply" | "dm", message: MessageView, room: { _id, name, kind },
                 space?: { _id, name }, done: boolean }
TypingUser     { user, name }
Member         { user, name, color, avatarUrl?, status, roles: Id[] }
```

## Functions

| Module | Function | Kind | Args | Returns |
|---|---|---|---|---|
| access | `status` | query | – | `{ registration: "open" \| "invite", restrictedEmails, setup, admission?, email? }` (no auth required; `admission` = `"admitted" \| "verifyEmail" \| "emailNotAllowed" \| "inviteRequired"` when signed in; `setup` = no space exists yet, so the first account may sign up) |
| profiles | `me` | query | – | `Profile \| null` (null when signed out, no auth required) |
| profiles | `update` | mutation | `name?, color?, avatar?: Id<"_storage">` | – |
| spaces | `list` | query | – | `SpaceSummary[]` |
| spaces | `get` | query | `space` | `SpaceInfo` |
| spaces | `create` | mutation | `name, accent?` | `Id<"spaces">` (creates `#general` chat room + `Lounge` lounge room) |
| spaces | `update` | mutation | `space, name?, accent?, icon?` | – (MANAGE_SPACE) |
| spaces | `members` | query | `space` | `Member[]` |
| spaces | `leave` | mutation | `space` | – |
| invites | `create` | mutation | `space, maxUses?, expiresInHours?` | `string` code (INVITE) |
| invites | `preview` | query | `code` | `{ spaceName, accent, memberCount } \| null` (no auth required) |
| invites | `accept` | mutation | `code` | `Id<"spaces">` (allowed before admission: the way into an invite-only server; the email rule still applies) |
| rooms | `list` | query | `space` | `RoomSummary[]` ordered by position |
| rooms | `get` | query | `room` | `Room & { myPerms }` |
| rooms | `create` | mutation | `space, name, topic?, kind, private` | `Id<"rooms">` (MANAGE_ROOMS) |
| rooms | `update` | mutation | `room, name?, topic?, audioProfile?, position?` | – (MANAGE_ROOMS) |
| rooms | `remove` | mutation | `room` | – (MANAGE_ROOMS) |
| rooms | `markRead` | mutation | `room` | – (skips no-op writes) |
| dms | `open` | mutation | `user` | `Id<"rooms">` (kind `dm`, no space; reuses existing) |
| dms | `list` | query | – | `{ room, other: Member, unread, lastMessageAt }[]` |
| messages | `list` | query, paginated | `room, paginationOpts` | `PaginationResult<MessageView>` newest first, top-level only |
| messages | `thread` | query | `root` | `{ root: MessageView, replies: MessageView[] }` |
| messages | `send` | mutation | `room, body, threadRoot?, attachments?: Id<"_storage">[]` | `Id<"messages">` (parses `@name` mentions → `mentions` rows; clears typing) |
| messages | `edit` | mutation | `message, body` | – (author only) |
| messages | `remove` | mutation | `message` | – (author or MANAGE_MESSAGES; soft delete) |
| messages | `search` | query | `space, text` | `MessageView[]` (max 50, FTS, only readable rooms) |
| files | `generateUploadUrl` | mutation | – | upload URL (RxFerrix storage) |
| reactions | `toggle` | mutation | `message, emoji` | – |
| typing | `start` | mutation | `room` | – (row expires after 4 s via scheduler) |
| typing | `list` | query | `room` | `TypingUser[]` (excludes caller) |
| presence | `heartbeat` | mutation | – | – (client calls every 60 s; offline after 150 s via scheduler) |
| presence | `setStatus` | mutation | `status, text?` | – |
| inbox | `list` | query | `includeDone?` | `InboxItem[]` (max 100, newest first) |
| inbox | `count` | query | – | `number` (not done) |
| inbox | `markDone` | mutation | `item` | – |
| inbox | `markAllDone` | mutation | – | – |
| voice | `join` | mutation | `room` | `{ token, url: "/rtc", iceServers: { urls: string[], username: string, credential: string }[], iceExpiresAt: number \| null }` – signs a `pingo_proto::MediaClaims` token (60 s) with env `PINGO_MEDIA_SECRET`; capabilities from perms. Optional coturn REST credentials last one hour; `iceExpiresAt` is Unix milliseconds. TURN disabled: `[]` and `null`. |
| voice | `participants` | query | `room` | `VoiceParticipant[]` |
| voice | `setState` | internal mutation | `room, user, muted, deafened, sharingScreen, video` | – (called by the SFU only; upserts `voice_states`) |
| voice | `left` | internal mutation | `room, user` | – (called by the SFU only) |

Error codes: `not_found`, `forbidden`, `invalid_name`, `name_taken`, `invalid_invite`, `rate_limited`, `too_long`,
`invite_required`, `email_not_allowed`, `email_not_verified`, `misconfigured` (an invalid registration setting).

## Registration policy

Read from RxFerrix Environment (dashboard or `pingo-server env set`), falling back to the process environment:

- `PINGO_REGISTRATION` = `open` (default) or `invite`. With `invite`, a user is admitted once they belong to a
  space; until the first space exists, any account is admitted so it can create one.
- `PINGO_ALLOWED_EMAILS`: addresses and domains (`ada@example.com, @example.com`). When set, the account's email
  must match and be verified, invites included.

RxFerrix's `before_sign_up` hook rejects disallowed accounts before they are created (password, magic link/code,
and OAuth). After setup, invite-only sign-ups must pass an allowed `redirectTo` URL with the path `/invite/<code>`;
the invite must exist, be unexpired, have uses remaining, and refer to an existing space. Checking sign-up does
not consume the invite: `invites.accept` still checks and consumes it transactionally after sign-in. Password
sign-ups with an allowed but unverified address must verify it before using protected functions. Existing accounts
are checked by the guard on every public client call and subscribed-query rerun. `access.status`, `profiles.me`,
`invites.preview` and dev-only utilities remain exempt; `invites.accept` checks the email rule only. Internal/server
calls are trusted and bypass the guard. The dynamic policy remains app code because RxFerrix's builder allowlists
are fixed at startup.

Limits: message body ≤ 4000 chars, room/space name 1–32 chars, ≤ 10 attachments, 20 reactions per message.
