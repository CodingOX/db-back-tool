# 项目改进建议清单

> 基于 2026-06-08 代码审计，按严重程度排序。

---

## 严重问题

### 1. 内存安全：AliyunOss upload 用同步 I/O 读取整个文件

`src/storage/aliyun_oss.rs:53` 使用 `std::fs::read(file_path)` 将整个文件读入内存，而 `src/storage/s3_compatible.rs:54` 使用 `tokio::fs::read`。同一个 trait 的不同实现行为不一致，且同步读取大备份文件会阻塞 tokio runtime。

**建议**：统一使用 `tokio::fs::read` 或改为流式上传，避免大文件撑爆内存。

### 2. 多处 `expect()` 和 `unwrap()` 会直接 panic

- `src/storage/tencent_cos.rs:40` — `file_name().unwrap()` 对无效路径直接崩溃
- `src/storage/tencent_cos.rs:128` — `CosClient::new().expect("init cos client failed")`
- `src/storage/aliyun_oss.rs:159` — `Bucket::new().expect("create aliyun oss bucket failed")`
- `src/storage/s3_compatible.rs:175` — `Bucket::new().expect("create s3 bucket failed")`

**建议**：全部改为返回 `Result`，将错误向上传播，不要在 library 层 panic。

### 3. 外部 7z 二进制依赖脆弱

`src/compression.rs` 完全依赖系统安装的 `7z` 命令。目标环境没有 `p7zip-full` 时整个备份流程直接失败。`Cargo.toml` 中已注释掉了 `sevenz-rust2`。

**建议**：启用 `sevenz-rust2` 作为纯 Rust 替代，消除系统依赖；或至少在启动时检查 7z 可用性并给出明确错误提示。

---

## 中等问题

### 4. Aliyun OSS 用 rust-s3 模拟而非官方 SDK

`src/storage/aliyun_oss.rs` 用 `rust-s3` crate 模拟阿里云 OSS 的 S3 兼容接口。阿里云有官方 Rust SDK，功能更完整（分片上传、STS 临时凭证、断点续传等）。

**建议**：评估是否替换为官方 `aliyun-oss-rust-sdk`。

### 5. 腾讯云 COS SDK 是非官方 crate

`cos-rust-sdk = "0.1.2"` — 版本号暗示社区维护，需关注其活跃度和兼容性。

**建议**：关注腾讯云官方是否有 Rust SDK 发布，或在有替代方案时迁移。

### 6. 备份流程无完整性校验

流程：dump → compress → upload，中间无任何校验步骤。文件可能损坏但不会被发现。

**建议**：至少在上传后对比文件大小；理想情况是上传前后各算一次 SHA256/MD5 hash 做完整性校验。

### 7. `cleanup_old_backups` 不检查时间就删除全部本地 .7z 文件

`src/utils.rs:65-91` — 删除 `backup_dir` 下所有 `.7z` 文件，不检查修改时间。而 COS 端 delete（`src/cli/command.rs:119`）通过 `is_yesterday_before` 做过期判断。两端行为不一致。

**建议**：`cleanup_old_backups` 也加入时间过滤逻辑，与 COS 端行为对齐。

---

## 小问题

### 8. 错误处理分散使用 `process::exit(1)`

`src/main.rs` 多处直接 `process::exit(1)`，但核心函数已返回 `Result`。

**建议**：main 中统一用 `?` 传播错误并做顶层 match，去掉分散的 exit，让错误处理集中在一处。

### 9. backup 命令删除原始 SQL 失败只打 error log

`src/cli/command.rs:32-34` — `remove_file` 失败时错误被吞掉，仅打印日志。这会导致磁盘空间逐渐泄漏。

**建议**：至少记录到 webhook 通知中，或实现定期清理机制。

### 10. Argon2 + AES-GCM + rand 只为加密配置文件

三个重量级 crate 仅用于 `encrypt`/`decrypt` 子命令。如果加密配置不是核心需求，可以砍掉以减小二进制体积和攻击面。

**建议**：评估是否保留 `encrypt` 命令；若保留则保持现状，若不常用可考虑外部脚本替代。

### 11. S3/Aliyun OSS 上传前把整个文件读进内存

与问题 1 相关 —— 目前的 `put_object` API 接受 `&[u8]`，大文件备份会直接 OOM。

**建议**：调研 `rust-s3` 是否支持分片上传 API，或改用流式读取 + multipart upload。

---

## 架构层面的改进考虑

| 维度 | 现状 | 建议 |
|---|---|---|
| 压缩库 | 外部 7z 二进制 | 用 `sevenz-rust2` 内建，消除系统依赖 |
| 配置校验 | 反序列化时报错，无业务逻辑校验 | 增加配置有效性检查（如选择了 cos provider 则对应字段必须填写） |
| 测试覆盖 | 仅 config/encrypt/storage/local 有测试 | 为核心流程（backup→compress→upload）增加集成测试 |
| 日志 | tracing + 控制台输出 | 考虑支持日志文件输出，便于 cron 场景排查 |
| 重试机制 | 无 | 上传/通知失败时加入指数退避重试 |
