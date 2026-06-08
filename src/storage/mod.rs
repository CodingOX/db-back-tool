use crate::error::Result;
use chrono::{DateTime, Utc};
use humansize::{DECIMAL, format_size};
use serde::{Deserialize, Serialize};
use std::borrow::Cow::{self, Borrowed};
use std::cmp::Ordering;
use std::path::Path;
use tabled::Tabled;

pub mod aliyun_oss;
pub mod local_storage;
pub mod s3_compatible;
pub mod tencent_cos;

#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    async fn upload(&self, file_path: &Path, cos_path: &str) -> Result<()>;
    async fn list(&self, key: &str) -> Result<Vec<CosItem>>;
    async fn delete(&self, backup_name: &str) -> Result<()>;
}

pub(crate) fn build_object_key(file_path: &Path, cos_path: &str) -> Result<(String, String)> {
    let file_name = file_path
        .file_name()
        .ok_or_else(|| {
            crate::Error::InvalidConfig(format!("Invalid file path: {}", file_path.display()))
        })?
        .to_string_lossy()
        .to_string();

    let object_key = if cos_path.ends_with('/') {
        format!("{}{}", cos_path, file_name)
    } else {
        format!("{}/{}", cos_path, file_name)
    };

    Ok((file_name, object_key))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CosItem {
    pub key: String,
    pub last_modified: DateTime<Utc>,
    pub size: u64,
}

impl PartialEq for CosItem {
    fn eq(&self, other: &Self) -> bool {
        self.last_modified == other.last_modified
    }
}

impl Eq for CosItem {}

impl PartialOrd for CosItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CosItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.last_modified.cmp(&other.last_modified)
    }
}

impl Tabled for CosItem {
    const LENGTH: usize = 3;
    fn headers() -> Vec<Cow<'static, str>> {
        vec![Borrowed("文件路径"), Borrowed("修改时间"), Borrowed("大小")]
    }
    fn fields(&self) -> Vec<Cow<'_, str>> {
        let human_size = format_size(self.size, DECIMAL);
        let last_modified = self.last_modified.format("%Y-%m-%d %H:%M").to_string();
        vec![
            self.key.clone().into(),
            last_modified.into(),
            human_size.into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::build_object_key;
    use std::path::Path;

    #[test]
    fn test_build_object_key_supports_prefix_without_trailing_slash() {
        let path = Path::new("/tmp/demo.7z");
        let (file_name, key) = build_object_key(path, "db").unwrap();
        assert_eq!(file_name, "demo.7z");
        assert_eq!(key, "db/demo.7z");
    }
}
