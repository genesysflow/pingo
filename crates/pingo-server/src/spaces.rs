//! Space membership and the default role establish the authorization boundary.
use crate::model::*;
use crate::{profiles, rooms};
use rxferrix::auth::User;
use rxferrix::prelude::*;
use rxferrix::storage::StoredFile;

#[object]
#[derive(Clone, Debug)]
pub struct SpaceSummary {
    #[serde(rename = "_id")]
    pub id: Id<Space>,
    pub name: String,
    pub slug: String,
    pub accent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub icon_url: Option<String>,
    pub unread: bool,
    pub mentions: u64,
}
#[object]
#[derive(Clone, Debug)]
pub struct SpaceInfo {
    #[serde(flatten)]
    pub space: Doc<Space>,
    pub my_perms: u64,
    pub member_count: u64,
    pub roles: Vec<Doc<Role>>,
}
#[query]
pub async fn list(ctx: &QueryCtx) -> Result<Vec<SpaceSummary>> {
    let user = ctx.auth.require_user()?;
    let memberships = ctx
        .db
        .query::<SpaceMember>()
        .with_index(SpaceMember::by_user, |q| q.eq(SpaceMember::user, user))
        .take(500)
        .await?;
    let mut out = Vec::new();
    for m in memberships {
        let Some(s) = ctx.db.get(m.space).await? else {
            continue;
        };
        if require_perm(&ctx.db, s.id, user, Perm::VIEW).await.is_err() {
            continue;
        }
        let summaries = rooms::summaries(ctx, s.id, user).await?;
        let icon_url = match s.icon {
            Some(id) => ctx.storage.get_url(id).await?,
            None => None,
        };
        out.push(SpaceSummary {
            id: s.id,
            name: s.name.clone(),
            slug: s.slug.clone(),
            accent: s.accent.clone(),
            icon_url,
            unread: summaries.iter().any(|r| r.unread > 0),
            mentions: summaries.iter().map(|r| r.mentions).sum(),
        });
    }
    Ok(out)
}
#[query]
pub async fn get(ctx: &QueryCtx, space: Id<Space>) -> Result<SpaceInfo> {
    let user = ctx.auth.require_user()?;
    let my_perms = require_perm(&ctx.db, space, user, Perm::VIEW).await?.bits();
    let space_doc = ctx
        .db
        .get(space)
        .await?
        .ok_or_else(|| user_error("not_found", "Space no longer exists"))?;
    let roles = ctx
        .db
        .query::<Role>()
        .with_index(Role::by_space, |q| q.eq(Role::space, space))
        .take(1000)
        .await?;
    let member_count = ctx
        .db
        .aggregate(SpaceMember::members_per_space)
        .get(|q| q.eq(SpaceMember::space, space))
        .await?
        .count;
    Ok(SpaceInfo {
        space: space_doc,
        my_perms,
        member_count,
        roles,
    })
}
#[mutation]
pub async fn create(ctx: &MutationCtx, name: String, accent: Option<String>) -> Result<Id<Space>> {
    let user = ctx.auth.require_user()?;
    create_for(ctx, user, name, accent).await
}
pub async fn create_for(
    ctx: &MutationCtx,
    user: Id<User>,
    name: String,
    accent: Option<String>,
) -> Result<Id<Space>> {
    let name = valid_name(&name)?;
    let base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let mut slug = base.clone();
    let mut suffix = 1;
    while ctx
        .db
        .query::<Space>()
        .with_index(Space::by_slug, |q| q.eq(Space::slug, slug.clone()))
        .first()
        .await?
        .is_some()
    {
        suffix += 1;
        slug = format!("{base}-{suffix}");
    }
    let space = ctx
        .db
        .insert(Space {
            name,
            slug,
            accent: accent.unwrap_or_else(|| "#ff6b35".into()),
            icon: None,
            owner: user,
        })
        .await?;
    ctx.db
        .insert(Role {
            space,
            name: "@everyone".into(),
            color: "#64748b".into(),
            perms: DEFAULT_PERMS.bits(),
            position: 0,
        })
        .await?;
    ctx.db
        .insert(SpaceMember {
            space,
            user,
            roles: vec![],
            nick: None,
        })
        .await?;
    for (position, name, kind) in [
        (0, "general", RoomKind::Chat),
        (1, "Lounge", RoomKind::Lounge),
    ] {
        ctx.db
            .insert(Room {
                space: Some(space),
                name: name.into(),
                topic: String::new(),
                kind,
                private: false,
                position,
                audio_profile: AudioProfile::Voice,
                participants: vec![],
                dm_key: None,
            })
            .await?;
    }
    Ok(space)
}
#[mutation]
pub async fn update(
    ctx: &MutationCtx,
    space: Id<Space>,
    name: Option<String>,
    accent: Option<String>,
    icon: Option<Id<StoredFile>>,
) -> Result<()> {
    require_perm(&ctx.db, space, ctx.auth.require_user()?, Perm::MANAGE_SPACE).await?;
    let mut patch = json!({});
    if let Some(n) = name {
        patch["name"] = json!(valid_name(&n)?);
    }
    if let Some(a) = accent {
        patch["accent"] = json!(a);
    }
    if let Some(i) = icon {
        crate::files::check_owned(ctx, i).await?;
        patch["icon"] = json!(i);
    }
    let old = ctx
        .db
        .get(space)
        .await?
        .ok_or_else(|| user_error("not_found", "Space no longer exists"))?;
    let value = json!(old.value);
    if patch
        .as_object()
        .is_some_and(|p| p.iter().any(|(k, v)| value.get(k) != Some(v)))
    {
        ctx.db.patch(space, patch).await?;
    }
    Ok(())
}
#[query]
pub async fn members(ctx: &QueryCtx, space: Id<Space>) -> Result<Vec<profiles::Member>> {
    require_perm(&ctx.db, space, ctx.auth.require_user()?, Perm::VIEW).await?;
    let rows = ctx
        .db
        .query::<SpaceMember>()
        .with_index(SpaceMember::by_space_user, |q| {
            q.eq(SpaceMember::space, space)
        })
        .take(1000)
        .await?;
    let mut out = Vec::new();
    for r in rows {
        out.push(profiles::member_view(ctx, r.user, r.roles.clone(), r.nick.clone()).await?);
    }
    out.sort_by_key(|m| (m.name.to_lowercase(), m.user.to_string()));
    Ok(out)
}
#[mutation]
pub async fn leave(ctx: &MutationCtx, space: Id<Space>) -> Result<()> {
    let user = ctx.auth.require_user()?;
    let s = ctx
        .db
        .get(space)
        .await?
        .ok_or_else(|| user_error("not_found", "Space no longer exists"))?;
    if s.owner == user {
        return Err(user_error(
            "forbidden",
            "The owner cannot leave their space",
        ));
    }
    if let Some(m) = member(&ctx.db, space, user).await? {
        // Drop call and typing state so a departed member cannot linger in the sidebar.
        for r in ctx
            .db
            .query::<Room>()
            .with_index(Room::by_space_pos, |q| q.eq(Room::space, Some(space)))
            .take(1000)
            .await?
        {
            crate::voice::remove_state(ctx, r.id, user).await?;
            crate::typing::clear(&ctx.db, r.id, user).await?;
            crate::inbox::forget_room(ctx, user, r.id).await?;
        }
        ctx.db.delete(m.id).await?;
    }
    Ok(())
}
