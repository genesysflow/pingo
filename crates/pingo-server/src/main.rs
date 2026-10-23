//! Pingo's data, auth, storage, and media admission backend.
//!
//! ```sh
//! cargo run -p pingo-server -- serve --dev --http 127.0.0.1:8090 --codegen-out apps/web/src/lib/rxferrix
//! ```
//! Configure the shared media key in RxFerrix Environment (read through `ctx.env`):
//! `cargo run -p pingo-server -- env set PINGO_MEDIA_SECRET` (reads the value from stdin).

mod dev;
mod dms;
mod files;
mod inbox;
mod invites;
mod messages;
mod model;
mod presence;
mod profiles;
mod reactions;
mod rooms;
mod spaces;
#[cfg(test)]
mod tests;
mod typing;
mod voice;

use rxferrix::RxFerrix;
use rxferrix::auth::{AuthConfig, Discord, GitHub, Google};

/// Integration point: merge `pingo_sfu::Sfu::router()` here using `app_routes` in the integration step.
fn mount_media(app: RxFerrix) -> RxFerrix {
    app
}

fn main() -> rxferrix::Result<()> {
    // RxFerrix has no codegen builder option; use the UI target when --dev omits an explicit override.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--dev")
        && !args
            .iter()
            .any(|a| a == "--codegen-out" || a.starts_with("--codegen-out="))
        && std::env::var_os("RXFERRIX_CODEGEN_OUT").is_none()
    {
        rxferrix::codegen::write("apps/web/src/lib/rxferrix")?;
    }
    let auth = AuthConfig::password()
        .magic_link()
        .oauth(GitHub::from_env())
        .oauth(Google::from_env())
        .oauth(Discord::from_env())
        .redirect_url(
            std::env::var("PINGO_APP_URL").unwrap_or_else(|_| "http://localhost:5173".into()),
        )
        .dev_mailer(dev::inbox());
    let mut app = RxFerrix::new()
        .auth(auth)
        .cron::<typing::prune>("prune-typing", "0 0 * * * *", ())
        .cron::<presence::prune>("prune-presence", "0 0 * * * *", ())
        .cron::<voice::prune>("prune-voice", "0 0 * * * *", ());
    if let Some(dir) = std::env::var_os("PINGO_WEB_DIR").filter(|d| !d.is_empty()) {
        app = app.static_dir(dir);
    }
    mount_media(app).run()
}
