use crate::config::{AllConfig, AppConfig, CosProvider};
use crate::crypt::aes::{
    EncryptedPackage, decrypt_data, encrypt_data, generate_key_from_password, generate_salt,
};
use crate::database::Database;
use crate::error::{Error, Result};
use crate::notify::Notify;
use crate::notify::webhook::{WebHookNotify, WebHookSendData};
use crate::storage::{Storage, build_object_key};
use crate::{compression, utils};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{error, info};

pub fn select_expired_remote_files(
    files: Vec<crate::storage::CosItem>,
    today: chrono::NaiveDate,
    retention_days: Option<u32>,
) -> Vec<crate::storage::CosItem> {
    files
        .into_iter()
        .filter(|item| {
            item.size > 0
                && crate::utils::is_expired_by_retention_days(
                    item.last_modified.date_naive(),
                    today,
                    retention_days,
                )
        })
        .collect()
}

pub fn find_remote_item_by_key<'a>(
    files: &'a [crate::storage::CosItem],
    object_key: &str,
) -> Option<&'a crate::storage::CosItem> {
    files.iter().find(|item| item.key == object_key)
}

async fn ensure_file_non_empty(path: &Path) -> Result<u64> {
    let metadata = tokio::fs::metadata(path).await.map_err(|e| Error::Io(std::io::Error::new(
        e.kind(),
        format!("Failed to stat file '{}': {}", path.display(), e),
    )))?;

    if metadata.len() == 0 {
        return Err(Error::Integrity(format!(
            "file '{}' is empty",
            path.display()
        )));
    }

    Ok(metadata.len())
}

async fn verify_remote_upload_size(
    storage: &dyn Storage,
    file_path: &Path,
    cos_path: &str,
    expected_size: u64,
) -> Result<()> {
    let (_, object_key) = build_object_key(file_path, cos_path)?;
    let files = storage
        .list(&object_key)
        .await
        .map_err(|e| Error::StorageList(e.to_string()))?;
    let remote_item = find_remote_item_by_key(&files, &object_key).ok_or_else(|| {
        Error::Integrity(format!(
            "uploaded object '{}' not found during verification",
            object_key
        ))
    })?;

    if remote_item.size != expected_size {
        return Err(Error::Integrity(format!(
            "uploaded object '{}' size mismatch: local={}, remote={}",
            object_key, expected_size, remote_item.size
        )));
    }

    Ok(())
}

pub async fn backup_database(
    db: &dyn Database,
    database_name: &str,
    back_dir: &Path,
    password: &str,
    notify: Option<WebHookNotify>,
) -> Result<()> {
    // 1. 备份数据库
    let backup_file = db.backup(database_name, back_dir).await?;
    info!("Database backup created: {:?}", backup_file);

    // 2. 压缩并加密
    let compressed_file = compression::compress_and_encrypt(&backup_file, password).await?;
    let compressed_size = ensure_file_non_empty(&compressed_file).await?;
    info!("Backup compressed: {:?}", compressed_file);
    info!("Compressed backup size: {} bytes", compressed_size);

    // 3. 删除原始SQL文件
    if let Err(e) = tokio::fs::remove_file(&backup_file).await {
        error!("Failed to remove temporary SQL file: {}", e);
    }

    if let Some(notify) = notify {
        let message = format!("数据库 {} 备份成功", database_name);
        let data = WebHookSendData::new("备份进度", message);
        notify
            .send(data)
            .await
            .map_err(|e| Error::Notification(e.to_string()))?;
    }

    info!(
        "Backup completed successfully for database: {}",
        database_name
    );
    Ok(())
}

pub async fn upload_to_cos(
    file: Option<String>,
    all: bool,
    config: &AppConfig,
    storage: Arc<dyn Storage>,
    notify: Option<WebHookNotify>,
) -> Result<()> {
    if let Some(file_path) = file {
        // 上传单个文件
        let path = PathBuf::from(&file_path);
        if !path.exists() {
            return Err(Error::FileNotFound(path));
        }
        let local_size = ensure_file_non_empty(&path).await?;
        storage
            .upload(&path, &config.cos_path)
            .await
            .map_err(|_| Error::StorageUpload {
                path: path.clone(),
                message: "upload failed".to_string(),
            })?;
        if config.cos_provider != CosProvider::LocalStorage {
            verify_remote_upload_size(storage.as_ref(), &path, &config.cos_path, local_size)
                .await?;
        }
        if let Some(notify) = notify {
            let message = format!("{} 上传成功", file_path);
            let data = WebHookSendData::new("备份进度", message);
            notify
                .send(data)
                .await
                .map_err(|e| Error::Notification(e.to_string()))?;
        }
        info!("File uploaded successfully: {}", file_path);
    } else if all {
        // 上传所有备份文件
        let uploaded_files =
            utils::upload_all_backups(&config.get_backup_dir(), storage.clone(), &config.cos_path)
                .await
                .map_err(|_| Error::Storage("upload failed".to_string()))?;
        if config.cos_provider != CosProvider::LocalStorage {
            for path in uploaded_files {
                let local_size = ensure_file_non_empty(&path).await?;
                verify_remote_upload_size(storage.as_ref(), &path, &config.cos_path, local_size)
                    .await?;
            }
        }
        if let Some(notify) = notify {
            let data = WebHookSendData::new("备份进度", "所有备份文件上传成功");
            notify
                .send(data)
                .await
                .map_err(|e| Error::Notification(e.to_string()))?;
        }
        info!("All backups uploaded successfully");
    } else {
        return Err(Error::CommandExecution(
            "Please specify either --file or --all flag".to_string(),
        ));
    }

    Ok(())
}

pub async fn delete_from_cos(
    key: Option<String>,
    all: bool,
    storage: &dyn Storage,
    prefix: &str,
    retention_days: Option<u32>,
) -> Result<()> {
    if let Some(key_str) = key {
        storage
            .delete(&key_str)
            .await
            .map_err(|e| Error::StorageDelete {
                key: key_str.clone(),
                message: e.to_string(),
            })?;
        info!("File deleted successfully: {}", key_str);
    } else if all {
        let files = storage
            .list(prefix)
            .await
            .map_err(|e| Error::StorageList(e.to_string()))?;

        let expired_files =
            select_expired_remote_files(files, chrono::Utc::now().date_naive(), retention_days);
        for entry in expired_files {
            storage
                .delete(&entry.key)
                .await
                .map_err(|_| Error::StorageDelete {
                    key: entry.key,
                    message: "delete failed".to_string(),
                })?;
        }

        info!("yesterday before backups delete successfully");
    } else {
        return Err(Error::CommandExecution(
            "Please specify either --key or --all flag".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        find_remote_item_by_key, select_expired_remote_files, verify_remote_upload_size,
    };
    use crate::error::Result;
    use crate::storage::CosItem;
    use crate::storage::Storage;
    use chrono::{NaiveDate, TimeZone, Utc};
    use std::path::Path;
    use tempfile::tempdir;
    use tokio::fs;
    use tokio::io::AsyncWriteExt;

    #[derive(Clone, Default)]
    struct MockStorage {
        list_items: Vec<CosItem>,
    }

    #[async_trait::async_trait]
    impl Storage for MockStorage {
        async fn upload(&self, _file_path: &Path, _cos_path: &str) -> Result<()> {
            Ok(())
        }

        async fn list(&self, _key: &str) -> Result<Vec<CosItem>> {
            Ok(self.list_items.clone())
        }

        async fn delete(&self, _backup_name: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_select_expired_remote_files_uses_retention_days() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        let files = vec![
            CosItem {
                key: "db/demo_20260608_010101.7z".to_string(),
                last_modified: Utc.from_utc_datetime(&today.and_hms_opt(1, 1, 1).unwrap()),
                size: 10,
            },
            CosItem {
                key: "db/demo_20260606_010101.7z".to_string(),
                last_modified: Utc.from_utc_datetime(
                    &NaiveDate::from_ymd_opt(2026, 6, 6)
                        .unwrap()
                        .and_hms_opt(1, 1, 1)
                        .unwrap(),
                ),
                size: 10,
            },
        ];

        let expired = select_expired_remote_files(files, today, Some(2));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].key, "db/demo_20260606_010101.7z");
    }

    #[test]
    fn test_find_remote_item_by_key_matches_exact_key() {
        let files = vec![CosItem {
            key: "db/demo.7z".to_string(),
            last_modified: Utc.from_utc_datetime(
                &NaiveDate::from_ymd_opt(2026, 6, 8)
                    .unwrap()
                    .and_hms_opt(1, 1, 1)
                    .unwrap(),
            ),
            size: 10,
        }];

        let matched = find_remote_item_by_key(&files, "db/demo.7z").unwrap();
        assert_eq!(matched.size, 10);
    }

    #[tokio::test]
    async fn test_verify_remote_upload_size_detects_mismatch() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("demo.7z");
        let mut file = fs::File::create(&file_path).await.unwrap();
        file.write_all(b"12345").await.unwrap();
        file.flush().await.unwrap();

        let storage = MockStorage {
            list_items: vec![CosItem {
                key: "db/demo.7z".to_string(),
                last_modified: Utc.from_utc_datetime(
                    &NaiveDate::from_ymd_opt(2026, 6, 8)
                        .unwrap()
                        .and_hms_opt(1, 1, 1)
                        .unwrap(),
                ),
                size: 3,
            }],
        };

        let err = verify_remote_upload_size(&storage, &file_path, "db", 5)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::Error::Integrity(message) if message.contains("size mismatch")));
    }
}

pub fn encrypt_yaml_file(source: &PathBuf, destination: &PathBuf, password: &str) -> Result<()> {
    // Read the source yaml file
    let toml_content = fs::read_to_string(source).map_err(Error::Io)?;

    // Parse yaml to validate it's valid
    let _config: AllConfig = serde_yml::from_str(&toml_content).map_err(Error::Yaml)?;

    // Generate a salt and derive a key from the password
    let salt = generate_salt();
    let key = generate_key_from_password(password.as_bytes(), &salt)?;

    // Encrypt the data
    let encrypted_data = encrypt_data(toml_content.as_bytes(), &key)?;

    // Prepare the encrypted package with salt
    let encrypted_package = EncryptedPackage {
        salt: salt.to_vec(),
        ciphertext: encrypted_data,
    };

    // Serialize and save
    let serialized = serde_json::to_string(&encrypted_package).map_err(Error::Json)?;

    fs::write(destination, serialized).map_err(Error::Io)?;

    info!(
        "File encrypted successfully: {} -> {}",
        source.display(),
        destination.display()
    );

    Ok(())
}

pub fn decrypt_yaml_file(encrypted_file: &PathBuf, password: &str) -> Result<AllConfig> {
    // Read the encrypted file
    let encrypted_content = fs::read_to_string(encrypted_file).map_err(Error::Io)?;

    // Parse the encrypted package
    let encrypted_package: EncryptedPackage =
        serde_json::from_str(&encrypted_content).map_err(Error::Json)?;

    // Derive the key from the password and salt
    let key = generate_key_from_password(password.as_bytes(), &encrypted_package.salt)?;

    // Decrypt the data
    let decrypted_data = decrypt_data(&encrypted_package.ciphertext, &key)?;

    // Parse the decrypted yaml to validate it's valid
    let decrypted_str = String::from_utf8(decrypted_data)
        .map_err(|_| Error::Decryption("Decrypted data is not valid UTF-8".to_string()))?;

    let config: AllConfig = serde_yml::from_str(&decrypted_str).map_err(Error::Yaml)?;

    Ok(config)
}
