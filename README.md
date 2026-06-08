# 使用说明

一款基于 Rust 开发的数据库备份工具，支持单实例 PostgreSQL/MySQL 数据库的自动备份、压缩，并可将备份文件上传至腾讯云 COS、阿里云 OSS 或兼容 S3 协议的其他云存储。

开发动机是本人维护着很多单体服务分布在各个云服务器上，每个单体服务都使用各自的数据库实例，因为甲方预算原因没有配置数据库主从备份。
但又有备份的需求，所以只好写个工具使用数据库自带的 dump 工具备份，再压缩并上传到云存储。

## 功能特性

- 支持 PostgreSQL\MySql 数据库自动备份
- 备份文件自动压缩；当前实现会尝试通过 `7z` 启用密码保护
- 一键上传备份到腾讯云 COS\阿里云 OSS\兼容S3协议的其他云存储
- 支持备份文件的批量上传、批量删除、列表查看
- 支持按 `retention_days` 统一清理本地与远端过期备份
- 支持基础完整性校验：压缩产物非空、远端支持列表时校验上传后对象大小
- 支持自定义配置文件
- 支持加密配置文件以防泄漏关键配置
- 支持 `--password-file` 和 `BACKUPDBTOOL_PASSWORD`，降低自动化场景下的密码泄露风险
- 支持 webhook 通知进度消息

## 前置条件

请确保服务器已安装 `7z`。  
安装命令（Debian/Ubuntu）：

```bash
sudo apt install p7zip-full
```

> 当前仓库未内建 `7z` 端到端集成测试。
> 如果运行时缺少 `7z`，当前版本会直接给出安装 `p7zip-full` 的明确提示。
> 代码会以 `7Z_PASSWORD` 环境变量配合 `-mhe=on` 调用 `7z`，但是否在你的目标环境里真正形成“必须输入密码才能列目录/解压”的归档，建议部署前自行验证一次。

---

## 快速开始

1. 从 [release 页面](https://github.com/iKeepLearn/db-back-tool/releases) 下载可执行文件的 zip 包。
2. 解压后，修改其中的 `config.yaml` 配置文件为正确的配置。
3. 如果使用加密配置文件，推荐在自动化场景中使用 `--password-file` 或 `BACKUPDBTOOL_PASSWORD`，不要把主密码直接写进 `-p`。

### 配置补充

`app` 节当前还有两个常用字段：

```yaml
app:
  retention_days: 2
  compress_password: "password"
```

- `retention_days`
  - 本地和远端统一按天保留
  - `delete --all` 会删除早于保留窗口的备份
  - 不配置时默认按最近 `2` 天处理
- `compress_password`
  - 传给 `7z` 的压缩密码

解密密码优先级：

```text
--password  >  --password-file  >  BACKUPDBTOOL_PASSWORD
```

---

## 常用命令示例

- **加密配置文件**

  ```bash
  ./backupdbtool --config config.yaml encrypt -d encrypted.yaml -p password
  ```
  > 加密完成并测试成功后可删除原始 `config.yaml` 文件

- **备份指定数据库**

  ```bash
  ./backupdbtool --config config.yaml backup <database_name>
  ```

  使用加密配置文件

   ```bash
  ./backupdbtool --config encrypted.yaml --password-file /etc/db-back-tool/password backup <database_name>
  ```

- **上传所有待上传备份文件**

  ```bash
  ./backupdbtool --config config.yaml upload --all
  ```

  使用加密配置文件

   ```bash
  ./backupdbtool --config encrypted.yaml --password-file /etc/db-back-tool/password upload --all
  ```

- **上传单个备份文件**

  ```bash
  ./backupdbtool --config config.yaml upload --file /path/to/filename.ext
  ```

  使用加密配置文件

   ```bash
  ./backupdbtool --config encrypted.yaml --password-file /etc/db-back-tool/password upload --file /path/to/filename.ext
  ```

- **删除所有过期备份以减少云存储成本**

  ```bash
  ./backupdbtool --config config.yaml delete --all
  ```

  使用加密配置文件

   ```bash
  ./backupdbtool --config encrypted.yaml --password-file /etc/db-back-tool/password delete --all
  ```

  > `delete --all` 当前会按 `retention_days` 同时清理本地和远端，不再是“本地全删、远端只删两天前”。

- **删除单个云存储文件**

  ```bash
  ./backupdbtool --config config.yaml delete --key key
  ```

  使用加密配置文件

   ```bash
  ./backupdbtool --config encrypted.yaml --password-file /etc/db-back-tool/password delete --key key
  ```

  > key 为云存储中的完整路径，比如想删除下方 list 中的 config.yaml 则 key 为 db/config.yaml。

  > 完整示例: ./backupdbtool --config config.yaml delete --key db/config.yaml。

- **列出所有备份文件**
  ```bash
  ./backupdbtool --config config.yaml list
  ```

  使用加密配置文件

   ```bash
  ./backupdbtool --config encrypted.yaml --password-file /etc/db-back-tool/password list
  ```

  ![list](images/list.png)

## 定时任务（Cron）推荐配置

- **每日凌晨 2 点自动备份数据库**

  ```bash
  0 2 * * * /path/to/backupdbtool --config /path/to/config.yaml backup <database_name>
  ```

  使用加密配置文件

   ```bash
  0 2 * * * /path/to/backupdbtool --config /path/to/encrypted.yaml --password-file /etc/db-back-tool/password backup <database_name>
  ```


- **每日凌晨 2:30 上传所有待上传备份**

  ```bash
  30 2 * * * /path/to/backupdbtool --config /path/to/config.yaml upload --all
  ```

  使用加密配置文件

   ```bash
  30 2 * * * /path/to/backupdbtool --config /path/to/encrypted.yaml --password-file /etc/db-back-tool/password upload --all
  ```

- **每周日凌晨 3 点删除所有过期备份以减少云存储成本**
  ```bash
  0 3 * * 0 /path/to/backupdbtool --config /path/to/config.yaml delete --all
  ```

  使用加密配置文件

   ```bash
   0 3 * * 0 /path/to/backupdbtool --config /path/to/encrypted.yaml --password-file /etc/db-back-tool/password delete --all
  ```

### 自动化密码建议

推荐：

```bash
echo "主密码" > /etc/db-back-tool/password
chmod 600 /etc/db-back-tool/password

./backupdbtool --config encrypted.yaml --password-file /etc/db-back-tool/password list
```

容器 / CI 可用环境变量：

```bash
BACKUPDBTOOL_PASSWORD="主密码" ./backupdbtool --config encrypted.yaml list
```

> 请将 `/path/to/backupdbtool` 和 `/path/to/config.yaml` 替换为实际路径，`<database_name>` 替换为目标数据库名称。

## 联系方式

如有疑问，请联系开发者。

![联系作者](images/ccwechat.jpg)
