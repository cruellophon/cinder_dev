use chrono::Utc;
use harness_store::{BumboStore, FileBackup, FileChangeStatus, Session, TurnCheckpoint};
use harness_tools::resolve_workspace_path;
use std::path::Path;

pub async fn record_file_backup_in_checkpoint(
    store: &BumboStore,
    session_id: &str,
    checkpoint_id: &str,
    title: &str,
    rel_path: &str,
    existed_before: bool,
    previous_content: Option<String>,
    new_content: Option<String>,
    diff: String,
) -> anyhow::Result<()> {
    store
        .update(|s| {
            if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                let cp = if let Some(existing) =
                    sess.checkpoints.iter_mut().find(|c| c.id == checkpoint_id)
                {
                    existing
                } else {
                    sess.checkpoints.push(TurnCheckpoint {
                        id: checkpoint_id.to_string(),
                        session_id: session_id.to_string(),
                        title: title.to_string(),
                        created_at: Utc::now(),
                        files: Vec::new(),
                    });
                    sess.checkpoints.last_mut().unwrap()
                };

                if let Some(existing_file) = cp.files.iter_mut().find(|f| f.rel_path == rel_path) {
                    // Keep the original pre-turn baseline (existed_before / previous_content)
                    existing_file.new_content = new_content;
                    existing_file.diff = diff;
                    existing_file.status = FileChangeStatus::PendingReview;
                } else {
                    cp.files.push(FileBackup {
                        rel_path: rel_path.to_string(),
                        existed_before,
                        previous_content,
                        new_content,
                        diff,
                        status: FileChangeStatus::PendingReview,
                    });
                }
                sess.updated_at = Utc::now();
            }
        })
        .await
}

pub async fn accept_checkpoint_file(
    store: &BumboStore,
    session_id: &str,
    checkpoint_id: &str,
    rel_path: &str,
) -> anyhow::Result<Session> {
    let updated = store
        .update(|s| {
            if let Some(sess) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                if let Some(cp) = sess.checkpoints.iter_mut().find(|c| c.id == checkpoint_id) {
                    if let Some(file) = cp.files.iter_mut().find(|f| f.rel_path == rel_path) {
                        file.status = FileChangeStatus::Accepted;
                    }
                }
                sess.updated_at = Utc::now();
                return Some(sess.clone());
            }
            None
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
    Ok(updated)
}

pub async fn revert_checkpoint_file(
    store: &BumboStore,
    session_id: &str,
    checkpoint_id: &str,
    rel_path: &str,
) -> anyhow::Result<Session> {
    let snap = store.snapshot().await;
    let sess = snap
        .sessions
        .iter()
        .find(|x| x.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
    let cp = sess
        .checkpoints
        .iter()
        .find(|c| c.id == checkpoint_id)
        .ok_or_else(|| anyhow::anyhow!("Checkpoint not found"))?;
    let backup = cp
        .files
        .iter()
        .find(|f| f.rel_path == rel_path)
        .ok_or_else(|| anyhow::anyhow!("File '{rel_path}' not found in checkpoint"))?;

    restore_backup_to_disk(Path::new(&sess.workspace_path), backup)?;

    let updated = store
        .update(|s| {
            if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                if let Some(cx) = sx.checkpoints.iter_mut().find(|c| c.id == checkpoint_id) {
                    if let Some(fx) = cx.files.iter_mut().find(|f| f.rel_path == rel_path) {
                        fx.status = FileChangeStatus::Reverted;
                    }
                }
                sx.updated_at = Utc::now();
                return Some(sx.clone());
            }
            None
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
    Ok(updated)
}

pub async fn rollback_entire_checkpoint(
    store: &BumboStore,
    session_id: &str,
    checkpoint_id: &str,
) -> anyhow::Result<Session> {
    let snap = store.snapshot().await;
    let sess = snap
        .sessions
        .iter()
        .find(|x| x.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
    let cp = sess
        .checkpoints
        .iter()
        .find(|c| c.id == checkpoint_id)
        .ok_or_else(|| anyhow::anyhow!("Checkpoint not found"))?;

    let ws_root = Path::new(&sess.workspace_path);
    for backup in cp.files.iter().rev() {
        if backup.status != FileChangeStatus::Reverted {
            restore_backup_to_disk(ws_root, backup)?;
        }
    }

    let updated = store
        .update(|s| {
            if let Some(sx) = s.sessions.iter_mut().find(|x| x.id == session_id) {
                if let Some(cx) = sx.checkpoints.iter_mut().find(|c| c.id == checkpoint_id) {
                    for fx in &mut cx.files {
                        fx.status = FileChangeStatus::Reverted;
                    }
                }
                sx.updated_at = Utc::now();
                return Some(sx.clone());
            }
            None
        })
        .await?
        .ok_or_else(|| anyhow::anyhow!("Session not found"))?;
    Ok(updated)
}

fn restore_backup_to_disk(workspace_root: &Path, backup: &FileBackup) -> anyhow::Result<()> {
    let abs = resolve_workspace_path(workspace_root, &backup.rel_path)?;
    if backup.existed_before {
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&abs, backup.previous_content.as_deref().unwrap_or(""))?;
    } else if abs.exists() {
        std::fs::remove_file(&abs)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_store::BumboPaths;
    use uuid::Uuid;

    #[test]
    fn checkpoint_revert_restores_original_file_content() {
        let dir = std::env::temp_dir().join(format!("bumbo_cp_test_{}", Uuid::new_v4()));
        let ws = dir.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let target_file = ws.join("sample.txt");
        std::fs::write(&target_file, "original content\n").unwrap();

        let paths = BumboPaths {
            app_data_dir: dir.join("app"),
            engines_dir: dir.join("app/engines"),
            state_file: dir.join("app/state.json"),
            default_vault_dir: dir.join("app/models"),
        };
        let store = BumboStore::init(paths).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let sid = store
                .update(|s| {
                    s.settings.active_workspace_path = ws.to_string_lossy().to_string();
                    s.sessions[0].workspace_path = ws.to_string_lossy().to_string();
                    s.sessions[0].id.clone()
                })
                .await
                .unwrap();

            let cp_id = Uuid::new_v4().to_string();
            std::fs::write(&target_file, "modified content\n").unwrap();
            record_file_backup_in_checkpoint(
                &store,
                &sid,
                &cp_id,
                "Test Turn",
                "sample.txt",
                true,
                Some("original content\n".into()),
                Some("modified content\n".into()),
                "--- a/sample.txt\n+++ b/sample.txt\n-original content\n+modified content\n"
                    .into(),
            )
            .await
            .unwrap();

            assert_eq!(
                std::fs::read_to_string(&target_file).unwrap(),
                "modified content\n"
            );

            let sess = revert_checkpoint_file(&store, &sid, &cp_id, "sample.txt")
                .await
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(&target_file).unwrap(),
                "original content\n"
            );
            assert_eq!(sess.checkpoints[0].files[0].status, FileChangeStatus::Reverted);
        });

        let _ = std::fs::remove_dir_all(&dir);
    }
}
