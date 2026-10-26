//! Upload grants are authenticated and throttled; attachments must belong to the sender.
use crate::model::{rate_limit, user_error};
use rxferrix::prelude::*;
use rxferrix::storage::StoredFile;
#[mutation]
#[allow(non_snake_case)]
pub async fn generateUploadUrl(ctx: &MutationCtx) -> Result<String> {
    let user = ctx.auth.require_user()?;
    rate_limit(ctx, user, "upload", 30).await?;
    ctx.storage.generate_upload_url().await
}
pub async fn check_owned(ctx: &MutationCtx, file: Id<StoredFile>) -> Result<()> {
    let user = ctx.auth.require_user()?;
    let f = ctx
        .storage
        .get_metadata(file)
        .await?
        .ok_or_else(|| user_error("not_found", "Uploaded file no longer exists"))?;
    if f.created_by.as_deref() != Some(&format!("user:{user}")) {
        return Err(user_error(
            "forbidden",
            "You can only attach your own uploads",
        ));
    }
    Ok(())
}
