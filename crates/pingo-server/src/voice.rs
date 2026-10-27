//! Short-lived connect tokens grant capabilities; only trusted SFU mutations write call state.
use crate::model::*;
use pingo_proto::MediaClaims;
use rxferrix::auth::User;
use rxferrix::prelude::*;
// Rust callers, including the integration's SFU hooks, keep the conventional snake_case name.
#[allow(unused_imports)]
pub use setState as set_state;

pub const DEV_MEDIA_SECRET: &str = "pingo-insecure-dev-media-secret";
#[object]
#[derive(Clone, Debug)]
pub struct VoiceParticipant {
    pub user: Id<User>,
    pub name: String,
    pub color: String,
    pub muted: bool,
    pub deafened: bool,
    pub sharing_screen: bool,
    pub video: bool,
}
#[object]
#[derive(Clone)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
}
impl std::fmt::Debug for IceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IceServer")
            .field("urls", &self.urls)
            .finish_non_exhaustive()
    }
}
#[object]
#[derive(Clone, Debug)]
pub struct JoinResult {
    pub token: String,
    #[ts(type = "\"/rtc\"")]
    pub url: String,
    pub ice_servers: Vec<IceServer>,
    /// Unix milliseconds; independent of the 60-second SFU admission token.
    pub ice_expires_at: Option<f64>,
}

async fn turn_setting(env: &rxferrix::env::Env, key: &str) -> Result<Option<String>> {
    // An empty dashboard value disables the process setting too.
    Ok(env
        .get(key)
        .await?
        .or_else(|| std::env::var(key).ok())
        .filter(|s| !s.trim().is_empty()))
}

async fn turn_servers(
    env: &rxferrix::env::Env,
    user: Id<User>,
    now: f64,
) -> Result<(Vec<IceServer>, Option<f64>)> {
    let urls = turn_setting(env, "PINGO_TURN_URLS").await?;
    let secret = turn_setting(env, "PINGO_TURN_SECRET").await?;
    let (urls, secret) = match (urls, secret) {
        (None, None) => return Ok((vec![], None)),
        (Some(urls), Some(secret)) => (urls, secret),
        _ => {
            return Err(user_error(
                "turn_configuration",
                "Set both PINGO_TURN_URLS and PINGO_TURN_SECRET, or clear both",
            ));
        }
    };
    let urls: Vec<String> = urls.split(',').map(str::trim).map(str::to_owned).collect();
    if urls.is_empty()
        || urls.len() > 8
        || urls
            .iter()
            .any(|url| pingo_proto::ice::TurnUrl::parse(url).is_err())
    {
        return Err(user_error(
            "turn_configuration",
            "PINGO_TURN_URLS must contain at most 8 comma-separated turn: or turns: URLs",
        ));
    }
    let expiry = (now / 1000.0).floor() as u64 + 3600;
    let username = format!("{expiry}:{user}");
    let credential = pingo_proto::ice::turn_password(&username, secret.as_bytes());
    Ok((
        vec![IceServer {
            urls,
            username,
            credential,
        }],
        Some(expiry as f64 * 1000.0),
    ))
}
// Signing and admission share the exact same missing/empty-key and dev policy.
pub async fn media_secret(env: &rxferrix::env::Env, dev: bool) -> Result<String> {
    // Environment first (rotatable at run time), then the process environment (Docker).
    let configured = env
        .get("PINGO_MEDIA_SECRET")
        .await?
        .filter(|s| !s.is_empty());
    match configured.or_else(|| {
        static PROCESS: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        PROCESS
            .get_or_init(|| {
                std::env::var("PINGO_MEDIA_SECRET")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .clone()
    }) {
        Some(s) => Ok(s),
        None if dev => {
            tracing::warn!(
                "PINGO_MEDIA_SECRET is unset; using the insecure development media secret"
            );
            Ok(DEV_MEDIA_SECRET.into())
        }
        None => Err(user_error(
            "forbidden",
            "PINGO_MEDIA_SECRET is unset; set the PINGO_MEDIA_SECRET environment variable or run `pingo-server env set PINGO_MEDIA_SECRET`",
        )),
    }
}

#[internal_query]
pub async fn admission_secret(ctx: &QueryCtx) -> Result<String> {
    media_secret(&ctx.env, ctx.is_dev()).await
}

/// Clear in bounded transactions before the fresh SFU can accept any sessions.
#[internal_mutation]
pub async fn reset(ctx: &MutationCtx) -> Result<u32> {
    let rows = ctx.db.query::<VoiceState>().take(500).await?;
    for row in &rows {
        ctx.db.delete(row.id).await?;
    }
    Ok(rows.len() as u32)
}

#[mutation]
pub async fn join(ctx: &MutationCtx, room: Id<Room>) -> Result<JoinResult> {
    let user = ctx.auth.require_user()?;
    let (_, perms) = require_room(&ctx.db, room, user, Perm::VIEW).await?;
    let secret = media_secret(&ctx.env, ctx.is_dev()).await?;
    let (ice_servers, ice_expires_at) = turn_servers(&ctx.env, user, ctx.now()).await?;
    let (name, color, _) = crate::profiles::identity(&ctx.db, user).await?;
    let claims = MediaClaims {
        room: room.to_string(),
        user: user.to_string(),
        name,
        color,
        can_speak: perms.contains(Perm::SPEAK),
        can_video: perms.contains(Perm::VIDEO),
        can_share: perms.contains(Perm::SHARE_SCREEN),
        exp: ctx.now() + 60_000.0,
    };
    Ok(JoinResult {
        token: pingo_proto::token::sign(&claims, secret.as_bytes()),
        url: "/rtc".into(),
        ice_servers,
        ice_expires_at,
    })
}
pub async fn views(ctx: &QueryCtx, room: Id<Room>) -> Result<Vec<VoiceParticipant>> {
    let rows = ctx
        .db
        .query::<VoiceState>()
        .with_index(VoiceState::by_room_user, |q| q.eq(VoiceState::room, room))
        .take(1000)
        .await?;
    let mut out = Vec::new();
    for s in rows {
        if require_room(&ctx.db, room, s.user, Perm::VIEW)
            .await
            .is_err()
        {
            continue;
        }
        let (name, color, _) = crate::profiles::identity(&ctx.db, s.user).await?;
        out.push(VoiceParticipant {
            user: s.user,
            name,
            color,
            muted: s.muted,
            deafened: s.deafened,
            sharing_screen: s.sharing_screen,
            video: s.video,
        });
    }
    Ok(out)
}
#[query]
pub async fn participants(ctx: &QueryCtx, room: Id<Room>) -> Result<Vec<VoiceParticipant>> {
    require_room(&ctx.db, room, ctx.auth.require_user()?, Perm::VIEW).await?;
    views(ctx, room).await
}
#[internal_mutation]
#[allow(non_snake_case)]
pub async fn setState(
    ctx: &MutationCtx,
    room: Id<Room>,
    user: Id<User>,
    muted: bool,
    deafened: bool,
    sharing_screen: bool,
    video: bool,
) -> Result<()> {
    let (_, perms) = require_room(&ctx.db, room, user, Perm::VIEW).await?;
    if (!muted && !perms.contains(Perm::SPEAK))
        || (video && !perms.contains(Perm::VIDEO))
        || (sharing_screen && !perms.contains(Perm::SHARE_SCREEN))
    {
        return Err(user_error(
            "forbidden",
            "Call state exceeds the user's media permissions",
        ));
    }
    let old = ctx
        .db
        .query::<VoiceState>()
        .with_index(VoiceState::by_room_user, |q| {
            q.eq(VoiceState::room, room).eq(VoiceState::user, user)
        })
        .first()
        .await?;
    if let Some(s) = old {
        // A repeated SFU snapshot is still a liveness update, at most once per minute when unchanged.
        if s.muted == muted
            && s.deafened == deafened
            && s.sharing_screen == sharing_screen
            && s.video == video
            && ctx.now() - s.updated_at < 60_000.0
        {
            return Ok(());
        }
        ctx.db.patch(s.id, json!({"muted": muted, "deafened": deafened, "sharingScreen": sharing_screen, "video": video, "updatedAt": ctx.now()})).await?;
    } else {
        ctx.db
            .insert(VoiceState {
                room,
                user,
                muted,
                deafened,
                sharing_screen,
                video,
                joined_at: ctx.now(),
                updated_at: ctx.now(),
            })
            .await?;
    }
    Ok(())
}
pub async fn remove_state(ctx: &MutationCtx, room: Id<Room>, user: Id<User>) -> Result<()> {
    if let Some(s) = ctx
        .db
        .query::<VoiceState>()
        .with_index(VoiceState::by_room_user, |q| {
            q.eq(VoiceState::room, room).eq(VoiceState::user, user)
        })
        .first()
        .await?
    {
        ctx.db.delete(s.id).await?;
    }
    Ok(())
}
#[internal_mutation]
pub async fn left(ctx: &MutationCtx, room: Id<Room>, user: Id<User>) -> Result<()> {
    remove_state(ctx, room, user).await
}
#[internal_mutation]
pub async fn prune(ctx: &MutationCtx) -> Result<u32> {
    let rows = ctx
        .db
        .query::<VoiceState>()
        .with_index(VoiceState::by_updated, |q| {
            q.lt(VoiceState::updated_at, ctx.now() - 43_200_000.0)
        })
        .take(1000)
        .await?;
    for s in &rows {
        ctx.db.delete(s.id).await?;
    }
    if rows.len() == 1000 {
        ctx.scheduler
            .run_after::<prune>(std::time::Duration::ZERO, ())
            .await?;
    }
    Ok(rows.len() as u32)
}
