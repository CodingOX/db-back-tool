pub mod mysql;
pub mod postgresql;
use crate::error::Result;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

#[async_trait::async_trait]
pub trait Database {
    async fn backup(&self, database_name: &str, backup_dir: &Path) -> Result<PathBuf>;
}

/// 将数据库导出命令的 stdout 直接落盘，避免把整份 dump 一次性读入内存。
pub(crate) async fn stream_dump_to_file(
    cmd: &mut Command,
    backup_path: &Path,
    failure_prefix: &str,
) -> Result<()> {
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| crate::Error::CommandExecution("failed to capture stdout".to_string()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| crate::Error::CommandExecution("failed to capture stderr".to_string()))?;

    if let Some(parent) = backup_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut file = tokio::fs::File::create(backup_path).await?;
    let stderr_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        stderr.read_to_end(&mut buffer).await.map(|_| buffer)
    });

    tokio::io::copy(&mut stdout, &mut file).await?;
    file.flush().await?;

    let status = child.wait().await?;
    let stderr_bytes = stderr_task
        .await
        .map_err(|e| crate::Error::CommandExecution(format!("stderr task failed: {e}")))??;

    if status.success() {
        Ok(())
    } else {
        Err(crate::Error::DatabaseBackup(format!(
            "{failure_prefix}: {}",
            String::from_utf8_lossy(&stderr_bytes)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::stream_dump_to_file;
    use crate::Error;
    use tempfile::tempdir;
    use tokio::process::Command;

    #[tokio::test]
    async fn test_stream_dump_to_file_writes_stdout_directly_to_file() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("dump.sql");
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("printf 'line1\\nline2\\n'");

        stream_dump_to_file(&mut cmd, &output, "dump failed")
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&output).await.unwrap();
        assert_eq!(content, "line1\nline2\n");
    }

    #[tokio::test]
    async fn test_stream_dump_to_file_returns_stderr_on_failure() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("dump.sql");
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo 'boom' >&2; exit 7");

        let err = stream_dump_to_file(&mut cmd, &output, "dump failed")
            .await
            .unwrap_err();

        match err {
            Error::DatabaseBackup(message) => {
                assert!(message.contains("dump failed"));
                assert!(message.contains("boom"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
