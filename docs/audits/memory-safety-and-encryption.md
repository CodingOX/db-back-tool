# 备份工具内存安全与加密现状审计、改造建议

> 撰写日期：2026-06-08
> 状态：待团队审核

---

## 一、项目背景

该项目 (`db-back-tool`) 用 Rust 编写，面向**单实例数据库**的备份场景（非主从架构）。核心流程：

```
pg_dump/mysqldump → .sql 文件 → 7z 加密压缩 → .7z 文件 → 上传云存储
```

目标用户是运维个人维护的单体服务，数据量通常在几百 MB 到几十 GB 不等。

---

## 二、当前加密架构总览

> 说明：本节区分“源码已验证事实”和“需运行时验证事项”。

### 2.1 配置文件加密（可选）

| 项目 | 说明 |
|------|------|
| 默认模式 | `config.yaml` 明文存储所有凭证 |
| 加密命令 | `backupdbtool --config config.yaml encrypt -d config.enc -p <password>` |
| 算法 | Argon2id 密钥派生 + AES-256-GCM |
| 加密范围 | 整个 YAML 文件内容 |
| 输出格式 | JSON `{salt: Base64, ciphertext: Base64}` |
| 运行时解密 | `backupdbtool --config config.enc -p <password> backup <db>` |

关键代码：`src/crypt/aes.rs`（加密原语）、`src/cli/command.rs:144-199`（文件级加解密）。

### 2.2 备份文件加密（当前实现）

| 项目 | 说明 |
|------|------|
| 工具 | 系统 `7z` 命令行（运行环境需自行安装） |
| 当前命令参数 | `7z a -t7z -m0=lzma2 -mhe=on <output> <input>` |
| 密码注入方式 | 环境变量 `7Z_PASSWORD=app.compress_password` |
| 密码来源 | `config.yaml` → `app.compress_password` |
| 是否可关闭 | **不可**，代码中无开关 |
| 已验证结论 | 产物扩展名固定为 `.7z`，且代码尝试启用头部加密 |
| 待运行时验证 | 当前仓库未内建 7z 集成测试，且本次审计环境未安装 `7z`，尚不能确认 `7Z_PASSWORD` 是否足以触发密码加密，以及无密码是否无法列目录 |

关键代码：`src/compression.rs:7-29`。

### 2.3 回答常见问题

**Q: 配置文件凭证是明文吗？**
A: 默认是。可以通过 `Encrypt` 子命令整体加密，运行时传 `--password` 解密。但密码通过命令行参数传递，会留在 shell 历史中。

**Q: 本地备份文件是否加密？**
A: 代码当前固定走 `.7z` 压缩流程，并尝试通过 `-mhe=on` + `7Z_PASSWORD` 启用加密；但由于当前环境缺少 `7z` 可执行文件，本次只能确认“实现意图”，不能把“无密码一定无法列目录”写成已验证事实。

---

## 三、OOM 问题分析

### 3.1 问题根源

整个备份流程中，两个关键节点使用全量内存加载，与数据量成正比增长：

#### 节点 1：数据库 dump — `cmd.output()` 全量缓冲

**文件：** `src/database/postgresql.rs:45`、`src/database/mysql.rs:44`

```rust
// 当前实现
let output = cmd.output().await?;     // stdout 全量进内存 Vec<u8>
file.write_all(&output.stdout).await?; // 再一次性写盘
```

`tokio::process::Command::output()` 内部将子进程 stdout/stderr **完全收集到内存**。一个 20GB 数据库 dump 出来的 SQL 可达 30-40GB，直接触发 OOM。

#### 节点 2：对象存储上传 — 全量读文件后上传

**文件：** `src/storage/s3_compatible.rs:54`、`src/storage/aliyun_oss.rs:53`

```rust
// S3 当前实现
let content = tokio::fs::read(file_path).await?; // 整个 .7z 文件进内存
self.bucket.put_object(&s3_key, &content).await?;
```

```rust
// Aliyun OSS 当前实现
let content = std::fs::read(file_path)?; // 整个 .7z 文件进内存
self.client.put_object(&s3_key, &content).await?;
```

如果 `.7z` 文件是 5GB，S3 和阿里云 OSS 路径都会先分配 5GB 左右内存，再执行上传。对大型备份同样致命。

### 3.2 风险矩阵

| 组件 | 代码位置 | 内存模型 | 风险等级 |
|------|---------|---------|---------|
| pg_dump/mysqldump | `database/postgresql.rs:45`, `database/mysql.rs:44` | 正比于数据库大小 | **高** |
| 7z 压缩 | `compression.rs:22` (`cmd.status()`) | 固定，几 MB | 无 |
| S3 上传 | `storage/s3_compatible.rs:54` (`tokio::fs::read`) | 正比于文件大小 | **高** |
| 腾讯 COS 上传 | `storage/tencent_cos.rs:45` (`put_object_from_file`) | 固定（流式） | 无 |
| 阿里云 OSS 上传 | `storage/aliyun_oss.rs:53` (`std::fs::read`) | 正比于文件大小 | **高** |
| LocalStorage | 无上传动作 | 不适用 | 无 |

### 3.3 影响评估

```
内存峰值 ≈ max(数据库 dump 大小, .7z 文件大小（S3 / 阿里云 OSS 场景）)
```

- 小数据库 (< 500MB)：影响不大，正常运行
- 中型数据库 (1-10GB)：可能 OOM，取决于机器内存
- 大数据库 (> 10GB)：几乎必然 OOM

---

## 四、修改方案

### 4.1 方案总览

核心思路：**将全量内存缓冲替换为流式管道 + 固定大小 buffer**，确保内存占用与数据量无关。

### 4.2 修改点 1：数据库 dump 改为管道直写文件

**目标：** pg_dump/mysqldump stdout 不落内存，直接写磁盘文件。

**方案：** 用 `cmd.stdout(Stdio::piped())` + 固定 buffer 循环读取写入。

```rust
// 改后（伪代码示意）
let mut child = Command::new("pg_dump")
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

let mut stdout = child.stdout.take().unwrap();
let mut file = File::create(&backup_path).await?;
let mut buf = vec![0u8; 64 * 1024]; // 64KB 固定 buffer

loop {
    let n = stdout.read(&mut buf).await?;
    if n == 0 { break; }
    file.write_all(&buf[..n]).await?;
}
```

**涉及文件：** `src/database/postgresql.rs`、`src/database/mysql.rs`

**内存影响：** 10GB → 64KB，降低 5 个数量级。

### 4.3 修改点 2：S3 / 阿里云 OSS 上传改为流式或分片上传

**目标：** 大文件不一次性读入内存。

**方案：** 对使用 `rust-s3` `put_object()` 的路径改为“逐块读取 + 流式/分片上传”。最终 API 选型需要先做一次库能力确认。

```rust
// 改后（伪代码示意）
let mut file = File::open(file_path).await?;
let mut buf = vec![0u8; 5 * 1024 * 1024]; // 5MB 分片

let upload_id = bucket.initiate_multipart_upload(&s3_key).await?;

let mut part_number = 1;
loop {
    let n = file.read(&mut buf).await?;
    if n == 0 { break; }
    bucket.put_multipart_chunk(&s3_key, &upload_id, part_number, &buf[..n]).await?;
    part_number += 1;
}

bucket.complete_multipart_upload(&s3_key, &upload_id, parts).await?;
```

**涉及文件：** `src/storage/s3_compatible.rs`、`src/storage/aliyun_oss.rs`

**注意：** 目前只能确认现有实现会全量读文件，尚未确认 `rust-s3` 是否对当前 S3 兼容目标和阿里云 OSS 目标都提供可用的 multipart API。如果库能力不足，可能需要切换到更合适的 SDK，或直接改成手工分片 HTTP 上传。

### 4.4 修改点 3（附带功能，不属于本次审计事实）：支持不加密备份

**目标：** 增加配置开关，允许跳过密码保护，直接使用普通压缩产物。

**方案：** 在 `AppConfig` 中增加 `compress_encrypt: bool` 字段，默认为 `true` 保持向后兼容。

```yaml
app:
  compress_encrypt: false  # 设为 false 则仅压缩不加密
  compress_password: "xxx" # encrypt=false 时忽略此项
```

```rust
// compression.rs 改后
if encrypt {
    cmd.arg("-mhe=on").env("7Z_PASSWORD", password);
}
```

### 4.5 修改范围汇总

| 修改项 | 文件 | 改动量 | 风险 |
|--------|------|--------|------|
| pg_dump 流式写文件 | `src/database/postgresql.rs` | ~20 行 | 低 |
| mysqldump 流式写文件 | `src/database/mysql.rs` | ~20 行 | 低 |
| S3 分片上传 | `src/storage/s3_compatible.rs` | ~50 行 | 中（需验证 `rust-s3` API 能力） |
| 加密开关 | `src/config.rs` + `src/compression.rs` | ~15 行 | 低 |

### 4.6 不在本次范围内的项目

- 并发备份支持（当前工具定位为单实例串行备份，非主从场景不需要）
- `cmd.output()` 同时收集 stderr 的问题（大数据库 dump 时 stderr 也可能很大，需一并改为流式）
- 7z 密码传递方式是否真正生效的运行时验证（需要安装 `7z` 后做端到端验证）

---

## 五、测试策略建议

1. **单元测试**：用 `tempfile` 创建含大量数据的模拟数据库，验证流式读写正确性
2. **内存压测**：在 512MB 内存的 Docker 容器中备份 5GB 数据库，确认 OOM 不再发生
3. **回归测试**：确保配置文件加密/解密、云存储上传/下载/删除功能不受影响
4. **对象存储验证**：需要真实 S3/MinIO/阿里云 OSS 环境验证分片上传或流式上传行为
5. **7z 端到端验证**：在安装了 `7z` 的环境里，确认 `7Z_PASSWORD` 传递方式是否真的生成了受密码保护且无法匿名列目录的归档

---

## 六、风险评估

| 风险 | 概率 | 缓解措施 |
|------|------|---------|
| `rust-s3` 不支持 multipart | 中 | 预研确认，必要时切 `aws-sdk-s3` |
| 管道模式下错误处理遗漏 | 低 | stderr 同样 pipe + buffer 收集 |
| 分片上传中途失败无回滚 | 中 | 实现 abort_multipart_upload 清理 |
