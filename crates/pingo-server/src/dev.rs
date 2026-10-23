//! A development inbox: in `--dev` without SMTP, auth emails go to rxferrix's dev mailer (which prints them to the
//! log) and are also kept in memory, so the e2e test can read sign-in codes with the dev-only `dev:last_code`
//! action. Outside `--dev` (or with SMTP configured) the inbox is never used and the action is refused.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};

use rxferrix::auth::{DevLogMailer, Email, Mailer};
use rxferrix::prelude::*;

static INBOX: LazyLock<Arc<DevInbox>> = LazyLock::new(Arc::default);

#[derive(Default)]
pub struct DevInbox {
    emails: Mutex<Vec<Email>>,
}

impl Mailer for DevInbox {
    fn send(
        &self,
        email: Email,
    ) -> Pin<Box<dyn Future<Output = rxferrix::Result<()>> + Send + '_>> {
        let mut emails = self.emails.lock().unwrap_or_else(|e| e.into_inner());
        emails.push(email.clone());
        let excess = emails.len().saturating_sub(200);
        emails.drain(..excess);
        drop(emails);
        // Still printed to the log, like every dev email.
        DevLogMailer.send(email)
    }
}

/// The inbox, for `AuthConfig::dev_mailer` (rxferrix only uses it in `--dev` without SMTP).
pub fn inbox() -> Arc<DevInbox> {
    INBOX.clone()
}

/// The newest 6-digit code emailed to `email`.
#[action(dev_only)]
pub async fn last_code(_ctx: &ActionCtx, email: String) -> Result<Option<String>> {
    let email = email.trim().to_lowercase();
    let emails = INBOX.emails.lock().unwrap_or_else(|e| e.into_inner());
    Ok(emails
        .iter()
        .rev()
        .filter(|e| e.to == email)
        .find_map(|e| code_in(&e.body)))
}

fn code_in(body: &str) -> Option<String> {
    let rest = &body[body.find("Your code: ")? + "Your code: ".len()..];
    let code: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (code.len() == 6).then_some(code)
}

/// Idempotent sample data for a fresh development server; never callable in production.
#[mutation(dev_only)]
pub async fn seed(ctx: &MutationCtx) -> Result<Id<crate::model::Space>> {
    use crate::model::*;
    use rxferrix::auth::User;
    if let Some(s) = ctx
        .db
        .query::<Space>()
        .with_index(Space::by_slug, |q| {
            q.eq(Space::slug, "pingo-demo".to_owned())
        })
        .first()
        .await?
    {
        return Ok(s.id);
    }
    let mut users = Vec::new();
    for (name, email, color) in [
        ("Alex", "alex@pingo.test", "#7c3aed"),
        ("Sam", "sam@pingo.test", "#059669"),
        ("Maya", "maya@pingo.test", "#e11d48"),
    ] {
        let user = match ctx
            .db
            .query::<User>()
            .with_index(User::by_email, |q| {
                q.eq(User::email, Some(email.to_owned()))
            })
            .first()
            .await?
        {
            Some(u) => u.id,
            None => {
                ctx.db
                    .insert(User {
                        email: Some(email.into()),
                        email_verified: true,
                        name: Some(name.into()),
                        image: None,
                    })
                    .await?
            }
        };
        if crate::profiles::profile_of(&ctx.db, user).await?.is_none() {
            ctx.db
                .insert(Profile {
                    user,
                    name: name.into(),
                    color: color.into(),
                    avatar: None,
                })
                .await?;
        }
        users.push(user);
    }
    let owner = ctx.auth.user_id().unwrap_or(users[0]);
    let space = crate::spaces::create_for(ctx, owner, "Pingo Demo".into(), None).await?;
    for user in &users {
        if crate::model::member(&ctx.db, space, *user).await?.is_none() {
            ctx.db
                .insert(SpaceMember {
                    space,
                    user: *user,
                    roles: vec![],
                    nick: None,
                })
                .await?;
        }
    }
    let general = ctx
        .db
        .query::<Room>()
        .with_index(Room::by_space_name, |q| {
            q.eq(Room::space, Some(space))
                .eq(Room::name, "general".to_owned())
        })
        .first()
        .await?
        .ok_or_else(|| user_error("not_found", "Missing general room"))?;
    let design = ctx
        .db
        .insert(Room {
            space: Some(space),
            name: "design".into(),
            topic: "Share ideas, sketches, and works in progress".into(),
            kind: RoomKind::Chat,
            private: false,
            position: 2,
            audio_profile: AudioProfile::Voice,
            participants: vec![],
            dm_key: None,
        })
        .await?;
    let mut root = None;
    for (i, body) in [
        "Welcome to Pingo! Grab a seat and say hello 👋",
        "I tried Studio mode with my guitar this morning. The difference is huge.",
        "@Maya could you share the latest call controls sketch?",
        "Absolutely! I’m simplifying the mute and share buttons.",
        "Let’s meet in the Lounge after lunch to try screen sharing.",
        "The first prototype is ready for feedback. What should we improve next?",
    ]
    .iter()
    .enumerate()
    {
        let room = if i == 5 { design } else { general.id };
        let id = ctx
            .db
            .insert(Message {
                room,
                author: users[i % users.len()],
                body: (*body).into(),
                thread_root: if i == 3 { root } else { None },
                attachments: vec![],
                edited_at: None,
                deleted: false,
            })
            .await?;
        if i == 2 {
            root = Some(id);
            crate::messages::notify(ctx, users[2], id, room, InboxKind::Mention).await?;
        }
        if i == 3
            && let Some(root) = root
        {
            let author = ctx.db.get(root).await?.unwrap().author;
            crate::messages::notify(ctx, author, id, room, InboxKind::Reply).await?;
        }
    }
    Ok(space)
}
