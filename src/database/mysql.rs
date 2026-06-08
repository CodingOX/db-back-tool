use super::{Database, stream_dump_to_file};
use crate::config::MySqlConfig;
use crate::error::Result;
use chrono::Utc;
use serde::Deserialize;
use std::ops::Deref;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct MySql(MySqlConfig);

impl Deref for MySql {
    type Target = MySqlConfig;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[async_trait::async_trait]
impl Database for MySql {
    async fn backup(&self, database_name: &str, backup_dir: &Path) -> Result<PathBuf> {
        let backup_filename = format!(
            "{}_{}.sql",
            database_name,
            Utc::now().format("%Y%m%d_%H%M%S")
        );
        let backup_path = backup_dir.join(&backup_filename);

        // 使用mysqldump进行备份
        let mut cmd = tokio::process::Command::new("mysqldump");

        cmd.arg("-h")
            .arg(&self.host)
            .arg("-P")
            .arg(self.port.to_string())
            .arg("-u")
            .arg(&self.username)
            .arg(database_name)
            .env("MYSQL_PWD", &self.password);

        // 直接把 mysqldump 的 stdout 流式写入目标文件，避免大库 dump 时撑爆内存。
        stream_dump_to_file(
            &mut cmd,
            &backup_path,
            &format!("mysqldump failed for database {database_name}"),
        )
        .await?;

        Ok(backup_path)
    }
}

impl MySql {
    pub fn new(config: &MySqlConfig) -> Self {
        MySql(MySqlConfig {
            host: config.host.clone(),
            port: config.port,
            username: config.username.clone(),
            password: config.password.clone(),
        })
    }
}
