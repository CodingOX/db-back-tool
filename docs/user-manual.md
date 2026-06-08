# db-back-tool 使用手册

> 版本：1.7.1 | Rust Edition 2024
> 适用场景：单实例 PostgreSQL / MySQL 数据库的自动备份、7z 压缩、云存储上传

---

## 目录

1. [项目概述](#1-项目概述)
2. [前置条件](#2-前置条件)
3. [安装](#3-安装)
4. [配置文件](#4-配置文件)
5. [命令参考](#5-命令参考)
6. [配置文件加密流程](#6-配置文件加密流程)
7. [Webhook 通知](#7-webhook-通知)
8. [定时任务 Cron](#8-定时任务-cron)
9. [加密机制说明](#9-加密机制说明)
10. [常见问题与排错](#10-常见问题与排错)
11. [已知限制](#11-已知限制)

---

## 1. 项目概述

一款 Rust 开发的命令行工具，核心流程：

```
pg_dump / mysqldump  →  .sql 文件  →  7z 加密压缩  →  .7z 文件  →  上传云存储
```

### 功能清单

| 功能 | 说明 |
|------|------|
| 数据库备份 | 支持 PostgreSQL（pg_dump）和 MySQL（mysqldump） |
| 压缩产物 | 固定为 `.7z`，代码会以 `-mhe=on` + `7Z_PASSWORD` 调用 `7z` |
| 云存储上传 | 腾讯云 COS / 阿里云 OSS / AWS S3 / 兼容 S3 协议存储（MinIO 等） |
| 本地存储 | 不上传云存储，仅保留本地 `.7z` 备份 |
| 批量操作 | 批量上传、批量删除过期备份、列表查看 |
| 基础完整性校验 | 压缩产物非空校验；远端支持列表时校验上传后对象大小 |
| 配置加密 | 整体加密 `config.yaml`，防止凭证泄漏 |
| Webhook 通知 | 备份/上传完成后推送进度消息 |

---

## 2. 前置条件

### 2.1 必需依赖

| 依赖 | 用途 | 安装（Debian/Ubuntu） |
|------|------|----------------------|
| `7z` | 压缩加密 | `sudo apt install p7zip-full` |
| `pg_dump` | PostgreSQL 备份 | `sudo apt install postgresql-client` |
| `mysqldump` | MySQL 备份 | `sudo apt install mysql-client` |

> 只备份 PostgreSQL 则无需安装 `mysqldump`，反之亦然。
> 如果运行时缺少 `7z`，当前版本会直接给出安装 `p7zip-full` 的明确提示。
> 当前仓库未内建 `7z` 端到端集成测试；是否真的生成“需要密码才能列目录/解压”的归档，建议在目标环境先做一次手工验证。

### 2.2 可选依赖

| 依赖 | 用途 |
|------|------|
| `openssl` | 部分 Linux 发行版运行时需要 |

---

## 3. 安装

从 [GitHub Releases](https://github.com/iKeepLearn/db-back-tool/releases) 下载对应平台的二进制包，解压即用。

```bash
# 解压
unzip backupdbtool-linux-amd64.zip

# 赋予执行权限
chmod +x backupdbtool

# 验证
./backupdbtool version
# 输出：backupdbtool v1.7.1
```

---

## 4. 配置文件

工具依赖 `config.yaml`（YAML 格式），需放在可执行文件同目录或通过 `--config` 指定路径。

### 4.1 完整配置模板

```yaml
# ==================== 应用基础配置 ====================
app:
  backup_dir: "backup_dir"          # 本地备份存储目录，支持 ~ 展开
  db_type: "postgresql"             # 数据库类型：postgresql | mysql
  cos_provider: "tencent_cos"       # 存储后端：tencent_cos | aliyun_oss | s3 | local
  cos_path: "db/"                   # 云存储路径前缀
  compress_password: "password"     # 传给 7z 的密码值（当前实现通过 7Z_PASSWORD 环境变量传递）

# ==================== 腾讯云 COS ====================
tencent_cos:
  secret_id: "AKIDxxxx"             # SecretId
  secret_key: "xxxx"                # SecretKey
  region: "ap-shanghai"             # 地域
  bucket: "bucket-1234567"          # 存储桶名称

# ==================== 阿里云 OSS ====================
aliyun_oss:
  secret_id: "AKIDxxxx"             # AccessKeyId
  secret_key: "xxxx"                # AccessKeySecret
  end_point: "oss-cn-shanghai.aliyuncs.com"
  bucket: "bucket-1234567"

# ==================== S3 兼容存储（AWS/MinIO 等） ====================
s3:
  secret_id: "AKIDxxxx"             # AccessKeyId
  secret_key: "xxxx"                # SecretKey
  end_point: "oss-cn-shanghai.aliyuncs.com"  # 自定义端点（兼容 S3 的服务必填）
  bucket: "bucket-1234567"
  region: "ap-shanghai"             # 区域，与 end_point 二选一（AWS S3 只用此项）

# ==================== PostgreSQL 连接 ====================
postgresql:
  host: "localhost"
  port: 5432
  username: "postgres"
  password: "postgres"

# ==================== MySQL 连接 ====================
mysql:
  host: "localhost"
  port: 3306
  username: "root"
  password: "password"

# ==================== Webhook 通知（可选） ====================
webhook:
  url: "https://api.example.com/webhook"   # Webhook URL
  token: "ISRvxxxx"                        # Bearer Token（可选）
```

### 4.2 字段说明

#### `app` 节

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `backup_dir` | string | 否 | `~/.dbbackup` | 备份文件本地存储目录 |
| `db_type` | string | 是 | `postgresql` | `postgresql` 或 `mysql` |
| `cos_provider` | string | 是 | `tencent_cos` | 云存储类型，设为 `local` 则仅本地存储 |
| `cos_path` | string | 是 | `db/` | 云存储中的目录前缀 |
| `compress_password` | string | 是 | `dbbackuppassword` | 传给 7z 的密码值 |

#### 存储后端（`tencent_cos` / `aliyun_oss` / `s3`）

| 字段 | 说明 |
|------|------|
| `secret_id` | 云服务 AccessKeyId |
| `secret_key` | 云服务 AccessKeySecret |
| `region` | 地域（腾讯 COS、AWS S3 必填） |
| `end_point` | 自定义端点（阿里云 OSS 必填，S3 兼容服务与 `region` 二选一） |
| `bucket` | 存储桶名称 |

#### 数据库（`postgresql` / `mysql`）

| 字段 | 类型 | 说明 |
|------|------|------|
| `host` | string | 主机地址 |
| `port` | u16 | 端口 |
| `username` | string | 用户名 |
| `password` | string | 密码（明文） |

#### `webhook`（可选）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `url` | string | 是 | Webhook 接收地址 |
| `token` | string | 否 | Bearer Token，传了会加 `Authorization: Bearer <token>` 头 |

如果不需要 Webhook，删除整个 `webhook` 节即可。

### 4.3 存储后端选择指南

| 场景 | `cos_provider` 值 | 需配置的节 |
|------|-------------------|-----------|
| 仅本地，不上传 | `local` | 无需云存储配置 |
| 腾讯云 COS | `tencent_cos` | `tencent_cos` 节 |
| 阿里云 OSS | `aliyun_oss` | `aliyun_oss` 节 |
| AWS S3 | `s3` | `s3` 节（填 `region`，不填 `end_point`） |
| MinIO / 兼容 S3 | `s3` | `s3` 节（填 `end_point`） |

---

## 5. 命令参考

### 5.1 全局参数

```bash
backupdbtool --config <路径> [--password <密码> | --password-file <文件>] <子命令>
```

| 参数 | 短参数 | 必填 | 说明 |
|------|--------|------|------|
| `--config` | `-c` | 是 | 配置文件路径（YAML 或加密后的 JSON） |
| `--password` | `-p` | 否 | 手动执行时显式传入解密密码；优先级最高 |
| `--password-file` | 无 | 否 | 从文件中读取解密密码，适合 crontab / systemd 自动化 |

解密密码优先级：

```text
--password  >  --password-file  >  BACKUPDBTOOL_PASSWORD
```

### 5.2 `backup` — 备份数据库

```bash
backupdbtool --config config.yaml backup <数据库名>
```

**执行流程：**
1. 调用 `pg_dump` / `mysqldump` 导出 SQL
2. 用 `7z` 压缩为 `.7z` 文件，并尝试通过 `-mhe=on` + `7Z_PASSWORD` 启用密码保护
3. 删除中间 `.sql` 文件
4. 发送 Webhook 通知（如已配置）

**示例：**
```bash
# 备份 PostgreSQL 数据库 mydb
./backupdbtool --config config.yaml backup mydb

# 使用加密配置文件
./backupdbtool --config config.enc --password-file /etc/db-back-tool/password backup mydb
```

### 5.3 `upload` — 上传备份文件

```bash
backupdbtool --config config.yaml upload [--file <路径> | --all]
```

| 参数 | 短参数 | 说明 |
|------|--------|------|
| `--file` | `-f` | 上传单个文件 |
| `--all` | `-a` | 上传 `backup_dir` 中所有 `.7z` 文件（并发上传） |

**示例：**
```bash
# 上传所有本地备份
./backupdbtool --config config.yaml upload --all

# 上传单个文件
./backupdbtool --config config.yaml upload --file /path/to/mydb_20260608_020000.7z
```

> `--all` 和 `--file` 必须二选一。

### 5.4 `delete` — 删除备份文件

```bash
backupdbtool --config config.yaml delete [--key <路径> | --all]
```

| 参数 | 短参数 | 说明 |
|------|--------|------|
| `--key` | `-k` | 删除云存储中的单个文件（完整路径） |
| `--all` | `-a` | 删除**两天前**的备份（本地 `backup_dir` 中所有 `.7z` + 云存储中两天前的文件） |

**示例：**
```bash
# 删除所有过期备份（本地 + 云端）
./backupdbtool --config config.yaml delete --all

# 删除云存储中的指定文件
./backupdbtool --config config.yaml delete --key db/mydb_20260606_020000.7z
```

> `--key` 的值是云存储中的完整路径。例如 `list` 命令输出的 `db/config.yaml`，则 `--key db/config.yaml`。

### 5.5 `encrypt` — 加密配置文件

```bash
backupdbtool --config config.yaml encrypt --destination <输出路径> --password <密码>
```

| 参数 | 短参数 | 必填 | 说明 |
|------|--------|------|------|
| `--destination` | `-d` | 是 | 加密输出文件路径 |
| `--password` | `-p` | 是 | 加密密码 |

**示例：**
```bash
./backupdbtool --config config.yaml encrypt -d config.enc -p "我的强密码"
```

加密成功后可删除原 `config.yaml`。详细流程见第 6 节。

### 5.6 `list` — 列出备份文件

```bash
backupdbtool --config config.yaml list
```

根据 `cos_provider` 不同：
- 云存储模式：列出云存储中 `cos_path` 下的所有文件
- 本地模式（`local`）：列出 `backup_dir` 中所有 `.7z` 文件

输出示例：
```
=== COS 文件列表 ===
 文件路径                         修改时间            大小
 db/mydb_20260608_020000.7z      2026-06-08 02:00   45.2 MB
 db/mydb_20260607_020000.7z      2026-06-07 02:00   44.8 MB
```

### 5.7 `version` — 版本号

```bash
./backupdbtool version
```

不需要 `--config` 参数。

---

## 6. 配置文件加密流程

### 6.1 加密原理

```
config.yaml  →  AES-256-GCM 加密  →  config.enc (JSON)
                    ↑
            Argon2id 密钥派生
                    ↑
              用户输入的密码
```

**输出格式（`config.enc`）：**
```json
{
  "salt": "<base64>",
  "ciphertext": "<base64>"
}
```

### 6.2 操作步骤

```bash
# 1. 确认 config.yaml 内容正确
./backupdbtool --config config.yaml list

# 2. 加密配置文件
./backupdbtool --config config.yaml encrypt -d config.enc -p "你的主密码"

# 3. 用加密后的配置验证功能正常
./backupdbtool --config config.enc --password-file /etc/db-back-tool/password list

# 4. 确认无误后删除明文配置
rm config.yaml
```

### 6.3 注意事项

- 配置文件加密密码**不会**存储在文件中，每次使用加密配置都需通过 `--password`、`--password-file` 或 `BACKUPDBTOOL_PASSWORD` 传入
- 如果忘记密码，加密配置文件**无法恢复**
- 自动化场景优先使用 `--password-file` 或 `BACKUPDBTOOL_PASSWORD`，避免在命令行参数中暴露密码

---

## 7. Webhook 通知

### 7.1 配置

```yaml
webhook:
  url: "https://your-server.com/api/webhook"
  token: "your-bearer-token"    # 可选
```

### 7.2 通知时机

| 事件 | 触发时机 |
|------|---------|
| 备份成功 | `backup` 命令完成后 |
| 上传成功 | `upload --file` 或 `upload --all` 完成后 |

### 7.3 通知格式

```json
POST <url>
Authorization: Bearer <token>
Content-Type: application/json

{
  "title": "备份进度",
  "message": "数据库 mydb 备份成功"
}
```

---

## 8. 定时任务 Cron

### 8.1 推荐配置

```cron
# 每天凌晨 2:00 备份数据库 mydb
0 2 * * * /opt/db-back-tool/backupdbtool --config /opt/db-back-tool/config.enc --password-file /etc/db-back-tool/password backup mydb >> /var/log/dbbackup.log 2>&1

# 每天凌晨 2:30 上传所有备份
30 2 * * * /opt/db-back-tool/backupdbtool --config /opt/db-back-tool/config.enc --password-file /etc/db-back-tool/password upload --all >> /var/log/dbbackup.log 2>&1

# 每周日凌晨 3:00 清理过期备份
0 3 * * 0 /opt/db-back-tool/backupdbtool --config /opt/db-back-tool/config.enc --password-file /etc/db-back-tool/password delete --all >> /var/log/dbbackup.log 2>&1
```

### 8.2 密码安全建议

Cron 中明文写密码存在风险。替代方案：

```bash
# 将密码存入受限权限文件
echo "你的主密码" > /etc/db-back-tool/password
chmod 600 /etc/db-back-tool/password

# Cron 中直接让程序读取密码文件
0 2 * * * /opt/db-back-tool/backupdbtool --config /opt/db-back-tool/config.enc --password-file /etc/db-back-tool/password backup mydb

# 容器 / CI 可用环境变量兜底
BACKUPDBTOOL_PASSWORD="你的主密码" /opt/db-back-tool/backupdbtool --config /opt/db-back-tool/config.enc list
```

---

## 9. 加密机制说明

### 9.1 涉及的加密层次

| 层次 | 是否强制 | 算法 | 密钥来源 |
|------|---------|------|---------|
| 配置文件整体加密 | 可选 | AES-256-GCM + Argon2id | 用户自定义密码 |
| 备份文件压缩流程 | 当前实现固定启用 | `7z a -t7z -m0=lzma2 -mhe=on` | `config.yaml` 的 `compress_password` 通过 `7Z_PASSWORD` 环境变量传入 |
| 备份文件是否真正受密码保护 | 需运行时验证 | 取决于目标环境中的 `7z` 行为 | 同上 |
| 云存储传输 | 取决于 SDK | 腾讯 COS / S3 均默认 HTTPS | - |

### 9.2 `-mhe=on` 的意义

如果 7z 在目标环境中正确启用了密码保护，`-mhe=on`（Encrypt Headers）会同时保护文件**元数据**（文件名、目录结构）。

但需要注意：当前仓库没有覆盖这段行为的自动化测试，本次文档只确认“代码尝试这样做”，不能把“无密码一定无法列目录”写成已验证事实。部署前建议手工执行一次：

```bash
7z l your-backup.7z
7z x your-backup.7z
```

### 9.3 `compress_password` 安全建议

- 该值写在 `config.yaml` 中，如果配置文件未加密，它就是明文
- 建议配合第 6 节的配置文件加密一起使用
- 如果目标环境里的 `7z` 的确按预期启用了密码保护，定期更换密码时要考虑旧备份仍使用旧密码

---

## 10. 常见问题与排错

### 10.1 "7z: command not found"

未安装 `p7zip-full`：
```bash
sudo apt install p7zip-full
```

### 10.2 "pg_dump failed for database xxx"

常见原因：
- 数据库主机/端口配置错误
- 用户名/密码错误
- 网络不通（检查防火墙和安全组）
- `pg_hba.conf` 未允许该 IP 的连接

排错方法：
```bash
# 手动验证连接
PGPASSWORD="密码" pg_dump -h 主机 -p 端口 -U 用户名 -d 数据库名 > /dev/null
```

### 10.3 "Configuration file path is required"

未传 `--config` 参数。除了 `version` 子命令外，其他命令都需要：
```bash
./backupdbtool --config config.yaml backup mydb
```

### 10.4 "yaml file decrypt failed"

配置文件解密失败，检查：
- 密码是否正确
- 文件是否真的是加密后的 JSON 格式（不是明文 YAML 格式）

### 10.5 S3 上传 "HTTP code: 403"

- 确认 `secret_id` / `secret_key` 正确
- 确认 `bucket` 名称正确且该 Key 有写入权限
- 使用 MinIO 等服务时确认 `end_point` 地址正确且可访问

### 10.6 `delete --all` 删除的逻辑

`delete --all` 执行两件事：
1. 按 `app.retention_days` 清理本地 `backup_dir` 中过期的 `.7z` 文件
2. 按 `app.retention_days` 清理云存储中过期的备份文件

如果未配置 `retention_days`，当前默认保留最近 `2` 天的数据，即保留今天和昨天，删除更早的备份。

### 10.7 解压 .7z 备份文件

```bash
7z x mydb_20260608_020000.7z
# 输入 compress_password 中设置的密码
```

---

## 11. 已知限制

### 11.1 内存安全（OOM 风险）

当前版本在处理大数据库时存在两个内存瓶颈，详见 `audits/memory-safety-and-encryption.md`：

| 环节 | 问题 | 影响 |
|------|------|------|
| 数据库 dump | 已改为 stdout 流式写盘 | 峰值内存已明显降低，仍需大文件实测 |
| S3 / 阿里云 OSS 上传 | 已改为文件句柄流式上传 | 峰值内存已明显降低，仍需目标环境实测 |

**建议：** 代码层面的 OOM 主链路已处理，但仍建议在目标环境对大数据库做一次真实压测。

### 11.2 功能限制

- 每次只能备份**单个数据库**，多库需多次调用
- 不支持增量备份，每次都是全量 dump
- 备份文件加密**不可关闭**（无开关）
- 当前只支持**按天保留**，不支持按“最近 N 份”保留
- 不支持自定义压缩格式，固定为 7z + LZMA2

### 11.3 环境限制

- 仅 Linux 环境测试过
- 依赖系统安装的 `pg_dump` / `mysqldump` / `7z`，版本差异可能影响兼容性
