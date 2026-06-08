# 备份文件保留策略 — 设计方案

> 状态：待审核  
> 日期：2026-06-08  
> 作者：alistar

---

## 一、背景与现状分析

### 1.1 项目概况

`db-back-tool` 是一款 Rust 编写的数据库备份工具，支持 PostgreSQL/MySQL 的 `dump` 备份、7z 压缩加密、上传至腾讯云 COS / 阿里云 OSS / S3 兼容存储。

典型使用场景：通过 Cron 定时执行 `backup` → `upload` → `delete` 命令链。

### 1.2 当前文件命名规则

备份文件名在 `src/database/postgresql.rs:25-29` 和 `src/database/mysql.rs:25-29` 中生成：

```
格式: {database_name}_{YYYYMMDD}_{HHMMSS}.sql
压缩后: {database_name}_{YYYYMMDD}_{HHMMSS}.7z
示例: mydb_20260608_143025.7z
```

日期时间戳包含在文件名中，精确到秒。所有备份文件**平铺在 `backup_dir` 单一目录下**，无按日分目录。

### 1.3 当前清理逻辑（存在问题）

| 场景 | 代码位置 | 行为 | 问题 |
|------|---------|------|------|
| **本地清理** | `src/utils.rs:65-91` `cleanup_old_backups` | 匹配 `*.7z` **全部删除** | 无任何保留策略 |
| **远程 COS 清理** | `src/cli/command.rs:119-123` | 用 `is_yesterday_before` 过滤，保留今天+昨天 | 硬编码 2 天，不可配 |
| **触发方式** | `src/main.rs:81` | `delete` 命令先执行本地全删，再执行远程删 | 本地清理过于激进 |

**核心问题**：

1. **本地清理无保留策略** — `cleanup_old_backups` 直接将 `backup_dir` 下所有 `.7z` 文件删光。如果用户先 `backup` 再 `delete`，刚备份的文件也会被删。
2. **远程 COS 保留天数硬编码** — `is_yesterday_before` 固定保留今天+昨天（约 2 天），无法按需调整。如果 Cron 每周日才跑一次 `delete`，中间 6 天的旧文件不会被清理。
3. **本地与远程逻辑不一致** — 本地全删、远程保留 2 天，行为分裂。
4. **无保留份数概念** — 仅按日期过滤，无法指定"保留最近 N 份"。

### 1.4 用户 Cron 推荐配置的隐患

README 推荐：每周日凌晨 3 点执行 `delete --all`。实际效果：

- **本地**：`cleanup_old_backups` 先执行，**删除全部** `.7z` 文件（包括刚备份的）
- **远程 COS**：`delete_from_cos` 再执行，删除昨天之前的文件

如果用户先执行了 `backup`（本地生成了 `.7z`）但还没执行 `upload`，`delete --all` 会把本地未上传的备份删掉。这是一个数据安全隐患。

---

## 二、修改目标

### 2.1 核心目标

实现**可配置的备份保留策略**，统一本地和远程的清理行为：

- 支持按**保留天数**配置（如保留最近 7 天）
- 支持按**保留份数**配置（如保留最近 10 份）
- 本地和远程使用**同一套保留逻辑**
- 通过配置文件控制，无需改代码

### 2.2 设计原则

- **KISS** — 不引入额外的持久化状态（不建 SQLite、不写索引文件），直接解析文件名中的时间戳
- **YAGNI** — 先实现保留天数，保留份数作为扩展项
- **向后兼容** — 不配置 `retention_days` 时保持现有行为（远程保留 2 天），本地清理改为安全默认值

### 2.3 文件命名不变

文件名中已包含精确到秒的时间戳（`YYYYMMDD_HHMMSS`），无需改变目录结构或引入元数据文件。解析文件名即可按天分组。

---

## 三、修改文件清单

### 3.1 `config.yaml` — 配置文件

新增 `retention_days` 配置项：

```yaml
app:
  backup_dir: "backup_dir"
  db_type: "postgresql"
  cos_provider: "tencent_cos"
  cos_path: "db/"
  compress_password: "password"
  retention_days: 7          # 新增：备份保留天数，不配置则默认保留 2 天
```

### 3.2 `src/config.rs` — 配置结构体

**`AppConfig` 新增字段：**

```rust
pub struct AppConfig {
    // ... 现有字段保持不变
    pub retention_days: Option<u32>,  // 新增：备份保留天数
}
```

**`Default` 实现中设置默认值：**

```rust
retention_days: None,  // None 时走兼容逻辑（默认 2 天）
```

### 3.3 `src/utils.rs` — 核心清理逻辑

**改动点 1：新增文件名日期解析函数**（约 15 行）

```rust
/// 从备份文件名中提取日期，格式: {db}_{YYYYMMDD}_{HHMMSS}.7z
fn parse_backup_date(filename: &str) -> Option<NaiveDate>
```

**改动点 2：重写 `cleanup_old_backups`**（约 30 行）

现有实现：
```rust
// 当前：匹配 *.7z → 全部删除
pub async fn cleanup_old_backups(backup_dir: &Path) -> Result<()>
```

改为：
```rust
/// 根据 retention_days 清理过期备份
/// - retention_days=Some(7): 保留最近 7 天的备份
/// - retention_days=None: 兼容模式，保留最近 2 天
pub async fn cleanup_old_backups(backup_dir: &Path, retention_days: Option<u32>) -> Result<()>
```

逻辑：
1. glob 匹配 `backup_dir/*.7z`
2. 解析每个文件名中的日期
3. 按天分组，保留最近 N 天
4. 删除超出保留期的文件

**改动点 3：弃用/删除 `is_yesterday_before`**

该函数被 `delete_from_cos` 调用，新的保留逻辑统一后在 `cleanup_old_backups` 和 `delete_from_cos` 中不再需要它。

### 3.4 `src/cli/command.rs` — 远程删除逻辑

**`delete_from_cos` 函数**（约改动 10 行）

现有实现用硬编码的 `is_yesterday_before` 过滤：
```rust
let yesterday_files: Vec<_> = files
    .clone()
    .into_iter()
    .filter(|item| utils::is_yesterday_before(item.last_modified) && item.size > 0)
    .collect();
```

改为使用 `retention_days` 参数：
```rust
let cutoff_date = Utc::now().date_naive() - Duration::days(retention_days as i64);
let expired_files: Vec<_> = files
    .into_iter()
    .filter(|item| item.last_modified.date_naive() < cutoff_date && item.size > 0)
    .collect();
```

### 3.5 `src/main.rs` — 调用点传参

**`delete` 命令分支**（约改动 3 行）：

```rust
Commands::Delete { key, all } => {
    info!("Starting delete...");
    // 传入 retention_days 配置
    utils::cleanup_old_backups(
        &app_config.get_backup_dir(),
        app_config.retention_days,
    ).await?;
    delete_from_cos(key, all, storage.as_ref(), &app_config.cos_path, app_config.retention_days).await
}
```

### 3.6 `src/storage/local_storage.rs` — 无需改动

`LocalStorage` 的 `list`/`delete` 接口保持不变，清理逻辑在 `utils.rs` 层完成。

---

## 四、变更影响评估

| 维度 | 说明 |
|------|------|
| **API 兼容性** | CLI 命令不变，仅配置文件新增可选字段，向后兼容 |
| **行为变更** | 未配置 `retention_days` 时，本地清理从"全删"变为"保留 2 天"，与远程行为一致 |
| **测试影响** | `cleanup_old_backups` 需要新增单测覆盖日期过滤逻辑 |
| **风险** | 低。核心改动在清理层，不影响备份和上传主流程 |

## 五、改动量估算

| 文件 | 改动类型 | 预估行数 |
|------|---------|---------|
| `config.yaml` | 新增配置项 | +2 |
| `src/config.rs` | 新增字段 | +3 |
| `src/utils.rs` | 重写清理逻辑 + 新增日期解析 | +50 / -25 |
| `src/cli/command.rs` | 修改过滤逻辑 | +10 / -8 |
| `src/main.rs` | 传参调整 | +3 |
| **合计** | | **约 70 行净增** |

---

## 六、扩展预留（不在本次实现范围）

- `retention_count` — 按保留份数而非天数
- 按数据库名分别配置保留策略
- `backup_dir` 按 `{db}/{YYYYMMDD}/` 子目录组织
