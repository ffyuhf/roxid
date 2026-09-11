//! 全屏 TUI 交互创建向导（迭代41 M146/M147，用户终裁 2026-09-11 09:26
//! 「全屏 TUI：引入 ratatui + crossterm，create -i 升级为 btop 式界面」）。
//!
//! 四步状态机：① 列表选基础模型（本地已装模型，输入即过滤）→ ② SYSTEM
//! 多行编辑 → ③ RUNTIME 单行编辑（示例常驻底部）→ ④ Modelfile 预览确认。
//! TUI 只负责收集输入并组装 Modelfile 文本；提交由调用方在终端恢复后走
//! 既有 `submit_create`（NDJSON 进度沿用现有渲染链）——TUI 与网络零耦合。
//!
//! 终端卫生：EnterAlternateScreen + panic hook 恢复 + LeaveAlternateScreen；
//! 初始化失败返回 Err 由调用方降级既有问答向导（TERM=dumb 等场景兜底）。
//!
//! 修改历史：M146/M147 新增 2026-09-11 09-35；
//! M166（迭代45 Q1-A 裁决 2026-09-12 01:43）：bracketed paste 启用 +
//! EditSystem 键位改造（Enter=完成、Alt+Enter=换行、Esc=取消向导）——
//! 原 Enter=换行/Esc=完成与通用 TUI 直觉相反（用户实测报告）；粘贴
//! 多行文本经 bracketed paste 整块到达，换行符保留不被拆解为按键序列
//! 2026-09-12 01-50；
//! M167（迭代45 Q2-A 裁决 01:43）：EditSystem 视口滚动（底部对齐跟随
//! 光标——长提示词粘贴后底部内容与光标恒可见）+ Preview 预览滚动
//! （Up/Down 单行、PgUp/PgDn 十行，渲染层钳上限）2026-09-12 01-52；
//! M168（迭代45 Q3-A 裁决 01:43）：光标列按显示宽折算（CJK 2 列/字，
//! 复用 crate::display_width 零新依赖）+ Wrap 折行 y 偏移
//! 2026-09-12 01-52

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use std::io::{self, Stdout};

/// 向导四步（步骤指示器与帮助行按此区分）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    /// ① 从本地已装模型中选择 FROM 基础模型
    PickFrom,
    /// ② 编辑 SYSTEM 系统提示（多行，可空跳过）
    EditSystem,
    /// ③ 编辑 RUNTIME 启动参数（单行，可空跳过）
    EditRuntime,
    /// ④ 预览组装后的 Modelfile 并确认
    Preview,
}

impl Step {
    /// 步骤标题（主体区块标题栏文案）
    fn title(self) -> &'static str {
        match self {
            Step::PickFrom => "(1/4) 选择基础模型（输入过滤，↑/↓ 移动）",
            Step::EditSystem => "(2/4) 系统提示 SYSTEM（可空跳过）",
            Step::EditRuntime => "(3/4) llama.cpp 启动参数 RUNTIME（可空跳过）",
            Step::Preview => "(4/4) 预览即将创建的 Modelfile",
        }
    }

    /// 底部帮助行（当前步骤可用键位）
    fn help(self) -> &'static str {
        match self {
            Step::PickFrom => {
                "↑/↓ 或 j/k 移动    Enter 选中    输入即过滤    Backspace 删过滤    Esc 取消"
            }
            // M166：Enter=完成 / Alt+Enter=换行（粘贴自动保留换行经 bracketed paste）
            Step::EditSystem => {
                "输入内容    Enter 完成    Alt+Enter 换行    粘贴自动保留换行    Esc/Ctrl+C 取消"
            }
            Step::EditRuntime => {
                "输入参数    Enter 完成（空=跳过 RUNTIME）    Esc 完成    Ctrl+C 取消"
            }
            // M167：预览滚动键（Up/Down 单行、PgUp/PgDn 十行）
            Step::Preview => "Enter 创建    Esc 取消    ↑/↓ 滚动（PgUp/PgDn 十行）",
        }
    }
}

/// 按键处理结果（事件循环据此决定流向）
enum KeyAction {
    /// 状态内已消化，仅重绘
    Continue,
    /// 用户取消整个向导
    Cancel,
    /// 预览步确认创建（携带组装完成的 Modelfile 文本）
    ConfirmCreate(String),
}

/// 向导应用状态（全部可变状态集中；渲染为纯读取，单测可直接构造断言）
struct WizardApp {
    /// 当前步骤
    step: Step,
    /// 新模型名（命令行给定，仅标题展示）
    model: String,
    /// 本地已装模型名全量（FROM 候选源）
    models: Vec<String>,
    /// 步骤① 输入的过滤串（前缀匹配）
    filter: String,
    /// 步骤① 列表选中态
    list_state: ListState,
    /// 步骤② 多行内容（每行按字符存储，光标 col 免 byte 换算；CJK 免疫）
    system_rows: Vec<Vec<char>>,
    /// 步骤② 光标（行号/行内字符列）
    sys_row: usize,
    sys_col: usize,
    /// 步骤③ RUNTIME 参数文本（单行，按字符）
    runtime_chars: Vec<char>,
    /// 步骤④ 预览滚动偏移（M167：Up/Down/PgUp/PgDn 调整，渲染层钳上限）
    preview_scroll: u16,
}

impl WizardApp {
    /// 构造初始状态（步骤① 起步，默认选中首项）
    ///
    /// - 参数 model：新模型名（标题展示）
    /// - 参数 models：本地已装模型名列表（FROM 候选）
    fn new(model: &str, models: Vec<String>) -> Self {
        let mut list_state = ListState::default();
        if !models.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            step: Step::PickFrom,
            model: model.to_string(),
            models,
            filter: String::new(),
            list_state,
            system_rows: vec![Vec::new()],
            sys_row: 0,
            sys_col: 0,
            runtime_chars: Vec::new(),
            preview_scroll: 0,
        }
    }

    /// 过滤后的候选（前缀匹配；空过滤即全量）
    fn filtered(&self) -> Vec<&str> {
        self.models
            .iter()
            .map(String::as_str)
            .filter(|m| m.starts_with(self.filter.as_str()))
            .collect()
    }

    /// 步骤① 当前选中项（过滤后列表非空才有值）
    fn selected_from(&self) -> Option<&str> {
        self.list_state
            .selected()
            .and_then(|i| self.filtered().get(i).copied())
    }

    /// 按键分发（按当前步骤路由到对应处理器）
    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyAction {
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            return KeyAction::Cancel;
        }
        match self.step {
            Step::PickFrom => self.on_key_pick(code),
            Step::EditSystem => self.on_key_system(code, mods),
            Step::EditRuntime => self.on_key_runtime(code),
            Step::Preview => self.on_key_preview(code),
        }
    }

    /// 粘贴事件分发（M166，bracketed paste：多行文本整块到达，换行符
    /// 保留在串内不被拆解为 Enter 按键——长系统提示词一次性粘贴不截断）。
    /// - PickFrom：滤除换行追加为过滤串
    /// - EditSystem：按换行拆分在光标处多行插入
    /// - EditRuntime：滤除换行追加为单行
    /// - Preview：忽略
    fn on_paste(&mut self, text: &str) {
        match self.step {
            Step::PickFrom => {
                for c in text.chars() {
                    if c != '\n' && c != '\r' {
                        self.filter.push(c);
                    }
                }
                self.list_state.select(if self.filtered().is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            Step::EditSystem => self.insert_paste_multiline(text),
            Step::EditRuntime => {
                for c in text.chars() {
                    if c != '\n' && c != '\r' {
                        self.runtime_chars.insert(self.runtime_chars.len(), c);
                    }
                }
            }
            Step::Preview => {}
        }
    }

    /// EditSystem 光标处多行插入：当前行按光标分裂为前/后段——粘贴首行
    /// 接前段、末行与后段拼接、中间行整行插入；光标落在插入内容末尾。
    /// 单行粘贴（无换行）合并不新增行（等同连续手输）。
    fn insert_paste_multiline(&mut self, text: &str) {
        // 统一按 \n 与 \r 拆行（兼容 CRLF 粘贴形态），与手输同按字符存储
        let lines: Vec<Vec<char>> = text
            .split(['\n', '\r'])
            .map(|l| l.chars().collect())
            .collect();
        if lines.is_empty() {
            return;
        }
        // 光标行兜底补齐（与 on_key_system 同防御：行号不越界）
        while self.system_rows.len() <= self.sys_row {
            self.system_rows.push(Vec::new());
        }
        let tail = self.system_rows[self.sys_row].split_off(self.sys_col); // 光标后段
        let head = std::mem::take(&mut self.system_rows[self.sys_row]);
        let n = lines.len();
        if n == 1 {
            // 单行：head + 粘贴行 + tail 合并，不新增行
            let mut row = head;
            row.extend(lines[0].iter().copied());
            row.extend(tail);
            self.system_rows[self.sys_row] = row;
            self.sys_col += lines[0].len();
            return;
        }
        let paste_last_len = lines[n - 1].len();
        // 首行 = head + 粘贴首行
        let mut first = head;
        first.extend(lines[0].iter().copied());
        self.system_rows[self.sys_row] = first;
        // 中间行（1..n-1）整行插入
        for (i, mid) in lines[1..n - 1].iter().enumerate() {
            self.system_rows.insert(self.sys_row + 1 + i, mid.clone());
        }
        // 末行 = 粘贴末行 + tail
        let mut last = lines[n - 1].clone();
        last.extend(tail);
        self.system_rows.insert(self.sys_row + n - 1, last);
        // 光标移至插入内容末尾（末行、粘贴末段结尾）
        self.sys_row += n - 1;
        self.sys_col = paste_last_len;
    }

    /// 步骤①：字符追加过滤（重置选中首项）；Enter 选中前进；Esc 取消。
    /// j/k vim 别名仅在未输入过滤时生效（过滤态下 j/k 为输入字符，
    /// 保证 j/k 开头模型名可过滤；分支置于 Char(c) 泛配之前防吞）
    fn on_key_pick(&mut self, code: KeyCode) -> KeyAction {
        match code {
            KeyCode::Up | KeyCode::Char('k') if self.filter.is_empty() => {
                self.move_selection(-1);
                KeyAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') if self.filter.is_empty() => {
                self.move_selection(1);
                KeyAction::Continue
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.list_state.select(if self.filtered().is_empty() {
                    None
                } else {
                    Some(0)
                });
                KeyAction::Continue
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.list_state.select(if self.filtered().is_empty() {
                    None
                } else {
                    Some(0)
                });
                KeyAction::Continue
            }
            KeyCode::Enter => {
                if self.selected_from().is_some() {
                    self.step = Step::EditSystem;
                }
                KeyAction::Continue
            }
            KeyCode::Esc => KeyAction::Cancel,
            _ => KeyAction::Continue,
        }
    }

    /// 步骤① 选中项移动（越界钳制；列表空时无操作）
    fn move_selection(&mut self, delta: i32) {
        let len = self.filtered().len();
        if len == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, len as i32 - 1) as usize;
        self.list_state.select(Some(next));
    }

    /// 步骤②：多行编辑（插入/退格/换行/四向移动）；Enter 或 Ctrl+S 完成，
    /// Alt+Enter 手动换行，Esc 取消向导（M166 键位改造，Q1-A 裁决
    /// 2026-09-12 01:43——原 Enter=换行/Esc=完成与通用 TUI 直觉相反）。
    /// 各分支独立借用行数据（避免跨 match 长可变借用）
    fn on_key_system(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyAction {
        // 光标行兜底补齐（防御状态漂移：行号不越界）
        while self.system_rows.len() <= self.sys_row {
            self.system_rows.push(Vec::new());
        }
        match code {
            KeyCode::Char(c) if !mods.contains(KeyModifiers::CONTROL) => {
                self.system_rows[self.sys_row].insert(self.sys_col, c);
                self.sys_col += 1;
                KeyAction::Continue
            }
            KeyCode::Backspace => {
                if self.sys_col > 0 {
                    self.sys_col -= 1;
                    self.system_rows[self.sys_row].remove(self.sys_col);
                } else if self.sys_row > 0 {
                    // 行首退格：当前行并入前一行尾（标准编辑器语义）
                    let merged_len = self.system_rows[self.sys_row - 1].len();
                    let tail = self.system_rows.remove(self.sys_row);
                    self.system_rows[self.sys_row - 1].extend(tail);
                    self.sys_row -= 1;
                    self.sys_col = merged_len;
                }
                KeyAction::Continue
            }
            // Alt+Enter：光标处分裂换行（后半段成为新行）——手动换行通道
            KeyCode::Enter if mods.contains(KeyModifiers::ALT) => {
                let tail = self.system_rows[self.sys_row].split_off(self.sys_col);
                self.system_rows.insert(self.sys_row + 1, tail);
                self.sys_row += 1;
                self.sys_col = 0;
                KeyAction::Continue
            }
            // Enter：完成编辑进入下一步（M166 主出口）
            KeyCode::Enter => {
                self.step = Step::EditRuntime;
                KeyAction::Continue
            }
            KeyCode::Left if self.sys_col > 0 => {
                self.sys_col -= 1;
                KeyAction::Continue
            }
            KeyCode::Right if self.sys_col < self.system_rows[self.sys_row].len() => {
                self.sys_col += 1;
                KeyAction::Continue
            }
            KeyCode::Up if self.sys_row > 0 => {
                self.sys_row -= 1;
                self.sys_col = self.sys_col.min(self.system_rows[self.sys_row].len());
                KeyAction::Continue
            }
            KeyCode::Down if self.sys_row + 1 < self.system_rows.len() => {
                self.sys_row += 1;
                self.sys_col = self.sys_col.min(self.system_rows[self.sys_row].len());
                KeyAction::Continue
            }
            // Esc：取消整个向导（与步骤①④ 全局 Esc=取消语义对齐，M166）
            KeyCode::Esc => KeyAction::Cancel,
            // Ctrl+S 完成（Enter 之外的编辑器惯例出口，保留）
            KeyCode::Char('s') if mods.contains(KeyModifiers::CONTROL) => {
                self.step = Step::EditRuntime;
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    /// 步骤③：单行编辑；Enter/Esc 完成（空内容=跳过 RUNTIME 指令）
    fn on_key_runtime(&mut self, code: KeyCode) -> KeyAction {
        match code {
            KeyCode::Char(c) => {
                self.runtime_chars.insert(self.runtime_chars.len(), c);
                KeyAction::Continue
            }
            KeyCode::Backspace => {
                self.runtime_chars.pop();
                KeyAction::Continue
            }
            KeyCode::Enter | KeyCode::Esc => {
                self.step = Step::Preview;
                self.preview_scroll = 0; // M167：进入预览重置滚动
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    /// 步骤④：Enter 组装 Modelfile 确认创建；Esc 取消；M167 滚动键翻看长文
    fn on_key_preview(&mut self, code: KeyCode) -> KeyAction {
        match code {
            KeyCode::Enter => {
                let from = self.selected_from().unwrap_or_default().to_string();
                let system: Vec<String> = self
                    .system_rows
                    .iter()
                    .map(|r| r.iter().collect())
                    .collect();
                let runtime: String = self.runtime_chars.iter().collect();
                KeyAction::ConfirmCreate(build_modelfile(&from, &system, &runtime))
            }
            // M167：预览滚动（Up/Down 单行、PgUp/PgDn 十行；渲染层钳上限）
            KeyCode::Up | KeyCode::PageUp => {
                let step = if matches!(code, KeyCode::PageUp) {
                    10
                } else {
                    1
                };
                self.preview_scroll = self.preview_scroll.saturating_sub(step);
                KeyAction::Continue
            }
            KeyCode::Down | KeyCode::PageDown => {
                let step = if matches!(code, KeyCode::PageDown) {
                    10
                } else {
                    1
                };
                self.preview_scroll = self.preview_scroll.saturating_add(step);
                KeyAction::Continue
            }
            KeyCode::Esc => KeyAction::Cancel,
            _ => KeyAction::Continue,
        }
    }
}

/// 组装 Modelfile 文本（纯函数，单测锚点；产物与既有问答向导
/// `create_wizard` 逐字节同构——FROM/SYSTEM 三引号/RUNTIME 三指令形态，
/// 与服务端 modelfile parse 往返兼容）
///
/// - 参数 from：基础模型名（FROM 指令值，非空）
/// - 参数 system_lines：SYSTEM 多行内容（空集或全空行=省略指令）
/// - 参数 runtime：RUNTIME 参数（空串=省略指令）
/// - 返回：Modelfile 全文（行尾统一 \n）
fn build_modelfile(from: &str, system_lines: &[String], runtime: &str) -> String {
    let mut text = format!("FROM {from}\n");
    let has_system = system_lines.iter().any(|l| !l.is_empty());
    if has_system {
        let body = system_lines.join("\n");
        text.push_str(&format!("SYSTEM \"\"\"{body}\"\"\"\n"));
    }
    if !runtime.is_empty() {
        text.push_str(&format!("RUNTIME {runtime}\n"));
    }
    text
}

/// TUI 入口：初始化终端 → 事件循环 → 恢复终端。
///
/// - 参数 model：新模型名（标题展示）
/// - 参数 models：本地已装模型名列表（FROM 候选）
/// - 返回：Ok(Some(Modelfile 文本))=确认创建；Ok(None)=用户取消；
///   Err=终端初始化/渲染失败（调用方降级问答向导）
pub fn run(model: &str, models: Vec<String>) -> io::Result<Option<String>> {
    let mut terminal = try_init()?;
    let mut app = WizardApp::new(model, models);
    let result = event_loop(&mut terminal, &mut app);
    try_restore()?;
    result
}

/// 终端初始化（raw mode + 备用屏幕 + bracketed paste + panic hook 兜底
/// 恢复——TUI 中途 panic 时先还原终端再走原 panic 输出，避免现场花屏
/// 锁死）。M166：启用 bracketed paste 使多行粘贴作为整块 Paste 事件
/// 到达（换行符保留在内容里，不被终端拆解为 Enter 按键序列）
fn try_init() -> io::Result<ratatui::Terminal<CrosstermBackend<Stdout>>> {
    set_panic_hook();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(io::stdout());
    ratatui::Terminal::new(backend)
}

/// 终端恢复（先退 raw mode，再退 bracketed paste 与备用屏幕——
/// 副作用大的先收拾；panic hook 路径同样覆盖 paste 序列清理）
fn try_restore() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen)?;
    Ok(())
}

/// panic hook 包装：恢复终端状态后转交原 hook（保证 panic 信息正常打印）
fn set_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = try_restore();
        original(info);
    }));
}

/// 事件循环（阻塞式 event::read，无轮询空转；仅消费 Press 事件——
/// 部分终端对同一按键发 Press/Release 双事件，不过滤会双写）。
/// 具体后端类型（Crossterm Error=io::Error，? 直通；测试路径不经此函数）
fn event_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<Stdout>>,
    app: &mut WizardApp,
) -> io::Result<Option<String>> {
    loop {
        terminal.draw(|frame| draw(app, frame))?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match app.on_key(key.code, key.modifiers) {
                    KeyAction::Cancel => return Ok(None),
                    KeyAction::ConfirmCreate(text) => return Ok(Some(text)),
                    KeyAction::Continue => {}
                }
            }
            // M166：bracketed paste 整块多行内容（换行保留）
            Event::Paste(text) => app.on_paste(&text),
            _ => {}
        }
    }
}

/// 单帧渲染（纯读取 app 状态；编辑步骤同步设置可见光标位置）
///
/// - 参数 app：向导状态
/// - 参数 frame：ratatui 帧上下文
fn draw(app: &WizardApp, frame: &mut Frame) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(frame.area());

    // 标题区：新模型名 + 步骤指示（青色强调对齐 CLI 横幅配色习惯）
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "交互创建模型：",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.model.clone(),
            Style::default().fg(ratatui::style::Color::Cyan),
        ),
    ]))
    .block(Block::bordered());
    frame.render_widget(title, chunks[0]);

    // 主体区：按步骤渲染列表/编辑框/预览
    match app.step {
        Step::PickFrom => draw_pick_from(app, frame, chunks[1]),
        Step::EditSystem => draw_editor(app, frame, chunks[1], true),
        Step::EditRuntime => draw_editor(app, frame, chunks[1], false),
        Step::Preview => draw_preview(app, frame, chunks[1]),
    }

    // 帮助行（两行布局：步骤帮助 + RUNTIME 示例常驻）
    let mut help_lines = vec![Line::from(app.step.help())];
    if app.step == Step::EditRuntime {
        help_lines.push(Line::from(
            "示例：--flash-attn --override-tensor exps=CPU -ngl 30 --no-mmap --jinja",
        ));
    }
    let help = Paragraph::new(help_lines).block(Block::new());
    frame.render_widget(help, chunks[2]);
}

/// 步骤① 渲染：过滤后候选列表（高亮选中；空态/无匹配给明确提示不空白）
fn draw_pick_from(app: &WizardApp, frame: &mut Frame, area: ratatui::layout::Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(app.step.title());
    let filtered = app.filtered();
    if app.models.is_empty() {
        let empty = Paragraph::new("本地没有已装模型——先 roxid pull <模型> 再来创建").block(block);
        frame.render_widget(empty, area);
        return;
    }
    let items: Vec<ListItem> = filtered.iter().map(|m| ListItem::new(*m)).collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    if filtered.is_empty() {
        let none = Paragraph::new(format!("无匹配「{}」的模型", app.filter)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(app.step.title()),
        );
        frame.render_widget(none, area);
        return;
    }
    frame.render_stateful_widget(list, area, &mut app.list_state.clone());
}

/// 步骤②/③ 渲染：编辑内容 + 可见光标（行首缩进 1 空格留边）。
/// M167：多行内容超可视高度时按「底部对齐跟随光标」滚动——光标行恒
/// 可见（nano/vim 标准视口行为；视口由光标位置派生，无独立滚动状态）。
/// M168：光标列按显示宽折算（CJK 宽字符 2 列，复用 crate::display_width
/// 零新依赖）；Wrap 折行时 y 按折行段偏移（近似口径：显示宽 ÷ 内区宽
/// 均匀折算——Wrap 实际按词界断行，此近似较原字符列大幅改善）
fn draw_editor(app: &WizardApp, frame: &mut Frame, area: ratatui::layout::Rect, multiline: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(app.step.title());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // M167 视口：内容行数超可视高度时底部对齐光标行（可视窗口切片）
    let vis_h = inner.height as usize;
    let view = if multiline {
        app.sys_row
            .saturating_sub(vis_h.saturating_sub(1))
            .min(app.system_rows.len().saturating_sub(1))
    } else {
        0
    };
    let lines: Vec<Line> = if multiline {
        app.system_rows[view..]
            .iter()
            .map(|r| Line::from(format!(" {}", r.iter().collect::<String>())))
            .collect()
    } else {
        vec![Line::from(format!(
            " {}",
            app.runtime_chars.iter().collect::<String>()
        ))]
    };
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
    // M168 光标定位：可视行号 + 光标前缀显示宽（+1 为行首缩进列；
    // 折行段偏移 = 显示宽 ÷ 内区宽，列内偏移 = 显示宽 % 内区宽）
    let (vis_row, col_chars, row_chars) = if multiline {
        (
            app.sys_row - view,
            app.sys_col,
            app.system_rows[app.sys_row].clone(),
        )
    } else {
        (0usize, app.runtime_chars.len(), app.runtime_chars.clone())
    };
    let prefix: String = row_chars.iter().take(col_chars).collect();
    let pw = crate::display_width(&prefix) as u16;
    let w = inner.width.max(1);
    let y_off = pw / w;
    let x_off = (1 + pw % w).min(w - 1); // 含缩进列，越界钳制到行尾
    frame.set_cursor_position(Position {
        x: inner.x + x_off,
        y: inner.y + vis_row as u16 + y_off,
    });
}

/// 步骤④ 渲染：组装后 Modelfile 全文预览。
/// M167：preview_scroll 滚动偏移（上限钳制到总行数防过度滚动空白）
fn draw_preview(app: &WizardApp, frame: &mut Frame, area: ratatui::layout::Rect) {
    let from = app.selected_from().unwrap_or_default();
    let system: Vec<String> = app.system_rows.iter().map(|r| r.iter().collect()).collect();
    let runtime: String = app.runtime_chars.iter().collect();
    let text = build_modelfile(from, &system, &runtime);
    let max_scroll = text.lines().count().saturating_sub(1) as u16;
    let preview = Paragraph::new(text)
        .scroll((app.preview_scroll.min(max_scroll), 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(app.step.title()),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(preview, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 组装产物与既有问答向导逐字节同构（三指令全量形态）
    #[test]
    fn build_modelfile_matches_legacy_wizard_output() {
        let out = build_modelfile(
            "llama3.2:3b",
            &["你是助手".to_string(), "第二行".to_string()],
            "--flash-attn -ngl 30",
        );
        assert_eq!(
            out,
            "FROM llama3.2:3b\nSYSTEM \"\"\"你是助手\n第二行\"\"\"\nRUNTIME --flash-attn -ngl 30\n"
        );
    }

    /// 空段省略：SYSTEM 全空行不产出指令；RUNTIME 空串不产出指令
    #[test]
    fn build_modelfile_omits_empty_sections() {
        assert_eq!(build_modelfile("m:1b", &[], ""), "FROM m:1b\n");
        assert_eq!(
            build_modelfile("m:1b", &["".to_string(), "".to_string()], ""),
            "FROM m:1b\n",
            "全空行视为未填写 SYSTEM"
        );
        assert_eq!(
            build_modelfile("m:1b", &["提示".to_string()], ""),
            "FROM m:1b\nSYSTEM \"\"\"提示\"\"\"\n"
        );
    }

    /// 步骤① 过滤：前缀匹配 + 空过滤全量
    #[test]
    fn pick_from_filters_by_prefix() {
        let mut app = WizardApp::new(
            "test",
            vec![
                "llama3.2:3b".to_string(),
                "qwen3:4b".to_string(),
                "smollm2:135m".to_string(),
            ],
        );
        assert_eq!(app.filtered().len(), 3);
        app.filter.push('s');
        assert_eq!(app.filtered(), vec!["smollm2:135m"]);
        app.filter.push('x');
        assert!(app.filtered().is_empty(), "无匹配时零候选");
    }

    /// 步骤② 多行编辑核心操作：插入/退格跨行合并/换行分裂
    ///（M166 后换行通道为 Alt+Enter，Enter 为完成）
    #[test]
    fn system_editor_editing_ops() {
        let mut app = WizardApp::new("t", vec!["m".to_string()]);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // 选中进入步骤②
        for c in "ab".chars() {
            app.on_key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        app.on_key(KeyCode::Enter, KeyModifiers::ALT); // Alt+Enter 换行
        app.on_key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(
            app.system_rows
                .iter()
                .map(|r| r.iter().collect::<String>())
                .collect::<Vec<_>>(),
            vec!["ab".to_string(), "c".to_string()],
            "Alt+Enter 应在光标处分裂换行"
        );
        // 光标先移回行首（c 之后 Backspace 只是删字符，不触发合并）
        app.on_key(KeyCode::Left, KeyModifiers::NONE);
        app.on_key(KeyCode::Backspace, KeyModifiers::NONE); // 行首退格 → 并行
        assert_eq!(app.system_rows.len(), 1);
        assert_eq!(app.system_rows[0].iter().collect::<String>(), "abc");
    }

    /// M166：粘贴多行整块插入——换行保留、光标落插入末尾；单行粘贴合并不增行
    #[test]
    fn paste_multiline_inserts_rows_and_moves_cursor() {
        let mut app = WizardApp::new("t", vec!["m".to_string()]);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // ① 选中 → ②
        for c in "AB".chars() {
            app.on_key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        app.on_key(KeyCode::Enter, KeyModifiers::ALT); // 换行后输入 CD
        for c in "CD".chars() {
            app.on_key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        // 光标回第一行行中（A|B 处）粘贴三行
        app.on_key(KeyCode::Left, KeyModifiers::NONE);
        app.on_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!((app.sys_row, app.sys_col), (0, 1));
        app.on_paste("x\ny\nz");
        assert_eq!(
            app.system_rows
                .iter()
                .map(|r| r.iter().collect::<String>())
                .collect::<Vec<_>>(),
            vec![
                "Ax".to_string(),
                "y".to_string(),
                "zB".to_string(),
                "CD".to_string()
            ],
            "三行粘贴应分裂为：首行接 A、末行接 B、中间整行"
        );
        assert_eq!(
            (app.sys_row, app.sys_col),
            (2, 1),
            "光标须落在粘贴末行内容结尾"
        );
        // 单行粘贴（无换行）：合并不新增行
        app.on_paste("Q");
        assert_eq!(app.system_rows.len(), 4, "单行粘贴不新增行");
        assert_eq!(
            app.system_rows[2].iter().collect::<String>(),
            "zQB".to_string()
        );
        assert_eq!(app.sys_col, 2, "单行粘贴后光标在 Q 之后");
    }

    /// M166：EditSystem 键位——Enter 完成、Esc 取消向导
    #[test]
    fn edit_system_enter_completes_esc_cancels() {
        let mut app = WizardApp::new("t", vec!["m".to_string()]);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // ① 选中 → ②
        app.on_key(KeyCode::Char('你'), KeyModifiers::NONE);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // ② Enter 完成
        assert_eq!(app.step, Step::EditRuntime, "Enter 必须完成进入步骤③");
        // Ctrl+S 仍为完成出口
        let mut app2 = WizardApp::new("t", vec!["m".to_string()]);
        app2.on_key(KeyCode::Enter, KeyModifiers::NONE);
        app2.on_key(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(app2.step, Step::EditRuntime, "Ctrl+S 完成出口保留");
        // Esc 取消整个向导
        let mut app3 = WizardApp::new("t", vec!["m".to_string()]);
        app3.on_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            matches!(
                app3.on_key(KeyCode::Esc, KeyModifiers::NONE),
                KeyAction::Cancel
            ),
            "步骤② Esc 必须取消向导（与①④语义对齐）"
        );
    }

    /// 步骤流转：② Enter → ③ Enter → ④ Enter 组装确认（全链按键驱动，
    /// M166 键位改造后完成通道为 Enter）
    #[test]
    fn full_flow_steps_advance_to_confirm() {
        let mut app = WizardApp::new("my-model", vec!["llama3.2:3b".to_string()]);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // ① 选中
        assert_eq!(app.step, Step::EditSystem);
        app.on_key(KeyCode::Char('你'), KeyModifiers::NONE);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // ② 完成（M166：Enter）
        assert_eq!(app.step, Step::EditRuntime);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // ③ 跳过（空）
        assert_eq!(app.step, Step::Preview);
        match app.on_key(KeyCode::Enter, KeyModifiers::NONE) {
            KeyAction::ConfirmCreate(text) => {
                assert_eq!(text, "FROM llama3.2:3b\nSYSTEM \"\"\"你\"\"\"\n")
            }
            _ => panic!("预览步 Enter 必须返回 ConfirmCreate"),
        }
    }

    /// Ctrl+C 任意步骤取消
    #[test]
    fn ctrl_c_cancels_from_any_step() {
        let mut app = WizardApp::new("t", vec!["m".to_string()]);
        assert!(matches!(
            app.on_key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyAction::Cancel
        ));
    }

    /// TestBackend 快照：四步界面关键文案与候选可见（渲染不 panic 且内容正确）。
    /// 取 draw 返回的 CompletedFrame.buffer（draw 结束即 swap_buffers，
    /// backend 当前缓冲已切为下一空帧，不可从 backend 取）
    #[test]
    fn render_snapshots_show_step_content() {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 14))
            .expect("TestBackend 构造");
        let mut app = WizardApp::new(
            "my-model",
            vec!["llama3.2:3b".to_string(), "qwen3:4b".to_string()],
        );
        // 步骤①：标题与候选可见（CJK 断言走压缩口径——宽字符 skip cell 间断）
        let snap1 = snapshot_text(&mut term, &app);
        assert!(
            compact(&snap1).contains("选择基础模型"),
            "步骤① 快照：{snap1}"
        );
        assert!(snap1.contains("llama3.2:3b"));
        // 步骤④：预览含 FROM 行（M166 后步骤② 完成键为 Enter）
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.step, Step::Preview);
        let snap4 = snapshot_text(&mut term, &app);
        assert!(snap4.contains("FROM llama3.2:3b"), "步骤④ 快照：{snap4}");
        assert!(snap4.contains("my-model"));
    }

    /// M167：视口底部跟随——20 行内容在 14 行终端中光标行（末行）恒可见、
    /// 首行滚出视口（长提示词粘贴后底部内容可见的行为学断言）
    #[test]
    fn edit_system_viewport_follows_cursor() {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 14))
            .expect("TestBackend 构造");
        let mut app = WizardApp::new("t", vec!["m".to_string()]);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // ① 选中 → ②
        let paste: String = (0..20).map(|i| format!("行{i}\n")).collect();
        app.on_paste(&paste); // 20 行 + 末空行，光标落末空行
        let snap = snapshot_text(&mut term, &app);
        let flat = compact(&snap); // CJK 宽字符 skip cell 间断，压缩口径匹配
        assert!(flat.contains("行19"), "末行（光标行）必须在视口内：{snap}");
        assert!(!flat.contains("行0"), "首行应已滚出视口：{snap}");
    }

    /// M167：预览滚动键（Down 单行 / PgUp 十行 saturating / 超限渲染钳制）
    #[test]
    fn preview_scroll_keys_and_clamp() {
        let mut app = WizardApp::new("my-model", vec!["llama3.2:3b".to_string()]);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.step, Step::Preview);
        assert_eq!(app.preview_scroll, 0, "进入预览须重置滚动");
        app.on_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.preview_scroll, 1);
        app.on_key(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(app.preview_scroll, 11, "PgDn 十行步进");
        app.on_key(KeyCode::PageUp, KeyModifiers::NONE);
        app.on_key(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(app.preview_scroll, 0, "PgUp 越零 saturating 归零");
        // 渲染钳制：滚动超总行数时渲染不 panic（快照可重复产出）
        app.preview_scroll = 999;
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 14))
            .expect("TestBackend 构造");
        let _ = snapshot_text(&mut term, &app);
    }

    /// M168：CJK 光标显示宽折算——3 个汉字后光标 x 按显示宽 6 列而非
    /// 字符数 3 列（60x14 布局：主体 inner.x=1、首行 y=4）
    #[test]
    fn cursor_x_uses_display_width() {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 14))
            .expect("TestBackend 构造");
        let mut app = WizardApp::new("t", vec!["m".to_string()]);
        app.on_key(KeyCode::Enter, KeyModifiers::NONE); // ① 选中 → ②
        for c in "汉字字".chars() {
            app.on_key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        use ratatui::backend::Backend as _; // get_cursor_position 在 Backend trait
        term.draw(|f| draw(&app, f)).expect("draw");
        // ratatui 0.30：经 Backend::get_cursor_position（&mut）取回置位坐标
        let pos = term
            .backend_mut()
            .get_cursor_position()
            .expect("光标必须置位");
        assert_eq!(pos.x, 1 + 1 + 6, "CJK 光标列须按显示宽 2 列/字折算");
        assert_eq!(pos.y, 4, "单行内容光标在主体区首行（y=4）");
    }

    /// 去空白压缩（TestBackend 宽字符 cell 后跟空格 skip cell，
    /// 中文子串匹配需压缩后进行）
    fn compact(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// 渲染一帧并拼接完成帧全文（逐 Cell 符号串联；行内连续子串断言有效）
    fn snapshot_text(
        term: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
        app: &WizardApp,
    ) -> String {
        term.draw(|f| draw(app, f))
            .expect("TestBackend draw")
            .buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect()
    }
}
