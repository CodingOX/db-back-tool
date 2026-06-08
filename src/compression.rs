// src/compression.rs
use crate::error::{Error, Result};
// use sevenz_rust2::{compress_to_path_encrypted, Password};
use std::path::{Path, PathBuf};
use tokio::process::Command;

fn explain_7z_spawn_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        Error::Compression(
            "7z executable not found. Please install p7zip-full first, for example: sudo apt install p7zip-full"
                .to_string(),
        )
    } else {
        Error::Io(error)
    }
}

pub async fn compress_and_encrypt(input_file: &Path, password: &str) -> Result<PathBuf> {
    let output_path = input_file.with_extension("7z");
    // let password = Password::from(password);
    // compress_to_path_encrypted(input_file, &output_path, password)?;

    let mut cmd = Command::new("7z");

    cmd.arg("a")
        .arg("-t7z")
        .arg("-m0=lzma2")
        .arg("-mhe=on") // 启用头部加密
        .arg(&output_path)
        .arg(input_file)
        .env("7Z_PASSWORD", password);

    let output = cmd.output().await.map_err(explain_7z_spawn_error)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            "7z compression failed".to_string()
        } else {
            format!("7z compression failed: {}", stderr)
        };
        return Err(Error::Compression(message));
    }

    Ok(output_path)
}

#[cfg(test)]
mod tests {
    use super::explain_7z_spawn_error;
    use crate::Error;

    #[test]
    fn test_explain_7z_spawn_error_includes_install_hint_for_missing_binary() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let mapped = explain_7z_spawn_error(error);

        assert!(matches!(mapped, Error::Compression(message) if message.contains("p7zip-full")));
    }
}
