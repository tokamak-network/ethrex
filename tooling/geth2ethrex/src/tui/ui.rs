use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
};

use super::app::{MigrationApp, MigrationStatus, format_duration};

pub fn draw(frame: &mut Frame, app: &MigrationApp) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // 제목
        Constraint::Length(4), // 경로 정보
        Constraint::Length(3), // 프로그레스 게이지
        Constraint::Length(3), // 속도/ETA/경과
        Constraint::Length(3), // 배치/재시도/스킵 통계
        Constraint::Min(5),    // 로그
        Constraint::Length(1), // 도움말
    ])
    .split(area);

    draw_title(frame, app, chunks[0]);
    draw_info(frame, app, chunks[1]);
    draw_gauge(frame, app, chunks[2]);
    draw_speed(frame, app, chunks[3]);
    draw_stats(frame, app, chunks[4]);
    draw_log(frame, app, chunks[5]);
    draw_help(frame, app, chunks[6]);
}

fn draw_title(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let (status_label, status_color) = match app.status {
        MigrationStatus::Waiting => ("대기 중", Color::Yellow),
        MigrationStatus::Running => ("실행 중", Color::Green),
        MigrationStatus::Completed => ("완료", Color::Cyan),
        MigrationStatus::Failed => ("오류", Color::Red),
    };

    let title = Line::from(vec![
        Span::styled(
            "geth2ethrex 마이그레이션",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[{status_label}]"),
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let block = Block::default().borders(Borders::ALL);
    let paragraph = Paragraph::new(title).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_info(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let db_type = if app.db_type.is_empty() {
        "감지 중...".to_string()
    } else {
        app.db_type.clone()
    };

    let source = if app.source_path.is_empty() {
        "-".to_string()
    } else {
        truncate_path(&app.source_path, 60)
    };

    let target = if app.target_path.is_empty() {
        "-".to_string()
    } else {
        truncate_path(&app.target_path, 60)
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("소스: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&source),
            Span::raw("  "),
            Span::styled(format!("[{db_type}]"), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("대상: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&target),
        ]),
        Line::from(vec![
            Span::styled("범위: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(if app.end_block == 0 {
                "계획 중...".to_string()
            } else {
                format!(
                    "#{} ~ #{}",
                    fmt_num(app.start_block),
                    fmt_num(app.end_block)
                )
            }),
        ]),
    ];

    let block = Block::default().borders(Borders::ALL).title("경로 정보");
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_gauge(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let ratio = app.progress_ratio();
    let pct = (ratio * 100.0) as u16;

    let label = if app.end_block == 0 {
        "대기 중...".to_string()
    } else {
        format!(
            "블록 #{} / #{} ({pct}%)",
            fmt_num(app.current_block),
            fmt_num(app.end_block)
        )
    };

    let gauge_color = match app.status {
        MigrationStatus::Completed => Color::Cyan,
        MigrationStatus::Failed => Color::Red,
        _ => Color::Green,
    };

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("진행률"))
        .gauge_style(Style::default().fg(gauge_color))
        .ratio(ratio)
        .label(label);

    frame.render_widget(gauge, area);
}

fn draw_speed(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let speed_str = if app.blocks_per_sec > 0.0 {
        format!("{:.0} 블록/초", app.blocks_per_sec)
    } else {
        "-".to_string()
    };

    let eta_str = match app.eta {
        Some(d) if d == std::time::Duration::ZERO && app.status == MigrationStatus::Completed => {
            "완료".to_string()
        }
        Some(d) => format!("약 {}", format_duration(d)),
        None => "-".to_string(),
    };

    let elapsed_str = format_duration(app.elapsed);

    let line = Line::from(vec![
        Span::styled("속도: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&speed_str),
        Span::raw("    "),
        Span::styled("ETA: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&eta_str),
        Span::raw("    "),
        Span::styled("경과: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&elapsed_str),
    ]);

    let block = Block::default().borders(Borders::ALL).title("속도");
    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_stats(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let line = Line::from(vec![
        Span::styled("배치: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("{}/{}", app.batch_number, app.total_batches)),
        Span::raw("    "),
        Span::styled(
            "가져온 블록: ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(fmt_num(app.imported_blocks)),
        Span::raw("    "),
        Span::styled("재시도: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            app.retries_performed.to_string(),
            Style::default().fg(if app.retries_performed > 0 {
                Color::Yellow
            } else {
                Color::Reset
            }),
        ),
        Span::raw("    "),
        Span::styled("스킵: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            app.skipped_blocks.to_string(),
            Style::default().fg(if app.skipped_blocks > 0 {
                Color::Yellow
            } else {
                Color::Reset
            }),
        ),
    ]);

    let block = Block::default().borders(Borders::ALL).title("통계");
    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_log(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let items: Vec<ListItem> = app
        .log_lines
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .map(|line| ListItem::new(Line::from(line.as_str())))
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title("로그 (최신순)");
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn draw_help(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let text = if app.is_finished() {
        "  q: 종료"
    } else {
        "  Ctrl+C: 중단 (터미널 자동 복원)"
    };

    let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

fn fmt_num(n: u64) -> String {
    // Simple thousands-separator formatter
    let s = n.to_string();
    let chars: Vec<char> = s.chars().rev().collect();
    let with_sep: String = chars
        .chunks(3)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(",");
    with_sep.chars().rev().collect()
}

fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_string();
    }
    let half = (max_len.saturating_sub(3)) / 2;
    // Use char boundaries to avoid panics on multi-byte UTF-8 paths
    let start_end = path
        .char_indices()
        .nth(half)
        .map(|(i, _)| i)
        .unwrap_or(path.len());
    let suffix_start = path
        .char_indices()
        .rev()
        .nth(half.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("{}...{}", &path[..start_end], &path[suffix_start..])
}
