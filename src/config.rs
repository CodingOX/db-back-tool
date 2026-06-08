use crate::cli::command::decrypt_yaml_file;
use crate::database::Database;
use crate::database::{mysql::MySql, postgresql::PostgreSql};
use crate::error::{Error, Result};
use crate::notify::webhook::WebHookNotify;
use crate::storage::Storage;
use crate::storage::aliyun_oss::AliyunOss;
use crate::storage::local_storage::LocalStorage;
use crate::storage::s3_compatible::S3Oss;
use crate::storage::tencent_cos::TencentCos;
use config::{Config, File};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::create_dir_all;

#[derive(Debug, Deserialize, Clone)]
pub struct AllConfig {
    pub app: AppConfig,
    pub tencent_cos: TencentCosConfig,
    pub postgresql: PostgreSqlConfig,
    pub mysql: MySqlConfig,
    pub aliyun_oss: AliyunOssConfig,
    pub s3: S3OssConfig,
    pub webhook: Option<WebHookConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub backup_dir: PathBuf,
    pub db_type: DbType,
    pub cos_provider: CosProvider,
    pub cos_path: String,
    pub compress_password: String,
    pub retention_days: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TencentCosConfig {
    pub secret_id: String,
    pub secret_key: String,
    pub region: String,
    pub bucket: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AliyunOssConfig {
    pub secret_id: String,
    pub secret_key: String,
    pub end_point: String,
    pub bucket: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct S3OssConfig {
    pub secret_id: String,
    pub secret_key: String,
    pub end_point: Option<String>,
    pub bucket: String,
    pub region: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PostgreSqlConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MySqlConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WebHookConfig {
    pub url: String,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub enum DbType {
    #[serde(rename = "postgresql")]
    Postgresql,
    #[serde(rename = "mysql")]
    MySql,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub enum CosProvider {
    #[serde(rename = "tencent_cos")]
    TencentCos,
    #[serde(rename = "aliyun_oss")]
    AliyunOss,
    #[serde(rename = "local")]
    LocalStorage,
    #[serde(rename = "s3")]
    S3,
}

impl Default for AppConfig {
    fn default() -> Self {
        let backup_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(".dbbackup");

        AppConfig {
            backup_dir,
            db_type: DbType::Postgresql,
            cos_provider: CosProvider::TencentCos,
            cos_path: "db/".into(),
            compress_password: "dbbackuppassword".into(),
            retention_days: None,
        }
    }
}

impl AppConfig {
    pub async fn confirm_backup_dir(&self) {
        let home_dir: PathBuf = AppConfig::default().backup_dir;
        let result = create_dir_all(&self.backup_dir).await;
        if result.is_err() {
            create_dir_all(home_dir).await.unwrap();
        }
    }

    pub fn get_backup_dir(&self) -> PathBuf {
        self.backup_dir.clone()
    }

    pub fn database(&self, config: &AllConfig) -> Box<dyn Database> {
        match self.db_type {
            DbType::Postgresql => {
                let postgresql = PostgreSql::new(&config.postgresql);
                Box::new(postgresql)
            }
            DbType::MySql => {
                let mysql = MySql::new(&config.mysql);
                Box::new(mysql)
            }
        }
    }
    pub async fn storage(&self, config: &AllConfig) -> Result<Arc<dyn Storage>> {
        let storage = match self.cos_provider {
            CosProvider::TencentCos => {
                let config = &config.tencent_cos;
                Arc::new(TencentCos::new(config)?) as Arc<dyn Storage>
            }
            CosProvider::AliyunOss => {
                let config = &config.aliyun_oss;
                Arc::new(AliyunOss::new(config)?) as Arc<dyn Storage>
            }
            CosProvider::LocalStorage => {
                let path = &config.app.get_backup_dir();
                let storage = LocalStorage::new(path.to_str().ok_or_else(|| {
                    Error::PathResolution("backup_dir is not valid UTF-8".to_string())
                })?)
                .await;
                Arc::new(storage) as Arc<dyn Storage>
            }
            CosProvider::S3 => {
                let config = &config.s3;
                Arc::new(S3Oss::new(config)?) as Arc<dyn Storage>
            }
        };
        Ok(storage)
    }
}

pub fn get_all_config(config_path: &str, password: Option<String>) -> Result<AllConfig> {
    if let Some(pwd) = password {
        // 如果提供了密码，尝试解密配置文件
        let encrypted_path = PathBuf::from(config_path);
        let config = decrypt_yaml_file(&encrypted_path, &pwd).map_err(|_e| {
            Error::Config(config::ConfigError::NotFound(
                "yaml file decrypt failed".to_string(),
            ))
        })?;
        return Ok(config);
    }
    let config_builder = Config::builder()
        // 加载配置文件
        .add_source(File::with_name(config_path))
        .build()?;

    let config = config_builder.try_deserialize()?;
    Ok(config)
}

pub fn resolve_password(
    cli_password: Option<String>,
    password_file: Option<String>,
) -> Result<Option<String>> {
    if let Some(password) = cli_password {
        return Ok(Some(password));
    }

    if let Some(path) = password_file {
        let password = std::fs::read_to_string(&path).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to read password file '{}': {}", path, e),
            ))
        })?;
        return Ok(Some(password.trim().to_string()));
    }

    Ok(std::env::var("BACKUPDBTOOL_PASSWORD").ok())
}

pub fn get_webhook(config: &AllConfig) -> Option<WebHookNotify> {
    config.webhook.as_ref().map(|webhook_config| {
        WebHookNotify::new(webhook_config.url.clone(), webhook_config.token.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs::File as StdFile;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_get_all_config() {
        // 创建一个临时目录
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_config.toml");

        // 构造一个配置文件内容
        let config_content = r#"
            [app]
            backup_dir = "/tmp/dbbackup"
            db_type = "postgresql"
            cos_provider = "tencent_cos"
            cos_path = "db/"
            compress_password = "testpassword"
            retention_days = 7

            [tencent_cos]
            secret_id = "testid"
            secret_key = "testkey"
            region = "ap-guangzhou"
            bucket = "testbucket"

            [postgresql]
            host = "localhost"
            port = 5432
            username = "user"
            password = "pass"

            [mysql]
            host = "localhost"
            port = 3306
            username = "user"
            password = "pass"

            [aliyun_oss]
            secret_id = "testid"
            secret_key = "testkey"
            end_point = "ap-guangzhou"
            bucket = "testbucket"

            [s3]      
              secret_id = "AKIDuhLs"                     
              secret_key = "dGnCj8"                       
              end_point = "oss-cn-shanghai.aliyuncs.com"  
              bucket = "bucket-1234567"                   
              region = "ap-shanghai"                     
        "#;

        // 写入临时配置文件
        let mut file = StdFile::create(&file_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();

        // 调用get_all_config
        let config = get_all_config(file_path.to_str().unwrap(), None).unwrap();

        // 断言配置内容
        assert_eq!(config.app.backup_dir, PathBuf::from("/tmp/dbbackup"));
        assert_eq!(config.app.db_type, DbType::Postgresql);
        assert_eq!(config.app.cos_provider, CosProvider::TencentCos);
        assert_eq!(config.app.cos_path, "db/");
        assert_eq!(config.app.compress_password, "testpassword");
        assert_eq!(config.app.retention_days, Some(7));

        assert_eq!(config.tencent_cos.secret_id, "testid");
        assert_eq!(config.tencent_cos.secret_key, "testkey");
        assert_eq!(config.tencent_cos.region, "ap-guangzhou");
        assert_eq!(config.tencent_cos.bucket, "testbucket");

        assert_eq!(config.postgresql.host, "localhost");
        assert_eq!(config.postgresql.port, 5432);
        assert_eq!(config.postgresql.username, "user");
        assert_eq!(config.postgresql.password, "pass");

        assert_eq!(config.s3.secret_id, "AKIDuhLs");
        assert_eq!(config.s3.secret_key, "dGnCj8");
        assert_eq!(
            config.s3.end_point,
            Some("oss-cn-shanghai.aliyuncs.com".to_string())
        );
        assert_eq!(config.s3.bucket, "bucket-1234567");
        assert_eq!(config.s3.region, Some("ap-shanghai".to_string()));
    }

    #[test]
    fn test_resolve_password_prefers_cli_password() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("pwd.txt");
        std::fs::write(&file_path, "from-file\n").unwrap();
        unsafe {
            env::set_var("BACKUPDBTOOL_PASSWORD", "from-env");
        }

        let resolved = resolve_password(
            Some("from-cli".to_string()),
            Some(file_path.to_string_lossy().to_string()),
        )
        .unwrap();

        assert_eq!(resolved, Some("from-cli".to_string()));
        unsafe {
            env::remove_var("BACKUPDBTOOL_PASSWORD");
        }
    }

    #[test]
    fn test_resolve_password_uses_password_file_before_env() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("pwd.txt");
        std::fs::write(&file_path, "from-file\n").unwrap();
        unsafe {
            env::set_var("BACKUPDBTOOL_PASSWORD", "from-env");
        }

        let resolved =
            resolve_password(None, Some(file_path.to_string_lossy().to_string())).unwrap();

        assert_eq!(resolved, Some("from-file".to_string()));
        unsafe {
            env::remove_var("BACKUPDBTOOL_PASSWORD");
        }
    }

    #[test]
    fn test_resolve_password_falls_back_to_env() {
        unsafe {
            env::set_var("BACKUPDBTOOL_PASSWORD", "from-env");
        }

        let resolved = resolve_password(None, None).unwrap();

        assert_eq!(resolved, Some("from-env".to_string()));
        unsafe {
            env::remove_var("BACKUPDBTOOL_PASSWORD");
        }
    }

    #[tokio::test]
    async fn test_storage_returns_error_for_non_utf8_local_backup_dir() {
        #[cfg(unix)]
        {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt;

            let config = get_all_config(
                tempdir()
                    .unwrap()
                    .path()
                    .join("dummy.toml")
                    .to_string_lossy()
                    .as_ref(),
                None,
            )
            .err();
            assert!(config.is_some() || config.is_none());

            let app = AppConfig {
                backup_dir: PathBuf::from(OsString::from_vec(vec![0x66, 0x6f, 0x80])),
                db_type: DbType::Postgresql,
                cos_provider: CosProvider::LocalStorage,
                cos_path: "db/".to_string(),
                compress_password: "pwd".to_string(),
                retention_days: None,
            };

            let all_config = AllConfig {
                app: app.clone(),
                tencent_cos: TencentCosConfig {
                    secret_id: "id".to_string(),
                    secret_key: "key".to_string(),
                    region: "ap-shanghai".to_string(),
                    bucket: "bucket".to_string(),
                },
                postgresql: PostgreSqlConfig {
                    host: "localhost".to_string(),
                    port: 5432,
                    username: "user".to_string(),
                    password: "pass".to_string(),
                },
                mysql: MySqlConfig {
                    host: "localhost".to_string(),
                    port: 3306,
                    username: "user".to_string(),
                    password: "pass".to_string(),
                },
                aliyun_oss: AliyunOssConfig {
                    secret_id: "id".to_string(),
                    secret_key: "key".to_string(),
                    end_point: "oss-cn-shanghai.aliyuncs.com".to_string(),
                    bucket: "bucket".to_string(),
                },
                s3: S3OssConfig {
                    secret_id: "id".to_string(),
                    secret_key: "key".to_string(),
                    end_point: Some("https://example.com".to_string()),
                    bucket: "bucket".to_string(),
                    region: None,
                },
                webhook: None,
            };

            let err = match app.storage(&all_config).await {
                Ok(_) => panic!("expected path resolution error"),
                Err(err) => err,
            };
            assert!(matches!(err, Error::PathResolution(_)));
        }
    }
}
