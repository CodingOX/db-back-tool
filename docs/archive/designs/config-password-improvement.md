# 配置文件密码传递机制改进方案

## 背景

`backupdbtool` 支持用 AES-256-GCM + Argon2id 加密配置文件（`config.yaml` → `config.enc`），避免数据库密码、COS 密钥等敏感信息以明文形式落盘。

加密后的配置文件在程序启动时需要解密，当前仅支持通过 CLI 参数 `-p`/`--password` 传入解密密码。

## 问题

自动化备份场景（crontab / systemd timer）下，`-p` 方式存在两个缺陷：

| 问题 | 说明 |
|------|------|
| **进程列表泄露** | 命令行参数对所有用户可见（`ps aux`），密码暴露在进程列表中 |
| **脚本明文存储** | crontab 或脚本中直接写 `-p "密码"`，密码以明文形式出现在多处 |

## 方案

### 新增两种密码传入渠道

```
优先级 (高→低):
--password  >  --password-file  >  BACKUPDBTOOL_PASSWORD 环境变量
```

| 渠道 | 形式 | 适用场景 |
|------|------|----------|
| `-p` / `--password` | CLI 参数 | 手动执行，交互式使用 |
| `--password-file` | 文件路径，程序读取文件内容作为密码 | crontab 自动化，文件系统权限保护 |
| `BACKUPDBTOOL_PASSWORD` | 环境变量 | CI/CD、容器化部署（K8s Secret → env） |

### 优先级设计原则

- `--password` 最高优先——显式传入，覆盖其他渠道，方便手动调试
- `--password-file` 次之——比环境变量更显式
- 环境变量兜底——与 12-Factor App 风格一致，天然适配容器和 CI

### 密码文件使用方式

```bash
# 创建密码文件
echo "我的主密码" > /etc/backupdbtool/.secret

# 限制仅 owner 可读写
chmod 600 /etc/backupdbtool/.secret

# 确保 owner 是运行备份的用户
chown backup:backup /etc/backupdbtool/.secret

# 使用
./backupdbtool --config config.enc --password-file /etc/backupdbtool/.secret backup mydb
```

文件权限 `600` 含义：

```
  Owner   Group   Others
  rw-     ---     ---
   6       0       0
```

- Owner：可读可写
- Group：无任何权限
- Others：无任何权限

即只有文件所有者才能读取密码，与「谁启动备份应用」对应——owner 就是运行备份的那个用户。

### 环境变量使用方式

```bash
# crontab 中不会暴露到 ps
0 2 * * * BACKUPDBTOOL_PASSWORD=xxx /usr/local/bin/backupdbtool --config /etc/backupdbtool/config.enc backup mydb

# K8s
env:
  - name: BACKUPDBTOOL_PASSWORD
    valueFrom:
      secretKeyRef:
        name: backup-secret
        key: config-password
```

## 代码改动

涉及 3 个文件，改动量约 40 行。

### 1. `src/cli/args.rs` — 新增 `--password-file` 参数

```rust
#[derive(Parser)]
#[command(name = "backupdbtool")]
pub struct Cli {
    // ... 现有字段 ...
    #[arg(short, long)]
    pub password: Option<String>,

    /// Read password from file (more secure than -p for automation)
    #[arg(long)]
    pub password_file: Option<String>,
}
```

### 2. `src/config.rs` — 新增密码解析函数

```rust
/// 按优先级解析配置解密密码
/// --password > --password-file > BACKUPDBTOOL_PASSWORD 环境变量
pub fn resolve_password(
    cli_password: Option<String>,
    password_file: Option<String>,
) -> Option<String> {
    if cli_password.is_some() {
        return cli_password;
    }
    if let Some(path) = password_file {
        let password = std::fs::read_to_string(&path)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|e| {
                tracing::error!("Failed to read password file '{}': {}", path, e);
                std::process::exit(1);
            });
        return Some(password);
    }
    std::env::var("BACKUPDBTOOL_PASSWORD").ok()
}
```

### 3. `src/main.rs` — 调用解析函数

```rust
// 原代码
let mut config = match get_all_config(&config_path, cli.password) {

// 改为
let password = resolve_password(cli.password, cli.password_file);
let mut config = match get_all_config(&config_path, password) {
```

`get_all_config` 函数签名不变，`Encrypt` 子命令不受影响（加密时必须用 `-p`，不需要从文件/环境变量读）。

## 不改动的部分

- `Encrypt` 子命令：已强制要求 `-p` 参数，不参与优先级链（加密场景是交互式操作，不需要自动化）
- `get_all_config`：签名不变，仍然接收 `Option<String>`
- 加密/解密核心逻辑（`src/crypt/aes.rs`）：完全不动

## 安全收益对比

| 场景 | 改前 | 改后 |
|------|------|------|
| 手动执行 | `-p "密码"` → ps 可见 | 不变，手动场景接受这个风险 |
| crontab 自动化 | `-p "密码"` → ps 可见 | `--password-file` → ps 不可见，文件权限 600 保护 |
| K8s CronJob | `-p "密码"` → 硬编码在 args | env var → Secret 注入 |
| CI/CD | `-p "密码"` → 日志可能打印 | env var → CI 变量 masking |

## 待确认

- [ ] 是否需要 `BACKUPDBTOOL_PASSWORD_FILE` 环境变量（环境变量级别的密码文件兜底）？
- [ ] 密码文件支持从文件读取还是直接接受，非文件需报错？
- [ ] 日志中是否需要 mask 密码来源（打印 `password provided via --password-file` 但不打印密码本身）？
