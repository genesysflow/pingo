//! Integration tests use a real temporary database and deterministic scheduler clock.
use crate::model::*;
use crate::{
    dev, dms, files, inbox, invites, messages, presence, profiles, reactions, rooms, spaces,
    typing, voice,
};
use futures_util::{SinkExt, StreamExt};
use rxferrix::auth::{
    AuthConfig, ClientInfo, CodeProof, MemoryMailer, SignInOutcome, SignUpResult, User,
};
use rxferrix::prelude::*;
use rxferrix::testing::TestApp;
use rxferrix::{AppOptions, FunctionRef, Guard, Identity};
use std::time::Duration;

fn app_options(dev: bool) -> AppOptions {
    AppOptions {
        dev,
        auth: AuthConfig::password()
            .magic_link()
            .redirect_url("http://localhost:5173")
            .before_sign_up(crate::access::before_sign_up),
        guard: Some(Guard::new(crate::access::guard)),
        ..Default::default()
    }
}

async fn test_app() -> TestApp {
    TestApp::with_options(app_options(true)).await.unwrap()
}

struct Fixture {
    t: TestApp,
    ann: Id<User>,
    bob: Id<User>,
    eve: Id<User>,
    space: Id<Space>,
    room: Id<Room>,
}
async fn user(t: &TestApp, name: &str) -> Id<User> {
    let id = t
        .create_user(&format!("{name}@example.com"), None)
        .await
        .unwrap();
    t.as_user(id)
        .mutation::<profiles::update>((Some(name.into()), None, None))
        .await
        .unwrap();
    id
}
async fn fixture() -> Fixture {
    let t = test_app().await;
    let ann = user(&t, "Ann").await;
    let bob = user(&t, "Bob").await;
    let eve = user(&t, "Eve").await;
    let space = t
        .as_user(ann)
        .mutation::<spaces::create>(("Team".into(), None))
        .await
        .unwrap();
    let invite = t
        .as_user(ann)
        .mutation::<invites::create>((space, None, None))
        .await
        .unwrap();
    t.as_user(bob)
        .mutation::<invites::accept>((invite,))
        .await
        .unwrap();
    let room = t.as_user(ann).query::<rooms::list>((space,)).await.unwrap()[0].id;
    Fixture {
        t,
        ann,
        bob,
        eve,
        space,
        room,
    }
}
fn code(err: rxferrix::Error) -> String {
    err.downcast_ref::<UserError>()
        .map_or_else(|| format!("{err:#}"), |e| e.code.clone())
}
async fn send(f: &Fixture, who: Id<User>, body: &str) -> Id<Message> {
    f.t.as_user(who)
        .mutation::<messages::send>((f.room, body.into(), None, None))
        .await
        .unwrap()
}
#[internal_mutation]
async fn everyone(ctx: &MutationCtx, space: Id<Space>, perms: u64) -> Result<()> {
    let role = ctx
        .db
        .query::<Role>()
        .with_index(Role::by_space, |q| q.eq(Role::space, space))
        .first()
        .await?
        .unwrap();
    ctx.db.patch(role.id, json!({"perms": perms})).await
}
#[internal_query]
async fn reads(ctx: &QueryCtx, room: Id<Room>, user: Id<User>) -> Result<Option<Doc<ReadState>>> {
    rooms::read_state(&ctx.db, room, user).await
}
#[internal_query]
async fn invite_row(ctx: &QueryCtx, code: String) -> Result<Doc<Invite>> {
    Ok(ctx
        .db
        .query::<Invite>()
        .with_index(Invite::by_code, |q| q.eq(Invite::code, code))
        .first()
        .await?
        .unwrap())
}

#[tokio::test]
async fn spaces_create_defaults_and_permissions() {
    let f = fixture().await;
    let as_ann = f.t.as_user(f.ann);
    let as_bob = f.t.as_user(f.bob);
    let listed = as_ann.query::<rooms::list>((f.space,)).await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|r| (r.name.as_str(), r.kind))
            .collect::<Vec<_>>(),
        [("general", RoomKind::Chat), ("Lounge", RoomKind::Lounge)]
    );
    let info = as_ann.query::<spaces::get>((f.space,)).await.unwrap();
    assert_eq!(info.my_perms, Perm::all().bits());
    assert_eq!(info.member_count, 2);
    assert_eq!(info.roles[0].name, "@everyone");
    assert_eq!(info.roles[0].perms, DEFAULT_PERMS.bits());
    assert_eq!(
        as_bob
            .query::<spaces::get>((f.space,))
            .await
            .unwrap()
            .my_perms,
        DEFAULT_PERMS.bits()
    );
    assert_eq!(
        code(
            as_bob
                .mutation::<rooms::create>((f.space, "secret".into(), None, RoomKind::Chat, true))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    as_ann
        .mutation::<spaces::update>((f.space, Some("Renamed".into()), None, None))
        .await
        .unwrap();
    assert_eq!(
        as_bob
            .query::<spaces::get>((f.space,))
            .await
            .unwrap()
            .space
            .name,
        "Renamed"
    );
    assert_eq!(
        code(
            as_ann
                .mutation::<spaces::leave>((f.space,))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert!(
        f.t.query_as::<profiles::me>(&Identity::default(), ())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        code(
            f.t.query_as::<spaces::list>(&Identity::default(), ())
                .await
                .unwrap_err()
        ),
        "unauthenticated"
    );
}
#[tokio::test]
async fn space_boundary_private_rooms_and_search() {
    let f = fixture().await;
    let private =
        f.t.as_user(f.ann)
            .mutation::<rooms::create>((f.space, "secret".into(), None, RoomKind::Chat, true))
            .await
            .unwrap();
    f.t.as_user(f.ann)
        .mutation::<messages::send>((private, "confidential needle".into(), None, None))
        .await
        .unwrap();
    send(&f, f.ann, "public needle").await;
    assert_eq!(
        code(
            f.t.as_user(f.bob)
                .query::<rooms::get>((private,))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert_eq!(
        code(
            f.t.as_user(f.bob)
                .query::<messages::list>((private, PaginationOpts::first(10)))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert_eq!(
        code(
            f.t.as_user(f.eve)
                .query::<rooms::get>((f.room,))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert_eq!(
        code(
            f.t.as_user(f.eve)
                .query::<spaces::get>((f.space,))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    let bob_hits =
        f.t.as_user(f.bob)
            .query::<messages::search>((f.space, "needle".into()))
            .await
            .unwrap();
    assert_eq!(bob_hits.len(), 1);
    assert_eq!(bob_hits[0].body, "public needle");
    assert_eq!(
        f.t.as_user(f.ann)
            .query::<messages::search>((f.space, "needle".into()))
            .await
            .unwrap()
            .len(),
        2
    );
    let other =
        f.t.as_user(f.eve)
            .mutation::<spaces::create>(("Elsewhere".into(), None))
            .await
            .unwrap();
    assert_eq!(
        code(
            f.t.as_user(f.ann)
                .query::<messages::search>((other, "needle".into()))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    // A member with MANAGE_ROOMS can create a private room; the space owner can still read it.
    f.t.mutation::<everyone>((f.space, (DEFAULT_PERMS | Perm::MANAGE_ROOMS).bits()))
        .await
        .unwrap();
    let private2 =
        f.t.as_user(f.bob)
            .mutation::<rooms::create>((f.space, "planning".into(), None, RoomKind::Chat, true))
            .await
            .unwrap();
    assert!(
        f.t.as_user(f.ann)
            .query::<rooms::get>((private2,))
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn invites_expire_count_uses_and_reuse_membership() {
    let f = fixture().await;
    let code1 =
        f.t.as_user(f.ann)
            .mutation::<invites::create>((f.space, Some(1), Some(1.0)))
            .await
            .unwrap();
    let p =
        f.t.query::<invites::preview>((code1.clone(),))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(p.member_count, 2);
    // Existing members do not consume a use.
    f.t.as_user(f.bob)
        .mutation::<invites::accept>((code1.clone(),))
        .await
        .unwrap();
    assert_eq!(
        f.t.query::<invite_row>((code1.clone(),))
            .await
            .unwrap()
            .uses,
        0
    );
    f.t.as_user(f.eve)
        .mutation::<invites::accept>((code1.clone(),))
        .await
        .unwrap();
    assert!(
        f.t.query::<invites::preview>((code1.clone(),))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        code(
            f.t.as_user(f.ann)
                .mutation::<invites::accept>((code1,))
                .await
                .unwrap_err()
        ),
        "invalid_invite"
    );
    let expiring =
        f.t.as_user(f.bob)
            .mutation::<invites::create>((f.space, None, Some(0.001)))
            .await
            .unwrap();
    f.t.advance_time(Duration::from_secs(4)).await.unwrap();
    assert!(
        f.t.query::<invites::preview>((expiring.clone(),))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        code(
            f.t.as_user(f.eve)
                .mutation::<invites::accept>((expiring,))
                .await
                .unwrap_err()
        ),
        "invalid_invite"
    );
}
#[tokio::test]
async fn dm_reuse_privacy_and_inbox() {
    let f = fixture().await;
    let room =
        f.t.as_user(f.ann)
            .mutation::<dms::open>((f.bob,))
            .await
            .unwrap();
    assert_eq!(
        f.t.as_user(f.bob)
            .mutation::<dms::open>((f.ann,))
            .await
            .unwrap(),
        room
    );
    let id =
        f.t.as_user(f.ann)
            .mutation::<messages::send>((room, "hello in private".into(), None, None))
            .await
            .unwrap();
    let dms = f.t.as_user(f.bob).query::<dms::list>(()).await.unwrap();
    assert_eq!(dms.len(), 1);
    assert_eq!(dms[0].other.user, f.ann);
    assert_eq!(dms[0].unread, 1);
    assert!(
        f.t.as_user(f.eve)
            .query::<dms::list>(())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        code(
            f.t.as_user(f.eve)
                .query::<messages::list>((room, PaginationOpts::first(10)))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    let items =
        f.t.as_user(f.bob)
            .query::<inbox::list>((None,))
            .await
            .unwrap();
    assert_eq!(items[0].kind, InboxKind::Dm);
    assert_eq!(items[0].message.id, id);
    assert!(items[0].space.is_none());
}
#[tokio::test]
async fn message_authorization_limits_and_soft_delete() {
    let f = fixture().await;
    let id = send(&f, f.bob, "first draft").await;
    assert_eq!(
        code(
            f.t.as_user(f.ann)
                .mutation::<messages::edit>((id, "hacked".into()))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    f.t.as_user(f.bob)
        .mutation::<messages::edit>((id, "final draft".into()))
        .await
        .unwrap();
    let m =
        f.t.as_user(f.ann)
            .query::<messages::list>((f.room, PaginationOpts::first(10)))
            .await
            .unwrap()
            .page
            .remove(0);
    assert_eq!(m.body, "final draft");
    assert!(m.edited_at.is_some());
    let other = send(&f, f.ann, "owner message").await;
    assert_eq!(
        code(
            f.t.as_user(f.bob)
                .mutation::<messages::remove>((other,))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert_eq!(
        code(
            f.t.as_user(f.eve)
                .mutation::<messages::send>((f.room, "no access".into(), None, None))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert_eq!(
        code(
            f.t.as_user(f.bob)
                .mutation::<messages::send>((f.room, "x".repeat(4001), None, None))
                .await
                .unwrap_err()
        ),
        "too_long"
    );
    f.t.as_user(f.ann)
        .mutation::<messages::remove>((id,))
        .await
        .unwrap(); // MANAGE_MESSAGES
    let list =
        f.t.as_user(f.bob)
            .query::<messages::list>((f.room, PaginationOpts::first(10)))
            .await
            .unwrap();
    let deleted = list.page.iter().find(|m| m.id == id).unwrap();
    assert!(deleted.deleted);
    assert!(deleted.body.is_empty());
    f.t.as_user(f.ann)
        .mutation::<messages::remove>((other,))
        .await
        .unwrap();
    assert_eq!(
        code(
            f.t.as_user(f.bob)
                .mutation::<messages::edit>((id, "resurrect".into()))
                .await
                .unwrap_err()
        ),
        "not_found"
    );
}
#[tokio::test]
async fn mentions_reply_inbox_and_completion() {
    let f = fixture().await;
    let root = send(&f, f.ann, "Hello @bOb, @Bob! email@Bob.invalid @Bobby").await;
    let items =
        f.t.as_user(f.bob)
            .query::<inbox::list>((None,))
            .await
            .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, InboxKind::Mention);
    assert_eq!(
        f.t.as_user(f.bob).query::<inbox::count>(()).await.unwrap(),
        1
    );
    assert_eq!(
        code(
            f.t.as_user(f.ann)
                .mutation::<inbox::markDone>((items[0].id,))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    f.t.as_user(f.bob)
        .mutation::<inbox::markDone>((items[0].id,))
        .await
        .unwrap();
    assert_eq!(
        f.t.as_user(f.bob).query::<inbox::count>(()).await.unwrap(),
        0
    );
    assert_eq!(
        f.t.as_user(f.bob)
            .query::<inbox::list>((Some(true),))
            .await
            .unwrap()
            .len(),
        1
    );
    f.t.as_user(f.bob)
        .mutation::<messages::send>((f.room, "thread response".into(), Some(root), None))
        .await
        .unwrap();
    assert_eq!(
        f.t.as_user(f.ann)
            .query::<inbox::list>((None,))
            .await
            .unwrap()[0]
            .kind,
        InboxKind::Reply
    );
    let thread =
        f.t.as_user(f.bob)
            .query::<messages::thread>((root,))
            .await
            .unwrap();
    assert_eq!(thread.root.reply_count, 1);
    assert_eq!(thread.replies.len(), 1);
    assert!(thread.root.last_reply_at.is_some());
    assert_eq!(
        f.t.as_user(f.bob)
            .query::<messages::list>((f.room, PaginationOpts::first(10)))
            .await
            .unwrap()
            .page
            .len(),
        1
    );
    send(&f, f.ann, "@everyone let's meet!").await;
    assert_eq!(
        f.t.as_user(f.bob).query::<inbox::count>(()).await.unwrap(),
        1
    ); // everyone default role
    assert_eq!(
        f.t.as_user(f.eve).query::<inbox::count>(()).await.unwrap(),
        0
    ); // outside the space
    f.t.as_user(f.bob)
        .mutation::<presence::heartbeat>(())
        .await
        .unwrap();
    send(&f, f.ann, "@here hello").await;
    assert_eq!(
        f.t.as_user(f.bob).query::<inbox::count>(()).await.unwrap(),
        2
    );
    f.t.as_user(f.bob)
        .mutation::<inbox::markAllDone>(())
        .await
        .unwrap();
    assert_eq!(
        f.t.as_user(f.bob).query::<inbox::count>(()).await.unwrap(),
        0
    );
    // Editing away a mention deletes its notification; a changed body with the same mention stays done.
    f.t.as_user(f.ann)
        .mutation::<messages::edit>((root, "updated @Bob".into()))
        .await
        .unwrap();
    assert_eq!(
        f.t.as_user(f.bob).query::<inbox::count>(()).await.unwrap(),
        0
    );
    f.t.as_user(f.ann)
        .mutation::<messages::edit>((root, "no ping".into()))
        .await
        .unwrap();
    assert!(
        f.t.as_user(f.bob)
            .query::<inbox::list>((Some(true),))
            .await
            .unwrap()
            .iter()
            .all(|i| i.message.id != root)
    );
}
#[tokio::test]
async fn unread_cap_pagination_and_noop_mark_read() {
    let f = fixture().await;
    for i in 0..105 {
        send(&f, f.ann, &format!("message {i}")).await;
    }
    assert_eq!(
        f.t.as_user(f.bob)
            .query::<rooms::list>((f.space,))
            .await
            .unwrap()[0]
            .unread,
        100
    );
    assert_eq!(
        f.t.as_user(f.ann)
            .query::<rooms::list>((f.space,))
            .await
            .unwrap()[0]
            .unread,
        0
    );
    let page =
        f.t.as_user(f.bob)
            .query::<messages::list>((f.room, PaginationOpts::first(3)))
            .await
            .unwrap();
    assert_eq!(
        page.page
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        ["message 104", "message 103", "message 102"]
    );
    let next =
        f.t.as_user(f.bob)
            .query::<messages::list>((f.room, PaginationOpts::after(page.continue_cursor, 3)))
            .await
            .unwrap();
    assert_eq!(next.page[0].body, "message 101");
    f.t.as_user(f.bob)
        .mutation::<rooms::markRead>((f.room,))
        .await
        .unwrap();
    assert_eq!(
        f.t.as_user(f.bob)
            .query::<rooms::list>((f.space,))
            .await
            .unwrap()[0]
            .unread,
        0
    );
    assert!(!f.t.as_user(f.bob).query::<spaces::list>(()).await.unwrap()[0].unread);
    let before = f.t.query::<reads>((f.room, f.bob)).await.unwrap().unwrap();
    let ts = f.t.app().db().write_log().latest();
    f.t.as_user(f.bob)
        .mutation::<rooms::markRead>((f.room,))
        .await
        .unwrap();
    assert_eq!(
        ts,
        f.t.app().db().write_log().latest(),
        "no-op did not commit a write"
    );
    assert_eq!(
        before.last_read_at,
        f.t.query::<reads>((f.room, f.bob))
            .await
            .unwrap()
            .unwrap()
            .last_read_at
    );
}
#[tokio::test]
async fn reactions_toggle_count_mine_and_limit() {
    let f = fixture().await;
    let id = send(&f, f.ann, "react to this").await;
    f.t.as_user(f.bob)
        .mutation::<reactions::toggle>((id, "🎉".into()))
        .await
        .unwrap();
    f.t.as_user(f.ann)
        .mutation::<reactions::toggle>((id, "🎉".into()))
        .await
        .unwrap();
    let m =
        f.t.as_user(f.bob)
            .query::<messages::list>((f.room, PaginationOpts::first(1)))
            .await
            .unwrap()
            .page
            .remove(0);
    assert_eq!(m.reactions[0].count, 2);
    assert!(m.reactions[0].mine);
    f.t.as_user(f.bob)
        .mutation::<reactions::toggle>((id, "🎉".into()))
        .await
        .unwrap();
    for i in 0..19 {
        f.t.as_user(f.bob)
            .mutation::<reactions::toggle>((id, format!(":{i}:")))
            .await
            .unwrap();
    }
    assert_eq!(
        code(
            f.t.as_user(f.ann)
                .mutation::<reactions::toggle>((id, "👍".into()))
                .await
                .unwrap_err()
        ),
        "too_long"
    );
    f.t.as_user(f.ann)
        .mutation::<messages::remove>((id,))
        .await
        .unwrap();
    assert_eq!(
        code(
            f.t.as_user(f.bob)
                .mutation::<reactions::toggle>((id, "🎉".into()))
                .await
                .unwrap_err()
        ),
        "not_found"
    );
}
#[tokio::test]
async fn typing_extends_expires_and_send_clears() {
    let f = fixture().await;
    f.t.as_user(f.ann)
        .mutation::<typing::start>((f.room,))
        .await
        .unwrap();
    assert!(
        f.t.as_user(f.ann)
            .query::<typing::list>((f.room,))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.t.as_user(f.bob)
            .query::<typing::list>((f.room,))
            .await
            .unwrap()[0]
            .name,
        "Ann"
    );
    f.t.advance_time(Duration::from_secs(2)).await.unwrap();
    f.t.as_user(f.ann)
        .mutation::<typing::start>((f.room,))
        .await
        .unwrap();
    f.t.advance_time(Duration::from_secs(3)).await.unwrap();
    assert_eq!(
        f.t.as_user(f.bob)
            .query::<typing::list>((f.room,))
            .await
            .unwrap()
            .len(),
        1
    );
    f.t.advance_time(Duration::from_secs(2)).await.unwrap();
    assert!(
        f.t.as_user(f.bob)
            .query::<typing::list>((f.room,))
            .await
            .unwrap()
            .is_empty()
    );
    f.t.as_user(f.ann)
        .mutation::<typing::start>((f.room,))
        .await
        .unwrap();
    send(&f, f.ann, "done").await;
    assert!(
        f.t.as_user(f.bob)
            .query::<typing::list>((f.room,))
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn presence_offline_after_150_seconds_and_preserves_dnd() {
    let f = fixture().await;
    f.t.as_user(f.ann)
        .mutation::<presence::setStatus>((PresenceStatus::Dnd, Some("Recording".into())))
        .await
        .unwrap();
    f.t.advance_time(Duration::from_secs(60)).await.unwrap();
    f.t.as_user(f.ann)
        .mutation::<presence::heartbeat>(())
        .await
        .unwrap();
    f.t.advance_time(Duration::from_secs(100)).await.unwrap();
    assert_eq!(
        f.t.as_user(f.ann)
            .query::<profiles::me>(())
            .await
            .unwrap()
            .unwrap()
            .status,
        PresenceStatus::Dnd
    );
    f.t.advance_time(Duration::from_secs(51)).await.unwrap();
    let p =
        f.t.as_user(f.ann)
            .query::<profiles::me>(())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(p.status, PresenceStatus::Offline);
    assert_eq!(p.status_text, "Recording");
    f.t.as_user(f.ann)
        .mutation::<presence::heartbeat>(())
        .await
        .unwrap();
    assert_eq!(
        f.t.as_user(f.ann)
            .query::<profiles::me>(())
            .await
            .unwrap()
            .unwrap()
            .status,
        PresenceStatus::Dnd
    );
    f.t.advance_time(Duration::from_secs(151)).await.unwrap();
    assert_eq!(
        f.t.as_user(f.ann)
            .query::<profiles::me>(())
            .await
            .unwrap()
            .unwrap()
            .status,
        PresenceStatus::Offline
    );
}
#[tokio::test]
async fn voice_tokens_capabilities_and_sfu_state() {
    let f = fixture().await;
    let joined =
        f.t.as_user(f.bob)
            .mutation::<voice::join>((f.room,))
            .await
            .unwrap();
    assert_eq!(joined.url, "/rtc");
    let claims =
        pingo_proto::token::verify(&joined.token, voice::DEV_MEDIA_SECRET.as_bytes(), 0.0).unwrap();
    assert_eq!(claims.room, f.room.to_string());
    assert_eq!(claims.user, f.bob.to_string());
    assert_eq!(claims.name, "Bob");
    assert!(claims.can_speak && claims.can_video && claims.can_share);
    assert!(
        pingo_proto::token::verify(
            &joined.token,
            voice::DEV_MEDIA_SECRET.as_bytes(),
            claims.exp + 1.0
        )
        .is_err()
    );
    f.t.mutation::<rxferrix::env::put>((
        "PINGO_MEDIA_SECRET".into(),
        Some("test-media-secret".into()),
    ))
    .await
    .unwrap();
    f.t.mutation::<everyone>((f.space, Perm::VIEW.bits()))
        .await
        .unwrap();
    let restricted =
        f.t.as_user(f.bob)
            .mutation::<voice::join>((f.room,))
            .await
            .unwrap();
    let c = pingo_proto::token::verify(&restricted.token, b"test-media-secret", 0.0).unwrap();
    assert!(!c.can_speak && !c.can_video && !c.can_share);
    let owner =
        f.t.as_user(f.ann)
            .mutation::<voice::join>((f.room,))
            .await
            .unwrap();
    assert!(
        pingo_proto::token::verify(&owner.token, b"test-media-secret", 0.0)
            .unwrap()
            .can_speak
    );
    assert_eq!(
        code(
            f.t.as_user(f.eve)
                .mutation::<voice::join>((f.room,))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert_eq!(
        code(
            f.t.mutation::<voice::setState>((f.room, f.bob, false, false, false, false))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    f.t.mutation::<voice::setState>((f.room, f.bob, true, false, false, false))
        .await
        .unwrap();
    let r =
        f.t.as_user(f.ann)
            .query::<rooms::list>((f.space,))
            .await
            .unwrap();
    assert_eq!(r[0].voice.len(), 1);
    assert_eq!(r[0].voice[0].user, f.bob);
    assert!(r[0].voice[0].muted);
    f.t.mutation::<voice::left>((f.room, f.bob)).await.unwrap();
    assert!(
        f.t.as_user(f.ann)
            .query::<voice::participants>((f.room,))
            .await
            .unwrap()
            .is_empty()
    );
    f.t.mutation::<voice::setState>((f.room, f.ann, false, false, true, true))
        .await
        .unwrap();
    f.t.advance_time(Duration::from_secs(12 * 3600 + 1))
        .await
        .unwrap();
    assert_eq!(f.t.mutation::<voice::prune>(()).await.unwrap(), 1);
    assert!(
        f.t.as_user(f.ann)
            .query::<rooms::list>((f.space,))
            .await
            .unwrap()[0]
            .voice
            .is_empty()
    );
}
#[tokio::test]
async fn missing_production_secret_and_dev_endpoints() {
    let t = TestApp::with_options(app_options(false)).await.unwrap();
    let ann = user(&t, "Ann").await;
    let space = t
        .as_user(ann)
        .mutation::<spaces::create>(("Production".into(), None))
        .await
        .unwrap();
    let room = t.as_user(ann).query::<rooms::list>((space,)).await.unwrap()[0].id;
    let err = t
        .as_user(ann)
        .mutation::<voice::join>((room,))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("PINGO_MEDIA_SECRET"));
    assert!(
        t.mutation_as::<dev::seed>(&Identity::default(), ())
            .await
            .is_err()
    );
    assert!(
        t.action_as::<dev::last_code>(&Identity::default(), ("ann@example.com".into(),))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn seed_is_idempotent_and_upload_requires_auth() {
    let t = test_app().await;
    let space = t
        .mutation_as::<dev::seed>(&Identity::default(), ())
        .await
        .unwrap();
    assert_eq!(
        t.mutation_as::<dev::seed>(&Identity::default(), ())
            .await
            .unwrap(),
        space
    );
    assert_eq!(
        code(
            t.mutation_as::<files::generateUploadUrl>(&Identity::default(), ())
                .await
                .unwrap_err()
        ),
        "unauthenticated"
    );
    let ann = user(&t, "RealUser").await;
    assert!(
        t.as_user(ann)
            .mutation::<files::generateUploadUrl>(())
            .await
            .unwrap()
            .contains("/api/storage/upload")
    );
}

#[internal_mutation]
async fn add_notifications(
    ctx: &MutationCtx,
    user: Id<User>,
    room: Id<Room>,
    message: Id<Message>,
    count: u32,
) -> Result<()> {
    for _ in 0..count {
        ctx.db
            .insert(Mention {
                user,
                room,
                message,
                kind: InboxKind::Mention,
                done: false,
            })
            .await?;
    }
    Ok(())
}
#[tokio::test]
async fn mark_all_done_keeps_later_arrivals_pending() {
    let f = fixture().await;
    let message = send(&f, f.ann, "Original message").await;
    f.t.mutation::<add_notifications>((f.bob, f.room, message, 1005))
        .await
        .unwrap();
    f.t.as_user(f.bob)
        .mutation::<inbox::markAllDone>(())
        .await
        .unwrap();
    send(&f, f.ann, "@Bob a new arrival").await;
    f.t.run_due_jobs().await.unwrap();
    assert_eq!(
        f.t.as_user(f.bob).query::<inbox::count>(()).await.unwrap(),
        1
    );
    assert_eq!(
        f.t.as_user(f.bob)
            .query::<inbox::list>((None,))
            .await
            .unwrap()[0]
            .message
            .body,
        "@Bob a new arrival"
    );
}
#[tokio::test]
async fn leaving_clears_live_state_and_pending_space_inbox() {
    let f = fixture().await;
    send(&f, f.ann, "@Bob see you in the call").await;
    f.t.as_user(f.bob)
        .mutation::<typing::start>((f.room,))
        .await
        .unwrap();
    f.t.mutation::<voice::set_state>((f.room, f.bob, false, false, false, false))
        .await
        .unwrap();
    f.t.as_user(f.bob)
        .mutation::<spaces::leave>((f.space,))
        .await
        .unwrap();
    assert_eq!(
        f.t.as_user(f.bob).query::<inbox::count>(()).await.unwrap(),
        0
    );
    assert!(
        f.t.as_user(f.ann)
            .query::<voice::participants>((f.room,))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        f.t.as_user(f.ann)
            .query::<typing::list>((f.room,))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        f.t.as_user(f.bob)
            .query::<spaces::list>(())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        code(
            f.t.as_user(f.bob)
                .query::<messages::list>((f.room, PaginationOpts::first(1)))
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
}
#[tokio::test]
async fn sends_are_rate_limited_and_window_resets() {
    let f = fixture().await;
    for _ in 0..120 {
        send(&f, f.ann, "within the minute").await;
    }
    assert_eq!(
        code(
            f.t.as_user(f.ann)
                .mutation::<messages::send>((f.room, "too fast".into(), None, None))
                .await
                .unwrap_err()
        ),
        "rate_limited"
    );
    f.t.advance_time(Duration::from_secs(61)).await.unwrap();
    send(&f, f.ann, "new minute").await;
}

async fn set_env(t: &TestApp, name: &str, value: Option<&str>) {
    t.mutation::<rxferrix::env::put>((name.to_owned(), value.map(str::to_owned)))
        .await
        .unwrap();
}

#[tokio::test]
async fn invite_only_admits_members_and_invitees_only() {
    let f = fixture().await;
    set_env(&f.t, crate::access::REGISTRATION, Some("invite")).await;
    // Existing members keep working when a server switches to invite-only.
    send(&f, f.ann, "still here").await;
    let status =
        f.t.as_user(f.eve)
            .query::<crate::access::status>(())
            .await
            .unwrap();
    assert_eq!(
        status.admission,
        Some(crate::access::Admission::InviteRequired)
    );
    for err in [
        f.t.as_user(f.eve)
            .mutation::<spaces::create>(("Mine".into(), None))
            .await
            .unwrap_err(),
        f.t.as_user(f.eve)
            .query::<spaces::list>(())
            .await
            .unwrap_err(),
        f.t.as_user(f.eve)
            .mutation::<presence::heartbeat>(())
            .await
            .unwrap_err(),
    ] {
        assert_eq!(code(err), "invite_required");
    }
    let invite =
        f.t.as_user(f.ann)
            .mutation::<invites::create>((f.space, None, None))
            .await
            .unwrap();
    f.t.as_user(f.eve)
        .mutation::<invites::accept>((invite,))
        .await
        .unwrap();
    assert_eq!(
        f.t.as_user(f.eve)
            .query::<spaces::list>(())
            .await
            .unwrap()
            .len(),
        1
    );
    // Leaving every space closes the door again.
    f.t.as_user(f.eve)
        .mutation::<spaces::leave>((f.space,))
        .await
        .unwrap();
    assert_eq!(
        code(
            f.t.as_user(f.eve)
                .query::<spaces::list>(())
                .await
                .unwrap_err()
        ),
        "invite_required"
    );
    set_env(&f.t, crate::access::REGISTRATION, Some("open")).await;
    f.t.as_user(f.eve).query::<spaces::list>(()).await.unwrap();
}

#[tokio::test]
async fn invite_only_lets_the_first_account_create_the_first_space() {
    let t = test_app().await;
    set_env(&t, crate::access::REGISTRATION, Some("invite")).await;
    assert!(
        t.query_as::<crate::access::status>(&Identity::default(), ())
            .await
            .unwrap()
            .setup
    );
    let first = password_sign_up(&t, "first@example.com", None)
        .await
        .unwrap();
    t.as_user(first)
        .mutation::<spaces::create>(("Home".into(), None))
        .await
        .unwrap();
    assert!(
        !t.query_as::<crate::access::status>(&Identity::default(), ())
            .await
            .unwrap()
            .setup
    );
    let second = t.create_user("second@example.com", None).await.unwrap();
    assert_eq!(
        code(
            t.as_user(second)
                .mutation::<spaces::create>(("Squat".into(), None))
                .await
                .unwrap_err()
        ),
        "invite_required"
    );
}

#[tokio::test]
async fn email_allowlist_needs_a_listed_and_verified_address() {
    let t = test_app().await;
    set_env(
        &t,
        crate::access::ALLOWED_EMAILS,
        Some("@acme.org, boss@example.com"),
    )
    .await;
    let listed = t.create_user("ada@acme.org", None).await.unwrap();
    let boss = t.create_user("boss@example.com", None).await.unwrap();
    let outsider = t.create_user("eve@example.com", None).await.unwrap();
    t.as_user(listed)
        .mutation::<spaces::create>(("Acme".into(), None))
        .await
        .unwrap();
    t.as_user(boss).query::<spaces::list>(()).await.unwrap();
    assert_eq!(
        code(
            t.as_user(outsider)
                .query::<spaces::list>(())
                .await
                .unwrap_err()
        ),
        "email_not_allowed"
    );
    // An invite doesn't override the email rule.
    let space = t.as_user(listed).query::<spaces::list>(()).await.unwrap()[0].id;
    let invite = t
        .as_user(listed)
        .mutation::<invites::create>((space, None, None))
        .await
        .unwrap();
    assert_eq!(
        code(
            t.as_user(outsider)
                .mutation::<invites::accept>((invite.clone(),))
                .await
                .unwrap_err()
        ),
        "email_not_allowed"
    );
    // A password sign-up claiming a listed address must confirm it first.
    let signed = t
        .app()
        .auth()
        .sign_up(
            t.app(),
            "mallory@acme.org",
            "correct-horse-battery".into(),
            None,
            None,
            rxferrix::auth::ClientInfo::default(),
        )
        .await
        .unwrap();
    let rxferrix::auth::service::SignUpResult::SignedIn(tokens) = signed else {
        panic!("verification is not required by the auth config");
    };
    let mallory: Id<User> = tokens.user_id.unwrap().parse().unwrap();
    let status = t
        .as_user(mallory)
        .query::<crate::access::status>(())
        .await
        .unwrap();
    assert_eq!(
        status.admission,
        Some(crate::access::Admission::VerifyEmail)
    );
    assert!(status.restricted_emails);
    assert_eq!(
        code(
            t.as_user(mallory)
                .mutation::<invites::accept>((invite,))
                .await
                .unwrap_err()
        ),
        "email_not_verified"
    );
    assert_eq!(
        code(
            t.as_user(mallory)
                .query::<spaces::list>(())
                .await
                .unwrap_err()
        ),
        "email_not_verified"
    );
}

#[tokio::test]
async fn a_bad_policy_value_closes_the_server() {
    let f = fixture().await;
    set_env(&f.t, crate::access::REGISTRATION, Some("everyone")).await;
    assert_eq!(
        code(
            f.t.as_user(f.ann)
                .query::<spaces::list>(())
                .await
                .unwrap_err()
        ),
        "misconfigured"
    );
    let status =
        f.t.query_as::<crate::access::status>(&Identity::default(), ())
            .await
            .unwrap_err();
    assert_eq!(code(status), "misconfigured");
}

async fn password_sign_up(
    app: &TestApp,
    email: &str,
    redirect: Option<&str>,
) -> std::result::Result<Id<User>, rxferrix::auth::AuthFailure> {
    let signed = app
        .app()
        .auth()
        .sign_up(
            app.app(),
            email,
            "correct-horse-battery".into(),
            None,
            redirect,
            ClientInfo::default(),
        )
        .await?;
    let SignUpResult::SignedIn(tokens) = signed else {
        panic!("test auth configuration should return a session");
    };
    Ok(tokens.user_id.unwrap().parse().unwrap())
}

#[internal_query]
async fn account_by_email(ctx: &QueryCtx, email: String) -> Result<Option<Doc<User>>> {
    ctx.db
        .query::<User>()
        .with_index(User::by_email, |q| q.eq(User::email, Some(email)))
        .first()
        .await
}

#[query]
async fn policy_probe_query(_ctx: &QueryCtx) -> Result<bool> {
    Ok(true)
}

#[mutation]
async fn policy_probe_mutation(ctx: &MutationCtx) -> Result<Id<User>> {
    ctx.db
        .insert(User {
            email: Some("probe@example.com".into()),
            email_verified: true,
            name: None,
            image: None,
        })
        .await
}

#[action]
async fn policy_probe_action(_ctx: &ActionCtx) -> Result<bool> {
    Ok(true)
}

#[tokio::test]
async fn invite_only_password_sign_up_refuses_before_creating_an_account() {
    let fixture = fixture().await;
    set_env(&fixture.t, crate::access::REGISTRATION, Some("invite")).await;
    let err = password_sign_up(&fixture.t, "blocked@example.com", None)
        .await
        .unwrap_err();
    assert_eq!(err.error.code, "invite_required");
    assert!(
        fixture
            .t
            .query::<account_by_email>(("blocked@example.com".into(),))
            .await
            .unwrap()
            .is_none()
    );
    set_env(&fixture.t, crate::access::REGISTRATION, Some("open")).await;
    password_sign_up(&fixture.t, "blocked@example.com", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn invite_redirect_allows_sign_up_and_only_accept_consumes_the_invite() {
    let fixture = fixture().await;
    set_env(&fixture.t, crate::access::REGISTRATION, Some("invite")).await;
    let invite = fixture
        .t
        .as_user(fixture.ann)
        .mutation::<invites::create>((fixture.space, Some(1), None))
        .await
        .unwrap();
    let redirect = format!("http://localhost:5173/invite/{invite}?from=email");
    let invited = password_sign_up(&fixture.t, "invited@example.com", Some(&redirect))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .t
            .query::<invite_row>((invite.clone(),))
            .await
            .unwrap()
            .uses,
        0
    );
    assert_eq!(
        code(
            fixture
                .t
                .as_user(invited)
                .query::<spaces::list>(())
                .await
                .unwrap_err()
        ),
        "invite_required"
    );
    let preview = fixture
        .t
        .query_as::<invites::preview>(&Identity::default(), (invite.clone(),))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preview.space_name, "Team");
    fixture
        .t
        .as_user(invited)
        .mutation::<invites::accept>((invite.clone(),))
        .await
        .unwrap();
    assert_eq!(
        fixture.t.query::<invite_row>((invite,)).await.unwrap().uses,
        1
    );
    assert_eq!(
        fixture
            .t
            .as_user(invited)
            .query::<spaces::list>(())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn sign_up_rejects_invalid_expired_and_exhausted_invite_redirects() {
    let fixture = fixture().await;
    set_env(&fixture.t, crate::access::REGISTRATION, Some("invite")).await;
    let expired = fixture
        .t
        .as_user(fixture.ann)
        .mutation::<invites::create>((fixture.space, None, Some(0.001)))
        .await
        .unwrap();
    let exhausted = fixture
        .t
        .as_user(fixture.ann)
        .mutation::<invites::create>((fixture.space, Some(1), None))
        .await
        .unwrap();
    fixture
        .t
        .as_user(fixture.eve)
        .mutation::<invites::accept>((exhausted.clone(),))
        .await
        .unwrap();
    fixture
        .t
        .advance_time(Duration::from_secs(4))
        .await
        .unwrap();
    for (name, invite) in [
        ("invalid", "does-not-exist".to_owned()),
        ("expired", expired),
        ("exhausted", exhausted),
    ] {
        let email = format!("{name}@example.com");
        let redirect = format!("http://localhost:5173/invite/{invite}");
        let err = password_sign_up(&fixture.t, &email, Some(&redirect))
            .await
            .unwrap_err();
        assert_eq!(err.error.code, "invalid_invite", "{name}");
        assert!(
            fixture
                .t
                .query::<account_by_email>((email,))
                .await
                .unwrap()
                .is_none()
        );
    }
    for path in ["/invite/", "/invite/a/extra", "/?invite=anything"] {
        let err = password_sign_up(
            &fixture.t,
            "malformed@example.com",
            Some(&format!("http://localhost:5173{path}")),
        )
        .await
        .unwrap_err();
        assert_eq!(err.error.code, "invite_required", "{path}");
    }
    assert!(
        fixture
            .t
            .query::<account_by_email>(("malformed@example.com".into(),))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn email_allowlist_rejects_sign_up_before_creating_an_account() {
    let app = test_app().await;
    set_env(&app, crate::access::ALLOWED_EMAILS, Some("@acme.org")).await;
    let err = password_sign_up(&app, "outsider@example.com", None)
        .await
        .unwrap_err();
    assert_eq!(err.error.code, "email_not_allowed");
    assert!(
        app.query::<account_by_email>(("outsider@example.com".into(),))
            .await
            .unwrap()
            .is_none()
    );
    let unverified = password_sign_up(&app, "new@acme.org", None).await.unwrap();
    assert!(
        !app.query::<account_by_email>(("new@acme.org".into(),))
            .await
            .unwrap()
            .unwrap()
            .email_verified
    );
    for err in [
        app.as_user(unverified)
            .query::<policy_probe_query>(())
            .await
            .unwrap_err(),
        app.as_user(unverified)
            .mutation::<policy_probe_mutation>(())
            .await
            .unwrap_err(),
        app.as_user(unverified)
            .action::<policy_probe_action>(())
            .await
            .unwrap_err(),
    ] {
        assert_eq!(code(err), "email_not_verified");
    }
    assert!(
        app.as_user(unverified)
            .query::<profiles::me>(())
            .await
            .is_ok()
    );
    assert_eq!(
        app.as_user(unverified)
            .query::<crate::access::status>(())
            .await
            .unwrap()
            .admission,
        Some(crate::access::Admission::VerifyEmail)
    );
    assert!(
        app.query::<account_by_email>(("probe@example.com".into(),))
            .await
            .unwrap()
            .is_none()
    );
    set_env(&app, crate::access::ALLOWED_EMAILS, None).await;
    password_sign_up(&app, "outsider@example.com", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn central_guard_covers_new_queries_mutations_and_actions() {
    let fixture = fixture().await;
    for err in [
        fixture
            .t
            .query_as::<policy_probe_query>(&Identity::default(), ())
            .await
            .unwrap_err(),
        fixture
            .t
            .mutation_as::<policy_probe_mutation>(&Identity::default(), ())
            .await
            .unwrap_err(),
        fixture
            .t
            .action_as::<policy_probe_action>(&Identity::default(), ())
            .await
            .unwrap_err(),
    ] {
        assert_eq!(code(err), "unauthenticated");
    }
    set_env(&fixture.t, crate::access::REGISTRATION, Some("invite")).await;
    for err in [
        fixture
            .t
            .as_user(fixture.eve)
            .query::<policy_probe_query>(())
            .await
            .unwrap_err(),
        fixture
            .t
            .as_user(fixture.eve)
            .mutation::<policy_probe_mutation>(())
            .await
            .unwrap_err(),
        fixture
            .t
            .as_user(fixture.eve)
            .action::<policy_probe_action>(())
            .await
            .unwrap_err(),
    ] {
        assert_eq!(code(err), "invite_required");
    }
    assert!(
        fixture
            .t
            .query::<account_by_email>(("probe@example.com".into(),))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .t
            .as_user(fixture.eve)
            .query::<profiles::me>(())
            .await
            .is_ok()
    );
    assert!(
        fixture
            .t
            .as_user(fixture.ann)
            .query::<policy_probe_query>(())
            .await
            .unwrap()
    );
    assert!(
        fixture
            .t
            .as_user(fixture.ann)
            .action::<policy_probe_action>(())
            .await
            .unwrap()
    );
    fixture
        .t
        .as_user(fixture.ann)
        .mutation::<policy_probe_mutation>(())
        .await
        .unwrap();
    set_env(&fixture.t, crate::access::REGISTRATION, Some("open")).await;
    assert!(
        fixture
            .t
            .as_user(fixture.eve)
            .query::<policy_probe_query>(())
            .await
            .unwrap()
    );
}

async fn emailed_proof(mailer: &MemoryMailer, email: &str, link: bool) -> CodeProof {
    let body = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(sent) = mailer.last_to(email) {
                return sent.body;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("auth email was not delivered");
    if link {
        let url = body
            .split_whitespace()
            .find(|word| word.contains("#rxferrix_magic_token="))
            .unwrap();
        let url = url::Url::parse(url).unwrap();
        let (_, token) = url.fragment().unwrap().split_once('=').unwrap();
        CodeProof::Token(token.into())
    } else {
        let rest = body.split_once("Your code: ").unwrap().1;
        let code: String = rest.chars().take_while(char::is_ascii_digit).collect();
        assert_eq!(code.len(), 6);
        CodeProof::Code {
            email: email.into(),
            code,
        }
    }
}

#[tokio::test]
async fn magic_link_and_code_sign_up_apply_current_policy_before_creating_accounts() {
    let mailer = MemoryMailer::new();
    let mut options = app_options(true);
    options.auth = options.auth.mailer(mailer.clone()).rate_limits(None);
    let app = TestApp::with_options(options).await.unwrap();
    let owner = user(&app, "Owner").await;
    let space = app
        .as_user(owner)
        .mutation::<spaces::create>(("Home".into(), None))
        .await
        .unwrap();
    let invite = app
        .as_user(owner)
        .mutation::<invites::create>((space, None, None))
        .await
        .unwrap();
    let redirect = format!("http://localhost:5173/invite/{invite}");
    for link in [false, true] {
        let email = format!("denied-{link}@acme.org");
        set_env(&app, crate::access::REGISTRATION, Some("open")).await;
        app.app()
            .auth()
            .request_magic_link(app.app(), &email, None)
            .await
            .unwrap();
        set_env(&app, crate::access::REGISTRATION, Some("invite")).await;
        let err = app
            .app()
            .auth()
            .verify_magic_link(
                app.app(),
                emailed_proof(&mailer, &email, link).await,
                ClientInfo::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.error.code, "invite_required");
        assert!(
            app.query::<account_by_email>((email,))
                .await
                .unwrap()
                .is_none()
        );

        let email = format!("outsider-{link}@example.com");
        app.app()
            .auth()
            .request_magic_link(app.app(), &email, Some(&redirect))
            .await
            .unwrap();
        set_env(&app, crate::access::ALLOWED_EMAILS, Some("@acme.org")).await;
        let err = app
            .app()
            .auth()
            .verify_magic_link(
                app.app(),
                emailed_proof(&mailer, &email, link).await,
                ClientInfo::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.error.code, "email_not_allowed");
        assert!(
            app.query::<account_by_email>((email,))
                .await
                .unwrap()
                .is_none()
        );

        let email = format!("invited-{link}@acme.org");
        app.app()
            .auth()
            .request_magic_link(app.app(), &email, Some(&redirect))
            .await
            .unwrap();
        let signed = app
            .app()
            .auth()
            .verify_magic_link(
                app.app(),
                emailed_proof(&mailer, &email, link).await,
                ClientInfo::default(),
            )
            .await
            .unwrap();
        let SignInOutcome::Tokens(tokens) = signed else {
            panic!("new account has no second factor");
        };
        let invited = tokens.user_id.unwrap().parse().unwrap();
        assert!(
            app.query::<account_by_email>((email,))
                .await
                .unwrap()
                .unwrap()
                .email_verified
        );
        assert_eq!(
            app.query::<invite_row>((invite.clone(),))
                .await
                .unwrap()
                .uses,
            u32::from(link)
        );
        app.as_user(invited)
            .mutation::<invites::accept>((invite.clone(),))
            .await
            .unwrap();
        assert!(
            app.as_user(invited)
                .query::<policy_probe_query>(())
                .await
                .unwrap()
        );
        set_env(&app, crate::access::ALLOWED_EMAILS, None).await;
    }
}

#[tokio::test]
async fn http_clients_cannot_bypass_the_central_guard() {
    let fixture = fixture().await;
    let address = fixture.t.serve().await.unwrap();
    let ann = fixture
        .t
        .app()
        .auth()
        .sign_in_as(fixture.t.app(), fixture.ann)
        .await
        .unwrap()
        .access_token;
    let eve = fixture
        .t
        .app()
        .auth()
        .sign_in_as(fixture.t.app(), fixture.eve)
        .await
        .unwrap()
        .access_token;
    set_env(&fixture.t, crate::access::REGISTRATION, Some("invite")).await;
    let client = reqwest::Client::new();
    for (kind, path) in [
        ("query", policy_probe_query::path()),
        ("mutation", policy_probe_mutation::path()),
        ("action", policy_probe_action::path()),
    ] {
        for (token, expected) in [(None, "unauthenticated"), (Some(&eve), "invite_required")] {
            let request = client
                .post(format!("http://{address}/api/{kind}"))
                .json(&json!({"path": path, "args": {}}));
            let request = match token {
                Some(token) => request.bearer_auth(token),
                None => request,
            };
            let reply: serde_json::Value = request.send().await.unwrap().json().await.unwrap();
            assert_eq!(reply["status"], "error", "{reply}");
            assert_eq!(reply["errorData"]["code"], expected, "{reply}");
        }
        let reply: serde_json::Value = client
            .post(format!("http://{address}/api/{kind}"))
            .bearer_auth(&ann)
            .json(&json!({"path": path, "args": {}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(reply["status"], "success", "{reply}");
    }
    let reply: serde_json::Value = client
        .post(format!("http://{address}/api/auth/signup"))
        .json(&json!({"email": "http-denied@example.com", "password": "correct-horse-battery"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reply["errorData"]["code"], "invite_required", "{reply}");
    assert!(
        fixture
            .t
            .query::<account_by_email>(("http-denied@example.com".into(),))
            .await
            .unwrap()
            .is_none()
    );
}

type PolicySocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn socket_message(ws: &mut PolicySocket) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = ws.next().await.expect("socket closed").unwrap();
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] != "Ping" {
                    return value;
                }
            }
        }
    })
    .await
    .expect("timed out waiting for policy update")
}

async fn policy_update(ws: &mut PolicySocket) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = socket_message(ws).await;
            assert_eq!(value["type"], "Transition", "{value}");
            if let Some(update) = value["modifications"]
                .as_array()
                .and_then(|mods| mods.iter().find(|m| m["queryId"] == 0))
            {
                return update.clone();
            }
        }
    })
    .await
    .expect("subscribed query did not react to admission changes")
}

#[tokio::test]
async fn subscribed_queries_recheck_policy_and_membership_after_each_change() {
    let fixture = fixture().await;
    let address = fixture.t.serve().await.unwrap();
    let token = fixture
        .t
        .app()
        .auth()
        .sign_in_as(fixture.t.app(), fixture.eve)
        .await
        .unwrap()
        .access_token;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{address}/api/sync"))
        .await
        .unwrap();
    for message in [
        json!({"type": "Connect", "sessionId": "policy-regression", "connectionCount": 0, "lastCloseReason": null, "clientVersion": "0.7.0"}),
        json!({"type": "Authenticate", "baseVersion": 0, "token": token}),
    ] {
        ws.send(tokio_tungstenite::tungstenite::Message::text(
            message.to_string(),
        ))
        .await
        .unwrap();
    }
    loop {
        let message = socket_message(&mut ws).await;
        assert_eq!(message["type"], "Transition", "{message}");
        if message["endVersion"]["identity"] == 1 {
            break;
        }
    }
    ws.send(tokio_tungstenite::tungstenite::Message::text(json!({
        "type": "ModifyQuerySet", "baseVersion": 0, "newVersion": 1,
        "modifications": [{"type": "Add", "queryId": 0, "udfPath": policy_probe_query::path(), "args": {}}]
    }).to_string())).await.unwrap();
    let update = policy_update(&mut ws).await;
    assert_eq!(update["type"], "QueryUpdated", "{update}");
    assert_eq!(update["value"], true);

    set_env(&fixture.t, crate::access::REGISTRATION, Some("invite")).await;
    let update = policy_update(&mut ws).await;
    assert_eq!(update["type"], "QueryFailed", "{update}");
    assert_eq!(update["errorData"]["code"], "invite_required", "{update}");
    let invite = fixture
        .t
        .as_user(fixture.ann)
        .mutation::<invites::create>((fixture.space, None, None))
        .await
        .unwrap();
    fixture
        .t
        .as_user(fixture.eve)
        .mutation::<invites::accept>((invite,))
        .await
        .unwrap();
    let update = policy_update(&mut ws).await;
    assert_eq!(update["type"], "QueryUpdated", "{update}");
    assert_eq!(update["value"], true);

    set_env(
        &fixture.t,
        crate::access::ALLOWED_EMAILS,
        Some("ann@example.com"),
    )
    .await;
    let update = policy_update(&mut ws).await;
    assert_eq!(update["type"], "QueryFailed", "{update}");
    assert_eq!(update["errorData"]["code"], "email_not_allowed", "{update}");
    set_env(&fixture.t, crate::access::ALLOWED_EMAILS, None).await;
    let update = policy_update(&mut ws).await;
    assert_eq!(update["type"], "QueryUpdated", "{update}");
    assert_eq!(update["value"], true);
    ws.close(None).await.unwrap();
}
