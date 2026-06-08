# 当前问题优先级清单

> 更新时间：2026-06-08
> 状态：待逐项处理

## P0

### 1. OOM / 大文件内存峰值过高

- 状态：部分已处理
- 影响：中大型数据库备份可能直接 OOM
- 代码位置：
  - `postgresql.rs`：`cmd.output()` 全量缓冲 dump 输出
  - `mysql.rs`：`cmd.output()` 全量缓冲 dump 输出
  - `s3_compatible.rs`：上传前全量读取备份文件
  - `aliyun_oss.rs`：上传前同步全量读取备份文件
- 说明：
  - dump 阶段的内存占用与数据库导出体积正相关
  - S3 / 阿里云 OSS 上传阶段的内存占用与 `.7z` 文件大小正相关
- 本次已处理：
  - `postgresql.rs` / `mysql.rs` 已改为 stdout 直接流式写盘
  - `s3_compatible.rs` / `aliyun_oss.rs` 已改为基于文件句柄的流式上传
- 剩余验证：
  - 需要补真实大文件场景验证，确认运行态峰值内存显著下降
  - 需要在目标环境验证 S3 / 阿里云 OSS 的流式上传兼容性
- 参考：`audits/memory-safety-and-encryption.md`

## P1

### 2. 本地 / 远端保留策略不一致，存在误删风险

- 状态：已处理
- 影响：原先 `delete --all` 可能删除刚生成但尚未上传的本地备份
- 代码位置：
  - `utils.rs`：本地按 `retention_days` 清理
  - `command.rs`：远端按 `retention_days` 清理
- 本次已处理：
  - 新增 `app.retention_days`
  - 本地和远端统一改成按天保留
  - 默认保留最近 2 天，避免历史“本地全删 / 远端保留昨天”的分裂行为
- 参考：`archive/designs/backup-retention-design.md`

### 3. 配置解密密码通过命令行传入，存在泄露风险

- 状态：已处理
- 影响：原先密码可能暴露在 `ps`、crontab、脚本中
- 代码位置：
  - `args.rs`
  - `config.rs`
  - `main.rs`
- 本次已处理：
  - 新增 `--password-file`
  - 新增 `BACKUPDBTOOL_PASSWORD` 环境变量兜底
  - 解密密码优先级统一为 `--password` > `--password-file` > `BACKUPDBTOOL_PASSWORD`
- 参考：`archive/designs/config-password-improvement.md`

## P2

### 4. 多处 `unwrap()` / `expect()` 导致进程直接 panic

- 状态：已处理
- 影响：原先路径异常、存储初始化异常时进程会直接崩溃
- 代码位置：
  - `tencent_cos.rs`
  - `aliyun_oss.rs`
  - `s3_compatible.rs`
- 本次已处理：
  - `TencentCos::new`、`AliyunOss::new`、`S3Oss::new` 已改为返回 `Result`
  - `AppConfig::storage()` 已改为向上传播初始化错误
  - `TencentCos::upload` 的无效路径 `unwrap()` 已改为显式错误
  - `LocalStorage` 初始化路径的 UTF-8 假设已改为显式错误

### 5. 上传与备份流程缺少完整性校验

- 状态：已处理
- 影响：原先文件损坏或远端上传不完整可能直到恢复时才暴露
- 代码位置：
  - `command.rs`
  - `compression.rs`
  - `storage/*.rs`
- 本次已处理：
  - 压缩产物落盘后会检查文件非空
  - `upload --file` / `upload --all` 会在支持远端列表的存储后端上校验远端对象大小
  - `upload_all_backups()` 不再吞掉并发上传错误
  - `local` 后端保留为仅做本地非空校验，不做远端大小核对

### 6. 外部 `7z` 依赖脆弱

- 状态：已处理
- 影响：原先目标环境缺少 `7z` 时只会给出模糊失败
- 代码位置：
  - `compression.rs`
- 本次已处理：
  - `7z` 缺失时会返回明确安装提示
  - `7z` 非零退出时会把 stderr 带回错误信息，便于直接排查

## P3

### 7. 文档与真实行为需要持续同步

- 状态：进行中
- 影响：用户会按错误文档理解当前行为
- 当前动作：
  - 已建立统一入口：`README.md`
  - 已归档旧设计稿与旧问题清单
  - 后续每修一个问题，同步更新 `user-manual.md`
