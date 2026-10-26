//! Message history and views share authorization with search, threads, and inbox hydration.
use crate::model::*;
use crate::profiles;
use rxferrix::auth::User;
use rxferrix::prelude::*;
use rxferrix::storage::StoredFile;
use std::collections::BTreeSet;

#[object]
#[derive(Clone, Debug)]
pub struct Author {
    pub user: Id<User>,
    pub name: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub avatar_url: Option<String>,
}
#[object]
#[derive(Clone, Debug)]
pub struct Attachment {
    pub id: Id<StoredFile>,
    pub url: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub height: Option<u32>,
}
#[object]
#[derive(Clone, Debug)]
pub struct ReactionGroup {
    pub emoji: String,
    pub count: u64,
    pub mine: bool,
}
#[object]
#[derive(Clone, Debug)]
pub struct MessageView {
    #[serde(rename = "_id")]
    pub id: Id<Message>,
    #[serde(rename = "_creationTime")]
    pub creation_time: f64,
    pub room: Id<Room>,
    pub author: Author,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_root: Option<Id<Message>>,
    pub reply_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_reply_at: Option<f64>,
    pub attachments: Vec<Attachment>,
    pub reactions: Vec<ReactionGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub edited_at: Option<f64>,
    pub deleted: bool,
}
#[object]
#[derive(Clone, Debug)]
pub struct Thread {
    pub root: MessageView,
    pub replies: Vec<MessageView>,
}
pub async fn view(ctx: &QueryCtx, me: Id<User>, m: Doc<Message>) -> Result<MessageView> {
    let p = profiles::view(ctx, m.author).await?;
    let author = Author {
        user: p.user,
        name: p.name,
        color: p.color,
        avatar_url: p.avatar_url,
    };
    let mut attachments = Vec::new();
    if !m.deleted {
        for id in &m.attachments {
            if let (Some(url), Some(meta)) = (
                ctx.storage.get_url(*id).await?,
                ctx.storage.get_metadata(*id).await?,
            ) {
                attachments.push(Attachment {
                    id: *id,
                    url,
                    name: meta.filename.clone().unwrap_or_else(|| "attachment".into()),
                    content_type: meta
                        .content_type
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".into()),
                    size: meta.size,
                    width: None,
                    height: None,
                });
            }
        }
    }
    let mut reactions = Vec::new();
    if !m.deleted {
        let mine = ctx
            .db
            .query::<Reaction>()
            .with_index(Reaction::by_message_user, |q| {
                q.eq(Reaction::message, m.id).eq(Reaction::user, me)
            })
            .take(20)
            .await?;
        for g in ctx
            .db
            .aggregate(Reaction::per_message_emoji)
            .groups(|q| q.eq(Reaction::message, m.id), Order::Asc, 20)
            .await?
        {
            if let Some(emoji) = g.group.get(1).and_then(|v| v.as_str()) {
                reactions.push(ReactionGroup {
                    emoji: emoji.into(),
                    count: g.value.count,
                    mine: mine.iter().any(|r| r.emoji == emoji),
                });
            }
        }
    }
    let reply_count = ctx
        .db
        .aggregate(Message::replies_per_root)
        .get(|q| q.eq(Message::thread_root, Some(m.id)))
        .await?
        .count;
    let last_reply_at = if reply_count > 0 {
        ctx.db
            .query::<Message>()
            .with_index(Message::by_thread, |q| {
                q.eq(Message::thread_root, Some(m.id))
            })
            .order(Order::Desc)
            .first()
            .await?
            .map(|r| r.creation_time)
    } else {
        None
    };
    Ok(MessageView {
        id: m.id,
        creation_time: m.creation_time,
        room: m.room,
        author,
        body: if m.deleted {
            String::new()
        } else {
            m.body.clone()
        },
        thread_root: m.thread_root,
        reply_count,
        last_reply_at,
        attachments,
        reactions,
        edited_at: m.edited_at,
        deleted: m.deleted,
    })
}
#[query]
pub async fn list(
    ctx: &QueryCtx,
    room: Id<Room>,
    pagination_opts: PaginationOpts,
) -> Result<PaginationResult<MessageView>> {
    let me = ctx.auth.require_user()?;
    require_room(&ctx.db, room, me, Perm::VIEW).await?;
    let page = ctx
        .db
        .query::<Message>()
        .with_index(Message::by_room_thread, |q| {
            q.eq(Message::room, room)
                .eq(Message::thread_root, None::<Id<Message>>)
        })
        .order(Order::Desc)
        .paginate(pagination_opts)
        .await?;
    page.try_map(|m| view(ctx, me, m)).await
}
#[query]
pub async fn thread(ctx: &QueryCtx, root: Id<Message>) -> Result<Thread> {
    let me = ctx.auth.require_user()?;
    let m = get_message(&ctx.db, root, me).await?;
    if m.thread_root.is_some() {
        return Err(user_error("not_found", "Choose the thread's root message"));
    }
    let rows = ctx
        .db
        .query::<Message>()
        .with_index(Message::by_thread, |q| {
            q.eq(Message::thread_root, Some(root))
        })
        .take(1000)
        .await?;
    let mut replies = Vec::new();
    for r in rows {
        replies.push(view(ctx, me, r).await?);
    }
    Ok(Thread {
        root: view(ctx, me, m).await?,
        replies,
    })
}
#[query]
pub async fn search(ctx: &QueryCtx, space: Id<Space>, text: String) -> Result<Vec<MessageView>> {
    let me = ctx.auth.require_user()?;
    require_perm(&ctx.db, space, me, Perm::VIEW).await?;
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    // Filter at the search index, before taking hits, so private rooms neither leak nor starve readable hits.
    let mut readable = BTreeSet::new();
    for r in ctx
        .db
        .query::<Room>()
        .with_index(Room::by_space_pos, |q| q.eq(Room::space, Some(space)))
        .take(1000)
        .await?
    {
        if require_room(&ctx.db, r.id, me, Perm::VIEW).await.is_ok() {
            readable.insert(r.id);
        }
    }
    let hits = ctx
        .db
        .query::<Message>()
        .with_search_index(Message::search_body, |q| q.search(Message::body, text))
        .filter(move |m| !m.deleted && readable.contains(&m.room))
        .take(50)
        .await?;
    let mut out = Vec::new();
    for m in hits {
        out.push(view(ctx, me, m).await?);
    }
    Ok(out)
}
fn validate_body(body: &str, attachments: usize) -> Result<String> {
    let body = body.trim();
    if body.chars().count() > 4000 || attachments > 10 {
        return Err(user_error(
            "too_long",
            "Use at most 4000 characters and 10 attachments",
        ));
    }
    if body.is_empty() && attachments == 0 {
        return Err(user_error(
            "invalid_name",
            "Write a message or attach a file",
        ));
    }
    Ok(body.into())
}
#[mutation]
pub async fn send(
    ctx: &MutationCtx,
    room: Id<Room>,
    body: String,
    thread_root: Option<Id<Message>>,
    attachments: Option<Vec<Id<StoredFile>>>,
) -> Result<Id<Message>> {
    let me = ctx.auth.require_user()?;
    let (r, _) = require_room(&ctx.db, room, me, Perm::SEND).await?;
    let attachments = attachments.unwrap_or_default();
    let body = validate_body(&body, attachments.len())?;
    for id in &attachments {
        crate::files::check_owned(ctx, *id).await?;
    }
    let root = match thread_root {
        Some(id) => {
            let m = get_message(&ctx.db, id, me).await?;
            if m.room != room || m.thread_root.is_some() || m.deleted {
                return Err(user_error(
                    "not_found",
                    "Thread root must be a message in this room",
                ));
            }
            Some(m)
        }
        None => None,
    };
    rate_limit(ctx, me, "send", 120).await?;
    let id = ctx
        .db
        .insert(Message {
            room,
            author: me,
            body: body.clone(),
            thread_root,
            attachments,
            edited_at: None,
            deleted: false,
        })
        .await?;
    notify_mentions(ctx, &r, me, id, &body).await?;
    if let Some(root) = root
        && root.author != me
    {
        notify(ctx, root.author, id, room, InboxKind::Reply).await?;
    }
    if r.kind == RoomKind::Dm {
        for user in &r.participants {
            if *user != me {
                notify(ctx, *user, id, room, InboxKind::Dm).await?;
            }
        }
    }
    crate::typing::clear(&ctx.db, room, me).await?;
    let sent_at = ctx.db.get(id).await?.map_or(ctx.now(), |m| m.creation_time);
    match crate::rooms::read_state(&ctx.db, room, me).await? {
        Some(s) => {
            if s.last_read_at < sent_at {
                ctx.db.patch(s.id, json!({"lastReadAt": sent_at})).await?;
            }
        }
        None => {
            ctx.db
                .insert(ReadState {
                    room,
                    user: me,
                    last_read_at: sent_at,
                })
                .await?;
        }
    }
    Ok(id)
}
pub async fn notify(
    ctx: &MutationCtx,
    user: Id<User>,
    message: Id<Message>,
    room: Id<Room>,
    kind: InboxKind,
) -> Result<()> {
    if require_room(&ctx.db, room, user, Perm::VIEW).await.is_ok() {
        ctx.db
            .insert(Mention {
                user,
                message,
                room,
                kind,
                done: false,
            })
            .await?;
    }
    Ok(())
}
/// A mention is bounded by word boundaries, including Unicode, to avoid matching @ann inside @anna.
fn mentioned(body: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    body.match_indices(&format!("@{name}")).any(|(at, token)| {
        body[..at].chars().next_back().is_none_or(|c| !word(c))
            && body[at + token.len()..]
                .chars()
                .next()
                .is_none_or(|c| !word(c))
    })
}
async fn notify_mentions(
    ctx: &MutationCtx,
    room: &Room,
    me: Id<User>,
    message: Id<Message>,
    body: &str,
) -> Result<()> {
    let Some(space) = room.space else {
        return Ok(());
    };
    let body = body.to_lowercase();
    let everyone = mentioned(&body, "everyone");
    let here = mentioned(&body, "here");
    let rows = ctx
        .db
        .query::<SpaceMember>()
        .with_index(SpaceMember::by_space_user, |q| {
            q.eq(SpaceMember::space, space)
        })
        .take(1000)
        .await?;
    let room_id = ctx
        .db
        .get(message)
        .await?
        .ok_or_else(|| user_error("not_found", "Message no longer exists"))?
        .room;
    for m in rows {
        if m.user == me {
            continue;
        }
        let (name, _, _) = profiles::identity(&ctx.db, m.user).await?;
        let name = m.nick.as_deref().unwrap_or(&name).to_lowercase();
        let online = if here {
            crate::presence::of(&ctx.db, m.user)
                .await?
                .is_some_and(|p| p.status == PresenceStatus::Online)
        } else {
            false
        };
        if everyone || online || mentioned(&body, &name) {
            notify(ctx, m.user, message, room_id, InboxKind::Mention).await?;
        }
    }
    Ok(())
}
#[mutation]
pub async fn edit(ctx: &MutationCtx, message: Id<Message>, body: String) -> Result<()> {
    let me = ctx.auth.require_user()?;
    let m = get_message(&ctx.db, message, me).await?;
    require_room(&ctx.db, m.room, me, Perm::SEND).await?;
    if m.author != me {
        return Err(user_error(
            "forbidden",
            "Only the author can edit a message",
        ));
    }
    if m.deleted {
        return Err(user_error("not_found", "This message was deleted"));
    }
    let body = validate_body(&body, m.attachments.len())?;
    if m.body == body {
        return Ok(());
    }
    ctx.db
        .patch(message, json!({"body": body, "editedAt": ctx.now()}))
        .await?;
    // Replace mention notifications, preserving completion for recipients that remain mentioned.
    let old = ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_message, |q| q.eq(Mention::message, message))
        .filter(|m| m.kind == InboxKind::Mention)
        .take(1000)
        .await?;
    for n in &old {
        ctx.db.delete(n.id).await?;
    }
    let r = ctx
        .db
        .get(m.room)
        .await?
        .ok_or_else(|| user_error("not_found", "Room no longer exists"))?;
    notify_mentions(ctx, &r, me, message, &body).await?;
    for n in ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_message, |q| q.eq(Mention::message, message))
        .filter(|m| m.kind == InboxKind::Mention)
        .take(1000)
        .await?
    {
        if old.iter().any(|o| o.user == n.user && o.done) {
            ctx.db.patch(n.id, json!({"done": true})).await?;
        }
    }
    Ok(())
}
#[mutation]
pub async fn remove(ctx: &MutationCtx, message: Id<Message>) -> Result<()> {
    let me = ctx.auth.require_user()?;
    let m = get_message(&ctx.db, message, me).await?;
    if m.author != me {
        require_room(&ctx.db, m.room, me, Perm::MANAGE_MESSAGES).await?;
    }
    if m.deleted {
        return Ok(());
    }
    ctx.db
        .patch(
            message,
            json!({"body": "", "attachments": [], "deleted": true}),
        )
        .await?;
    purge_related(ctx, message).await
}
pub async fn purge_related(ctx: &MutationCtx, message: Id<Message>) -> Result<()> {
    for n in ctx
        .db
        .query::<Mention>()
        .with_index(Mention::by_message, |q| q.eq(Mention::message, message))
        .take(3000)
        .await?
    {
        ctx.db.delete(n.id).await?;
    }
    for r in ctx
        .db
        .query::<Reaction>()
        .with_index(Reaction::by_message_user, |q| {
            q.eq(Reaction::message, message)
        })
        .take(20)
        .await?
    {
        ctx.db.delete(r.id).await?;
    }
    Ok(())
}
#[cfg(test)]
mod unit_tests {
    #[test]
    fn mention_boundaries() {
        assert!(super::mentioned("hi @ann, @mary jane!", "ann"));
        assert!(super::mentioned("hi @ann, @mary jane!", "mary jane"));
        assert!(!super::mentioned("@anna email@ann.com", "ann"));
        assert!(!super::mentioned("@everyone_else", "everyone"));
    }
}
