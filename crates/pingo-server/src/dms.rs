//! Canonical pair keys make opening the same DM idempotent, including simultaneous requests.
use crate::model::*;
use crate::{profiles, rooms};
use rxferrix::auth::User;
use rxferrix::prelude::*;
#[object]
#[derive(Clone, Debug)]
pub struct DmSummary {
    pub room: Id<Room>,
    pub other: profiles::Member,
    pub unread: u32,
    pub last_message_at: f64,
}
#[mutation]
pub async fn open(ctx: &MutationCtx, user: Id<User>) -> Result<Id<Room>> {
    let me = crate::access::require_user(&ctx.auth, &ctx.env, &ctx.db).await?;
    if me == user {
        return Err(user_error("forbidden", "Choose another person"));
    }
    if ctx.db.get(user).await?.is_none() {
        return Err(user_error("not_found", "User no longer exists"));
    }
    let mut pair = [me.to_string(), user.to_string()];
    pair.sort();
    let key = Some(pair.join(":"));
    if let Some(r) = ctx
        .db
        .query::<Room>()
        .with_index(Room::by_dm_key, |q| q.eq(Room::dm_key, key.clone()))
        .first()
        .await?
    {
        return Ok(r.id);
    }
    let room = ctx
        .db
        .insert(Room {
            space: None,
            name: "Direct message".into(),
            topic: String::new(),
            kind: RoomKind::Dm,
            private: true,
            position: 0,
            audio_profile: AudioProfile::Voice,
            participants: vec![me, user],
            dm_key: key,
        })
        .await?;
    for user in [me, user] {
        ctx.db.insert(DmMember { user, room }).await?;
    }
    Ok(room)
}
#[query]
pub async fn list(ctx: &QueryCtx) -> Result<Vec<DmSummary>> {
    let me = crate::access::require_user(&ctx.auth, &ctx.env, &ctx.db).await?;
    let rows = ctx
        .db
        .query::<DmMember>()
        .with_index(DmMember::by_user_room, |q| q.eq(DmMember::user, me))
        .take(500)
        .await?;
    let mut out = Vec::new();
    for row in rows {
        let (r, _) = require_room(&ctx.db, row.room, me, Perm::VIEW).await?;
        let Some(other) = r.participants.iter().find(|u| **u != me).copied() else {
            continue;
        };
        let last = ctx
            .db
            .query::<Message>()
            .with_index(Message::by_room, |q| q.eq(Message::room, r.id))
            .order(Order::Desc)
            .first()
            .await?;
        out.push(DmSummary {
            room: r.id,
            other: profiles::member_view(ctx, other, vec![], None).await?,
            unread: rooms::unread(&ctx.db, r.id, me).await?,
            last_message_at: last.map_or(r.creation_time, |m| m.creation_time),
        });
    }
    out.sort_by(|a, b| {
        b.last_message_at
            .total_cmp(&a.last_message_at)
            .then_with(|| a.room.as_str().cmp(&b.room.as_str()))
    });
    Ok(out)
}
