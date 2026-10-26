//! Who may use this server: anyone, invited people only, and/or certain email addresses.
//!
//! RxFerrix checks sign-ups before creating accounts and guards public client functions centrally.
//! Policy reads stay reactive so changing admission rules also updates existing subscriptions.
//!
//! Configuration is read from RxFerrix Environment first (dashboard or `pingo-server env set`), so it
//! can change at run time and queries rerun when it does; otherwise from the process environment
//! (Docker), which is fixed at startup.
use crate::model::{SpaceMember, user_error};
use rxferrix::QueryDb;
use rxferrix::auth::{SignUpAttempt, SignUpMethod, User};
use rxferrix::env::Env;
use rxferrix::prelude::*;
use rxferrix::{App, GuardCtx, GuardedFunction};
use std::sync::OnceLock;

pub const REGISTRATION: &str = "PINGO_REGISTRATION";
pub const ALLOWED_EMAILS: &str = "PINGO_ALLOWED_EMAILS";

#[object]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    /// Anyone who can create an account may use Pingo.
    Open,
    /// New accounts must accept an invite link before they can do anything else.
    Invite,
}

/// The `PINGO_ALLOWED_EMAILS` list: exact addresses and whole domains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmailRule {
    addresses: Vec<String>,
    domains: Vec<String>,
}

impl EmailRule {
    /// Entries are separated by commas, semicolons or whitespace. `ada@example.com` allows one
    /// address; `@example.com` or `example.com` allows the whole domain (not its subdomains).
    pub fn parse(list: &str) -> std::result::Result<Self, String> {
        let mut rule = Self::default();
        for entry in list
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
        {
            let valid_domain = |d: &str| {
                d.contains('.')
                    && !d.starts_with('.')
                    && !d.ends_with('.')
                    && d.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            };
            match entry.split_once('@') {
                Some(("", domain)) if valid_domain(domain) => rule.domains.push(domain.into()),
                Some((local, domain)) if !local.is_empty() && valid_domain(domain) => {
                    rule.addresses.push(entry.clone())
                }
                None if valid_domain(&entry) => rule.domains.push(entry.clone()),
                _ => {
                    return Err(format!(
                        "{ALLOWED_EMAILS}: `{entry}` is not an email address or domain"
                    ));
                }
            }
        }
        Ok(rule)
    }
    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty() && self.domains.is_empty()
    }
    pub fn allows(&self, email: &str) -> bool {
        let email = email.trim().to_lowercase();
        self.addresses.contains(&email)
            || email
                .rsplit_once('@')
                .is_some_and(|(_, domain)| self.domains.iter().any(|d| d == domain))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub registration: Registration,
    /// `None` when every address is welcome.
    pub emails: Option<EmailRule>,
}

impl Policy {
    pub fn parse(
        registration: Option<&str>,
        emails: Option<&str>,
    ) -> std::result::Result<Self, String> {
        let registration = match registration
            .map(str::trim)
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "" | "open" => Registration::Open,
            "invite" | "invites" | "invite-only" | "invite_only" => Registration::Invite,
            other => {
                return Err(format!(
                    "{REGISTRATION}: expected `open` or `invite`, got `{other}`"
                ));
            }
        };
        let emails = match emails {
            Some(list) => Some(EmailRule::parse(list)?).filter(|r| !r.is_empty()),
            None => None,
        };
        Ok(Self {
            registration,
            emails,
        })
    }
}

/// Process environment, read once: it can't change while the server runs.
fn process_env(name: &'static str) -> Option<&'static str> {
    static REGISTRATION_VAR: OnceLock<Option<String>> = OnceLock::new();
    static EMAILS_VAR: OnceLock<Option<String>> = OnceLock::new();
    let cell = if name == REGISTRATION {
        &REGISTRATION_VAR
    } else {
        &EMAILS_VAR
    };
    cell.get_or_init(|| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
        .as_deref()
}

/// Validates the process environment at startup, so a typo stops the server instead of
/// silently leaving registration open.
pub fn check_process_env() -> std::result::Result<Policy, String> {
    Policy::parse(process_env(REGISTRATION), process_env(ALLOWED_EMAILS))
}

pub async fn policy(env: &Env) -> Result<Policy> {
    let registration = env
        .get(REGISTRATION)
        .await?
        .filter(|v| !v.trim().is_empty());
    let emails = env
        .get(ALLOWED_EMAILS)
        .await?
        .filter(|v| !v.trim().is_empty());
    let registration = registration.as_deref().or(process_env(REGISTRATION));
    let emails = emails.as_deref().or(process_env(ALLOWED_EMAILS));
    // A bad value in Environment closes the server rather than opening it.
    Policy::parse(registration, emails).map_err(|message| {
        user_error(
            "misconfigured",
            format!("Registration is misconfigured. {message}"),
        )
    })
}

#[object]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    /// An allowlist is set and the address hasn't been confirmed yet.
    VerifyEmail,
    EmailNotAllowed,
    InviteRequired,
}

/// Why `user` may or may not use Pingo under `policy`. Membership in any space counts as
/// admission, so switching a running server to invite-only never locks existing people out. On a
/// server with no members yet, the first account is let in so it can create the first space.
pub async fn admission(db: &QueryDb, policy: &Policy, user: Id<User>) -> Result<Admission> {
    if let Some(rule) = &policy.emails {
        let account = db.get(user).await?;
        let Some(email) = account.as_ref().and_then(|u| u.email.clone()) else {
            return Ok(Admission::EmailNotAllowed);
        };
        if !rule.allows(&email) {
            return Ok(Admission::EmailNotAllowed);
        }
        // Without this, anyone could sign up with a password as boss@company.com.
        if !account.is_some_and(|u| u.email_verified) {
            return Ok(Admission::VerifyEmail);
        }
    }
    if policy.registration == Registration::Open {
        return Ok(Admission::Admitted);
    }
    let member = db
        .query::<SpaceMember>()
        .with_index(SpaceMember::by_user, |q| q.eq(SpaceMember::user, user))
        .first()
        .await?;
    if member.is_some() || setting_up(db).await? {
        return Ok(Admission::Admitted);
    }
    Ok(Admission::InviteRequired)
}

/// No one has joined a space yet: the first account may sign up even when invites are required.
async fn setting_up(db: &QueryDb) -> Result<bool> {
    Ok(db.query::<SpaceMember>().first().await?.is_none())
}

fn refusal(admission: Admission) -> Error {
    match admission {
        Admission::Admitted => unreachable!("only refusals are errors"),
        Admission::VerifyEmail => user_error(
            "email_not_verified",
            "Confirm your email address to start using this server.",
        ),
        Admission::EmailNotAllowed => user_error(
            "email_not_allowed",
            "This server only accepts certain email addresses, and yours isn't one of them.",
        ),
        Admission::InviteRequired => user_error(
            "invite_required",
            "This server is invite-only. Open an invite link to join a space.",
        ),
    }
}

pub async fn guard(ctx: GuardCtx, function: GuardedFunction) -> Result<()> {
    if function.is::<status>()
        || function.is::<crate::profiles::me>()
        || function.is::<crate::invites::preview>()
        || function.is::<crate::dev::seed>()
        || function.is::<crate::dev::last_code>()
    {
        return Ok(());
    }
    let user = ctx.auth.require_user()?;
    let mut policy = policy(&ctx.env).await?;
    if function.is::<crate::invites::accept>() {
        policy.registration = Registration::Open;
    }
    match admission(&ctx.db, &policy, user).await? {
        Admission::Admitted => Ok(()),
        refused => Err(refusal(refused)),
    }
}

pub async fn before_sign_up(app: App, attempt: SignUpAttempt) -> Result<()> {
    let code = attempt.redirect_to.as_deref().and_then(invite_code);
    app.query::<check_sign_up>((
        attempt.email,
        attempt.email_verified,
        attempt.method == SignUpMethod::Password,
        code,
    ))
    .await
}

fn invite_code(redirect: &str) -> Option<String> {
    let url = url::Url::parse(redirect).ok()?;
    url.path()
        .strip_prefix("/invite/")
        .filter(|code| !code.is_empty() && !code.contains('/'))
        .map(str::to_owned)
}

#[internal_query]
pub async fn check_sign_up(
    ctx: &QueryCtx,
    email: Option<String>,
    email_verified: bool,
    password: bool,
    code: Option<String>,
) -> Result<()> {
    let policy = policy(&ctx.env).await?;
    if let Some(rule) = &policy.emails {
        if !email.as_deref().is_some_and(|email| rule.allows(email)) {
            return Err(refusal(Admission::EmailNotAllowed));
        }
        if !email_verified && !password {
            return Err(refusal(Admission::VerifyEmail));
        }
    }
    if policy.registration == Registration::Invite && !setting_up(&ctx.db).await? {
        let code = code.ok_or_else(|| refusal(Admission::InviteRequired))?;
        crate::invites::require_valid(&ctx.db, code, ctx.now()).await?;
    }
    Ok(())
}

#[object]
#[derive(Clone, Debug)]
pub struct AccessStatus {
    pub registration: Registration,
    /// Only certain addresses may use this server.
    pub restricted_emails: bool,
    /// No space exists yet, so the first account may sign up without an invite.
    pub setup: bool,
    /// The caller's standing; absent when signed out.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub admission: Option<Admission>,
    /// The caller's address, so the UI can say which one needs confirming.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub email: Option<String>,
}

/// No sign-in required: the sign-up page uses it to explain the policy.
#[query]
pub async fn status(ctx: &QueryCtx) -> Result<AccessStatus> {
    let policy = policy(&ctx.env).await?;
    let (admission, email) = match ctx.auth.user_id() {
        Some(user) => (
            Some(admission(&ctx.db, &policy, user).await?),
            ctx.db.get(user).await?.and_then(|u| u.email.clone()),
        ),
        None => (None, None),
    };
    Ok(AccessStatus {
        registration: policy.registration,
        restricted_emails: policy.emails.is_some(),
        setup: setting_up(&ctx.db).await?,
        admission,
        email,
    })
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn signup_redirect_only_extracts_invitation_paths() {
        assert_eq!(
            invite_code("https://chat.example/invite/secret?source=email#auth"),
            Some("secret".into())
        );
        for redirect in [
            "https://chat.example/",
            "https://chat.example/?invite=secret",
            "https://chat.example/invite/",
            "https://chat.example/invite/secret/extra",
            "/invite/secret",
            "not a URL",
        ] {
            assert_eq!(invite_code(redirect), None);
        }
    }

    #[test]
    fn parses_registration_and_email_rules() {
        assert_eq!(
            Policy::parse(None, None).unwrap(),
            Policy {
                registration: Registration::Open,
                emails: None
            }
        );
        assert_eq!(
            Policy::parse(Some(" Invite "), None).unwrap().registration,
            Registration::Invite
        );
        assert!(Policy::parse(Some("closed"), None).is_err());
        assert_eq!(Policy::parse(None, Some("  ")).unwrap().emails, None);
        let rule = EmailRule::parse("ada@Example.com, @acme.org; studio.dev").unwrap();
        assert!(rule.allows("ADA@example.com"));
        assert!(!rule.allows("bob@example.com"));
        assert!(rule.allows("bob@acme.org"));
        assert!(!rule.allows("bob@eu.acme.org"), "subdomains are separate");
        assert!(rule.allows("x@studio.dev"));
        assert!(!rule.allows("acme.org"));
        assert!(EmailRule::parse("not an email").is_err());
        assert!(EmailRule::parse("@").is_err());
        assert!(EmailRule::parse("a@b").is_err());
    }
}
