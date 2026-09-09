# 安装与卸载

[English](../installation.md) | 中文

---

## 前置条件

| 项 | 要求 |
|---|---|
| 操作系统 | Linux（amd64 / arm64）；WSL1 不受支持（WSL2 可用） |
| 二进制形态 | musl 静态链接单二进制——无 glibc / 动态库依赖 |
| 安装脚本依赖 | POSIX sh + coreutils（`install`、`mkdir`）；远程下载另需 `curl`、`tar`（`.tar.zst` 资产另需 `zstd`） |
| 源码构建 | Rust stable 工具链（`cargo build --release`） |
| GPU 驱动 | roxid 不安装驱动——GPU 分载依赖 llama.cpp `--fit`，驱动由用户自理；Linux 官方预编译后端经 Vulkan 加速，需原生 CUDA 时可设 `ROXID_LLAMA_SERVER` 指向自编译产物 |

> 安装脚本的交互提示文案为中文。

## 方式一：安装脚本（推荐）

### 获取脚本

Release 页提供独立脚本资产（首个 Release 发布后可用）：

```sh
curl -fsSL -o roxid_install.sh \
  https://github.com/ffyuhf/roxid/releases/latest/download/roxid_install.sh
sh roxid_install.sh
```

也可以直接使用仓库根目录的 [`roxid_install.sh`](../../roxid_install.sh)。

### 交互流程

```text
执行操作 [1]安装 roxid  [2]卸载 roxid（默认: 1）:          ← 回车默认安装
安装范围 [1]系统全局 /usr/local/bin  [2]用户级 ~/.local/bin: ← root 运行默认 1，非 root 默认 2
二进制来源 [1]本地构建产物  [2]远程下载（默认: 1）:          ← 本地默认 target/release/roxid，可输入自定义路径
本地 roxid 路径（回车使用默认）:                            ← 仅来源=本地时询问
GitHub 代理前缀（回车直连）:                                ← 仅来源=远程且未设 ROXID_GH_PROXY、
                                                            ROXID_DOWNLOAD_URL 时询问
```

- **安装范围**：系统全局落位 `/usr/local/bin`（非 root 自动加 sudo）；用户级落位 `~/.local/bin`（全程免 sudo）。
- **二进制来源**：本地构建产物或远程下载。远程 URL 解析优先级：`ROXID_DOWNLOAD_URL` 完整直链（最高优先，永不拼代理）> 可选 GitHub 代理前缀 + 脚本内置基地址 + `ROXID_VERSION`（默认 latest），资产名 `roxid-linux-<arch>.tar.gz`；URL 以 `.tar.gz`/`.tgz`/`.tar.zst` 结尾先解压取包内 `roxid`，否则视为裸二进制直落。代理前缀与 roxid 运行时 `ROXID_GH_PROXY` 同语义（前缀拼接，见[配置](configuration.md)）；输入缺尾斜杠时自动补齐。内置基地址本身亦可由 `ROXID_RELEASE_BASE` 环境变量覆盖（自建镜像场景）。
- **覆盖安装保护**：检测到 roxid 正在运行时询问是否终止（默认 Y）。
- **落位后**：`install -m755` 覆盖式落位并校验可执行；用户级且 `~/.local/bin` 不在 PATH 时打印加入 PATH 的指引。

### systemd 服务（可选，默认不创建）

| 级别 | 条件 | unit 路径 | 说明 |
|---|---|---|---|
| 系统实例 | 系统全局安装 + systemd 运行中 | `/etc/systemd/system/roxid.service` | 以**安装发起用户**身份运行（root 执行时回退 `SUDO_USER`，数据目录绑定该用户 `~/.roxid`）；需 sudo；`Restart=always` |
| 用户实例 | 用户级安装 + `systemctl --user` 可用 | `~/.config/systemd/user/roxid.service` | 全程免 root；随登录会话启停；可附问开启 `loginctl enable-linger`（默认 N）实现免登录持久运行 |

两级服务均为 `ExecStart=<BINDIR>/roxid serve`，确认创建时才 `enable --now`。

### 非交互安装（CI / 脚本化）

已设置的环境变量跳过对应交互问句：

| 环境变量 | 取值 | 作用 |
|---|---|---|
| `ROXID_INSTALL_SCOPE` | `user` / `system` | 安装范围 |
| `ROXID_INSTALL_SOURCE` | `local` / `remote` | 二进制来源 |
| `ROXID_LOCAL_BIN` | 文件路径 | 本地构建产物路径（来源=local 时） |
| `ROXID_DOWNLOAD_URL` | 完整直链 | 远程下载地址（优先级最高，永不拼代理） |
| `ROXID_VERSION` | tag（默认 `latest`） | 配合内置基地址拼接资产 URL |
| `ROXID_GH_PROXY` | 代理前缀 | 拼接在内置基地址 URL 之前的 GitHub 代理前缀（与 roxid 运行时同名变量语义一致）；设置后跳过代理问句 |
| `ROXID_RELEASE_BASE` | 基地址 URL | 覆盖脚本内置发布基地址（自建镜像） |

## 方式二：手动安装二进制

从 Release 页下载对应架构 tarball（内含 `roxid` 二进制 + `roxid_install.sh`）：

```sh
tar -xzf roxid-linux-amd64.tar.gz
install -m755 roxid ~/.local/bin/roxid
```

## 方式三：源码构建

```sh
git clone https://github.com/ffyuhf/roxid.git
cd roxid
cargo build --release
# 产物：target/release/roxid
```

依赖链纯 Rust + rustls-tls（无 openssl），无需系统级 C 库。

## 下载包哈希校验

每个 Release 资产均附带同名 `.sha256` 文件（tarball 与安装脚本各自附带）：

```sh
sha256sum -c roxid-linux-amd64.tar.gz.sha256
sha256sum -c roxid_install.sh.sha256
```

> 当前未提供 GPG 签名，哈希文件与资产同源同工作流上传。

## 环境变量与 PATH

用户级安装后若 `~/.local/bin` 不在 PATH：

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.profile   # 或 ~/.bashrc
```

重新登录后生效。验证：`roxid -v`。

## 升级

升级 = 重新运行安装脚本（`install -m755` 天然覆盖旧版本）：

```sh
sh roxid_install.sh        # 远程来源 + latest 自动取最新版
ROXID_VERSION=v0.2.0 sh roxid_install.sh   # 指定版本
```

覆盖安装时若 roxid 正在运行，脚本默认终止后继续。

## 卸载与残留清理

入口两种：`sh roxid_install.sh --uninstall`（或 `-u`）直达；无参数运行时菜单选 `2`。

满卸按以下顺序清理六类落痕：

| # | 落痕 | 默认 | 行为 |
|---|---|---|---|
| 1 | 运行实例 | Y 终止 | `pgrep -x roxid` 检测；选 N 则中止卸载 |
| 2 | 系统实例服务 | — | unit 存在才动作：`sudo systemctl disable --now roxid` + 删 unit + `daemon-reload`；无权限降级为 warning 并打印手动命令 |
| 3 | 用户实例服务 | — | `systemctl --user` 停用 + 删 unit；linger 开启时附问关闭（默认 N） |
| 4 | shell 补全 | N 保留 | 三处落位合并一次询问：`~/.local/share/bash-completion/completions/roxid`、`~/.zfunc/_roxid`（含 `.zshrc` 三行激活块满理，块外零触碰）、`~/.config/fish/completions/roxid.fish` |
| 5 | 数据目录 | N 保留 | 先 `du -sh` 展示 `models` / `llama.cpp` / `config.toml` / `auth.json` 体积，再逐项询问删除；目录取 `ROXID_HOME`（缺省 `~/.roxid`）；全空后一并移除 |
| 6 | 二进制 | — | `/usr/local/bin/roxid` 与 `~/.local/bin/roxid` 检测到即删（可能并存，逐处处理） |

卸载完成后复核：

```sh
pgrep -x roxid     # 应无输出
ls ~/.roxid        # 应不存在或仅剩保留项
```

### 数据目录清单（供手动清理参考）

```text
~/.roxid/
├─ config.toml     # setup 引导与代理配置
├─ auth.json       # signin 凭据（权限 0600）
├─ models/         # 模型文件（ollama / HF / derived 三目录）
└─ llama.cpp/      # 后端运行时缓存（多版本 + manual/）
```

## 下一步

- [快速开始](quickstart.md)
- [CLI 命令参考](cli.md)
- [配置](configuration.md)
