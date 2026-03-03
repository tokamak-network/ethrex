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
        Constraint::Length(3), // Title
        Constraint::Length(4), // Path info
        Constraint::Length(3), // Progress gauge
        Constraint::Length(3), // Speed / ETA / elapsed
        Constraint::Length(3), // Batch/retry/skip stats
        Constraint::Min(5),    // Log
        Constraint::Length(1), // Help
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
        MigrationStatus::Waiting => ("Waiting", Color::Yellow),
        MigrationStatus::Running => ("Running", Color::Green),
        MigrationStatus::Completed => ("Completed", Color::Cyan),
        MigrationStatus::Failed => ("Failed", Color::Red),
    };

    let title = Line::from(vec![
        Span::styled(
            "Geth DB Migration",
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
        "Detecting...".to_string()
    } else {
        app.db_type.clone()
    };

    // Aggressive truncation: fit to available width minus labels
    let available_width = area.width.saturating_sub(20) as usize;

    let source = if app.source_path.is_empty() {
        "-".to_string()
    } else {
        truncate_path(&app.source_path, available_width.min(50))
    };

    let target = if app.target_path.is_empty() {
        "-".to_string()
    } else {
        truncate_path(&app.target_path, available_width.min(50))
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Source: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&source),
            Span::raw("  "),
            Span::styled(format!("[{db_type}]"), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Target: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&target),
        ]),
        Line::from(vec![
            Span::styled("Range: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(if app.end_block == 0 {
                "Planning...".to_string()
            } else {
                format!(
                    "#{} ~ #{}",
                    fmt_num(app.start_block),
                    fmt_num(app.end_block)
                )
            }),
        ]),
    ];

    let block = Block::default().borders(Borders::ALL).title("Paths");
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_gauge(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let ratio = app.progress_ratio();
    let pct = (ratio * 100.0) as u16;

    let label = if app.end_block == 0 {
        "Waiting...".to_string()
    } else {
        format!(
            "Block #{} / #{} ({pct}%)",
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
        .block(Block::default().borders(Borders::ALL).title("Progress"))
        .gauge_style(Style::default().fg(gauge_color))
        .ratio(ratio)
        .label(label);

    frame.render_widget(gauge, area);
}

fn draw_speed(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let speed_str = if app.blocks_per_sec > 0.0 {
        format!("{:.0} blocks/s", app.blocks_per_sec)
    } else {
        "-".to_string()
    };

    let eta_str = match app.eta {
        Some(d) if d == std::time::Duration::ZERO && app.status == MigrationStatus::Completed => {
            "Done".to_string()
        }
        Some(d) => format!("~{}", format_duration(d)),
        None => "-".to_string(),
    };

    let elapsed_str = format_duration(app.elapsed);

    let line = Line::from(vec![
        Span::styled("Speed: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&speed_str),
        Span::raw("    "),
        Span::styled("ETA: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&eta_str),
        Span::raw("    "),
        Span::styled("Elapsed: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&elapsed_str),
    ]);

    let block = Block::default().borders(Borders::ALL).title("Speed");
    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_stats(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let line = Line::from(vec![
        Span::styled("Batch: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("{}/{}", app.batch_number, app.total_batches)),
        Span::raw("    "),
        Span::styled(
            "Imported: ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(fmt_num(app.imported_blocks)),
        Span::raw("    "),
        Span::styled("Retries: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            app.retries_performed.to_string(),
            Style::default().fg(if app.retries_performed > 0 {
                Color::Yellow
            } else {
                Color::Reset
            }),
        ),
        Span::raw("    "),
        Span::styled("Skipped: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            app.skipped_blocks.to_string(),
            Style::default().fg(if app.skipped_blocks > 0 {
                Color::Yellow
            } else {
                Color::Reset
            }),
        ),
    ]);

    let block = Block::default().borders(Borders::ALL).title("Stats");
    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_log(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let width = area.width.saturating_sub(3) as usize; // Account for borders
    let max_lines = area.height.saturating_sub(2) as usize;

    // Filter logs: if multiple "Account batch" logs exist, keep only the latest
    let mut filtered_logs: Vec<String> = Vec::new();
    let mut has_account_batch = false;

    for line in app.log_lines.iter().rev() {
        if line.contains("[state] Account batch:") {
            // Skip duplicate account batch logs, keep only first (latest) one
            if has_account_batch {
                continue;
            }
            has_account_batch = true;
        }
        filtered_logs.push(line.clone());
        if filtered_logs.len() >= max_lines {
            break;
        }
    }

    // Truncate long lines to fit display width
    let items: Vec<ListItem> = filtered_logs
        .iter()
        .map(|line| {
            let truncated = if line.len() > width {
                format!("{}…", &line[..width.saturating_sub(1)])
            } else {
                line.clone()
            };
            ListItem::new(Line::from(truncated))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Log (newest first)");
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn draw_help(frame: &mut Frame, app: &MigrationApp, area: Rect) {
    let text = if app.is_finished() {
        "  q: quit"
    } else {
        "  Ctrl+C: stop (terminal auto-restores)"
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
