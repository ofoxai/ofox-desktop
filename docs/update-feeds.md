# 双更新清单与旧 Mac 迁移

内置助手分支将整个应用的最低系统提升到 macOS 13.5。历史 1.3.8 安装包不修改，旧 Mac 客户端首次升级采用手动下载安装。

## 两套清单

| 清单 | 读取者 | Mac | Windows |
| --- | --- | --- | --- |
| `latest.json` | 已安装的旧客户端 | 固定保留 `ofox_desktop_1.3.8.dmg` | 继续推进有效的新版本 |
| `latest-v2.json` | 新客户端 | 当前发布的兼容安装包 | 当前发布的安装包 |

旧客户端只比较全局版本和 URL 中的版本，不能识别系统门槛。必须保留旧 Mac 条目；删除条目会触发下载页回退，仍然提示升级。即使运行在 macOS 13.5+，旧 Mac 客户端也需要手动迁移一次，之后才使用 v2 清单。未经正式发布，不把开发分支当作已上线版本。

v2 包含 `schemaVersion: 2` 和 `minimumMacOS`。新客户端从系统固定命令读取实际 macOS 版本；未知或低于清单要求时不提供下载，并在三种界面语言中提示原因。缺失平台条目、旧协议或损坏的系统门槛同样拒绝，不回退到旧清单。

## 发布条件与操作

发布版本必须高于 1.3.8；同版本标签的时间后缀不能触发升级。继续遵守同一最终提交的 CI、Windows Probe、dry-run、签名、公证及安装验收门槛。当前分支未改变公开清单、历史安装包或已安装应用。

正式和应急发布均调用 `scripts/update-manifests.mjs publish`：

1. 暂存安装包。CI 将副本的大小和 SHA-256 与已验证的发布证明核对；应急路径再次校验暂存 DMG 的实际系统门槛。
2. 使用 R2 的条件写入取得共同发布锁，在锁内读取并校验两套源站清单。仅明确的 S3 `NoSuchKey` 能初始化 v2；读取失败、JSON `null`、损坏或缺少已发布平台不能当作空清单。
3. 合并两套清单中较新的非 Mac 条目，再应用本次构建的平台，保留旧 Mac 1.3.8。
4. 安装包只允许条件创建。已存在时读取并核对完整大小和 SHA-256；字节不同则拒绝，字节一致才是幂等重试。
5. 先写 v2，再写旧清单，每次都匹配此前读取的 ETag。核对源站及公开清单后释放锁。

这些条件与对象写入在 R2 内原子判定，不采用先检查、再无条件覆盖。[Cloudflare R2 条件写入](https://developers.cloudflare.com/r2/api/s3/api/)、[AWS CLI PutObject 参数](https://docs.aws.amazon.com/cli/latest/reference/s3api/put-object.html)

应急脚本改用配置好的 R2 S3 凭据和支持 `IfMatch`／`IfNoneMatch` 的 AWS CLI，启动时检查 CLI 能力；同时提供 `CLOUDFLARE_ACCOUNT_ID`。它不复用 Wrangler 登录态，不自动创建凭据。通过标准 AWS profile 或环境提供凭据，禁止把密钥写进命令参数、仓库或日志。

`scripts/release.sh --dry-run` 需要实际候选 DMG 和源站读取权限；只验证暂存副本与清单，不取得远端锁、不上传。完整参数数组兼容 macOS 系统 Bash 3.2，含空格的路径仍作为一个参数传递。

## 中断与恢复

两套清单是分别条件写入，不是跨对象事务。如果 v2 已写、旧清单失败，下次显式重试会先合并两清单中较新的非 Mac 条目，修复分平台分叉。安装包的同字节核对避免重试替换已发布版本。

锁位于 `release-control/update-feed-lock.json`，没有基于时间的自动偷锁。AWS 写请求不自动重试。取得锁的结果未知时，不猜测拥有权、不继续上传；释放结果未知时报告需要恢复。进程崩溃可能留下 active 锁。必须先确认原发布进程已结束，并核对源站两套清单，再由操作人员按当前 ETag 恢复锁状态；不能直接删除锁来绕过仍在运行的发布。

公开核对须使用客户端 User-Agent：

```bash
curl -fsSL -A ofox-desktop https://desktop.ofox.ai/latest-v2.json
curl -fsSL -A ofox-desktop https://desktop.ofox.ai/latest.json
```

首次正式切换时还需验证官网下载入口的手动迁移说明、旧 Mac 不提示不兼容包、Windows 正常更新和新客户端读取 v2。模拟 S3、单元测试或本机 macOS 26 结果均不能替代真实 R2 发布、macOS 13.5、Windows、签名及真实计费验收；未通过前不发布安装包。
