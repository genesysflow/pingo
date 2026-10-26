//! Profile views resolve an account fallback so first sign-in needs no initialization mutation.
use crate::model::{self, PresenceStatus, Profile, Role};
use rxferrix::QueryDb;
use rxferrix::auth::User;
use rxferrix::prelude::*;
use rxferrix::storage::StoredFile;

#[object]
#[derive(Clone, Debug)]
pub struct ProfileView {
    pub user: Id<User>,
    pub name: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub avatar_url: Option<String>,
    pub status: PresenceStatus,
    pub status_text: String,
}
#[object]
#[derive(Clone, Debug)]
pub struct Member {
    pub user: Id<User>,
    pub name: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub avatar_url: Option<String>,
    pub status: PresenceStatus,
    pub roles: Vec<Id<Role>>,
}
pub async fn profile_of(db: &QueryDb, user: Id<User>) -> Result<Option<Doc<Profile>>> {
    db.query::<Profile>()
        .with_index(Profile::by_user, |q| q.eq(Profile::user, user))
        .first()
        .await
}
pub async fn identity(
    db: &QueryDb,
    user: Id<User>,
) -> Result<(String, String, Option<Id<StoredFile>>)> {
    if let Some(p) = profile_of(db, user).await? {
        return Ok((p.name.clone(), p.color.clone(), p.avatar));
    }
    let account = db.get(user).await?;
    let name = account
        .and_then(|u| {
            u.name.clone().or_else(|| {
                u.email
                    .as_ref()
                    .map(|e| e.split('@').next().unwrap_or(e).to_owned())
            })
        })
        .unwrap_or_else(|| "someone".into());
    Ok((name, "#64748b".into(), None))
}
pub async fn view(ctx: &QueryCtx, user: Id<User>) -> Result<ProfileView> {
    let (name, color, avatar) = identity(&ctx.db, user).await?;
    let avatar_url = match avatar {
        Some(id) => ctx.storage.get_url(id).await?,
        None => None,
    };
    let presence = crate::presence::of(&ctx.db, user).await?;
    Ok(ProfileView {
        user,
        name,
        color,
        avatar_url,
        status: presence
            .as_ref()
            .map_or(PresenceStatus::Offline, |p| p.status),
        status_text: presence.map_or_else(String::new, |p| p.status_text.clone()),
    })
}
pub async fn member_view(
    ctx: &QueryCtx,
    user: Id<User>,
    roles: Vec<Id<Role>>,
    nick: Option<String>,
) -> Result<Member> {
    let p = view(ctx, user).await?;
    Ok(Member {
        user,
        name: nick.unwrap_or(p.name),
        color: p.color,
        avatar_url: p.avatar_url,
        status: p.status,
        roles,
    })
}
#[query]
pub async fn me(ctx: &QueryCtx) -> Result<Option<ProfileView>> {
    match ctx.auth.user_id() {
        Some(user) => Ok(Some(view(ctx, user).await?)),
        None => Ok(None),
    }
}
#[mutation]
pub async fn update(
    ctx: &MutationCtx,
    name: Option<String>,
    color: Option<String>,
    avatar: Option<Id<StoredFile>>,
) -> Result<()> {
    let user = crate::access::require_user(&ctx.auth, &ctx.env, &ctx.db).await?;
    let (old_name, old_color, old_avatar) = identity(&ctx.db, user).await?;
    let name = match name {
        Some(n) => model::valid_name(&n)?,
        None => old_name.clone(),
    };
    let color = color.unwrap_or_else(|| old_color.clone());
    if color.len() != 7
        || !color.starts_with('#')
        || !color[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(model::user_error("invalid_name", "Use a #rrggbb color"));
    }
    if let Some(id) = avatar {
        crate::files::check_owned(ctx, id).await?;
    }
    let avatar = avatar.or(old_avatar);
    if let Some(p) = profile_of(&ctx.db, user).await? {
        if name != old_name || color != old_color || avatar != old_avatar {
            ctx.db
                .patch(
                    p.id,
                    json!({"name": name, "color": color, "avatar": avatar}),
                )
                .await?;
        }
    } else {
        ctx.db
            .insert(Profile {
                user,
                name,
                color,
                avatar,
            })
            .await?;
    }
    Ok(())
}
