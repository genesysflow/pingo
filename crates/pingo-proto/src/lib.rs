//! Contracts shared by `pingo-server` (RxFerrix app), `pingo-sfu`, `pingo-media` and the web client.
//!
//! Wire format is JSON with camelCase fields and a `"type"` tag, so the TypeScript side can mirror it by hand
//! (`apps/web/src/lib/proto.ts`). Keep both in sync when you change anything here.

pub mod signal;
pub mod token;

pub use signal::{ClientMsg, DataMsg, ParticipantInfo, ServerMsg, TrackInfo, TrackKind};
pub use token::{MediaClaims, TokenError};
