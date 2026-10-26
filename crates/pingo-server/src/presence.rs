//! Heartbeats are coarse; a scheduled mutation, rather than query wall time, makes users offline.
use crate::model::*;
use rxferrix::QueryDb;
use rxferrix::auth::User;
use rxferrix::prelude::*;
pub async fn of(db: &QueryDb, user: Id<User>) -> Result<Option<Doc<Presence>>> {
    db.query::<Presence>()
        .with_index(Presence::by_user, |q| q.eq(Presence::user, user))
        .first()
        .await
}
#[mutation]
pub async fn heartbeat(ctx: &MutationCtx) -> Result<()> {
    let user = ctx.auth.require_user()?;
    touch(ctx, user, None, None).await
}
#[mutation]
#[allow(non_snake_case)]
pub async fn setStatus(
    ctx: &MutationCtx,
    status: PresenceStatus,
    text: Option<String>,
) -> Result<()> {
    let user = ctx.auth.require_user()?;
    if text.as_ref().is_some_and(|s| s.chars().count() > 200) {
        return Err(user_error(
            "too_long",
            "Status text is limited to 200 characters",
        ));
    }
    touch(ctx, user, Some(status), text).await
}
async fn touch(
    ctx: &MutationCtx,
    user: Id<User>,
    status: Option<PresenceStatus>,
    text: Option<String>,
) -> Result<()> {
    let existing = of(&ctx.db, user).await?;
    let expires_at = ctx.now() + 150_000.0;
    if let Some(p) = existing {
        let preferred = status.unwrap_or(p.preferred_status);
        let text = text.unwrap_or_else(|| p.status_text.clone());
        if status.is_some()
            && p.preferred_status == preferred
            && p.status == preferred
            && p.status_text == text
        {
            return Ok(());
        }
        // Ignore over-frequent heartbeats; clients need only one per minute.
        if status.is_none() && ctx.now() - p.last_heartbeat < 60_000.0 && p.status == preferred {
            return Ok(());
        }
        if p.last_heartbeat == ctx.now() && p.status == preferred && p.status_text == text {
            return Ok(());
        }
        ctx.db.patch(p.id, json!({"status": preferred, "preferredStatus": preferred, "statusText": text, "lastHeartbeat": ctx.now(), "expiresAt": expires_at})).await?;
        // Offline rows have no pending expiry job; all other rows keep their existing job.
        if p.expires_at <= ctx.now() {
            ctx.scheduler.run_at::<expire>(expires_at, (p.id,)).await?;
        }
    } else {
        let status = status.unwrap_or(PresenceStatus::Online);
        let id = ctx
            .db
            .insert(Presence {
                user,
                status,
                preferred_status: status,
                status_text: text.unwrap_or_default(),
                last_heartbeat: ctx.now(),
                expires_at,
            })
            .await?;
        ctx.scheduler.run_at::<expire>(expires_at, (id,)).await?;
    }
    Ok(())
}
#[internal_mutation]
pub async fn expire(ctx: &MutationCtx, id: Id<Presence>) -> Result<()> {
    if let Some(p) = ctx.db.get(id).await? {
        if p.expires_at <= ctx.now() {
            if p.status != PresenceStatus::Offline {
                ctx.db
                    .patch(id, json!({"status": PresenceStatus::Offline}))
                    .await?;
            }
        } else {
            ctx.scheduler.run_at::<expire>(p.expires_at, (id,)).await?;
        }
    }
    Ok(())
}
#[internal_mutation]
pub async fn prune(ctx: &MutationCtx) -> Result<u32> {
    let rows = ctx
        .db
        .query::<Presence>()
        .with_index(Presence::by_expires, |q| {
            q.lte(Presence::expires_at, ctx.now())
        })
        .filter(|p| p.status != PresenceStatus::Offline)
        .take(1000)
        .await?;
    for p in &rows {
        ctx.db
            .patch(p.id, json!({"status": PresenceStatus::Offline}))
            .await?;
    }
    if rows.len() == 1000 {
        ctx.scheduler
            .run_after::<prune>(std::time::Duration::ZERO, ())
            .await?;
    }
    Ok(rows.len() as u32)
}
