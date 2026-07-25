use std::{collections::VecDeque, io, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use bollard::Docker;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::{
    docker::{self, BuildRequest, ImageChoice, Progress},
    magisk,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Images,
    Target,
}

#[derive(Debug, Clone)]
enum Status {
    Idle,
    Building,
    Success(String),
    Error(String),
}

struct App {
    images: Vec<ImageChoice>,
    selected: usize,
    target: String,
    focus: Focus,
    status: Status,
    logs: VecDeque<String>,
    apk_path: Option<PathBuf>,
    sender: UnboundedSender<Progress>,
    receiver: UnboundedReceiver<Progress>,
}

impl App {
    fn new(images: Vec<ImageChoice>, apk_path: Option<PathBuf>) -> Self {
        let target = images
            .first()
            .map(|image| docker::default_target(&image.reference))
            .unwrap_or_default();
        let (sender, receiver) = unbounded_channel();
        Self {
            images,
            selected: 0,
            target,
            focus: Focus::Images,
            status: Status::Idle,
            logs: VecDeque::new(),
            apk_path,
            sender,
            receiver,
        }
    }

    fn selected_image(&self) -> Option<&ImageChoice> {
        self.images.get(self.selected)
    }

    fn select_delta(&mut self, delta: isize) {
        if self.images.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.images.len() - 1);
        if let Some(image) = self.selected_image() {
            self.target = docker::default_target(&image.reference);
        }
    }

    fn push_log(&mut self, line: impl Into<String>) {
        if self.logs.len() == 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(line.into());
    }

    fn poll_progress(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Progress::Log(line) => self.push_log(line),
                Progress::Finished(target) => {
                    self.push_log(format!("完成：{target}"));
                    self.status = Status::Success(target);
                }
                Progress::Failed(message) => {
                    self.push_log(format!("错误：{message}"));
                    self.status = Status::Error(message);
                }
            }
        }
    }

    fn start_build(&mut self, docker: Docker) {
        if matches!(self.status, Status::Building) {
            return;
        }
        let Some(image) = self.selected_image().cloned() else {
            self.status = Status::Error("没有可用的 redroid 原版镜像".into());
            return;
        };
        if let Err(error) = docker::validate_image_reference(&self.target) {
            self.status = Status::Error(error.to_string());
            return;
        }

        self.logs.clear();
        self.status = Status::Building;
        docker::spawn_build(
            docker,
            BuildRequest {
                base: image.reference,
                architecture: image.architecture,
                target: self.target.clone(),
                apk_path: self.apk_path.clone(),
            },
            self.sender.clone(),
        );
    }
}

pub async fn run(
    docker: Docker,
    images: Vec<ImageChoice>,
    apk_path: Option<PathBuf>,
) -> Result<()> {
    enable_raw_mode().context("启用终端 raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("进入终端备用屏幕")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("初始化终端")?;

    let result = run_loop(&mut terminal, docker, App::new(images, apk_path)).await;

    disable_raw_mode().context("退出终端 raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen).context("退出终端备用屏幕")?;
    terminal.show_cursor().context("恢复光标")?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    docker: Docker,
    mut app: App,
) -> Result<()> {
    loop {
        app.poll_progress();
        terminal.draw(|frame| draw(frame, &app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
            || (key.code == KeyCode::Char('q') && app.focus == Focus::Images)
        {
            break;
        }

        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                app.focus = match app.focus {
                    Focus::Images => Focus::Target,
                    Focus::Target => Focus::Images,
                };
            }
            KeyCode::Up if app.focus == Focus::Images => app.select_delta(-1),
            KeyCode::Down if app.focus == Focus::Images => app.select_delta(1),
            KeyCode::Home if app.focus == Focus::Images => {
                app.select_delta(-(app.selected as isize))
            }
            KeyCode::End if app.focus == Focus::Images => {
                app.select_delta(app.images.len() as isize)
            }
            KeyCode::Backspace if app.focus == Focus::Target => {
                app.target.pop();
            }
            KeyCode::Char(character)
                if app.focus == Focus::Target && !matches!(app.status, Status::Building) =>
            {
                app.target.push(character);
            }
            KeyCode::Enter => app.start_build(docker.clone()),
            _ => {}
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Percentage(42),
            Constraint::Length(5),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " redroid-helper ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("· 生成集成 Magisk 的 redroid 派生镜像"),
    ]))
    .block(Block::default().borders(Borders::ALL))
    .alignment(Alignment::Center);
    frame.render_widget(title, rows[0]);

    draw_images(frame, rows[1], app);
    draw_target(frame, rows[2], app);
    draw_logs(frame, rows[3], app);

    let help = Paragraph::new("↑/↓ 选择  Tab 切换  Enter 构建  Esc/q 退出")
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, rows[4]);
}

fn draw_images(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.focus == Focus::Images;
    let border = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let title = format!(" 1. 选择本地原版 redroid 镜像（{} 个） ", app.images.len());

    if app.images.is_empty() {
        let empty = Paragraph::new(
            "Docker 中没有发现 redroid/redroid:* 原版镜像。\n\n请先运行：docker pull redroid/redroid:16.0.0-latest",
        )
        .block(Block::default().title(title).borders(Borders::ALL).border_style(Style::default().fg(border)))
        .wrap(Wrap { trim: true });
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<_> = app
        .images
        .iter()
        .map(|image| {
            ListItem::new(Line::from(vec![
                Span::styled(&image.reference, Style::default().fg(Color::White)),
                Span::styled(
                    format!("  {}  {}", image.architecture, format_size(image.size)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border)),
        )
        .highlight_symbol("▶ ")
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_target(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.focus == Focus::Target;
    let border = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let source = if app.apk_path.is_some() {
        "本地 APK"
    } else {
        "校验后下载"
    };
    let text = vec![
        Line::from(app.target.as_str()),
        Line::styled(
            format!("Magisk {} · {source}", magisk::VERSION),
            Style::default().fg(Color::DarkGray),
        ),
    ];
    let input = Paragraph::new(text).block(
        Block::default()
            .title(" 2. 输出镜像标签 ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border)),
    );
    frame.render_widget(input, area);
    if focused {
        let cursor_x = area.x
            + 1
            + app
                .target
                .chars()
                .count()
                .min(area.width.saturating_sub(3) as usize) as u16;
        frame.set_cursor_position((cursor_x, area.y + 1));
    }
}

fn draw_logs(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let (title, color) = match &app.status {
        Status::Idle => (" 3. 构建日志 · 按 Enter 开始 ".to_string(), Color::DarkGray),
        Status::Building => (" 3. 正在构建… ".to_string(), Color::Yellow),
        Status::Success(target) => (format!(" ✓ 已生成 {target} "), Color::Green),
        Status::Error(message) => (format!(" ✗ {message} "), Color::Red),
    };
    let height = area.height.saturating_sub(2) as usize;
    let lines: Vec<_> = app
        .logs
        .iter()
        .skip(app.logs.len().saturating_sub(height))
        .map(|line| Line::raw(line.as_str()))
        .collect();
    let logs = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(logs, area);
}

fn format_size(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GiB", bytes / 1024.0 / 1024.0 / 1024.0)
    } else {
        format!("{:.0} MiB", bytes / 1024.0 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_image_sizes() {
        assert_eq!(format_size(512 * 1024 * 1024), "512 MiB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.0 GiB");
    }
}
