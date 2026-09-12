//! Runner 注册表：模型名 → 活跃 llama-server 实例的多模型调度核心（M6）。
//!
//! - acquire：命中即续期复用；未命中则「查仓库 → 备运行时 → 拉实例」加载
//!   （M27 碴2：同模型经 per-model 加载锁串行防双拉起（M6 语义保持）；
//!   跨模型完全并行（R2-A 裁决）——耗时加载在全局写锁外执行，
//!   全局锁仅覆盖注册表条目的瞬时读写）
//! - reaper：后台定时扫描 keep_alive 到期实例并卸载
//! - list_running / stop：/api/ps 与 stop 命令的数据源
//!
//! 修改历史：M6 新增 2026-08-24 19:20；
//! M26 死实例自愈（迭代6 P0碴1）：acquire 快/慢路径拒绝已崩溃实例，
//! 清理回收后重新拉起（兑现变更日志 M3 声称的崩溃重启语义）2026-08-26 06-46
//! M27 在途保护（迭代7 P0碴1）：acquire 发放 RunnerLease 租约（在途计数+1），
//! reaper 只卸载「到期且无在途请求」的实例（R1/R3 裁决 2026-08-26 21:11）
//! 2026-08-26 21-22
//! M27 加载锁重构（迭代7 P0碴2）：per-model 加载锁表，耗时加载移出全局写锁
//! （同模型串行防双拉起、跨模型完全并行，R2-A 裁决 2026-08-26 21:11）
//! 2026-08-26 21-35
//! M28 碴13 换端口重试（迭代8）：拉起失败重取端口重试上限 3 次——兑现
//! scheduler/mod.rs「M6 注册表层做换端口重试」的书面承诺（原实现一次
//! 失败即 502；alloc_port 探测与子进程真实绑定间存在 TOCTOU 竞态）
//! 2026-08-30 06-20
//! M29 碴6（迭代9）：重建路径停机移出全局写锁（原持 runners 写锁
//! kill+wait 进程，与 M27 R2-A「全局锁仅覆盖注册表条目瞬时读写」裁决
//! 矛盾；对齐 reap_expired 锁外回收先例）2026-09-05 12-55
//! M35 D5（迭代15）：RunningModel 增 details（官方 ps 六字段对象，meta
//! 缺失时空对象）与 context_length（runner 每 slot 真实生效值，0/未知
//! 省略键）两字段，list_running 注册点填充 2026-09-07 21-46
//! M54（迭代19 碴B）：acquire 复用判定增加第四键——resolve-only 解析的
//! 应然后端路径与实例记录不一致（runtime use 切换默认版本/env 改指向）
//! 时触发重建；原仅 ctx/RUNTIME 双键，后端版本切换后运行实例无感知
//! 2026-09-09 20-32
//! M99（迭代31）：acquire 变体名拼接宿主架构（arm64 宿主不再误下
//! ubuntu-x64 包）；spawn_with_port_retry 对 ArchMismatch 确定性失败
//! 直接透出不换端口重试（原盲目重试 3 次刷误导日志；用户裁决 Q4
//! 2026-09-10 18:21）2026-09-10 18-30；
//! M169（迭代45 Q4-A 裁决 2026-09-12 01:44）：重建路径停机后台化——
//! 旧实例 kill+wait 与新实例加载并行，模型切换不再等待旧进程退出
//! 与驱动显存回收滞后（用户实测切换过慢根因：串行停机全部计入
//! 新请求等待）2026-09-12 01-55；
//! M170（迭代45 Q5-A 裁决 2026-09-12 01:45）：显存/内存不足判型 +
//! 空闲实例 LRU 自动卸载腾位重试（对齐 ollama 语义；在途实例受
//! M27 保护绝不卸载）2026-09-12 01-55；
//! M193+M194（迭代51 Q1-A/Q2 裁决 2026-09-12 18:03）：服务类别判定
//! 贯通（spawn_spec_from_meta 填 service_class/has_mtp）+ 同类互斥
//! 主动换载——加载新实例前卸载同服务类别的其他空闲实例（同步等退出：
//! 腾显存是加载前置，区别于 M169 后台化），在途/异类零触碰
//! 2026-09-12 18-21

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::error::{RoxidError, RoxidResult};
use crate::repo::{self, ModelMeta, ModelRef};
use crate::runtime::{effective_variant, ensure_llama_server};
use crate::scheduler::{alloc_port, Runner, RunnerLease, SpawnSpec};

/// 默认 keep_alive（与原版 Ollama 一致：5 分钟）
pub const DEFAULT_KEEP_ALIVE: Duration = Duration::from_secs(5 * 60);

/// /api/ps 条目（字段与原版对齐）
#[derive(Debug, Clone, Serialize)]
pub struct RunningModel {
    /// 完整模型名
    pub name: String,
    /// 模型名（不含 tag）
    pub model: String,
    /// 占用字节数（GGUF 大小）
    pub size: u64,
    /// 显存占用（nvidia-smi 进程级查询；无 GPU/查询失败为 0，
    /// 与原版 Ollama CPU 加载语义一致。来源：用户确认 R2-1A 2026-08-24 19:59）
    pub size_vram: u64,
    /// 内容摘要
    pub digest: String,
    /// keep_alive 到期时刻（RFC3339）
    pub expires_at: String,
    /// 模型详情（官方 ps details 六字段对象；meta 缺失时空对象。M35 D5）
    pub details: serde_json::Value,
    /// 每 slot 上下文长度（真实生效值；0/未知省略键。M35 D5）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    /// GPU 层数（stderr 加载日志解析；None=未解析，原因见 gpu_layers_note。
    /// 迭代36 M126，Q3/Q8 裁决 2026-09-11）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_layers: Option<u32>,
    /// 模型总层数（同上来源；与 gpu_layers 成对出现）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_layers: Option<u32>,
    /// 层数解析失败原因（gpu_layers 为 None 时输出；Q3 裁决：
    /// 失败直书原因不降级，来源 2026-09-11 05:52）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_layers_note: Option<String>,
}

/// Runner 注册表
pub struct RunnerRegistry {
    /// model:tag → 活跃实例（外层 RwLock 保护键集合，内层 Mutex 保护单实例状态）
    runners: RwLock<HashMap<String, Arc<Mutex<Runner>>>>,
    /// model:tag → 该模型的加载互斥锁（M27 碴2：同模型串行防双拉起；
    /// 跨模型各持各锁完全并行——锁条目为小对象常驻，同键复用天然幂等）
    loading: RwLock<HashMap<String, Arc<Mutex<()>>>>,
    /// 模型仓库根目录
    models_root: PathBuf,
}

impl RunnerRegistry {
    /// 构造注册表。
    ///
    /// - 参数 models_root：模型仓库根目录（config::models_root()）
    pub fn new(models_root: PathBuf) -> Self {
        Self {
            runners: RwLock::new(HashMap::new()),
            loading: RwLock::new(HashMap::new()),
            models_root,
        }
    }

    /// 获取或拉起模型的 runner：命中即续期；未命中全流程加载。
    ///
    /// - 参数 name：用户输入模型名（可缺省 tag）
    /// - 参数 keep_alive：请求级 keep_alive（Ollama 语义：本次请求后空闲窗口）
    /// - 参数 want_ctx：请求级每 slot 上下文（D4c；None 表示无要求）。
    ///   与实例不一致时按原版语义卸载旧实例并以新窗口重建
    /// - 返回：实例租约（M27 碴1：请求全程持有，Drop 时计数减一并重计窗口；
    ///   端口经 lease.port() 获取）
    pub async fn acquire(
        &self,
        name: &str,
        keep_alive: Duration,
        want_ctx: Option<u32>,
        want_runtime: Option<String>,
    ) -> RoxidResult<RunnerLease> {
        let full = ModelRef::parse(name)?.full_name();

        // 1) 读锁快速路径：命中且 ctx 匹配且实例存活即续期、计入在途并发放租约
        //    （M26 碴1：死实例不放行，落入写锁路径清理重拉；
        //    M39：runtime 匹配同判——请求级 flags 与实例不一致走重建；
        //    M54：后端二进制路径同判——runtime use 切换后不一致走重建）
        {
            let readers = self.runners.read().await;
            if let Some(runner) = readers.get(&full) {
                let mut guard = runner.lock().await;
                let ok = want_ctx.map_or(true, |w| {
                    guard.ctx_per_slot() == 0 || guard.ctx_per_slot() == w
                }) && runtime_matches(
                    want_runtime.as_deref(),
                    guard.runtime_flags(),
                    guard.model_runtime(),
                ) && !guard.is_process_dead()
                    && llama_bin_matches(guard.llama_server_bin());
                if ok {
                    guard.refresh_keep_alive(keep_alive);
                    guard.enter_request();
                    drop(guard);
                    return Ok(RunnerLease::new(runner.clone()));
                }
                // ctx 不匹配或实例已崩溃：走写锁重建（原版语义）
            }
        }

        // 2) 未命中/ctx 不匹配/实例已死：取该模型专属加载锁。
        //    M27（碴2，R2-A 裁决：完全并行）：同模型经此锁串行（防双拉起，M6
        //    语义保持）；跨模型各持各锁互不阻塞——耗时加载不再横跨全局写锁
        let model_lock = {
            let mut guards = self.loading.write().await;
            guards.entry(full.clone()).or_default().clone()
        };
        let _model_guard = model_lock.lock().await;

        // 3) 双重检查：等模型锁期间可能已被并发请求加载/重建
        let mut stale: Option<Arc<Mutex<Runner>>> = None;
        {
            let mut writers = self.runners.write().await;
            if let Some(runner) = writers.get(&full) {
                let mut guard = runner.lock().await;
                let ok = want_ctx.map_or(true, |w| {
                    guard.ctx_per_slot() == 0 || guard.ctx_per_slot() == w
                }) && runtime_matches(
                    want_runtime.as_deref(),
                    guard.runtime_flags(),
                    guard.model_runtime(),
                ) && !guard.is_process_dead()
                    && llama_bin_matches(guard.llama_server_bin());
                if ok {
                    guard.refresh_keep_alive(keep_alive);
                    guard.enter_request();
                    drop(guard);
                    let handle = runner.clone(); // 先克隆句柄再释放写锁（借用顺序）
                    drop(writers);
                    return Ok(RunnerLease::new(handle));
                }
                // ctx 不匹配或实例已崩溃（M26 碴1）：移除出表后走重建
                drop(guard);
                // M29 碴6：写锁内仅 remove 收集句柄——停机（kill+wait 进程
                // 退出等待）移至锁外执行，兑现 R2-A「全局锁仅瞬时读写」；
                // 模型锁仍持有（同模型串行），无并发双重回收
                stale = writers.remove(&full);
            }
        }
        // 锁外停机（M169 迭代45 Q4-A 裁决 2026-09-12 01:44）：后台化——
        // 旧实例 kill+wait 移入 tokio::spawn 与新实例加载完全并行，
        // 重建不再等待旧进程退出与驱动显存回收滞后（原串行停机把该
        // 时长全部计入新请求等待，用户实测模型切换过慢的根因）。
        // 句柄已 remove 出表：stop/并发 acquire 不受影响；shutdown_mut
        // 对已退出进程幂等（M26 前置）；同模型加载锁仍持有，无并发双拉
        if let Some(old) = stale {
            tokio::spawn(async move {
                if let Err(e) = old.lock().await.shutdown_mut().await {
                    tracing::warn!("后台卸载旧实例失败：{e}");
                }
            });
        }

        // 4) 全流程加载（全局锁外：读盘/探测/下载/健康等待不再阻塞其他模型
        //    与 /api/ps、stop 的注册表访问）
        let r = ModelRef::parse(name)?;
        let meta = repo::find_model(&self.models_root, &full)?;
        // 迭代51 M194（Q1-A 裁决 2026-09-12 18:03「同类空闲立即卸载」）：
        // 同类互斥主动换载——加载前卸载同服务类别的其他空闲实例（同步等
        // 退出：腾显存是加载的前置条件，区别于 M169 重建停机后台化）；
        // 在途同类零触碰（M27 保护，其请求结束后按 keep_alive 驻留至下次
        // 同类请求）；异类（embedding/TTS vs 生成）实例零触碰
        let class = super::service_class(&meta.family);
        for victim in self.evict_same_class_idle(&full, class).await {
            let name = victim.lock().await.model_name.clone();
            tracing::info!("同类换载：卸载空闲实例 {name}（服务类别 {class:?}）腾位");
            if let Err(e) = victim.lock().await.shutdown_mut().await {
                tracing::warn!("同类换载卸载实例失败：{e}");
            }
        }
        // M99（迭代31）：变体名含宿主架构片段——未知架构在此报 5xx
        //（文案引导手动链逃生口），arm64 宿主不再误下 ubuntu-x64 包
        // 迭代50 M189：变体来源改 effective_variant——config default_variant
        //（use <tag> <词> 写入的真实目录名）优先，缺省回退探测（零变化）
        let variant = effective_variant()?;
        let bin = ensure_llama_server(&variant).await?;
        let mut spec = spawn_spec_from_meta(&bin, &self.models_root, &r, &meta);
        if let Some(w) = want_ctx {
            spec.ctx_size = w; // D4c：请求级窗口覆盖元数据默认
        }
        // M39：请求级 RUNTIME 完全覆盖持久层（roxid run --runtime 临时语义）
        if let Some(rt) = want_runtime {
            spec.runtime_flags = Some(rt);
        }
        // M28 碴13：拉起经换端口重试包装（失败重取端口，上限 3 次）；
        // M170（迭代45 Q5-A 裁决 2026-09-12 01:45）：重试耗尽且判型为
        // 资源不足时，逐个卸载空闲实例（LRU）腾位后重试——对齐 ollama
        // 显存不足自动卸载语义；空闲实例清空仍失败则透出原错误
        let runner = match spawn_with_port_retry(&full, |port| {
            Runner::spawn_llama_server(full.clone(), port, keep_alive, &spec)
        })
        .await
        {
            Ok(r) => r,
            Err(first) if is_resource_exhausted(&first.to_string()) => {
                let mut last = first;
                let mut spawned = None;
                while let Some(idle) = self.evict_one_idle(&full).await {
                    let name = idle.lock().await.model_name.clone();
                    tracing::info!("显存/内存不足：自动卸载空闲实例 {name} 腾位重试");
                    // 同步等待退出（腾显存必须等驱动回收——与重建停机的
                    // 后台化不同，此处卸载是加载成功的前置条件）
                    if let Err(e) = idle.lock().await.shutdown_mut().await {
                        tracing::warn!("卸载空闲实例失败：{e}");
                    }
                    match spawn_with_port_retry(&full, |port| {
                        Runner::spawn_llama_server(full.clone(), port, keep_alive, &spec)
                    })
                    .await
                    {
                        Ok(r) => {
                            spawned = Some(r);
                            break;
                        }
                        Err(e) => last = e,
                    }
                }
                match spawned {
                    Some(r) => r,
                    None => {
                        tracing::warn!("无空闲实例可卸载，资源不足错误透出：{last}");
                        return Err(last);
                    }
                }
            }
            Err(e) => return Err(e),
        };
        let arc = Arc::new(Mutex::new(runner));
        // 5) 短写锁落表（临界区仅条目插入；模型锁保证同模型不会并发到达此处）
        self.runners.write().await.insert(full, arc.clone());
        arc.lock().await.enter_request();
        Ok(RunnerLease::new(arc))
    }

    /// 列出运行中模型（/api/ps）。
    ///
    /// - 返回：全部活跃实例快照（size_vram 来自单次 nvidia-smi 进程查询）
    pub async fn list_running(&self) -> Vec<RunningModel> {
        // 一次 nvidia-smi 查询全部 GPU 进程显存，多实例共享结果
        let vram = crate::scheduler::gpu_process_vram_map().await;
        let mut out = Vec::new();
        let readers = self.runners.read().await;
        for (full, runner) in readers.iter() {
            let mut guard = runner.lock().await;
            // 崩溃实例不展示（下一次 acquire 会重新加载）
            if guard.is_process_dead() {
                continue;
            }
            let (model, _) = full.split_once(':').unwrap_or((full, ""));
            // M35 D5：details 六字段对象（与 tags/show 同形态；meta 缺失空对象）
            let details = match repo::find_model(&self.models_root, full) {
                Ok(m) => {
                    let families = if m.families.is_empty() {
                        serde_json::json!(null)
                    } else {
                        serde_json::json!(m.families)
                    };
                    serde_json::json!({
                        "parent_model": "",
                        "format": "gguf",
                        "family": m.family,
                        "families": families,
                        "parameter_size": m.parameter_size,
                        "quantization_level": m.quantization_level,
                    })
                }
                Err(_) => serde_json::json!({}),
            };
            // 迭代36 M126：本实例层卸载解析结果（None → note 说明原因）
            let layer_split = guard.gpu_layer_split();
            out.push(RunningModel {
                name: full.clone(),
                model: model.to_string(),
                size: guard.size,
                // PID 命中 GPU 进程映射取真实显存；未命中（CPU 后端/无驱动）为 0
                size_vram: guard.pid().and_then(|p| vram.get(&p).copied()).unwrap_or(0),
                digest: String::new(), // 由调用方按需补充（/api/ps 层查 meta）
                expires_at: rfc3339_after(guard.keep_alive_remaining()),
                details,
                // M35 D5：真实生效上下文（启动 --ctx-size 或 D4c 重建值；0 省略）
                context_length: match guard.ctx_per_slot() {
                    0 => None,
                    c => Some(c),
                },
                // 迭代36 M126：层占比（Q3 精确口径——解析失败直书原因不降级）
                gpu_layers: layer_split.map(|(g, _)| g),
                total_layers: layer_split.map(|(_, t)| t),
                gpu_layers_note: layer_split
                    .map_or_else(|| Some("stderr 未匹配层卸载行".to_string()), |_| None),
            });
        }
        out
    }

    /// 停止模型（stop 命令）：移除注册并关闭进程。
    ///
    /// - 参数 name：模型名
    /// - 返回：Ok(()) 表示已停止；未运行时 ModelNotFound
    pub async fn stop(&self, name: &str) -> RoxidResult<()> {
        let full = ModelRef::parse(name)?.full_name();
        let removed = self.runners.write().await.remove(&full);
        match removed {
            Some(runner) => runner.lock().await.shutdown_mut().await,
            None => Err(RoxidError::ModelNotFound(full)),
        }
    }

    /// 启动后台卸载循环（keep_alive 到期实例自动关闭）。
    ///
    /// - 参数 interval：扫描间隔（建议 5s）
    pub fn spawn_reaper(self: &Arc<Self>, interval: Duration) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let Some(reg) = weak.upgrade() else { return };
                reg.reap_expired().await;
            }
        });
    }

    /// 单轮到期卸载：收集「到期且无在途请求」的实例，锁外逐个回收。
    /// M27（碴1）：在途实例（in_flight > 0）跳过本轮、下轮扫描再判——
    /// 推理中的请求绝不因 keep_alive 窗口耗尽而被掐断（对齐原版语义；
    /// 窗口由租约释放时重算，见 RunnerLease）。
    pub async fn reap_expired(&self) {
        // 收集到期项（异步逐个探测，随后移除；卸载在锁外执行避免阻塞 acquire）
        let mut expired: Vec<Arc<Mutex<Runner>>> = Vec::new();
        {
            let mut writers = self.runners.write().await;
            let mut expired_keys = Vec::new();
            for (k, r) in writers.iter() {
                let guard = r.lock().await;
                if guard.is_expired() && guard.in_flight() == 0 {
                    expired_keys.push(k.clone());
                }
            }
            for k in expired_keys {
                if let Some(r) = writers.remove(&k) {
                    expired.push(r);
                }
            }
        }
        for runner in expired {
            if let Err(e) = runner.lock().await.shutdown_mut().await {
                tracing::warn!("卸载实例失败：{e}");
            }
        }
    }

    /// M170（迭代45 Q5-A 裁决 2026-09-12 01:45）：卸载一个空闲实例腾
    /// 显存/内存——按 keep_alive 到期时刻最早优先（LRU：最久未使用先
    /// 卸载，对齐 ollama 官方语义）；在途实例（in_flight > 0）受 M27
    /// 在途保护绝不卸载；正在加载的模型自身排除。
    ///
    /// - 参数 exclude：当前正在加载的模型全名（不得自卸）
    /// - 返回：被移出注册表的实例句柄（调用方负责等待其退出以释放资源）；
    ///   无符合条件的空闲实例返回 None
    async fn evict_one_idle(&self, exclude: &str) -> Option<Arc<Mutex<Runner>>> {
        let mut writers = self.runners.write().await;
        let mut victim: Option<(String, std::time::Instant)> = None;
        for (k, r) in writers.iter() {
            if k == exclude {
                continue; // 加载目标自身不得自卸
            }
            let exp = {
                let guard = r.lock().await;
                if guard.in_flight() > 0 {
                    continue; // M27 在途保护：推理中的实例绝不卸载
                }
                guard.expires_at()
            };
            // match 形态（避免 map_or 简化告警）：无候选或到期更早者胜出
            let earlier = match victim.as_ref() {
                None => true,
                Some((_, e)) => exp < *e,
            };
            if earlier {
                victim = Some((k.clone(), exp));
            }
        }
        let (key, _) = victim?;
        writers.remove(&key)
    }

    /// 迭代51 M194（Q1-A 裁决 2026-09-12 18:03）：同类互斥主动换载——
    /// 移除与新加载请求同服务类别的全部空闲实例（写锁内挑选 remove，
    /// 锁外由调用方同步等待退出以释放显存）；在途实例（M27 保护）与
    /// 异类实例零触碰；加载目标自身排除。
    ///
    /// - 参数 loading_full：正在加载的模型全名（不得自卸）
    /// - 参数 class：新请求的服务类别（M193 service_class 单一事实源）
    /// - 返回：被移出注册表的同类空闲实例句柄列表（调用方逐个等待退出）
    async fn evict_same_class_idle(
        &self,
        loading_full: &str,
        class: super::ServiceClass,
    ) -> Vec<Arc<Mutex<Runner>>> {
        let mut writers = self.runners.write().await;
        let mut victims = Vec::new();
        let mut keys = Vec::new();
        for (k, r) in writers.iter() {
            if k == loading_full {
                continue; // 加载目标自身排除
            }
            let same_class = {
                let guard = r.lock().await;
                if guard.in_flight() > 0 {
                    continue; // M27 在途保护：推理中的实例绝不卸载
                }
                guard.service_class() == class
            };
            if same_class {
                keys.push(k.clone());
            }
        }
        for k in keys {
            if let Some(r) = writers.remove(&k) {
                victims.push(r);
            }
        }
        victims
    }

    /// 测试辅助：直接注入替身实例（绕过仓库查找与真实拉起）
    #[cfg(test)]
    pub(crate) async fn inject(&self, runner: Runner) -> Arc<Mutex<Runner>> {
        let arc = Arc::new(Mutex::new(runner));
        self.runners
            .write()
            .await
            .insert(runner_model_name(&arc).await, arc.clone());
        arc
    }
}

/// 取实例的模型名（注入辅助）
#[cfg(test)]
async fn runner_model_name(arc: &Arc<Mutex<Runner>>) -> String {
    arc.lock().await.model_name.clone()
}

/// M39 + 迭代42 D2（N-1 清偿，R1-A 裁决 2026-09-11 21:00）：runtime
/// 重建比较——请求未指定（None）时仅复用「未被请求级覆盖」的实例
///（effective == model_default，对齐 num_ctx 宽容语义：宽容属于实例侧
/// 默认形态，非请求侧缺省）；指定时须与实例生效串逐字一致。
/// 原 `None => true` 使缺省请求误用 --rope-* 类临时覆盖实例（语义性
/// flag 行为传染面）；模型自带 RUNTIME 指令属默认形态，不受影响。
///
/// - 参数 want：请求级 RUNTIME 串
/// - 参数 effective：实例生效 RUNTIME 串（模型默认或请求覆盖）
/// - 参数 model_default：模型元数据 RUNTIME 指令串（默认形态基准）
/// - 返回：true 表示可复用
fn runtime_matches(
    want: Option<&str>,
    effective: Option<&str>,
    model_default: Option<&str>,
) -> bool {
    match want {
        None => effective == model_default,
        Some(w) => effective == Some(w),
    }
}

/// M170（迭代45 Q5-A 裁决 2026-09-12 01:45）：资源不足错误判型——对
/// spawn 失败错误全文（含 M97 stderr 尾部摘要）做特征子串匹配（大小写
/// 不敏感）。宽容设计：漏判时维持现状报错（不误触发卸载），特征表
/// 集中便于按 llama.cpp 版本措辞扩展；跨模型显存不足场景的根治通道。
///
/// - 参数 err_text：spawn 失败的错误全文
/// - 返回：true 表示判定为显存/内存资源不足
fn is_resource_exhausted(err_text: &str) -> bool {
    let lower = err_text.to_ascii_lowercase();
    [
        "out of memory",        // CUDA/系统内存通用形态
        "outofmemoryerror",     // CUDA API 异常名形态
        "failed to allocate",   // ggml CPU/GPU 张量分配形态
        "cuda_malloc_failed",   // llama.cpp CUDA 形态
        "cumalloc failed",      // llama.cpp 新版形态
        "not enough memory",    // 系统内存不足形态
        "insufficient memory",  // Vulkan vkAllocateMemory 形态
        "out of device memory", // Vulkan 设备显存形态
    ]
    .iter()
    .any(|pat| lower.contains(pat))
}

/// M54b：后端二进制路径比对（复用第四键）——resolve-only 解析当前应然
/// 路径与实例记录不一致（runtime use 切换默认版本 / env 改指向）时触发
/// 重建，切换后首个请求即用新版本（对齐 D4c/M39 重建语义）；resolve
/// 不可得（env 指向不存在 / 锁定链未缓存）或实例路径为空（测试替身）时
/// 保守放行，维持三键语义（加载路径由 ensure 完整链兜底）。
///
/// - 参数 instance_bin：实例记录的二进制路径（Runner::llama_server_bin）
/// - 返回：true 表示可复用（一致或不可比对）
fn llama_bin_matches(instance_bin: &std::path::Path) -> bool {
    // M99（迭代31）：asset_variant 未知架构 → resolve 不可得同样保守放行
    //（加载路径 ensure 完整链承担显式报错）
    // 迭代50 M189：变体来源改 effective_variant——use 切换变体后应然路径
    // 变化触发重建（M54 切换即时生效语义自然覆盖）
    let Some(expected) = crate::runtime::effective_variant()
        .ok()
        .and_then(|v| crate::runtime::resolve_llama_server_path(&v))
    else {
        return true; // resolve 不可得：保守放行
    };
    instance_bin.as_os_str().is_empty() || instance_bin == &expected
}

/// 由模型元数据构造 llama-server 启动参数
///
/// - 参数 bin：runtime 模块解析出的 llama-server 二进制路径
/// - 参数 root：模型仓库根目录
/// - 参数 r：模型引用
/// - 参数 meta：模型元数据
/// - 返回：完整启动参数
fn spawn_spec_from_meta(
    bin: &std::path::Path,
    root: &std::path::Path,
    r: &ModelRef,
    meta: &ModelMeta,
) -> SpawnSpec {
    let dir = r.dir(root);
    let ctx_size = meta
        .parameters
        .get("num_ctx")
        .and_then(|v| v.as_u64())
        .unwrap_or(4096) as u32;
    // slot 并行数（M19，Q2/M19-R1 裁决）：元数据 num_parallel 优先，
    // 否则环境变量 ROXID_NUM_PARALLEL→OLLAMA_NUM_PARALLEL，缺省 4
    let parallel = meta
        .parameters
        .get("num_parallel")
        .and_then(|v| v.as_u64())
        .map(|n| n.max(1) as u32)
        .unwrap_or_else(|| {
            crate::scheduler::resolve_num_parallel(
                std::env::var(crate::scheduler::ENV_NUM_PARALLEL).ok(),
                std::env::var(crate::scheduler::ENV_NUM_PARALLEL_FALLBACK).ok(),
            )
        });
    // 迭代51 M196 + 迭代56 M209：GGUF header 一次解析贯通 MTP 头与 MoE
    // 双判定（复用同次 IO，读 header 失败宽容 false——投机注入降级
    // ngram-mod dense 档不阻断加载）2026-09-13 01-14
    let gguf_meta = crate::registry::gguf::parse_metadata(&dir.join(&meta.files.model));
    SpawnSpec {
        llama_server_bin: bin.to_path_buf(),
        gguf: dir.join(&meta.files.model),
        mmproj: meta.files.mmproj.as_ref().map(|m| dir.join(m)),
        lora: meta.adapters.iter().map(|a| dir.join(a)).collect(),
        ctx_size,
        parallel,
        // M39：RUNTIME 指令透传（spawn_args 内 shell 风格分词追加）
        runtime_flags: meta.runtime.clone(),
        // 迭代42 D2（N-1）：默认形态基准（请求级覆盖发生在 acquire 层，
        // 仅改 runtime_flags，本字段恒为模型指令串）
        model_runtime: meta.runtime.clone(),
        // 迭代51 M193：服务类别（M194 同类换载判定键，词根规则单一事实源）
        service_class: super::service_class(&meta.family),
        // 迭代51 M196：MTP 头检测（llama.cpp auto-detect 同款标志
        // nextn.eh_proj）
        has_mtp: gguf_meta.as_ref().map(|g| g.has_mtp).unwrap_or(false),
        // 迭代56 M209：MoE 检测（llama.cpp fit.cpp LAYER_FRACTION_MOE
        // 同款标志 ffn_*_exps tensor）——ngram 投机参数分档依据
        is_moe: gguf_meta.map(|g| g.is_moe).unwrap_or(false),
    }
}

/// 拉起并按需换端口重试（M28 碴13）：每次尝试重取端口（alloc_port 顺序游标
/// 自然推进），失败重试上限 3 次后透出最后错误。泛型 spawn_fn 便于单测注入。
/// M99（迭代31，Q4）：ArchMismatch 类确定性失败与端口无关——换端口重试
/// 徒增 3 次噪音日志与等待，直接透出（原实现盲目重试 3 次并刷误导性
///「重取端口重试」日志，用户裁决 2026-09-10 18:21）。
///
/// - 参数 model_full：完整模型名（日志与兜底错误信息用）
/// - 参数 spawn_fn：端口 → 拉起 Future（生产为 Runner::spawn_llama_server）
/// - 返回：就绪的 Runner
async fn spawn_with_port_retry<F, Fut>(model_full: &str, mut spawn_fn: F) -> RoxidResult<Runner>
where
    F: FnMut(u16) -> Fut,
    Fut: std::future::Future<Output = RoxidResult<Runner>>,
{
    let mut last_err: Option<RoxidError> = None;
    for _ in 0..3u32 {
        let port = alloc_port();
        match spawn_fn(port).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                // M99：确定性失败（架构不匹配）不换端口重试直接透出——
                // 重试必然同果，且「重取端口重试」文案误导排查方向
                if matches!(e, RoxidError::ArchMismatch(_)) {
                    tracing::error!(
                        "llama-server 拉起失败（确定性错误，不重试）model={model_full}：{e}"
                    );
                    return Err(e);
                }
                tracing::warn!("llama-server 拉起失败（重取端口重试）model={model_full}：{e}");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        RoxidError::RunnerFailure(format!("llama-server 拉起重试耗尽：{model_full}"))
    }))
}

/// N 秒后的 RFC3339（复用 registry 模块的日期换算）
fn rfc3339_after(d: Duration) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    crate::registry::now_rfc3339_with(now + d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 迭代42 D2（N-1，R1-A 裁决）：runtime 复用判定三参语义——缺省请求
    /// 仅复用未被请求级覆盖的实例；带串请求精确匹配；模型指令属默认形态
    #[test]
    fn runtime_matches_strict_isolation_semantics() {
        // 双缺省：复用（既有语义）
        assert!(runtime_matches(None, None, None));
        // 模型自带指令（未被覆盖）：缺省请求可复用（默认形态，非传染面）
        assert!(runtime_matches(
            None,
            Some("--threads 2"),
            Some("--threads 2")
        ));
        // 请求级覆盖实例：缺省请求不复用（碴面——原 None => true 误放行）
        assert!(!runtime_matches(None, Some("--rope-scaling 0.5"), None));
        assert!(!runtime_matches(
            None,
            Some("--rope-scaling 0.5"),
            Some("--threads 2")
        ));
        // 带串请求：同串复用、异串重建（既有语义）
        assert!(runtime_matches(
            Some("--rope-scaling 0.5"),
            Some("--rope-scaling 0.5"),
            None
        ));
        assert!(!runtime_matches(Some("-ngl 30"), Some("-ngl 99"), None));
        // 带串请求不复用干净实例（既有语义）
        assert!(!runtime_matches(Some("-ngl 30"), None, None));
        // 覆盖串恰等于模型默认：缺省请求复用无行为差异（串相同）
        assert!(runtime_matches(None, Some("-ngl 30"), Some("-ngl 30")));
    }
    use std::process::Stdio;
    use tokio::process::Command;

    /// 构造测试注册表与 sleep 替身
    async fn stub_registry(
        model: &str,
        keep_alive: Duration,
    ) -> (Arc<RunnerRegistry>, Arc<Mutex<Runner>>) {
        let reg = Arc::new(RunnerRegistry::new(
            std::env::temp_dir().join("roxid-reg-test"),
        ));
        let mut cmd = Command::new("sleep");
        cmd.arg("300").stdout(Stdio::null()).stderr(Stdio::null());
        let runner = Runner::spawn_with(model, 0, keep_alive, &mut cmd)
            .await
            .unwrap();
        let arc = reg.inject(runner).await;
        (reg, arc)
    }

    /// M170：资源不足判型纯函数——特征命中/大小写不敏感/无关错误不命中
    #[test]
    fn resource_exhaustion_patterns() {
        assert!(is_resource_exhausted(
            "llama-server 启动即退出：CUDA error: out of memory"
        ));
        assert!(is_resource_exhausted("failed to allocate 4096 MiB"));
        assert!(
            is_resource_exhausted("cudaMalloc failed: Out Of Memory"),
            "大小写不敏感"
        );
        assert!(
            is_resource_exhausted("vkAllocateMemory: insufficient memory"),
            "Vulkan 形态"
        );
        assert!(is_resource_exhausted("out of device memory"));
        // 无关错误不得误判（宽容漏判、严禁误杀）
        assert!(!is_resource_exhausted(
            "llama-server 启动即退出：wrong number of tensors"
        ));
        assert!(!is_resource_exhausted("健康检查超时 300s"));
        assert!(!is_resource_exhausted("重取端口重试"));
    }

    /// M170：evict_one_idle——最早到期优先（LRU）、在途不卸（M27 保护）、
    /// 加载目标自身排除；空闲清空返回 None
    #[tokio::test]
    async fn evict_picks_earliest_idle_and_spares_inflight() {
        async fn inject_stub(
            reg: &Arc<RunnerRegistry>,
            model: &str,
            keep_alive: Duration,
        ) -> Arc<Mutex<Runner>> {
            let mut cmd = Command::new("sleep");
            cmd.arg("300").stdout(Stdio::null()).stderr(Stdio::null());
            let runner = Runner::spawn_with(model, 0, keep_alive, &mut cmd)
                .await
                .unwrap();
            reg.inject(runner).await
        }
        let reg = Arc::new(RunnerRegistry::new(
            std::env::temp_dir().join("roxid-reg-evict"),
        ));
        let a_older = inject_stub(&reg, "stub:older", Duration::from_secs(1)).await;
        let _a_newer = inject_stub(&reg, "stub:newer", Duration::from_secs(300)).await;
        let a_busy = inject_stub(&reg, "stub:busy", Duration::from_secs(1)).await;
        a_busy.lock().await.enter_request(); // 在途（到期但不卸）
        tokio::time::sleep(Duration::from_millis(80)).await; // older/busy 窗口耗尽

        // 第一次驱逐：older（到期最早=最久未用，且空闲；busy 在途跳过）
        let v1 = reg
            .evict_one_idle("stub:target")
            .await
            .expect("须命中 older");
        assert_eq!(v1.lock().await.model_name, "stub:older");
        v1.lock().await.shutdown_mut().await.unwrap(); // 清理替身

        // 第二次驱逐：newer（空闲可驱逐——LRU 按到期时刻，非仅到期实例）
        let v2 = reg.evict_one_idle("stub:self").await.expect("须命中 newer");
        assert_eq!(v2.lock().await.model_name, "stub:newer");
        v2.lock().await.shutdown_mut().await.unwrap();

        // 第三次：仅剩 busy（在途）与 self（排除）→ 无候选
        assert!(
            reg.evict_one_idle("stub:self").await.is_none(),
            "在途与自身排除后必须无候选"
        );
        // 清理余下替身
        a_older.lock().await.shutdown_mut().await.unwrap();
        a_busy.lock().await.shutdown_mut().await.unwrap();
    }

    /// 迭代51 M194：同类互斥换载作用域——同类空闲命中卸载、异类保留、
    /// 在途同类跳过（M27 保护）、加载目标自身排除。
    /// 来源：Q1-A 裁决 2026-09-12 18:03「同类空闲立即卸载」。
    #[tokio::test]
    async fn evict_same_class_idle_scopes_to_class_and_idleness() {
        use crate::scheduler::ServiceClass;
        async fn inject_stub(
            reg: &Arc<RunnerRegistry>,
            model: &str,
            class: ServiceClass,
        ) -> Arc<Mutex<Runner>> {
            let mut cmd = Command::new("sleep");
            cmd.arg("300").stdout(Stdio::null()).stderr(Stdio::null());
            let mut runner = Runner::spawn_with(model, 0, Duration::from_secs(300), &mut cmd)
                .await
                .unwrap();
            runner.override_service_class_for_test(class);
            reg.inject(runner).await
        }
        let reg = Arc::new(RunnerRegistry::new(
            std::env::temp_dir().join("roxid-reg-sameclass"),
        ));
        // 四替身：同类空闲 / 异类空闲(embedding) / 同类在途 / 加载目标自身
        inject_stub(&reg, "stub:gen", ServiceClass::Generation).await;
        inject_stub(&reg, "stub:embed", ServiceClass::Embedding).await;
        let gen_busy = inject_stub(&reg, "stub:genbusy", ServiceClass::Generation).await;
        gen_busy.lock().await.enter_request(); // 在途（同类但不卸，M27 保护）
        let self_idle = inject_stub(&reg, "stub:self", ServiceClass::Generation).await;

        let victims = reg
            .evict_same_class_idle("stub:self", ServiceClass::Generation)
            .await;
        // 仅同类空闲 gen 被移出：异类/在途/自身全保留
        assert_eq!(victims.len(), 1, "仅同类空闲实例命中");
        assert_eq!(victims[0].lock().await.model_name, "stub:gen");
        for v in &victims {
            v.lock().await.shutdown_mut().await.unwrap(); // 清理受害者替身
        }

        // embedding 视角：加载 embedding 模型时仅 embedding 空闲被卸
        let v2 = reg
            .evict_same_class_idle("stub:loading", ServiceClass::Embedding)
            .await;
        assert_eq!(v2.len(), 1);
        assert_eq!(v2[0].lock().await.model_name, "stub:embed");
        v2[0].lock().await.shutdown_mut().await.unwrap();

        // 清理余下替身（在途与自身）
        gen_busy.lock().await.shutdown_mut().await.unwrap();
        self_idle.lock().await.shutdown_mut().await.unwrap();
    }

    /// stop：注入实例 → 停止 → 再停报不存在
    #[tokio::test]
    async fn stop_removes_and_errors_on_missing() {
        let (reg, arc) = stub_registry("stub:a", Duration::from_secs(300)).await;
        reg.stop("stub:a").await.unwrap();
        assert!(arc.lock().await.is_process_dead());
        assert!(reg.stop("stub:a").await.is_err());
    }

    /// acquire 未安装模型必须报 ModelNotFound
    #[tokio::test]
    async fn acquire_missing_model_errors() {
        let reg = RunnerRegistry::new(std::env::temp_dir().join("roxid-reg-none"));
        let err = reg
            .acquire("no-such-model:xx", Duration::from_secs(60), None, None)
            .await;
        assert!(matches!(err, Err(RoxidError::ModelNotFound(_))));
    }

    /// M26（碴1）：死实例自愈——acquire 拒绝已崩溃实例并清理出注册表。
    /// 判据：旧缺陷行为下快路径命中返回 Ok（死实例句柄）；修复后清理死实例
    /// 并走加载路径，因仓库无此模型报 ModelNotFound，且实例不再可 stop。
    #[tokio::test]
    async fn acquire_recovers_from_dead_runner() {
        let reg = Arc::new(RunnerRegistry::new(
            std::env::temp_dir().join("roxid-reg-dead"),
        ));
        // 注入立即退出的替身（模拟崩溃后残留注册表的死实例）
        let mut cmd = Command::new("true");
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let runner = Runner::spawn_with("stub:dead", 0, Duration::from_secs(300), &mut cmd)
            .await
            .unwrap();
        let _arc = reg.inject(runner).await;
        tokio::time::sleep(Duration::from_millis(150)).await; // 等 true 退出

        let err = match reg
            .acquire("stub:dead", Duration::from_secs(60), None, None)
            .await
        {
            Ok(_) => panic!("死实例不得被放行（旧缺陷行为：命中快路径返回死句柄）"),
            Err(e) => e,
        };
        assert!(
            matches!(err, RoxidError::ModelNotFound(_)),
            "必须清理死实例并走重新加载路径，实际：{err}"
        );
        assert!(
            reg.stop("stub:dead").await.is_err(),
            "死实例必须已被移出注册表（不可再 stop）"
        );
    }

    /// /api/ps 快照：注入实例可见（sleep 替身非 GPU 进程 → size_vram 必为 0）
    #[tokio::test]
    async fn list_running_shows_injected() {
        let (reg, _arc) = stub_registry("stub:ps", Duration::from_secs(300)).await;
        let list = reg.list_running().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "stub:ps");
        assert_eq!(list[0].size_vram, 0, "非 GPU 进程显存必须为 0（回退语义）");
    }

    /// M27（碴1）：在途保护与窗口重算——到期实例有在途请求时 reap 不得卸载；
    /// 请求离开后窗口自离开时刻重算，再到期方可卸载（R3 裁决语义）
    #[tokio::test]
    async fn reaper_spares_inflight_and_releases_after_leave() {
        let (reg, arc) = stub_registry("stub:inflight", Duration::from_millis(60)).await;
        // 模拟请求在途（等同 acquire 发放租约的计数效果）
        arc.lock().await.enter_request();
        tokio::time::sleep(Duration::from_millis(100)).await; // 窗口耗尽但在途
        reg.reap_expired().await;
        assert_eq!(reg.list_running().await.len(), 1, "在途实例必须被跳过");
        // 请求离开：计数归零且窗口自此刻重算
        arc.lock().await.leave_request();
        assert!(
            !arc.lock().await.is_expired(),
            "窗口必须自离开时刻重算（R3）"
        );
        reg.reap_expired().await;
        assert_eq!(reg.list_running().await.len(), 1, "重算后未到期不得卸载");
        // 再次等到期：可卸载
        tokio::time::sleep(Duration::from_millis(100)).await;
        reg.reap_expired().await;
        assert!(
            reg.list_running().await.is_empty(),
            "租约释放且到期后必须卸载"
        );
        assert!(arc.lock().await.is_process_dead());
    }

    /// M27（碴2）：并发 acquire 不死锁——不同模型并发清理各自死实例（跨模型
    /// 并行路径）；同模型并发经 per-model 锁串行后均报错（防双拉起语义保持）
    #[tokio::test]
    async fn concurrent_acquire_no_deadlock() {
        let reg = Arc::new(RunnerRegistry::new(
            std::env::temp_dir().join("roxid-reg-par"),
        ));
        // 注入两个已崩溃替身（并发触发"清理→重新加载"路径）
        for name in ["stub:a", "stub:b"] {
            let mut cmd = Command::new("true");
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
            let runner = Runner::spawn_with(name, 0, Duration::from_secs(300), &mut cmd)
                .await
                .unwrap();
            reg.inject(runner).await;
        }
        tokio::time::sleep(Duration::from_millis(150)).await; // 等 true 退出
        let (ra, rb) = (reg.clone(), reg.clone());
        let (ea, eb) = tokio::join!(
            ra.acquire("stub:a", Duration::from_secs(60), None, None),
            rb.acquire("stub:b", Duration::from_secs(60), None, None),
        );
        assert!(matches!(ea, Err(RoxidError::ModelNotFound(_))), "a: {ea:?}");
        assert!(matches!(eb, Err(RoxidError::ModelNotFound(_))), "b: {eb:?}");
        // 同模型并发：per-model 锁串行，两请求先后走加载路径失败（无死锁即语义保持）
        let (rc, rd) = (reg.clone(), reg.clone());
        let (ec, ed) = tokio::join!(
            rc.acquire("stub:missing", Duration::from_secs(60), None, None),
            rd.acquire("stub:missing", Duration::from_secs(60), None, None),
        );
        assert!(ec.is_err() && ed.is_err(), "同模型并发串行后均须报错");
    }

    /// M29 碴6：重建路径停机在全局写锁外执行——ctx 不一致触发重建不死锁
    /// （超时断言：写锁不因进程停机等待而长期占用的行为学验证）
    #[tokio::test]
    async fn rebuild_releases_write_lock_before_shutdown() {
        use std::process::Stdio;
        // seed 仓库模型使重建走到 spawn（真实 spawn 会失败报错——本测试只验证
        // 「写锁先释放」：并发 ps 在重建期间不被阻塞超过停机等待时长）
        let root = std::env::temp_dir().join("roxid-reg-rebuild/models");
        let dir = root.join("stub/rb");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf"), b"gguf").unwrap();
        crate::repo::model_json::save_meta(
            &dir,
            &crate::repo::ModelMeta {
                runtime: None,
                name: "stub:rb".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "local-create".into(),
                created_at: "2026-09-05T00:00:00Z".into(),
            },
        )
        .unwrap();
        // 注意：RunnerRegistry 构造时传入的根目录须含该模型（重造注册表）
        let reg = Arc::new(RunnerRegistry::new(root.clone()));
        let mut cmd = Command::new("sleep");
        cmd.arg("300").stdout(Stdio::null()).stderr(Stdio::null());
        let runner = Runner::spawn_with("stub:rb", 0, Duration::from_secs(300), &mut cmd)
            .await
            .unwrap();
        reg.inject(runner).await;

        // 触发重建（want_ctx=Some(4096) 与替身 ctx=0 判定不匹配）与并发读
        // 并发读必须在写锁释放后立即可得（不受停机等待拖累——替身进程 kill
        // 极快，此处以「两操作均在时限内完成」作行为学断言，死锁场景超时）
        let (ra, rb) = (reg.clone(), reg.clone());
        let rebuild = tokio::spawn(async move {
            ra.acquire("stub:rb", Duration::from_secs(60), Some(4096), None)
                .await
        });
        let list = tokio::spawn(async move { rb.list_running().await });
        let (rebuild_res, list_res) = tokio::time::timeout(Duration::from_secs(30), async {
            (rebuild.await, list.await)
        })
        .await
        .expect("M29 碴6：重建与并发读均须完成（写锁外停机，无死锁）");
        // 重建对替身场景最终报错（sleep 非 llama-server，健康检查失败）或
        // 移除后 ModelNotFound——只需不挂起即验证锁行为
        let _ = rebuild_res;
        let _ = list_res;
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M54b：复用第四键——resolve 应然路径与实例记录一致时复用（秒回租约）；
    /// 不一致（env 改指向模拟 runtime use 切换）触发重建：旧替身被移出
    /// 注册表，后续加载因替身非 llama-server 报错（重建路径行为学断言）
    #[tokio::test]
    async fn acquire_rebuilds_on_llama_bin_mismatch() {
        use std::process::Stdio;
        // env 互斥（注记 B 先例）：本测试操纵 ROXID_LLAMA_SERVER（ensure/
        // resolve 链第一优先级），与 ensure 系测试（读 env）共用
        // ROXID_HOME_TEST_LOCK 串行，防并行互扰
        let _env_guard = crate::config::ROXID_HOME_TEST_LOCK.lock().unwrap();
        // env 指向的「应然二进制」文件（仅存在性判定，无执行位不会被拉起）
        let bin_a = std::env::temp_dir().join("roxid-m54-bin-a");
        let bin_b = std::env::temp_dir().join("roxid-m54-bin-b");
        std::fs::write(&bin_a, b"stub").unwrap();
        std::fs::write(&bin_b, b"stub").unwrap();
        std::env::set_var(crate::runtime::ENV_LLAMA_SERVER_OVERRIDE, &bin_a);

        // seed 仓库模型 meta（重建路径 find_model 需要）
        let root = std::env::temp_dir().join("roxid-reg-m54/models");
        let dir = root.join("stub/m54");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.gguf"), b"gguf").unwrap();
        crate::repo::model_json::save_meta(
            &dir,
            &crate::repo::ModelMeta {
                runtime: None,
                name: "stub:m54".into(),
                family: "llama".into(),
                families: vec![],
                parameter_size: "1B".into(),
                quantization_level: "Q4".into(),
                system: String::new(),
                template: None,
                parameters: Default::default(),
                messages: vec![],
                layer_digests: Default::default(),
                adapters: vec![],
                files: crate::repo::ModelFiles {
                    model: "model.gguf".into(),
                    mmproj: None,
                },
                digest: String::new(),
                license: String::new(),
                source: "local-create".into(),
                created_at: "2026-09-09T00:00:00Z".into(),
            },
        )
        .unwrap();
        let reg = Arc::new(RunnerRegistry::new(root.clone()));
        let mut cmd = Command::new("sleep");
        cmd.arg("300").stdout(Stdio::null()).stderr(Stdio::null());
        let mut runner = Runner::spawn_with("stub:m54", 0, Duration::from_secs(300), &mut cmd)
            .await
            .unwrap();
        runner.set_llama_server_bin(bin_a.clone());
        reg.inject(runner).await;

        // ① 路径一致：快路径复用——sleep 替身不可健康检查，若走重建必报错，
        //    Ok 即证明复用未重建
        let lease = reg
            .acquire("stub:m54", Duration::from_secs(60), None, None)
            .await;
        assert!(lease.is_ok(), "路径一致必须复用：{lease:?}");

        // ② 路径不一致（env 改指向 bin_b，模拟 runtime use 切换）：触发重建
        //    ——旧替身移出注册表；加载路径 env 命中 bin_b（无执行位/非服务）
        //    spawn 或健康检查失败报错
        std::env::set_var(crate::runtime::ENV_LLAMA_SERVER_OVERRIDE, &bin_b);
        let rebuilt = reg
            .acquire("stub:m54", Duration::from_secs(60), None, None)
            .await;
        assert!(
            rebuilt.is_err(),
            "路径不一致必须走重建（替身场景加载失败报错）"
        );
        assert!(
            reg.list_running().await.is_empty(),
            "旧实例必须已被移出注册表"
        );

        std::env::remove_var(crate::runtime::ENV_LLAMA_SERVER_OVERRIDE);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&bin_a);
        let _ = std::fs::remove_file(&bin_b);
    }

    /// M99（迭代31，Q4）：ArchMismatch 确定性失败不换端口重试——
    /// 单次尝试即透出（换端口对架构问题无意义，重试必然同果）
    #[tokio::test]
    async fn port_retry_skips_arch_mismatch() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let a = attempts.clone();
        let err = match spawn_with_port_retry("t:arch", move |_port| {
            a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Err(RoxidError::ArchMismatch("注入架构不匹配".into())) }
        })
        .await
        {
            Err(e) => e,
            Ok(_) => panic!("注入失败闭包不应成功"),
        };
        assert!(
            matches!(err, RoxidError::ArchMismatch(_)),
            "必须原样透出 ArchMismatch：{err:?}"
        );
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "确定性失败仅尝试 1 次（不换端口重试）"
        );
    }

    /// M28 碴13：换端口重试编排——两次注入失败后第三次成功（重试次数与
    /// 端口推进断言）；连续失败 3 次后透出最后错误
    #[tokio::test]
    async fn spawn_retry_exhausts_and_recovers() {
        use std::sync::atomic::{AtomicU32, Ordering};
        // 失败两轮后成功（sleep 替身构造真 Runner）
        let attempts = Arc::new(AtomicU32::new(0));
        let seen_ports: Arc<std::sync::Mutex<Vec<u16>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let a = attempts.clone();
        let sp = seen_ports.clone();
        let runner = spawn_with_port_retry("t:retry", move |port| {
            let a = a.clone();
            let sp = sp.clone();
            async move {
                sp.lock().unwrap().push(port);
                if a.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(RoxidError::RunnerFailure(format!("注入失败 #{port}")))
                } else {
                    let mut cmd = Command::new("sleep");
                    cmd.arg("300").stdout(Stdio::null()).stderr(Stdio::null());
                    Runner::spawn_with("t:retry", port, Duration::from_secs(60), &mut cmd).await
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 3, "两次失败后第三次成功");
        {
            let ports = seen_ports.lock().unwrap();
            assert_eq!(ports.len(), 3, "每次尝试必须重取端口");
            assert_ne!(ports[0], ports[1], "重试端口不得重复");
            assert_ne!(ports[1], ports[2], "重试端口不得重复");
        }
        runner.shutdown().await.unwrap();

        // 连续失败：3 次尝试后透出最后错误
        let attempts2 = Arc::new(AtomicU32::new(0));
        let a2 = attempts2.clone();
        let err = match spawn_with_port_retry("t:fail", move |_port| {
            let a2 = a2.clone();
            async move {
                a2.fetch_add(1, Ordering::SeqCst);
                Err(RoxidError::RunnerFailure("always-fail".into()))
            }
        })
        .await
        {
            Ok(_) => panic!("连续失败场景必须报错"),
            Err(e) => e,
        };
        assert_eq!(attempts2.load(Ordering::SeqCst), 3, "重试上限 3 次");
        assert!(
            err.to_string().contains("always-fail"),
            "必须透出最后错误：{err}"
        );
    }

    /// 真实集成验收：acquire 全链路（仓库→运行时→拉起→推理→/api/ps→stop）
    /// 前置：M5 已拉取 smollm:135m 至 /tmp/roxid-pull-e2e；
    ///       M2 已缓存 llama-server 至 /tmp/roxid-rt-e2e
    #[tokio::test]
    #[ignore = "真实模型推理，验收时手动执行"]
    async fn acquire_real_model_and_chat() {
        std::env::set_var(
            crate::runtime::ENV_LLAMA_SERVER_OVERRIDE,
            "/tmp/roxid-rt-e2e/llama.cpp/b10605/ubuntu-x64/llama-server",
        );
        let reg = Arc::new(RunnerRegistry::new(std::path::PathBuf::from(
            "/tmp/roxid-pull-e2e/models",
        )));
        let lease = reg
            .acquire("smollm:135m", Duration::from_secs(60), None, None)
            .await
            .unwrap();
        let port = lease.port().await;

        // 真实推理：经 llama-server 的 OpenAI 兼容端点
        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .json(&serde_json::json!({
                "messages": [{"role": "user", "content": "Say hello in one word."}],
                "max_tokens": 16
            }))
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "推理请求必须成功");
        let body: serde_json::Value = resp.json().await.unwrap();
        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default();
        assert!(!text.is_empty(), "必须返回生成文本：{body}");

        let ps = reg.list_running().await;
        assert_eq!(ps.len(), 1, "/api/ps 必须看到运行实例");
        assert_eq!(ps[0].name, "smollm:135m");
        reg.stop("smollm:135m").await.unwrap();
        assert!(reg.list_running().await.is_empty(), "stop 后必须清空");
        std::env::remove_var(crate::runtime::ENV_LLAMA_SERVER_OVERRIDE);
    }
}
