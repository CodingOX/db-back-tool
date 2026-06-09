use crate::error::{Error, Result};
use crate::storage::{CosItem, Storage};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use futures::future::join_all;
use glob::glob;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};
use tabled::Table;
use tokio::task::JoinHandle;
use tracing::{error, info};

pub fn resolve_path(path_str: &str) -> Result<PathBuf> {
    let resolved_path = if path_str.starts_with("~") {
        let expanded_str = shellexpand::tilde(path_str);
        PathBuf::from(expanded_str.to_string())
    } else {
        PathBuf::from(path_str)
    };

    if resolved_path.exists() {
        std::fs::canonicalize(&resolved_path)
            .map_err(|e| Error::PathResolution(format!("Could not canonicalize path: {}", e)))
    } else {
        Ok(resolved_path)
    }
}

pub fn is_yesterday_before(date: DateTime<Utc>) -> bool {
    let today = Utc::now().date_naive();
    let yesterday = today.pred_opt();
    match yesterday {
        Some(yest) => date.date_naive() < yest,
        None => false,
    }
}

pub fn retention_days_or_default(retention_days: Option<u32>) -> u32 {
    retention_days.unwrap_or(2)
}

pub fn is_expired_by_retention_days(
    item_date: NaiveDate,
    today: NaiveDate,
    retention_days: Option<u32>,
) -> bool {
    let retention_days = i64::from(retention_days_or_default(retention_days));
    let cutoff = today - Duration::days(retention_days.saturating_sub(1));
    item_date < cutoff
}

pub fn parse_backup_date(filename: &str) -> Option<NaiveDate> {
    let name = filename.strip_suffix(".7z")?;
    let mut segments = name.rsplitn(3, '_');
    let _time_part = segments.next()?;
    let date_part = segments.next()?;
    NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()
}

pub async fn upload_all_backups(
    backup_dir: &Path,
    storage: Arc<dyn Storage>,
    cos_path: &str,
) -> Result<Vec<PathBuf>> {
    let pattern = backup_dir.join("*.7z").to_string_lossy().to_string();

    let files = glob(&pattern).map_err(|e| Error::PathResolution(e.to_string()))?;

    let files: Vec<PathBuf> = files.into_iter().filter_map(|file| file.ok()).collect();

    let mut tasks: Vec<JoinHandle<Result<()>>> = Vec::with_capacity(files.len());

    let cos_path = cos_path.to_owned();
    for file in &files {
        let storage = storage.clone();
        let cos_path = cos_path.clone();
        let file = file.clone();
        let handle: JoinHandle<Result<()>> =
            tokio::spawn(async move { storage.upload(&file, &cos_path).await });
        tasks.push(handle);
    }

    for task in join_all(tasks).await {
        match task {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                return Err(Error::CommandExecution(format!(
                    "upload task join failed: {}",
                    err
                )));
            }
        }
    }

    Ok(files)
}

pub async fn cleanup_old_backups(backup_dir: &Path, retention_days: Option<u32>) -> Result<()> {
    let pattern = backup_dir.join("*.7z").to_string_lossy().to_string();

    let files = glob(&pattern).map_err(|e| Error::PathResolution(e.to_string()))?;
    let today = Utc::now().date_naive();
    let mut known_dates = BTreeSet::new();

    for path in files.flatten() {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(date) = parse_backup_date(file_name) {
            known_dates.insert(date);
        }
    }

    let files = glob(&pattern).map_err(|e| Error::PathResolution(e.to_string()))?;

    for entry in files {
        match entry {
            Ok(path) => {
                let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let Some(file_date) = parse_backup_date(file_name) else {
                    continue;
                };
                if known_dates.contains(&file_date)
                    && is_expired_by_retention_days(file_date, today, retention_days)
                {
                    info!("Remove file: {:?}", &path);
                    if let Err(e) = tokio::fs::remove_file(&path).await {
                        error!(
                            "Failed to remove old backup {}: {}",
                            &path.display().to_string(),
                            e
                        );
                    } else {
                        info!("Removed old backup: {}", &path.display().to_string());
                    }
                }
            }
            Err(e) => {
                error!("Error reading file: {}", e);
            }
        }
    }

    Ok(())
}

pub fn list_table(files: Vec<CosItem>) -> Result<()> {
    let table = Table::new(&files).to_string();
    println!("=== COS 文件列表 ===");
    println!("{}", table);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::env;
    use std::fs::File;
    use tempfile::tempdir;
    use tokio::fs;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn test_resolve_path_existing() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        File::create(&file_path).unwrap();

        let resolved = resolve_path(file_path.to_str().unwrap()).unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("testfile.txt"));
    }

    #[test]
    fn test_resolve_path_non_existing() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("not_exist.txt");

        let resolved = resolve_path(file_path.to_str().unwrap()).unwrap();
        assert_eq!(resolved, file_path);
    }

    #[test]
    fn test_resolve_path_with_tilde() {
        let home = env::var("HOME").unwrap();
        let test_path = "~/testfile";
        let resolved = resolve_path(test_path).unwrap();
        assert!(resolved.starts_with(&home));
        assert!(resolved.ends_with("testfile"));
    }

    #[test]
    fn test_parse_backup_date_extracts_date_from_filename() {
        let date = parse_backup_date("mydb_20260608_143025.7z").unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 6, 8).unwrap());
    }

    #[test]
    fn test_is_expired_by_retention_days_keeps_recent_two_days_by_default() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        assert!(!is_expired_by_retention_days(
            NaiveDate::from_ymd_opt(2026, 6, 8).unwrap(),
            today,
            None,
        ));
        assert!(!is_expired_by_retention_days(
            NaiveDate::from_ymd_opt(2026, 6, 7).unwrap(),
            today,
            None,
        ));
        assert!(is_expired_by_retention_days(
            NaiveDate::from_ymd_opt(2026, 6, 6).unwrap(),
            today,
            None,
        ));
    }

    #[tokio::test]
    async fn test_cleanup_old_backups_keeps_recent_files_by_retention_days() {
        let dir = tempdir().unwrap();
        let today = Utc::now().date_naive();
        let yesterday = today - Duration::days(1);
        let two_days_ago = today - Duration::days(2);

        for name in [
            format!("demo_{}_010101.7z", today.format("%Y%m%d")),
            format!("demo_{}_010101.7z", yesterday.format("%Y%m%d")),
            format!("demo_{}_010101.7z", two_days_ago.format("%Y%m%d")),
        ] {
            let path = dir.path().join(&name);
            let mut file = fs::File::create(&path).await.unwrap();
            file.write_all(b"data").await.unwrap();
            file.flush().await.unwrap();
        }

        cleanup_old_backups(dir.path(), Some(2)).await.unwrap();

        assert!(
            dir.path()
                .join(format!("demo_{}_010101.7z", today.format("%Y%m%d")))
                .exists()
        );
        assert!(
            dir.path()
                .join(format!("demo_{}_010101.7z", yesterday.format("%Y%m%d")))
                .exists()
        );
        assert!(
            !dir.path()
                .join(format!("demo_{}_010101.7z", two_days_ago.format("%Y%m%d")))
                .exists()
        );
    }
}
