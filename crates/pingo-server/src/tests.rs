//! Integration tests use a real temporary database and deterministic scheduler clock.
use crate::model::*;
use crate::{
    dev, dms, files, inbox, invites, messages, presence, profiles, reactions, rooms, spaces,
    typing, voice,
};
use rxferrix::auth::User;
use rxferrix::prelude::*;
use rxferrix::testing::TestApp;
use std::time::Duration;

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
    let t = TestApp::new().await.unwrap();
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
    assert!(f.t.query::<profiles::me>(()).await.unwrap().is_none());
    assert_eq!(
        code(f.t.query::<spaces::list>(()).await.unwrap_err()),
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
    let t = TestApp::with_options(rxferrix::AppOptions::default())
        .await
        .unwrap();
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
    assert!(t.mutation::<dev::seed>(()).await.is_err());
}
#[tokio::test]
async fn seed_is_idempotent_and_upload_requires_auth() {
    let t = TestApp::new().await.unwrap();
    let space = t.mutation::<dev::seed>(()).await.unwrap();
    assert_eq!(t.mutation::<dev::seed>(()).await.unwrap(), space);
    assert_eq!(
        code(
            t.mutation::<files::generateUploadUrl>(())
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
    let t = TestApp::new().await.unwrap();
    set_env(&t, crate::access::REGISTRATION, Some("invite")).await;
    assert!(t.query::<crate::access::status>(()).await.unwrap().setup);
    let first = user(&t, "First").await;
    t.as_user(first)
        .mutation::<spaces::create>(("Home".into(), None))
        .await
        .unwrap();
    assert!(!t.query::<crate::access::status>(()).await.unwrap().setup);
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
    let t = TestApp::with_auth(rxferrix::auth::AuthConfig::password())
        .await
        .unwrap();
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
                .mutation::<invites::accept>((invite,))
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
    let status = f.t.query::<crate::access::status>(()).await.unwrap_err();
    assert_eq!(code(status), "misconfigured");
}
