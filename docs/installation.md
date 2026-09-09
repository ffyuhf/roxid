# Installation & Uninstall

English | [中文](zh/installation.md)

---

## Prerequisites

| Item | Requirement |
|---|---|
| OS | Linux (amd64 / arm64); WSL1 is not supported (WSL2 works) |
| Binary form | musl static single binary — no glibc / shared-library dependencies |
| Installer dependencies | POSIX sh + coreutils (`install`, `mkdir`); remote downloads additionally need `curl` and `tar` (`zstd` for `.tar.zst` assets) |
| Build from source | Rust stable toolchain (`cargo build --release`) |
| GPU drivers | roxid does not install drivers — GPU dispatch relies on llama.cpp `--fit`; drivers are the user's responsibility. Official prebuilt Linux backends use Vulkan; for native CUDA set `ROXID_LLAMA_SERVER` to your own build |

> The installer's interactive prompts are written in Chinese.

## Option 1: Install script (recommended)

### Get the script

A standalone script asset is attached to each Release (available once the first Release is published):

```sh
curl -fsSL -o roxid_install.sh \
  https://github.com/ffyuhf/roxid/releases/latest/download/roxid_install.sh
sh roxid_install.sh
```

You can also use [`roxid_install.sh`](../roxid_install.sh) from the repository root.

### Interactive flow

```text
执行操作 [1]安装 roxid  [2]卸载 roxid（默认: 1）:           ← Enter = install
安装范围 [1]系统全局 /usr/local/bin  [2]用户级 ~/.local/bin: ← default 1 as root, 2 otherwise
二进制来源 [1]本地构建产物  [2]远程下载（默认: 1）:          ← local defaults to target/release/roxid
本地 roxid 路径（回车使用默认）:                            ← asked only when source = local
GitHub 代理前缀（回车直连）:                                ← asked only when source = remote and neither
                                                            ROXID_GH_PROXY nor ROXID_DOWNLOAD_URL is set
```

- **Install scope**: system-wide lands in `/usr/local/bin` (sudo added automatically for non-root); user-level lands in `~/.local/bin` (no sudo at all).
- **Binary source**: local build artifact or remote download. Remote URL resolution order: full direct link in `ROXID_DOWNLOAD_URL` (highest priority, never proxied) > optional GitHub proxy prefix + built-in release base + `ROXID_VERSION` (default `latest`), asset name `roxid-linux-<arch>.tar.gz`; URLs ending in `.tar.gz`/`.tgz`/`.tar.zst` are unpacked first, anything else is treated as a raw binary. The proxy prefix uses the same prefix-concatenation semantics as the roxid runtime `ROXID_GH_PROXY` (see [Configuration](configuration.md)); a missing trailing `/` is appended automatically. The built-in release base itself can also be overridden via the `ROXID_RELEASE_BASE` environment variable (self-hosted mirror scenarios).
- **Overwrite protection**: if roxid is running, the script asks whether to terminate it (default Y).
- **After placement**: `install -m755` overwrites, then verifies executability; if `~/.local/bin` is not on PATH, guidance is printed.

### systemd services (optional, not created by default)

| Level | Condition | Unit path | Notes |
|---|---|---|---|
| System instance | system-wide install + systemd running | `/etc/systemd/system/roxid.service` | Runs as the **installing user** (falls back to `SUDO_USER` when run via sudo, so the data dir stays on that user's `~/.roxid`); needs sudo; `Restart=always` |
| User instance | user-level install + `systemctl --user` available | `~/.config/systemd/user/roxid.service` | No root required; starts and stops with the login session; optional `loginctl enable-linger` prompt (default N) for run-without-login |

Both use `ExecStart=<BINDIR>/roxid serve` and only run `enable --now` upon explicit confirmation.

### Non-interactive install (CI / scripting)

Set environment variables to skip the corresponding prompts:

| Variable | Values | Purpose |
|---|---|---|
| `ROXID_INSTALL_SCOPE` | `user` / `system` | Install scope |
| `ROXID_INSTALL_SOURCE` | `local` / `remote` | Binary source |
| `ROXID_LOCAL_BIN` | file path | Local build artifact path (when source = local) |
| `ROXID_DOWNLOAD_URL` | full direct link | Remote download URL (highest priority, never prefixed by the proxy) |
| `ROXID_VERSION` | tag (default `latest`) | Combined with the built-in release base |
| `ROXID_GH_PROXY` | proxy prefix | GitHub proxy prefix prepended to the built-in release base URLs (same semantics as the roxid runtime variable); when set, the proxy prompt is skipped |
| `ROXID_RELEASE_BASE` | release base URL | Overrides the built-in release base (self-hosted mirror) |

## Option 2: Manual binary install

Download the tarball for your architecture from the Release page (contains the `roxid` binary + `roxid_install.sh`):

```sh
tar -xzf roxid-linux-amd64.tar.gz
install -m755 roxid ~/.local/bin/roxid
```

## Option 3: Build from source

```sh
git clone https://github.com/ffyuhf/roxid.git
cd roxid
cargo build --release
# artifact: target/release/roxid
```

The dependency chain is pure Rust + rustls-tls (no openssl); no system C libraries are required.

## Verifying download hashes

Every Release asset ships with a matching `.sha256` file (one for each tarball and for the installer):

```sh
sha256sum -c roxid-linux-amd64.tar.gz.sha256
sha256sum -c roxid_install.sh.sha256
```

> GPG signatures are not provided at this time; the hash files are produced and uploaded by the same workflow as the assets.

## Environment variables and PATH

After a user-level install, if `~/.local/bin` is not on PATH:

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.profile   # or ~/.bashrc
```

Log in again for it to take effect. Verify with `roxid -v`.

## Upgrading

Upgrading = re-run the installer (`install -m755` naturally overwrites the old version):

```sh
sh roxid_install.sh                        # remote source + latest picks the newest
ROXID_VERSION=v0.2.0 sh roxid_install.sh   # pin a version
```

If roxid is running during an overwrite install, the script terminates it before continuing (default Y).

## Uninstall and cleanup

Two entry points: `sh roxid_install.sh --uninstall` (or `-u`) goes straight to uninstall; or choose `2` in the menu when run without arguments.

Full uninstall cleans six categories of traces in order:

| # | Trace | Default | Behavior |
|---|---|---|---|
| 1 | Running instance | Y terminate | Detected via `pgrep -x roxid`; choosing N aborts the uninstall |
| 2 | System service | — | Only if the unit exists: `sudo systemctl disable --now roxid` + remove unit + `daemon-reload`; without privileges, degrades to a warning with manual commands |
| 3 | User service | — | `systemctl --user` stop + remove unit; linger prompt if enabled (default N) |
| 4 | Shell completions | N keep | One combined prompt for all detected placements: `~/.local/share/bash-completion/completions/roxid`, `~/.zfunc/_roxid` (including the three-line `.zshrc` activation block, nothing outside the block is touched), `~/.config/fish/completions/roxid.fish` |
| 5 | Data directory | N keep | Shows sizes of `models` / `llama.cpp` / `config.toml` / `auth.json` via `du -sh` first, then asks per item; honors `ROXID_HOME` (default `~/.roxid`); removes the directory if left empty |
| 6 | Binaries | — | `/usr/local/bin/roxid` and `~/.local/bin/roxid` are removed wherever found (they may coexist) |

Post-uninstall verification:

```sh
pgrep -x roxid     # expect no output
ls ~/.roxid        # expect missing, or only the items you kept
```

### Data directory layout (for manual cleanup reference)

```text
~/.roxid/
├─ config.toml     # setup wizard state and proxy configuration
├─ auth.json       # signin credentials (permission 0600)
├─ models/         # model files (ollama / HF / derived)
└─ llama.cpp/      # backend runtime cache (multi-version + manual/)
```

## Next steps

- [Quickstart](quickstart.md)
- [CLI Reference](cli.md)
- [Configuration](configuration.md)
