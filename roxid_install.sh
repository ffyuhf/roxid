#!/bin/sh
# roxid 安装/卸载脚本（基于 ollama 官方安装脚本骨架裁剪扩展）
# 入口：无参数 → 交互菜单先问「安装/卸载」（回车默认安装，等价原行为）；
#       --uninstall/-u 直达卸载流程；其他参数 error 提示
# 功能：交互式安装 roxid 二进制，支持两种安装范围与两种二进制来源——
#   安装范围：1) 系统全局 /usr/local/bin（需 root/sudo）
#             2) 用户级   ~/.local/bin（全程免 sudo）
#   二进制来源：1) 本地构建产物（默认 target/release/roxid，可自定义路径）
#               2) 远程下载（地址空置占位，见 ROXID_RELEASE_BASE 说明）
#   systemd 服务（两级，可选，默认不创建）：
#     - 系统全局模式 + systemd 运行中：系统实例 /etc/systemd/system/roxid.service
#     - 用户级模式 + systemctl --user 可用：用户实例 ~/.config/systemd/user/roxid.service
#       （用户实例可选 loginctl enable-linger 未登录持久运行）
# 卸载（迭代22）：满卸 roxid 全部落痕——运行实例 → systemd 两级服务 →
#   shell 补全 → ~/.roxid 数据目录（体积展示+逐项询问，默认 N 保留）→
#   二进制两处落位；补全/数据清理一律默认 N，仅明确确认才删除
# 剥离自官方脚本的部分：macOS 分支、GPU/CUDA/ROCm/JetPack 驱动安装
#   （roxid 推理后端由运行时自管于 ~/.roxid/llama.cpp/，GPU 分载依赖 llama.cpp --fit）
#
# 非交互逃生口（环境变量，设置后跳过对应交互问句；未设置一律走交互）：
#   ROXID_INSTALL_SCOPE=user|system    安装范围
#   ROXID_INSTALL_SOURCE=local|remote  二进制来源
#   ROXID_LOCAL_BIN=<路径>             本地构建产物路径（来源=local 时）
#   ROXID_DOWNLOAD_URL=<直链>          远程完整直链（来源=remote 时，最高优先，不拼代理）
#   ROXID_VERSION=<tag>                远程版本号（默认 latest，配合 ROXID_RELEASE_BASE）
#   ROXID_GH_PROXY=<前缀>              GitHub 代理前缀（来源=remote 且非直链时生效；非空即用
#                                      并跳过代理问句；语义同软件本体：前缀拼接在 Releases
#                                      URL 之前；ROXID_DOWNLOAD_URL 直链不受影响）
#
# 修改历史：
#   2026-09-11 04-19 横幅与全面体验优化（迭代34，计划 v1.0.0 用户批准于
#                     2026-09-11 04:19；裁决 Q1/Q2 2026-09-11 04:16/04:17）：
#                     新增 ANSI Shadow 风格 ROXID 横幅（所有入口统一最先打印）、
#                     green/cyan 颜色扩展、阶段分隔线、系统环境信息展示（systemd
#                     两级状态前置探测一次、展示与配置判定两处复用）、安装/卸载
#                     收尾区块美化；安装/卸载逻辑与交互序列零变化（纯呈现层）
#   2026-09-10 03-55 修复两缺陷（迭代26，计划 v1.0.0 用户批准于 2026-09-10 03:55）：
#                     缺陷1 用户实例探测仅凭退出码把 degraded（存在无关失败 unit，
#                     用户总线仍可用）误判不可用，改按 running|degraded 输出状态匹配
#                     对齐系统实例分支；缺陷2 install_success 无条件宣称 API 已可用，
#                     改 SERVICE_STARTED 标志按实际启动状态分支，未启动时打印手动
#                     启动指引（roxid serve 前台运行 / 重跑脚本配置 systemd 服务）
#   2026-09-10 03-25 远程下载支持 GitHub 代理前缀（迭代25，计划 v1.0.0 用户批准于
#                     2026-09-10 03:21；裁决 Q1/Q2 2026-09-10 03:17/03:18：env
#                     ROXID_GH_PROXY 优先 + 交互问句兜底、直链不拼代理；尾斜杠卫生 D3；
#                     ROXID_RELEASE_BASE 改可环境变量覆盖 D8；其余安装/卸载逻辑零变化）
#   2026-09-10 02-35 远程基地址填入（迭代23，计划 v1.2.0 用户批准于 2026-09-10 02:31；
#                     裁决 R3/R4 2026-09-10 02:22：仓库地址 https://github.com/ffyuhf/roxid；
#                     仅填常量与注释，安装/卸载逻辑零变化；tarball 资产内亦随包分发本脚本）
#   2026-09-10 02:03 新增卸载功能（迭代22，计划 v1.0.0 用户批准于 2026-09-10 02:01；
#                     三项裁决 Q1–Q3 见 计划书 1.4 表；安装流程语义零变化）
#   2026-09-10 01:45 新增（迭代21，计划 v1.1.0 用户批准于 2026-09-10 01:44；
#                     六项裁决 Q1–Q6 见 计划书 1.4 表）

# 远程下载基地址（迭代23 填入，裁决 R3/R4 2026-09-10 02:22——仓库 https://github.com/ffyuhf/roxid；
# 配合 ROXID_VERSION 拼接资产名 roxid-linux-<arch>.tar.gz；ROXID_DOWNLOAD_URL 直链优先级更高；
# 迭代25 D8：同名环境变量可覆盖默认值——服务 e2e 模拟与自建镜像，未设置时行为与常量一致）
ROXID_RELEASE_BASE="${ROXID_RELEASE_BASE:-https://github.com/ffyuhf/roxid/releases}"

# Wrap script in main function so that a truncated partial download doesn't end
# up executing half a script.
main() {

set -eu

red="$( (/usr/bin/tput bold || :; /usr/bin/tput setaf 1 || :) 2>&-)"
plain="$( (/usr/bin/tput sgr0 || :) 2>&-)"
# 迭代34：成功/信息双色扩展（green=成功与状态前缀，cyan=横幅/分隔线/信息标签）；
# 沿用 tput 失败回退空串模式——非 TTY 下与既有 red/plain 行为完全一致
green="$( (/usr/bin/tput bold || :; /usr/bin/tput setaf 2 || :) 2>&-)"
cyan="$( (/usr/bin/tput bold || :; /usr/bin/tput setaf 6 || :) 2>&-)"

status() { echo "${green}>>>${plain} $*"; }
error() { echo "${red}错误:${plain} $*"; exit 1; }
warning() { echo "${red}警告:${plain} $*"; }

TEMP_DIR=$(mktemp -d)
cleanup() { rm -rf "$TEMP_DIR"; }
trap cleanup EXIT

available() { command -v "$1" >/dev/null; }
require() {
    local MISSING=''
    for TOOL in "$@"; do
        if ! available "$TOOL"; then
            MISSING="$MISSING $TOOL"
        fi
    done
    echo "$MISSING"
}

# 交互读取一行输入；回车取默认值；stdin 关闭（EOF）时优雅终止而非死循环
# 参数：$1 提示文本  $2 默认值；结果写入全局变量 REPLY
ask_with_default() {
    local prompt="$1"
    local default="$2"
    local answer=''
    printf '%s' "$prompt"
    read answer || { status '输入流已关闭（EOF），安装中止。'; exit 1; }
    REPLY="${answer:-$default}"
}

# 打印 roxid 横幅（迭代34，裁决 Q2 2026-09-11 04:17：所有入口统一最先打印——
# 无参数菜单 / --uninstall 直达 / 环境变量非交互模式均打印；六行大字为用户
# 提供的 ANSI Shadow 风格原文，逐字内嵌不改写；颜色经 tput 失败回退，非 TTY
# 自动降级纯文本）
print_banner() {
    printf '%s\n' "${cyan}██████╗   ██████╗  ██╗  ██╗ ██╗ ██████╗
██╔══██╗ ██╔═══██╗ ╚██╗██╔╝ ██║ ██╔══██╗
██████╔╝ ██║   ██║  ╚███╔╝  ██║ ██║  ██║
██╔══██╗ ██║   ██║  ██╔██╗  ██║ ██║  ██║
██║  ██╗ ╚██████╔╝ ██╔╝ ██╗ ██║ ██████╔╝
╚═╝  ╚═╝  ╚═════╝  ╚═╝  ╚═╝ ╚═╝ ╚═════╝${plain}"
    printf '%s\n' "${cyan}roxid 安装/卸载脚本${plain}"
}

# 阶段分隔线（迭代34，裁决 Q1 2026-09-11 04:16：全面体验优化——统一阶段视觉
# 边界；36 个横线与横幅视觉宽度协调）
separator() { printf '%s\n' "${cyan}────────────────────────────────────${plain}"; }

print_banner
separator

###########################################
# 平台与架构检查（仅 Linux，对齐官方骨架）
###########################################

[ "$(uname -s)" = "Linux" ] || error '本脚本仅支持 Linux。'

ARCH=$(uname -m)
case "$ARCH" in
    x86_64) ARCH="amd64" ;;
    aarch64|arm64) ARCH="arm64" ;;
    *) error "不支持的架构: $ARCH" ;;
esac

# WSL 检测仅用于 systemd 不可用时的提示语（GPU 相关逻辑已剥离）
IS_WSL2=false
KERN=$(uname -r)
case "$KERN" in
    *icrosoft*WSL2 | *icrosoft*wsl2) IS_WSL2=true;;
    *icrosoft) error "Microsoft WSL1 不受支持。请使用 WSL2：wsl --set-version <distro> 2" ;;
    *) ;;
esac

# 环境信息展示（迭代34，裁决 Q1 2026-09-11 04:16）：只读探测无副作用；systemd
# 两级状态探测一次、两处使用——本块展示与后期服务配置分支（running|degraded
# 判定，迭代26 修正语义保持不变）复用同一前置结果，省重复调用；函数内赋值为
# 全局变量（POSIX sh 无 local 语义差异，此处有意泄漏供后期复用）
print_env_info() {
    status "系统环境:"
    echo "  - 系统: $(uname -s) $KERN"
    echo "  - 架构: $ARCH"
    if [ "$IS_WSL2" = true ]; then
        echo "  - 运行环境: WSL2"
    fi
    if available systemctl; then
        SYSTEMCTL_SYSTEM_RUNNING="$(systemctl is-system-running 2>/dev/null || true)"
        echo "  - systemd 系统实例: $SYSTEMCTL_SYSTEM_RUNNING"
    else
        SYSTEMCTL_SYSTEM_RUNNING=''
        echo "  - systemd 系统实例: 未安装 systemctl"
    fi
    SYSTEMCTL_USER_RUNNING="$(systemctl --user is-system-running 2>/dev/null || true)"
    if [ -n "$SYSTEMCTL_USER_RUNNING" ]; then
        echo "  - systemd 用户实例: $SYSTEMCTL_USER_RUNNING"
    else
        echo "  - systemd 用户实例: 不可用"
    fi
}

print_env_info
separator

###########################################
# 卸载流程（迭代22）：满卸 roxid 全部落痕
# 清理顺序：运行实例 → systemd 两级服务 → shell 补全 → 数据目录 → 二进制
# 安全约定：补全与数据清理一律默认 N 保留，仅用户明确确认才删除
# （用户裁决 Q2/Q3 2026-09-10 01:57–01:58）
###########################################

uninstall_flow() {
    # 1. 运行实例终止（默认 Y：卸载场景二进制可能正被占用；选 N 报错中止）
    if available pgrep && pgrep -x roxid >/dev/null 2>&1; then
        warning "检测到 roxid 正在运行。"
        ask_with_default "终止运行中的 roxid 后继续卸载? [Y/n]: " "Y"
        case "$REPLY" in
            n|N|no)
                error "已取消卸载。请先手动停止 roxid（roxid 停止或 kill），再重新运行本脚本。"
                ;;
            *)
                pkill -x roxid 2>/dev/null || true
                sleep 2
                ;;
        esac
    fi

    # 2. systemd 系统实例满卸（unit 存在才动作；无 root/sudo 降级提示，不阻断其余清理）
    if [ -f /etc/systemd/system/roxid.service ]; then
        status "发现系统实例服务 /etc/systemd/system/roxid.service。"
        UNINSTALL_SUDO=
        if [ "$(id -u)" -ne 0 ]; then
            if available sudo; then
                UNINSTALL_SUDO="sudo"
            else
                warning "无 root/sudo 权限，跳过系统实例服务清理。请手动执行:
  sudo systemctl disable --now roxid
  sudo rm /etc/systemd/system/roxid.service
  sudo systemctl daemon-reload"
            fi
        fi
        if [ "$(id -u)" -eq 0 ] || [ -n "$UNINSTALL_SUDO" ]; then
            $UNINSTALL_SUDO systemctl disable --now roxid 2>/dev/null || true
            $UNINSTALL_SUDO rm -f /etc/systemd/system/roxid.service
            $UNINSTALL_SUDO systemctl daemon-reload 2>/dev/null || true
            status "系统实例服务已停用并删除。"
        fi
    fi

    # 3. systemd 用户实例满卸（全程无 root；--user 停用失败容忍——unit 文件仍删除）
    USER_UNIT_PATH="$HOME/.config/systemd/user/roxid.service"
    if [ -f "$USER_UNIT_PATH" ]; then
        status "发现用户实例服务 $USER_UNIT_PATH。"
        systemctl --user disable --now roxid 2>/dev/null \
            || warning "systemctl --user 停用失败（可能在非登录会话运行）。unit 文件仍将删除。"
        rm -f "$USER_UNIT_PATH"
        systemctl --user daemon-reload 2>/dev/null || true
        status "用户实例服务已删除。"

        # linger 满卸附问（默认 N：服务已删，驻留开关留着无实际影响）
        # 检测双法：loginctl 属性优先，回退 linger 标记文件存在性
        UNINSTALL_USER=$(whoami 2>/dev/null || printf '%s' "${USER:-}")
        LINGER_ON=false
        if available loginctl; then
            if loginctl show-user "$UNINSTALL_USER" --property=Linger 2>/dev/null \
                | grep -q '^Linger=yes'; then
                LINGER_ON=true
            fi
        fi
        if [ "$LINGER_ON" = false ] && [ -e "/var/lib/systemd/linger/$UNINSTALL_USER" ]; then
            LINGER_ON=true
        fi
        if [ "$LINGER_ON" = true ]; then
            ask_with_default "检测到 linger 已开启，关闭登录驻留（loginctl disable-linger）? [y/N]: " "N"
            case "$REPLY" in
                y|Y|yes)
                    if loginctl disable-linger 2>/dev/null; then
                        status "linger 已关闭。"
                    else
                        warning "disable-linger 执行失败，请手动执行: loginctl disable-linger"
                    fi
                    ;;
                *)
                    status "保留 linger 开关。"
                    ;;
            esac
        fi
    fi

    # 4. shell 补全满理（三处落位合并一次询问，默认 N）
    #    zsh 落位含 .zshrc 激活块（三行标记块，格式对齐 completion.rs 的
    #    ZSHRC_ACTIVATE_BLOCK；标记行全文件唯一，幂等判定同源）
    ZSH_FUNC_PATH="$HOME/.zfunc/_roxid"
    ZSHRC_PATH="$HOME/.zshrc"
    ZSHRC_HAS_BLOCK=false
    if [ -f "$ZSHRC_PATH" ] && grep -q '^# roxid completion$' "$ZSHRC_PATH" 2>/dev/null; then
        ZSHRC_HAS_BLOCK=true
    fi
    COMPLETION_COUNT=0
    COMPLETION_LIST=''
    if [ -f "$HOME/.local/share/bash-completion/completions/roxid" ]; then
        COMPLETION_COUNT=$((COMPLETION_COUNT + 1))
        COMPLETION_LIST="$COMPLETION_LIST  - bash: $HOME/.local/share/bash-completion/completions/roxid
"
    fi
    if [ -f "$ZSH_FUNC_PATH" ] || [ "$ZSHRC_HAS_BLOCK" = true ]; then
        COMPLETION_COUNT=$((COMPLETION_COUNT + 1))
        if [ -f "$ZSH_FUNC_PATH" ]; then
            COMPLETION_LIST="$COMPLETION_LIST  - zsh: $ZSH_FUNC_PATH
"
        fi
        if [ "$ZSHRC_HAS_BLOCK" = true ]; then
            COMPLETION_LIST="$COMPLETION_LIST  - zsh: $ZSHRC_PATH 激活块（三行）
"
        fi
    fi
    if [ -f "$HOME/.config/fish/completions/roxid.fish" ]; then
        COMPLETION_COUNT=$((COMPLETION_COUNT + 1))
        COMPLETION_LIST="$COMPLETION_LIST  - fish: $HOME/.config/fish/completions/roxid.fish
"
    fi

    if [ "$COMPLETION_COUNT" -gt 0 ]; then
        status "检测到以下 shell 补全落位:"
        printf '%s' "$COMPLETION_LIST"
        ask_with_default "删除以上 shell 补全? [y/N]: " "N"
        case "$REPLY" in
            y|Y|yes)
                rm -f "$HOME/.local/share/bash-completion/completions/roxid"
                rm -f "$HOME/.config/fish/completions/roxid.fish"
                rm -f "$ZSH_FUNC_PATH"
                if [ "$ZSHRC_HAS_BLOCK" = true ]; then
                    # 满理 .zshrc：sed 范围删除标记块（端点 = 标记行 → autoload 行，
                    # 恰好覆盖 roxid 写入的三行连续块，块外内容零触碰）
                    ZSHRC_CLEAN="$TEMP_DIR/zshrc.clean"
                    if sed '\|^# roxid completion$|,\|^autoload -Uz compinit && compinit$|d' \
                        "$ZSHRC_PATH" > "$ZSHRC_CLEAN" 2>/dev/null; then
                        if [ -s "$ZSHRC_CLEAN" ]; then
                            # cp 覆盖内容保留原文件属主与权限（mv 会带入 TEMP_DIR 属性）
                            cp "$ZSHRC_CLEAN" "$ZSHRC_PATH"
                        else
                            # .zshrc 仅含 roxid 激活块（roxid 创建）→ 一并移除
                            rm -f "$ZSHRC_PATH"
                        fi
                        status ".zshrc 激活块已满理。"
                    else
                        warning ".zshrc 满理失败，请手动删除以下三行:
  # roxid completion
  fpath=(~/.zfunc \$fpath)
  autoload -Uz compinit && compinit"
                    fi
                fi
                status "shell 补全已删除（重开终端生效）。"
                ;;
            *)
                status "保留 shell 补全。"
                ;;
        esac
    fi

    # 5. 数据目录逐项清理（体积展示 → 逐项询问，默认 N）
    #    目录取值对齐 roxid 本体 roxid_home() 语义：ROXID_HOME 环境变量优先
    DATA_DIR="${ROXID_HOME:-$HOME/.roxid}"
    if [ -d "$DATA_DIR" ]; then
        status "数据目录 $DATA_DIR 各子项体积:"
        if available du; then
            du -sh "$DATA_DIR/models" "$DATA_DIR/llama.cpp" 2>/dev/null || true
            if [ -f "$DATA_DIR/config.toml" ]; then
                du -sh "$DATA_DIR/config.toml" 2>/dev/null || true
            fi
            if [ -f "$DATA_DIR/auth.json" ]; then
                du -sh "$DATA_DIR/auth.json" 2>/dev/null || true
            fi
        else
            ls -la "$DATA_DIR"
        fi

        ask_with_default "删除模型目录 $DATA_DIR/models（含全部模型文件）? [y/N]: " "N"
        case "$REPLY" in
            y|Y|yes) rm -rf "$DATA_DIR/models"; status "已删除 models。" ;;
            *) status "保留 models。" ;;
        esac
        ask_with_default "删除 llama.cpp 运行时缓存 $DATA_DIR/llama.cpp? [y/N]: " "N"
        case "$REPLY" in
            y|Y|yes) rm -rf "$DATA_DIR/llama.cpp"; status "已删除 llama.cpp。" ;;
            *) status "保留 llama.cpp。" ;;
        esac
        if [ -f "$DATA_DIR/config.toml" ]; then
            ask_with_default "删除配置文件 $DATA_DIR/config.toml? [y/N]: " "N"
            case "$REPLY" in
                y|Y|yes) rm -f "$DATA_DIR/config.toml"; status "已删除 config.toml。" ;;
                *) status "保留 config.toml。" ;;
            esac
        fi
        if [ -f "$DATA_DIR/auth.json" ]; then
            ask_with_default "删除登录凭据 $DATA_DIR/auth.json? [y/N]: " "N"
            case "$REPLY" in
                y|Y|yes) rm -f "$DATA_DIR/auth.json"; status "已删除 auth.json。" ;;
                *) status "保留 auth.json。" ;;
            esac
        fi
        # 询问外内容全无且目录为空 → 一并移除空目录（rmdir 仅能删空目录，天然安全）
        if [ -z "$(ls -A "$DATA_DIR" 2>/dev/null)" ]; then
            rmdir "$DATA_DIR" 2>/dev/null && status "数据目录已空，一并移除: $DATA_DIR" || true
        fi
    else
        status "未发现数据目录 $DATA_DIR，跳过数据清理。"
    fi

    # 6. 二进制两处落位检测删除（可能并存，逐处处理；系统侧无权限降级提示）
    BIN_REMOVED=0
    if [ -f /usr/local/bin/roxid ]; then
        status "发现系统全局二进制 /usr/local/bin/roxid。"
        UNINSTALL_SUDO=
        if [ "$(id -u)" -eq 0 ]; then
            UNINSTALL_SUDO=
        elif available sudo; then
            UNINSTALL_SUDO="sudo"
        fi
        if [ "$(id -u)" -eq 0 ] || [ -n "$UNINSTALL_SUDO" ]; then
            $UNINSTALL_SUDO rm -f /usr/local/bin/roxid
            status "已删除 /usr/local/bin/roxid。"
            BIN_REMOVED=1
        else
            warning "无 root/sudo 权限，跳过系统全局二进制删除。请手动执行: sudo rm /usr/local/bin/roxid"
        fi
    fi
    if [ -f "$HOME/.local/bin/roxid" ]; then
        rm -f "$HOME/.local/bin/roxid"
        status "已删除 $HOME/.local/bin/roxid。"
        BIN_REMOVED=1
    fi
    if [ "$BIN_REMOVED" -eq 0 ]; then
        warning "未发现 roxid 二进制（/usr/local/bin/roxid 与 $HOME/.local/bin/roxid 均不存在）。"
    fi
}

# 卸载收尾提示（与安装侧 install_success 对称；不经 trap——卸载在安装
# trap install_success EXIT 设置点之前分流退出，直接调用即可）
# 迭代34：分隔线包围的成功区块（呈现升级，卸载逻辑零变化）
uninstall_success() {
    separator
    status '✓ roxid 卸载完成。'
    status "复核建议: pgrep -x roxid（应无输出）与 ls ${ROXID_HOME:-$HOME/.roxid}（应不存在或仅剩保留项）。"
    separator
}

###########################################
# 入口分流（迭代22）：--uninstall/-u 直达卸载；无参数交互菜单先问
# 「安装/卸载」（回车默认安装，等价迭代21 无参数行为）；未知参数 error
# （用户裁决 Q1 2026-09-10 01:57：菜单与参数两种形态组合）
###########################################

MODE=install
case "${1:-}" in
    --uninstall|-u) MODE=uninstall ;;
    "") ;;
    *) error "未知参数: $1（可选: --uninstall 卸载；无参数进入交互菜单）" ;;
esac

if [ "$MODE" = "install" ] && [ -z "${1:-}" ]; then
    while true; do
        ask_with_default "执行操作 [1]安装 roxid  [2]卸载 roxid（默认: 1）: " "1"
        case "$REPLY" in
            1|install|i) break ;;
            2|uninstall|u) MODE=uninstall; break ;;
            *) warning "无效输入: $REPLY（可选 1/2/install/uninstall）" ;;
        esac
    done
fi

if [ "$MODE" = "uninstall" ]; then
    uninstall_flow
    uninstall_success
    exit 0
fi

###########################################
# 交互问句 1：安装范围（环境变量 ROXID_INSTALL_SCOPE 可跳过）
# 默认值随上下文：root 运行默认系统全局，非 root 默认用户级
###########################################

if [ -n "${ROXID_INSTALL_SCOPE:-}" ]; then
    case "$ROXID_INSTALL_SCOPE" in
        system) SCOPE=system ;;
        user) SCOPE=user ;;
        *) error "ROXID_INSTALL_SCOPE 无效值: $ROXID_INSTALL_SCOPE（可选 user/system）" ;;
    esac
    status "安装范围（来自环境变量）: $SCOPE"
else
    if [ "$(id -u)" -eq 0 ]; then
        SCOPE_DEFAULT_INPUT="1"
        SCOPE_DEFAULT_DESC="系统全局"
    else
        SCOPE_DEFAULT_INPUT="2"
        SCOPE_DEFAULT_DESC="用户级"
    fi
    while true; do
        ask_with_default "安装范围 [1]系统全局 /usr/local/bin  [2]用户级 ~/.local/bin（默认: $SCOPE_DEFAULT_DESC）: " "$SCOPE_DEFAULT_INPUT"
        case "$REPLY" in
            1|system|s) SCOPE=system; break ;;
            2|user|u) SCOPE=user; break ;;
            *) warning "无效输入: $REPLY（可选 1/2/system/user）" ;;
        esac
    done
fi

if [ "$SCOPE" = "system" ]; then
    BINDIR="/usr/local/bin"
    SUDO=
    if [ "$(id -u)" -ne 0 ]; then
        available sudo || error "系统全局安装需要 root 权限。请以 root 运行，或安装 sudo 后重试。"
        SUDO="sudo"
    fi
else
    BINDIR="$HOME/.local/bin"
    SUDO=
fi

###########################################
# 交互问句 2：二进制来源（环境变量 ROXID_INSTALL_SOURCE 可跳过）
###########################################

if [ -n "${ROXID_INSTALL_SOURCE:-}" ]; then
    case "$ROXID_INSTALL_SOURCE" in
        local) SOURCE=local ;;
        remote) SOURCE=remote ;;
        *) error "ROXID_INSTALL_SOURCE 无效值: $ROXID_INSTALL_SOURCE（可选 local/remote）" ;;
    esac
    status "二进制来源（来自环境变量）: $SOURCE"
else
    while true; do
        ask_with_default "二进制来源 [1]本地构建产物  [2]远程下载（默认: 1）: " "1"
        case "$REPLY" in
            1|local|l) SOURCE=local; break ;;
            2|remote|r) SOURCE=remote; break ;;
            *) warning "无效输入: $REPLY（可选 1/2/local/remote）" ;;
        esac
    done
fi

# 定位脚本自身目录：本地构建产物的默认路径基准（脚本可从任意 cwd 运行）
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

# 本地构建产物路径解析（仅来源=local 时；环境变量 ROXID_LOCAL_BIN 可跳过路径问句）
if [ "$SOURCE" = "local" ]; then
    if [ -n "${ROXID_LOCAL_BIN:-}" ]; then
        BIN_SRC="$ROXID_LOCAL_BIN"
        [ -f "$BIN_SRC" ] || error "ROXID_LOCAL_BIN 指向的文件不存在: $BIN_SRC"
    else
        ask_with_default "本地 roxid 路径（回车使用默认）: " "$SCRIPT_DIR/target/release/roxid"
        BIN_SRC="$REPLY"
        [ -f "$BIN_SRC" ] || error "未找到 roxid 二进制: $BIN_SRC
  请先执行 cargo build --release，或输入正确的文件路径。"
    fi
fi

###########################################
# 远程下载分支（D3/D4：直链优先 > 占位基地址；tar/裸二进制形态判定）
###########################################

# 解析最终下载 URL 并输出；无法解析（两处地址皆空）时返回非 0
# 迭代25：直链分支原样返回不拼代理（Q2 裁决 2026-09-10 03:18）；Releases 分支前置
# 拼接 GH_PROXY（Q1 裁决 03:17，语义同软件本体 download.rs asset_url 的直接字符串
# 拼接；GH_PROXY 为空即直连，与现状一致）
resolve_remote_url() {
    if [ -n "${ROXID_DOWNLOAD_URL:-}" ]; then
        printf '%s\n' "$ROXID_DOWNLOAD_URL"
        return 0
    fi
    if [ -z "$ROXID_RELEASE_BASE" ]; then
        return 1
    fi
    ROXID_VER="${ROXID_VERSION:-latest}"
    if [ "$ROXID_VER" = "latest" ]; then
        printf '%s\n' "${GH_PROXY}${ROXID_RELEASE_BASE}/latest/download/roxid-linux-$ARCH.tar.gz"
    else
        printf '%s\n' "${GH_PROXY}${ROXID_RELEASE_BASE}/download/$ROXID_VER/roxid-linux-$ARCH.tar.gz"
    fi
}

if [ "$SOURCE" = "remote" ]; then
    # GitHub 代理前缀解析（迭代25，裁决 Q1 2026-09-10 03:17）：
    #   ROXID_GH_PROXY 非空即用且跳过问句；未设置时交互询问，回车默认空=直连；
    #   ROXID_DOWNLOAD_URL 直链已设时代理无作用（Q2：直链不拼），跳过问句不打扰；
    #   语义同软件本体 gh_proxy_prefix()：前缀拼接在 GitHub Releases URL 之前，
    #   脚本不写死任何默认代理网址（对齐本体 Q11 裁决精神 2026-08-24 19:04）
    GH_PROXY=''
    if [ -z "${ROXID_DOWNLOAD_URL:-}" ]; then
        if [ -n "${ROXID_GH_PROXY:-}" ]; then
            GH_PROXY="$ROXID_GH_PROXY"
        else
            ask_with_default "GitHub 代理前缀（回车直连）: " ""
            GH_PROXY="$REPLY"
        fi
        # 尾斜杠卫生（D3）：前缀非空且不以 / 结尾时补齐，防手敲漏斜杠产生坏地址
        case "$GH_PROXY" in
            ''|*/) ;;
            *) GH_PROXY="$GH_PROXY/" ;;
        esac
    fi

    # tar.zst 资产需要额外工具，按形态按需声明
    REMOTE_NEEDS="curl tar"
    if ! DOWNLOAD_URL=$(resolve_remote_url); then
        error "远程下载地址未配置。两种配置方法：
  1. 运行前设置环境变量: export ROXID_DOWNLOAD_URL=<tarball 或二进制完整直链>
  2. 编辑本脚本顶部 ROXID_RELEASE_BASE 常量（上传发布仓库后填入 releases 基地址）"
    fi

    case "$DOWNLOAD_URL" in
        *.tar.zst) REMOTE_NEEDS="curl tar zstd" ;;
    esac

    NEEDS=$(require $REMOTE_NEEDS)
    if [ -n "$NEEDS" ]; then
        status "错误: 以下工具缺失，无法远程下载:"
        for NEED in $NEEDS; do
            echo "  - $NEED"
        done
        exit 1
    fi

    # D5：代理生效时两行展示（前缀 + 最终拼接地址），供用户目视确认
    if [ -n "$GH_PROXY" ]; then
        status "GitHub 代理前缀: $GH_PROXY"
    fi
    status "下载地址: $DOWNLOAD_URL"
    mkdir -p "$TEMP_DIR/unpack"
    # 形态判定：压缩包先下载到 TEMP_DIR 再本地解压取包内 roxid；其余视为裸二进制
    case "$DOWNLOAD_URL" in
        *.tar.gz|*.tgz)
            curl --fail --show-error --location --progress-bar \
                -o "$TEMP_DIR/roxid-pkg.tgz" "$DOWNLOAD_URL"
            tar -xzf "$TEMP_DIR/roxid-pkg.tgz" -C "$TEMP_DIR/unpack"
            BIN_SRC=$(find "$TEMP_DIR/unpack" -type f -name roxid | head -n 1)
            [ -n "$BIN_SRC" ] || error "压缩包内未找到 roxid 二进制。"
            ;;
        *.tar.zst)
            curl --fail --show-error --location --progress-bar \
                -o "$TEMP_DIR/roxid-pkg.tar.zst" "$DOWNLOAD_URL"
            zstd -d -c "$TEMP_DIR/roxid-pkg.tar.zst" | tar -xf - -C "$TEMP_DIR/unpack"
            BIN_SRC=$(find "$TEMP_DIR/unpack" -type f -name roxid | head -n 1)
            [ -n "$BIN_SRC" ] || error "压缩包内未找到 roxid 二进制。"
            ;;
        *)
            curl --fail --show-error --location --progress-bar \
                -o "$TEMP_DIR/roxid" "$DOWNLOAD_URL"
            BIN_SRC="$TEMP_DIR/roxid"
            ;;
    esac
fi

###########################################
# 覆盖安装保护（R4）：roxid 正在运行时询问终止
###########################################

if available pgrep && pgrep -x roxid >/dev/null 2>&1; then
    warning "检测到 roxid 正在运行。"
    ask_with_default "终止运行中的 roxid 后继续安装? [Y/n]: " "Y"
    case "$REPLY" in
        n|N|no)
            error "已取消安装。请先手动停止 roxid（roxid 停止或 kill），再重新运行本脚本。"
            ;;
        *)
            pkill -x roxid 2>/dev/null || true
            sleep 2
            ;;
    esac
fi

separator

###########################################
# 落位（install 天然覆盖旧版本；升级 = 重新运行本脚本）
###########################################

status "安装 roxid 到 $BINDIR ..."
if [ "$SCOPE" = "system" ]; then
    $SUDO install -o0 -g0 -m755 -d "$BINDIR"
    $SUDO install -o0 -g0 -m755 "$BIN_SRC" "$BINDIR/roxid"
else
    mkdir -p "$BINDIR"
    install -m755 "$BIN_SRC" "$BINDIR/roxid"
fi

[ -x "$BINDIR/roxid" ] || error "落位校验失败: $BINDIR/roxid 不可执行"

###########################################
# PATH 指引（仅用户级且 ~/.local/bin 不在 PATH 时）
###########################################

if [ "$SCOPE" = "user" ]; then
    case ":$PATH:" in
        *":$BINDIR:"*) ;;
        *)
            status "注意: $BINDIR 不在当前 PATH 中。"
            status "请将 export PATH=\"\$HOME/.local/bin:\$PATH\" 加入 ~/.profile（或 ~/.bashrc）后重新登录。"
            ;;
    esac
fi

# 服务启动标志（迭代26 缺陷2 修正 2026-09-10 03-55）：仅当 systemd 服务实际执行
# enable --now 成功才置 1（set -eu 下失败即中止到不了置位行）；install_success 据
# 此分支——未启动（探测跳过/用户拒绝创建/无 systemctl）时不再虚假宣称 API 已可用
SERVICE_STARTED=0

install_success() {
    separator
    if [ "$SERVICE_STARTED" -eq 1 ]; then
        status "✓ roxid 服务已启动。"
        status 'The roxid API is now available at 127.0.0.1:11434.'
    else
        status '未配置 roxid 自启动服务。手动启动方式：'
        status "  前台运行: $BINDIR/roxid serve"
        status '  或重新运行本脚本，在 systemd 服务询问处选择 y 创建并启动'
        status '服务启动后 API 监听 127.0.0.1:11434。'
    fi
    status "安装完成。可执行文件: $BINDIR/roxid"
    status '首次运行 roxid 将进入 setup 引导（llama.cpp 后端下载源配置）。'
    separator
}
trap install_success EXIT

###########################################
# systemd 服务（两级可选，默认 N；确认创建才 enable --now）
###########################################

# 系统实例服务以"安装发起用户"身份运行（roxid 数据目录绑定 ~/.roxid/，须与交互 CLI 一致）
# root（sudo）执行时回退到 SUDO_USER 原用户，避免服务落在 /root/.roxid
resolve_system_service_user() {
    if [ "$(id -u)" -eq 0 ]; then
        SERVICE_USER="${SUDO_USER:-root}"
    else
        SERVICE_USER=$(whoami)
    fi
    # 用户 home：优先 getent passwd；getent 缺失或查无时回退拼 /home/<user>
    SERVICE_HOME=""
    if available getent; then
        SERVICE_HOME=$(getent passwd "$SERVICE_USER" | cut -d: -f6)
    fi
    [ -n "$SERVICE_HOME" ] || SERVICE_HOME="/home/$SERVICE_USER"
    SERVICE_GROUP=$(id -gn "$SERVICE_USER" 2>/dev/null || printf '%s' "$SERVICE_USER")
}

configure_systemd_system() {
    resolve_system_service_user
    ask_with_default "创建系统实例服务 /etc/systemd/system/roxid.service 并启动? [y/N]: " "N"
    case "$REPLY" in
        y|Y|yes) ;;
        *) status "跳过系统实例服务创建。"; return 0 ;;
    esac

    status "创建系统实例服务（运行用户: $SERVICE_USER，数据目录: $SERVICE_HOME/.roxid）..."
    cat <<EOF | $SUDO tee /etc/systemd/system/roxid.service >/dev/null
[Unit]
Description=roxid Service
After=network-online.target

[Service]
ExecStart=$BINDIR/roxid serve
User=$SERVICE_USER
Group=$SERVICE_GROUP
Restart=always
RestartSec=3
Environment="PATH=$PATH"
Environment="HOME=$SERVICE_HOME"
WorkingDirectory=$SERVICE_HOME

[Install]
WantedBy=default.target
EOF
    $SUDO systemctl daemon-reload
    $SUDO systemctl enable --now roxid
    SERVICE_STARTED=1   # enable --now 成功后置位（迭代26 缺陷2，2026-09-10 03-55）
    status "系统实例服务已创建并启动。"
}

configure_systemd_user() {
    ask_with_default "创建用户实例服务 ~/.config/systemd/user/roxid.service 并启动? [y/N]: " "N"
    case "$REPLY" in
        y|Y|yes) ;;
        *) status "跳过用户实例服务创建。"; return 0 ;;
    esac

    status "创建用户实例服务（systemctl --user，无需 root）..."
    USER_UNIT_DIR="$HOME/.config/systemd/user"
    mkdir -p "$USER_UNIT_DIR"
    cat > "$USER_UNIT_DIR/roxid.service" <<EOF
[Unit]
Description=roxid Service
After=network-online.target

[Service]
ExecStart=$BINDIR/roxid serve
Restart=always
RestartSec=3
Environment="PATH=$PATH"

[Install]
WantedBy=default.target
EOF
    systemctl --user daemon-reload
    systemctl --user enable --now roxid
    SERVICE_STARTED=1   # enable --now 成功后置位（迭代26 缺陷2，2026-09-10 03-55）
    status "用户实例服务已创建并启动。"

    # linger：开启后服务开机即运行（无需登录）；默认 N（不开启则随登录会话启停）
    ask_with_default "开启 loginctl enable-linger 使服务无需登录持久运行? [y/N]: " "N"
    case "$REPLY" in
        y|Y|yes)
            if ! loginctl enable-linger 2>/dev/null; then
                warning "enable-linger 执行失败（部分环境需管理员授权）。服务将随登录会话启停。"
            else
                status "linger 已开启，服务将在开机后自动运行。"
            fi
            ;;
        *)
            status "未开启 linger。服务将在用户登录后运行，登出全部会话后停止。"
            ;;
    esac
}

if [ "$SCOPE" = "system" ]; then
    if available systemctl; then
        # 迭代34：复用环境信息块前置探测结果（print_env_info 已赋值），
        # running|degraded 判定语义与迭代26 保持不变，省重复调用
        case "$SYSTEMCTL_SYSTEM_RUNNING" in
            running|degraded)
                configure_systemd_system
                ;;
            *)
                warning "systemd 未在运行，跳过系统实例服务配置。"
                ;;
        esac
    else
        warning "未找到 systemctl，跳过系统实例服务配置。"
    fi
else
    # 用户实例探测（迭代26 缺陷1 修正 2026-09-10 03-55）：按 is-system-running
    # 输出状态匹配 running|degraded——degraded 仅表示存在无关失败 unit，用户总线
    # 仍可用；原实现仅凭退出码（degraded 退出码为 1）误判不可用。
    # 迭代34：探测前移至环境信息块（print_env_info，stderr 静默 + || true 护栏
    # 兼容 set -eu），此处复用结果，判定语义不变
    case "$SYSTEMCTL_USER_RUNNING" in
        running|degraded)
            configure_systemd_user
            ;;
        *)
            warning "systemd 用户实例不可用，跳过用户实例服务配置。"
            if [ "$IS_WSL2" = true ]; then
                warning "参见 https://learn.microsoft.com/en-us/windows/wsl/systemd#how-to-enable-systemd 启用 WSL2 systemd"
            fi
            ;;
    esac
fi

# 提示：本行之后逻辑已全部完成，EXIT trap 输出 install_success 收尾

}

main "$@"
