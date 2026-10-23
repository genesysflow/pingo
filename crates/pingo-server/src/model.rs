//! Durable data and shared authorization; ephemeral signals stay on the media data channel.
use bitflags::bitflags;
use rxferrix::QueryDb;
use rxferrix::auth::User;
use rxferrix::prelude::*;
use rxferrix::storage::StoredFile;

#[object]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomKind {
    Chat,
    Lounge,
    Stage,
    Dm,
}
#[object]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioProfile {
    Voice,
    Studio,
}
#[object]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceStatus {
    Online,
    Idle,
    Dnd,
    Offline,
}
#[object]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InboxKind {
    Mention,
    Reply,
    Dm,
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Perm: u64 {
        const VIEW = 1 << 0;
        const SEND = 1 << 1;
        const MANAGE_MESSAGES = 1 << 2;
        const MANAGE_ROOMS = 1 << 3;
        const MANAGE_SPACE = 1 << 4;
        const INVITE = 1 << 5;
        const SPEAK = 1 << 6;
        const VIDEO = 1 << 7;
        const SHARE_SCREEN = 1 << 8;
        const MODERATE_VOICE = 1 << 9;
    }
}
pub const DEFAULT_PERMS: Perm = Perm::VIEW
    .union(Perm::SEND)
    .union(Perm::INVITE)
    .union(Perm::SPEAK)
    .union(Perm::VIDEO)
    .union(Perm::SHARE_SCREEN);

#[table(name = "spaces")]
#[index(by_slug = [slug])]
pub struct Space {
    pub name: String,
    pub slug: String,
    pub icon: Option<Id<StoredFile>>,
    pub accent: String,
    pub owner: Id<User>,
}
#[table(name = "space_members")]
#[index(by_space_user = [space, user])]
#[index(by_user = [user])]
#[aggregate(members_per_space, group = [space])]
pub struct SpaceMember {
    pub space: Id<Space>,
    pub user: Id<User>,
    pub roles: Vec<Id<Role>>,
    pub nick: Option<String>,
}
#[table(name = "roles")]
#[index(by_space = [space])]
pub struct Role {
    pub space: Id<Space>,
    pub name: String,
    pub color: String,
    pub perms: u64,
    pub position: i32,
}
#[table(name = "rooms")]
#[index(by_space_pos = [space, position])]
#[index(by_space_name = [space, name])]
#[index(by_dm_key = [dm_key])]
pub struct Room {
    pub space: Option<Id<Space>>,
    pub name: String,
    pub topic: String,
    pub kind: RoomKind,
    pub private: bool,
    pub position: i32,
    pub audio_profile: AudioProfile,
    /// Explicit access for private rooms and DMs; space owners retain access to space rooms.
    pub participants: Vec<Id<User>>,
    pub dm_key: Option<String>,
}
#[table(name = "messages")]
#[index(by_room = [room])]
#[index(by_room_thread = [room, thread_root])]
#[index(by_thread = [thread_root])]
#[search_index(search_body, search = body, filter = [room])]
#[aggregate(messages_per_room, group = [room])]
#[aggregate(replies_per_root, group = [thread_root])]
pub struct Message {
    pub room: Id<Room>,
    pub author: Id<User>,
    pub body: String,
    pub thread_root: Option<Id<Message>>,
    pub attachments: Vec<Id<StoredFile>>,
    pub edited_at: Option<f64>,
    pub deleted: bool,
}
#[table(name = "profiles")]
#[index(by_user = [user])]
pub struct Profile {
    pub user: Id<User>,
    pub name: String,
    pub color: String,
    pub avatar: Option<Id<StoredFile>>,
}
#[table(name = "reactions")]
#[index(by_message_user = [message, user, emoji])]
#[aggregate(per_message_emoji, group = [message, emoji])]
pub struct Reaction {
    pub message: Id<Message>,
    pub user: Id<User>,
    pub emoji: String,
}
#[table(name = "read_states")]
#[index(by_room_user = [room, user])]
pub struct ReadState {
    pub room: Id<Room>,
    pub user: Id<User>,
    pub last_read_at: f64,
}
#[table(name = "typing")]
#[index(by_room_user = [room, user])]
#[index(by_expires = [expires_at])]
pub struct Typing {
    pub room: Id<Room>,
    pub user: Id<User>,
    pub expires_at: f64,
}
#[table(name = "presence")]
#[index(by_user = [user])]
#[index(by_expires = [expires_at])]
pub struct Presence {
    pub user: Id<User>,
    pub status: PresenceStatus,
    pub status_text: String,
    pub last_heartbeat: f64,
    pub expires_at: f64,
    /// Preserve the user's idle/DND preference across a disconnect.
    pub preferred_status: PresenceStatus,
}
#[table(name = "voice_states")]
#[index(by_room_user = [room, user])]
#[index(by_updated = [updated_at])]
pub struct VoiceState {
    pub room: Id<Room>,
    pub user: Id<User>,
    pub muted: bool,
    pub deafened: bool,
    pub sharing_screen: bool,
    pub video: bool,
    pub joined_at: f64,
    pub updated_at: f64,
}
#[table(name = "mentions")]
#[index(by_user_done = [user, done])]
#[index(by_user_room_done = [user, room, done])]
#[index(by_message = [message])]
#[aggregate(pending_per_user, group = [user, done])]
#[aggregate(pending_per_room, group = [user, room, done])]
pub struct Mention {
    pub user: Id<User>,
    pub message: Id<Message>,
    pub room: Id<Room>,
    pub kind: InboxKind,
    pub done: bool,
}
#[table(name = "invites")]
#[index(by_code = [code])]
pub struct Invite {
    pub space: Id<Space>,
    pub code: String,
    pub uses: u32,
    pub max_uses: Option<u32>,
    pub expires_at: Option<f64>,
}
/// A small per-user fixed-window counter, independent of message documents.
#[table(name = "rate_limits")]
#[index(by_user_kind = [user, kind])]
pub struct RateLimit {
    pub user: Id<User>,
    pub kind: String,
    pub count: u32,
    pub resets_at: f64,
}

pub fn user_error(code: &str, message: impl Into<String>) -> Error {
    UserError::new(code, message).into()
}
pub fn valid_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 32 || name.chars().any(char::is_control) {
        return Err(user_error(
            "invalid_name",
            "Names must have 1–32 characters",
        ));
    }
    Ok(name.to_owned())
}
pub async fn member(
    db: &QueryDb,
    space: Id<Space>,
    user: Id<User>,
) -> Result<Option<Doc<SpaceMember>>> {
    db.query::<SpaceMember>()
        .with_index(SpaceMember::by_space_user, |q| {
            q.eq(SpaceMember::space, space).eq(SpaceMember::user, user)
        })
        .first()
        .await
}
pub async fn require_perm(
    db: &QueryDb,
    space: Id<Space>,
    user: Id<User>,
    needed: Perm,
) -> Result<Perm> {
    let s = db
        .get(space)
        .await?
        .ok_or_else(|| user_error("not_found", "Space no longer exists"))?;
    if s.owner == user {
        return Ok(Perm::all());
    }
    let m = member(db, space, user)
        .await?
        .ok_or_else(|| user_error("forbidden", "Join this space first"))?;
    let roles = db
        .query::<Role>()
        .with_index(Role::by_space, |q| q.eq(Role::space, space))
        .take(1000)
        .await?;
    let mut perms = Perm::empty();
    for role in roles {
        if role.name == "@everyone" || m.roles.contains(&role.id) {
            perms |= Perm::from_bits_truncate(role.perms);
        }
    }
    if !perms.contains(needed) {
        return Err(user_error(
            "forbidden",
            "Your roles do not allow this action",
        ));
    }
    Ok(perms)
}
pub async fn require_room(
    db: &QueryDb,
    room: Id<Room>,
    user: Id<User>,
    needed: Perm,
) -> Result<(Doc<Room>, Perm)> {
    let r = db
        .get(room)
        .await?
        .ok_or_else(|| user_error("not_found", "Room no longer exists"))?;
    let perms = match r.space {
        Some(space) => {
            let perms = require_perm(db, space, user, needed | Perm::VIEW).await?;
            let owner = db.get(space).await?.is_some_and(|s| s.owner == user);
            if r.private && !owner && !r.participants.contains(&user) {
                return Err(user_error("forbidden", "This room is private"));
            }
            perms
        }
        None if r.kind == RoomKind::Dm && r.participants.contains(&user) => {
            if !DEFAULT_PERMS.contains(needed) {
                return Err(user_error(
                    "forbidden",
                    "This action is not available in DMs",
                ));
            }
            DEFAULT_PERMS
        }
        None => return Err(user_error("forbidden", "This conversation is private")),
    };
    Ok((r, perms))
}
pub async fn get_message(
    db: &QueryDb,
    message: Id<Message>,
    user: Id<User>,
) -> Result<Doc<Message>> {
    let m = db
        .get(message)
        .await?
        .ok_or_else(|| user_error("not_found", "Message no longer exists"))?;
    require_room(db, m.room, user, Perm::VIEW).await?;
    Ok(m)
}
pub async fn rate_limit(ctx: &MutationCtx, user: Id<User>, kind: &str, max: u32) -> Result<()> {
    let row = ctx
        .db
        .query::<RateLimit>()
        .with_index(RateLimit::by_user_kind, |q| {
            q.eq(RateLimit::user, user)
                .eq(RateLimit::kind, kind.to_owned())
        })
        .first()
        .await?;
    match row {
        Some(r) if r.resets_at > ctx.now() => {
            if r.count >= max {
                return Err(user_error(
                    "rate_limited",
                    "Please wait a moment before trying again",
                ));
            }
            ctx.db.patch(r.id, json!({"count": r.count + 1})).await?;
        }
        Some(r) => {
            ctx.db
                .patch(r.id, json!({"count": 1, "resetsAt": ctx.now() + 60_000.0}))
                .await?;
            ctx.scheduler
                .run_at::<expire_rate>(ctx.now() + 60_000.0, (r.id,))
                .await?;
        }
        None => {
            let id = ctx
                .db
                .insert(RateLimit {
                    user,
                    kind: kind.into(),
                    count: 1,
                    resets_at: ctx.now() + 60_000.0,
                })
                .await?;
            ctx.scheduler
                .run_at::<expire_rate>(ctx.now() + 60_000.0, (id,))
                .await?;
        }
    }
    Ok(())
}
#[internal_mutation]
pub async fn expire_rate(ctx: &MutationCtx, id: Id<RateLimit>) -> Result<()> {
    if let Some(r) = ctx.db.get(id).await?
        && r.resets_at <= ctx.now()
    {
        ctx.db.delete(id).await?;
    }
    Ok(())
}
/// A user-scoped index into DM rooms avoids scanning unrelated conversations.
#[table(name = "dm_members")]
#[index(by_user_room = [user, room])]
pub struct DmMember {
    pub user: Id<User>,
    pub room: Id<Room>,
}
