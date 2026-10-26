//! Invite use counts and membership are updated in one transaction.
use crate::model::*;
use rxferrix::prelude::*;
#[object]
#[derive(Clone, Debug)]
pub struct Preview {
    pub space_name: String,
    pub accent: String,
    pub member_count: u64,
}
fn valid(i: &Invite, now: f64) -> bool {
    i.expires_at.is_none_or(|e| e > now) && i.max_uses.is_none_or(|m| i.uses < m)
}
async fn find(db: &rxferrix::QueryDb, code: String) -> Result<Option<Doc<Invite>>> {
    db.query::<Invite>()
        .with_index(Invite::by_code, |q| q.eq(Invite::code, code))
        .first()
        .await
}
#[mutation]
pub async fn create(
    ctx: &MutationCtx,
    space: Id<Space>,
    max_uses: Option<u32>,
    expires_in_hours: Option<f64>,
) -> Result<String> {
    require_perm(
        &ctx.db,
        space,
        crate::access::require_user(&ctx.auth, &ctx.env, &ctx.db).await?,
        Perm::INVITE,
    )
    .await?;
    if max_uses == Some(0)
        || expires_in_hours.is_some_and(|h| !h.is_finite() || h <= 0.0 || h > 8760.0)
    {
        return Err(user_error(
            "invalid_invite",
            "Use a positive use limit and an expiry within a year",
        ));
    }
    // The randomly generated document ID provides an unguessable invite code without external I/O.
    let id = ctx
        .db
        .insert(Invite {
            space,
            code: String::new(),
            uses: 0,
            max_uses,
            expires_at: expires_in_hours.map(|h| ctx.now() + h * 3_600_000.0),
        })
        .await?;
    if let Some(hours) = expires_in_hours {
        ctx.scheduler
            .run_at::<expire>(ctx.now() + hours * 3_600_000.0, (id,))
            .await?;
    }
    let code = id.to_string();
    ctx.db.patch(id, json!({"code": code})).await?;
    Ok(code)
}
#[query]
pub async fn preview(ctx: &QueryCtx, code: String) -> Result<Option<Preview>> {
    let Some(i) = find(&ctx.db, code).await? else {
        return Ok(None);
    };
    if !valid(&i, ctx.now()) {
        return Ok(None);
    }
    let Some(s) = ctx.db.get(i.space).await? else {
        return Ok(None);
    };
    let member_count = ctx
        .db
        .aggregate(SpaceMember::members_per_space)
        .get(|q| q.eq(SpaceMember::space, s.id))
        .await?
        .count;
    Ok(Some(Preview {
        space_name: s.name.clone(),
        accent: s.accent.clone(),
        member_count,
    }))
}
#[mutation]
pub async fn accept(ctx: &MutationCtx, code: String) -> Result<Id<Space>> {
    // The way into an invite-only server, so only the email rule applies here.
    let user = crate::access::require_invitable(&ctx.auth, &ctx.env, &ctx.db).await?;
    let i = find(&ctx.db, code)
        .await?
        .filter(|i| valid(i, ctx.now()))
        .ok_or_else(|| {
            user_error(
                "invalid_invite",
                "This invite expired or has no uses remaining",
            )
        })?;
    if ctx.db.get(i.space).await?.is_none() {
        return Err(user_error("invalid_invite", "This space no longer exists"));
    }
    if member(&ctx.db, i.space, user).await?.is_none() {
        ctx.db
            .insert(SpaceMember {
                space: i.space,
                user,
                roles: vec![],
                nick: None,
            })
            .await?;
        ctx.db.patch(i.id, json!({"uses": i.uses + 1})).await?;
    }
    Ok(i.space)
}

/// Expiry must write so a live anonymous preview becomes invalid even when the space is otherwise quiet.
#[internal_mutation]
pub async fn expire(ctx: &MutationCtx, id: Id<Invite>) -> Result<()> {
    if let Some(i) = ctx.db.get(id).await?
        && i.expires_at.is_some_and(|e| e <= ctx.now())
    {
        ctx.db.delete(id).await?;
    }
    Ok(())
}
