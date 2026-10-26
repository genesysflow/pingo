//! One pending expiry job per typing row reschedules itself when typing extends the deadline.
use crate::model::*;
use rxferrix::auth::User;
use rxferrix::prelude::*;
use rxferrix::{MutationDb, QueryDb};
#[object]
#[derive(Clone, Debug)]
pub struct TypingUser {
    pub user: Id<User>,
    pub name: String,
}
async fn of(db: &QueryDb, room: Id<Room>, user: Id<User>) -> Result<Option<Doc<Typing>>> {
    db.query::<Typing>()
        .with_index(Typing::by_room_user, |q| {
            q.eq(Typing::room, room).eq(Typing::user, user)
        })
        .first()
        .await
}
pub async fn clear(db: &MutationDb, room: Id<Room>, user: Id<User>) -> Result<()> {
    if let Some(t) = of(db, room, user).await? {
        db.delete(t.id).await?;
    }
    Ok(())
}
#[mutation]
pub async fn start(ctx: &MutationCtx, room: Id<Room>) -> Result<()> {
    let user = ctx.auth.require_user()?;
    require_room(&ctx.db, room, user, Perm::SEND).await?;
    let expires_at = ctx.now() + 4000.0;
    match of(&ctx.db, room, user).await? {
        Some(t) => {
            if t.expires_at != expires_at {
                ctx.db.patch(t.id, json!({"expiresAt": expires_at})).await?;
            }
        }
        None => {
            let id = ctx
                .db
                .insert(Typing {
                    room,
                    user,
                    expires_at,
                })
                .await?;
            ctx.scheduler.run_at::<expire>(expires_at, (id,)).await?;
        }
    }
    Ok(())
}
#[query]
pub async fn list(ctx: &QueryCtx, room: Id<Room>) -> Result<Vec<TypingUser>> {
    let me = ctx.auth.require_user()?;
    require_room(&ctx.db, room, me, Perm::VIEW).await?;
    let rows = ctx
        .db
        .query::<Typing>()
        .with_index(Typing::by_room_user, |q| q.eq(Typing::room, room))
        .take(50)
        .await?;
    let mut out = Vec::new();
    for t in rows {
        if t.user != me
            && require_room(&ctx.db, room, t.user, Perm::VIEW)
                .await
                .is_ok()
        {
            let (name, _, _) = crate::profiles::identity(&ctx.db, t.user).await?;
            out.push(TypingUser { user: t.user, name });
        }
    }
    Ok(out)
}
#[internal_mutation]
pub async fn expire(ctx: &MutationCtx, id: Id<Typing>) -> Result<()> {
    if let Some(t) = ctx.db.get(id).await? {
        if t.expires_at <= ctx.now() {
            ctx.db.delete(id).await?;
        } else {
            ctx.scheduler.run_at::<expire>(t.expires_at, (id,)).await?;
        }
    }
    Ok(())
}
#[internal_mutation]
pub async fn prune(ctx: &MutationCtx) -> Result<u32> {
    let rows = ctx
        .db
        .query::<Typing>()
        .with_index(Typing::by_expires, |q| q.lte(Typing::expires_at, ctx.now()))
        .take(1000)
        .await?;
    for t in &rows {
        ctx.db.delete(t.id).await?;
    }
    if rows.len() == 1000 {
        ctx.scheduler
            .run_after::<prune>(std::time::Duration::ZERO, ())
            .await?;
    }
    Ok(rows.len() as u32)
}
