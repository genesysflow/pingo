//! Sidebar reads use room-scoped ranges; unread work stops at the badge limit.
use crate::model::*;
use crate::voice::{self, VoiceParticipant};
use rxferrix::QueryDb;
use rxferrix::auth::User;
use rxferrix::prelude::*;
#[object]
#[derive(Clone, Debug)]
pub struct RoomSummary {
    #[serde(rename = "_id")]
    pub id: Id<Room>,
    pub name: String,
    pub topic: String,
    pub kind: RoomKind,
    pub private: bool,
    pub position: i32,
    pub unread: u32,
    pub mentions: u64,
    pub voice: Vec<VoiceParticipant>,
}
#[object]
#[derive(Clone, Debug)]
pub struct RoomInfo {
    #[serde(flatten)]
    pub room: Doc<Room>,
    pub my_perms: u64,
}
pub async fn read_state(
    db: &QueryDb,
    room: Id<Room>,
    user: Id<User>,
) -> Result<Option<Doc<ReadState>>> {
    db.query::<ReadState>()
        .with_index(ReadState::by_room_user, |q| {
            q.eq(ReadState::room, room).eq(ReadState::user, user)
        })
        .first()
        .await
}
pub async fn unread(db: &QueryDb, room: Id<Room>, user: Id<User>) -> Result<u32> {
    let since = read_state(db, room, user)
        .await?
        .map_or(0.0, |r| r.last_read_at);
    let rows = db
        .query::<Message>()
        .with_index(Message::by_room, |q| {
            q.eq(Message::room, room).gt(Message::_creation_time, since)
        })
        .filter(move |m| m.author != user && !m.deleted)
        .take(100)
        .await?;
    Ok(rows.len() as u32)
}
pub async fn summaries(
    ctx: &QueryCtx,
    space: Id<Space>,
    user: Id<User>,
) -> Result<Vec<RoomSummary>> {
    let rows = ctx
        .db
        .query::<Room>()
        .with_index(Room::by_space_pos, |q| q.eq(Room::space, Some(space)))
        .take(1000)
        .await?;
    let mut out = Vec::new();
    for r in rows {
        if require_room(&ctx.db, r.id, user, Perm::VIEW).await.is_err() {
            continue;
        }
        let mentions = ctx
            .db
            .aggregate(Mention::pending_per_room)
            .get(|q| {
                q.eq(Mention::user, user)
                    .eq(Mention::room, r.id)
                    .eq(Mention::done, false)
            })
            .await?
            .count;
        out.push(RoomSummary {
            id: r.id,
            name: r.name.clone(),
            topic: r.topic.clone(),
            kind: r.kind,
            private: r.private,
            position: r.position,
            unread: unread(&ctx.db, r.id, user).await?,
            mentions,
            voice: voice::views(ctx, r.id).await?,
        });
    }
    Ok(out)
}
#[query]
pub async fn list(ctx: &QueryCtx, space: Id<Space>) -> Result<Vec<RoomSummary>> {
    let user = ctx.auth.require_user()?;
    require_perm(&ctx.db, space, user, Perm::VIEW).await?;
    summaries(ctx, space, user).await
}
#[query]
pub async fn get(ctx: &QueryCtx, room: Id<Room>) -> Result<RoomInfo> {
    let (room, perms) = require_room(&ctx.db, room, ctx.auth.require_user()?, Perm::VIEW).await?;
    Ok(RoomInfo {
        room,
        my_perms: perms.bits(),
    })
}
async fn available(
    db: &QueryDb,
    space: Id<Space>,
    name: &str,
    except: Option<Id<Room>>,
) -> Result<()> {
    let old = db
        .query::<Room>()
        .with_index(Room::by_space_name, |q| {
            q.eq(Room::space, Some(space))
                .eq(Room::name, name.to_owned())
        })
        .first()
        .await?;
    if old.is_some_and(|r| Some(r.id) != except) {
        return Err(user_error(
            "name_taken",
            "A room with this name already exists",
        ));
    }
    Ok(())
}
#[mutation]
pub async fn create(
    ctx: &MutationCtx,
    space: Id<Space>,
    name: String,
    topic: Option<String>,
    kind: RoomKind,
    private: bool,
) -> Result<Id<Room>> {
    let user = ctx.auth.require_user()?;
    require_perm(&ctx.db, space, user, Perm::MANAGE_ROOMS).await?;
    if kind == RoomKind::Dm {
        return Err(user_error("forbidden", "Use dms.open for direct messages"));
    }
    let name = valid_name(&name)?;
    available(&ctx.db, space, &name, None).await?;
    let position = ctx
        .db
        .query::<Room>()
        .with_index(Room::by_space_pos, |q| q.eq(Room::space, Some(space)))
        .order(Order::Desc)
        .first()
        .await?
        .map_or(0, |r| r.position.saturating_add(1));
    ctx.db
        .insert(Room {
            space: Some(space),
            name,
            topic: topic.unwrap_or_default().chars().take(200).collect(),
            kind,
            private,
            position,
            audio_profile: AudioProfile::Voice,
            participants: if private { vec![user] } else { vec![] },
            dm_key: None,
        })
        .await
}
#[mutation]
pub async fn update(
    ctx: &MutationCtx,
    room: Id<Room>,
    name: Option<String>,
    topic: Option<String>,
    audio_profile: Option<AudioProfile>,
    position: Option<i32>,
) -> Result<()> {
    let (r, _) = require_room(&ctx.db, room, ctx.auth.require_user()?, Perm::MANAGE_ROOMS).await?;
    let mut patch = json!({});
    if let Some(n) = name {
        let n = valid_name(&n)?;
        if let Some(space) = r.space {
            available(&ctx.db, space, &n, Some(room)).await?;
        }
        if r.name != n {
            patch["name"] = json!(n);
        }
    }
    if let Some(t) = topic {
        let t: String = t.chars().take(200).collect();
        if r.topic != t {
            patch["topic"] = json!(t);
        }
    }
    if let Some(a) = audio_profile
        && r.audio_profile != a
    {
        patch["audioProfile"] = json!(a);
    }
    if let Some(p) = position
        && r.position != p
    {
        patch["position"] = json!(p);
    }
    if patch.as_object().is_some_and(|p| !p.is_empty()) {
        ctx.db.patch(room, patch).await?;
    }
    Ok(())
}
#[mutation]
pub async fn remove(ctx: &MutationCtx, room: Id<Room>) -> Result<()> {
    require_room(&ctx.db, room, ctx.auth.require_user()?, Perm::MANAGE_ROOMS).await?;
    ctx.db.delete(room).await?;
    // No public function can read orphaned history. Bounded batches reclaim it asynchronously.
    ctx.scheduler
        .run_after::<cleanup>(std::time::Duration::ZERO, (room,))
        .await?;
    Ok(())
}
#[internal_mutation]
pub async fn cleanup(ctx: &MutationCtx, room: Id<Room>) -> Result<()> {
    let rows = ctx
        .db
        .query::<Message>()
        .with_index(Message::by_room, |q| q.eq(Message::room, room))
        .take(100)
        .await?;
    for m in &rows {
        crate::messages::purge_related(ctx, m.id).await?;
        ctx.db.delete(m.id).await?;
    }
    let mut more = rows.len() == 100;
    let state = ctx
        .db
        .query::<ReadState>()
        .with_index(ReadState::by_room_user, |q| q.eq(ReadState::room, room))
        .take(100)
        .await?;
    more |= state.len() == 100;
    for r in state {
        ctx.db.delete(r.id).await?;
    }
    let state = ctx
        .db
        .query::<Typing>()
        .with_index(Typing::by_room_user, |q| q.eq(Typing::room, room))
        .take(100)
        .await?;
    more |= state.len() == 100;
    for r in state {
        ctx.db.delete(r.id).await?;
    }
    let state = ctx
        .db
        .query::<VoiceState>()
        .with_index(VoiceState::by_room_user, |q| q.eq(VoiceState::room, room))
        .take(100)
        .await?;
    more |= state.len() == 100;
    for r in state {
        ctx.db.delete(r.id).await?;
    }
    if more {
        ctx.scheduler
            .run_after::<cleanup>(std::time::Duration::ZERO, (room,))
            .await?;
    }
    Ok(())
}
#[mutation]
#[allow(non_snake_case)]
pub async fn markRead(ctx: &MutationCtx, room: Id<Room>) -> Result<()> {
    let user = ctx.auth.require_user()?;
    require_room(&ctx.db, room, user, Perm::VIEW).await?;
    let latest = ctx
        .db
        .query::<Message>()
        .with_index(Message::by_room, |q| q.eq(Message::room, room))
        .order(Order::Desc)
        .first()
        .await?;
    let Some(latest) = latest else { return Ok(()) };
    let existing = read_state(&ctx.db, room, user).await?;
    if existing
        .as_ref()
        .is_some_and(|r| r.last_read_at >= latest.creation_time)
    {
        return Ok(());
    }
    let last_read_at = ctx.now().max(latest.creation_time);
    if let Some(r) = existing {
        ctx.db
            .patch(r.id, json!({"lastReadAt": last_read_at}))
            .await?;
    } else {
        ctx.db
            .insert(ReadState {
                room,
                user,
                last_read_at,
            })
            .await?;
    }
    Ok(())
}
