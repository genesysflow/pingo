//! Catch-up items are user-scoped and recheck room access before hydrating message content.
use crate::messages::{self, MessageView};
use crate::model::*;
use rxferrix::prelude::*;
#[object]
#[derive(Clone, Debug)]
pub struct InboxRoom {
    #[serde(rename = "_id")]
    pub id: Id<Room>,
    pub name: String,
    pub kind: RoomKind,
}
#[object]
#[derive(Clone, Debug)]
pub struct InboxSpace {
    #[serde(rename = "_id")]
    pub id: Id<Space>,
    pub name: String,
}
#[object]
#[derive(Clone, Debug)]
pub struct InboxItem {
    #[serde(rename = "_id")]
    pub id: Id<Mention>,
    pub kind: InboxKind,
    pub message: MessageView,
    pub room: InboxRoom,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub space: Option<InboxSpace>,
    pub done: bool,
}
#[query]
pub async fn list(ctx: &QueryCtx, include_done: Option<bool>) -> Result<Vec<InboxItem>> {
    let user = ctx.auth.require_user()?;
    let mut rows = ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_user_done, |q| {
            q.eq(Mention::user, user).eq(Mention::done, false)
        })
        .order(Order::Desc)
        .take(100)
        .await?;
    if include_done.unwrap_or(false) {
        rows.extend(
            ctx.db
                .query::<Mention>()
                .with_index(Mention::by_user_done, |q| {
                    q.eq(Mention::user, user).eq(Mention::done, true)
                })
                .order(Order::Desc)
                .take(100)
                .await?,
        );
        rows.sort_by(|a, b| {
            b.creation_time
                .total_cmp(&a.creation_time)
                .then_with(|| b.id.as_str().cmp(&a.id.as_str()))
        });
        rows.truncate(100);
    }
    let mut out = Vec::new();
    for n in rows {
        let Ok((r, _)) = require_room(&ctx.db, n.room, user, Perm::VIEW).await else {
            continue;
        };
        let Some(m) = ctx.db.get(n.message).await?.filter(|m| !m.deleted) else {
            continue;
        };
        let space = match r.space {
            Some(id) => ctx.db.get(id).await?.map(|s| InboxSpace {
                id,
                name: s.name.clone(),
            }),
            None => None,
        };
        out.push(InboxItem {
            id: n.id,
            kind: n.kind,
            message: messages::view(ctx, user, m).await?,
            room: InboxRoom {
                id: r.id,
                name: r.name.clone(),
                kind: r.kind,
            },
            space,
            done: n.done,
        });
    }
    Ok(out)
}
#[query]
pub async fn count(ctx: &QueryCtx) -> Result<u64> {
    let user = ctx.auth.require_user()?;
    Ok(ctx
        .db
        .aggregate(Mention::pending_per_user)
        .get(|q| q.eq(Mention::user, user).eq(Mention::done, false))
        .await?
        .count)
}
#[mutation]
#[allow(non_snake_case)]
pub async fn markDone(ctx: &MutationCtx, item: Id<Mention>) -> Result<()> {
    let user = ctx.auth.require_user()?;
    let n = ctx
        .db
        .get(item)
        .await?
        .ok_or_else(|| user_error("not_found", "Inbox item no longer exists"))?;
    if n.user != user {
        return Err(user_error("forbidden", "This is another person's inbox"));
    }
    if !n.done {
        ctx.db.patch(item, json!({"done": true})).await?;
    }
    Ok(())
}
/// A viewport acknowledges specific messages, so opening a room cannot consume unread threads.
#[mutation]
#[allow(non_snake_case)]
pub async fn markRead(ctx: &MutationCtx, messages: Vec<Id<Message>>) -> Result<()> {
    let user = ctx.auth.require_user()?;
    if messages.len() > 100 {
        return Err(user_error(
            "too_long",
            "Mark at most 100 messages read at once",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for id in messages {
        if !seen.insert(id) {
            continue;
        }
        get_message(&ctx.db, id, user).await?;
        let mut pagination = PaginationOpts::first(100);
        loop {
            let page = ctx
                .db
                .query::<Mention>()
                .with_index(Mention::by_user_message, |q| {
                    q.eq(Mention::user, user).eq(Mention::message, id)
                })
                .paginate(pagination)
                .await?;
            for item in page.page {
                if !item.done {
                    ctx.db.patch(item.id, json!({"done": true})).await?;
                }
            }
            if page.is_done {
                break;
            }
            pagination = PaginationOpts::after(page.continue_cursor, 100);
        }
    }
    Ok(())
}

/// Replying completes the conversation being answered, preserving other rooms and threads.
pub async fn finish_conversation(
    ctx: &MutationCtx,
    user: Id<rxferrix::auth::User>,
    room: Id<Room>,
    root: Option<Id<Message>>,
    through: f64,
    cursor: Option<String>,
) -> Result<()> {
    let page = ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_user_room_done, |q| {
            q.eq(Mention::user, user)
                .eq(Mention::room, room)
                .eq(Mention::done, false)
                .lte(Mention::_creation_time, through)
        })
        .paginate(cursor.map_or_else(
            || PaginationOpts::first(200),
            |c| PaginationOpts::after(c, 200),
        ))
        .await?;
    for item in page.page {
        if let Some(message) = ctx.db.get(item.message).await?
            && (message.thread_root == root || root == Some(message.id))
        {
            ctx.db.patch(item.id, json!({"done": true})).await?;
        }
    }
    if !page.is_done {
        ctx.scheduler
            .run_after::<finish_conversation_remaining>(
                std::time::Duration::ZERO,
                (user, room, root, through, page.continue_cursor),
            )
            .await?;
    }
    Ok(())
}

#[internal_mutation]
pub async fn finish_conversation_remaining(
    ctx: &MutationCtx,
    user: Id<rxferrix::auth::User>,
    room: Id<Room>,
    root: Option<Id<Message>>,
    through: f64,
    cursor: String,
) -> Result<()> {
    finish_conversation(ctx, user, room, root, through, Some(cursor)).await
}
#[mutation]
#[allow(non_snake_case)]
pub async fn markAllDone(ctx: &MutationCtx) -> Result<()> {
    let user = ctx.auth.require_user()?;
    let latest = ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_user_done, |q| {
            q.eq(Mention::user, user).eq(Mention::done, false)
        })
        .order(Order::Desc)
        .first()
        .await?;
    if let Some(latest) = latest {
        finish_batch(ctx, user, latest.creation_time).await?;
    }
    Ok(())
}
async fn finish_batch(
    ctx: &MutationCtx,
    user: Id<rxferrix::auth::User>,
    through: f64,
) -> Result<()> {
    let rows = ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_user_done, |q| {
            q.eq(Mention::user, user)
                .eq(Mention::done, false)
                .lte(Mention::_creation_time, through)
        })
        .take(1000)
        .await?;
    for n in &rows {
        ctx.db.patch(n.id, json!({"done": true})).await?;
    }
    if rows.len() == 1000 {
        ctx.scheduler
            .run_after::<finish_remaining>(std::time::Duration::ZERO, (user, through))
            .await?;
    }
    Ok(())
}
/// The cutoff keeps notifications arriving after "mark all done" out of its background batches.
#[internal_mutation]
pub async fn finish_remaining(
    ctx: &MutationCtx,
    user: Id<rxferrix::auth::User>,
    through: f64,
) -> Result<()> {
    finish_batch(ctx, user, through).await
}

pub async fn forget_room(
    ctx: &MutationCtx,
    user: Id<rxferrix::auth::User>,
    room: Id<Room>,
) -> Result<()> {
    let latest = ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_user_room_done, |q| {
            q.eq(Mention::user, user)
                .eq(Mention::room, room)
                .eq(Mention::done, false)
        })
        .order(Order::Desc)
        .first()
        .await?;
    if let Some(n) = latest {
        forget_batch(ctx, user, room, n.creation_time).await?;
    }
    Ok(())
}
async fn forget_batch(
    ctx: &MutationCtx,
    user: Id<rxferrix::auth::User>,
    room: Id<Room>,
    through: f64,
) -> Result<()> {
    let rows = ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_user_room_done, |q| {
            q.eq(Mention::user, user)
                .eq(Mention::room, room)
                .eq(Mention::done, false)
                .lte(Mention::_creation_time, through)
        })
        .take(1000)
        .await?;
    for n in &rows {
        ctx.db.delete(n.id).await?;
    }
    if rows.len() == 1000 {
        ctx.scheduler
            .run_after::<forget_remaining>(std::time::Duration::ZERO, (user, room, through))
            .await?;
    }
    Ok(())
}
#[internal_mutation]
pub async fn forget_remaining(
    ctx: &MutationCtx,
    user: Id<rxferrix::auth::User>,
    room: Id<Room>,
    through: f64,
) -> Result<()> {
    forget_batch(ctx, user, room, through).await
}
