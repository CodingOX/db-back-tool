use super::{Database, stream_dump_to_file};
use crate::config::PostgreSqlConfig;
use crate::error::Result;
use chrono::Utc;
use serde::Deserialize;
use std::ops::Deref;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct PostgreSql(PostgreSqlConfig);

impl Deref for PostgreSql {
    type Target = PostgreSqlConfig;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[async_trait::async_trait]
impl Database for PostgreSql {
    async fn backup(&self, database_name: &str, backup_dir: &Path) -> Result<PathBuf> {
        let backup_filename = format!(
            "{}_{}.sql",
            database_name,
            Utc::now().format("%Y%m%d_%H%M%S")
        );
        let backup_path = backup_dir.join(&backup_filename);

        // 使用pg_dump进行备份
        let mut cmd = tokio::process::Command::new("pg_dump");

        cmd.arg("-h")
            .arg(&self.host)
            .arg("-p")
            .arg(self.port.to_string())
            .arg("-U")
            .arg(&self.username)
            .arg("-d")
            .arg(database_name)
            .env("PGPASSWORD", &self.password);

        // 直接把 pg_dump 的 stdout 流式写入目标文件，避免大库 dump 时撑爆内存。
        stream_dump_to_file(
            &mut cmd,
            &backup_path,
            &format!("pg_dump failed for database {database_name}"),
        )
        .await?;

        Ok(backup_path)
    }
}

impl PostgreSql {
    pub fn new(config: &PostgreSqlConfig) -> Self {
        PostgreSql(PostgreSqlConfig {
            host: config.host.clone(),
            port: config.port,
            username: config.username.clone(),
            password: config.password.clone(),
        })
    }
}
