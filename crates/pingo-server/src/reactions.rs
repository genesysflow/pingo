//! Reaction toggles stay in one transaction and never touch the message itself.
use crate::model::*;
use rxferrix::prelude::*;
#[mutation]
pub async fn toggle(ctx: &MutationCtx, message: Id<Message>, emoji: String) -> Result<()> {
    let user = ctx.auth.require_user()?;
    let m = get_message(&ctx.db, message, user).await?;
    require_room(&ctx.db, m.room, user, Perm::SEND).await?;
    if m.deleted {
        return Err(user_error("not_found", "This message was deleted"));
    }
    let emoji = emoji.trim().to_owned();
    if emoji.is_empty() || emoji.chars().count() > 32 {
        return Err(user_error(
            "invalid_name",
            "Use an emoji or a short emoji name",
        ));
    }
    let old = ctx
        .db
        .query::<Reaction>()
        .with_index(Reaction::by_message_user, |q| {
            q.eq(Reaction::message, message)
                .eq(Reaction::user, user)
                .eq(Reaction::emoji, emoji.clone())
        })
        .first()
        .await?;
    if let Some(r) = old {
        ctx.db.delete(r.id).await?;
    } else {
        let count = ctx
            .db
            .aggregate(Reaction::per_message_emoji)
            .sum_over(|q| q.eq(Reaction::message, message))
            .await?
            .count;
        if count >= 20 {
            return Err(user_error(
                "too_long",
                "A message can have at most 20 reactions",
            ));
        }
        ctx.db
            .insert(Reaction {
                message,
                user,
                emoji,
            })
            .await?;
    }
    Ok(())
}
