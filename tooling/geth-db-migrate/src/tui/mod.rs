pub mod app;
pub mod event;
pub mod ui;

use std::{io, time::Duration};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use self::{app::MigrationApp, event::ProgressEvent};

const TICK_MS: u64 = 50;

/// Sets up the panic hook to restore the terminal even on unexpected panics.
fn setup_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort terminal restore
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}

/// Initializes the terminal for TUI rendering.
fn enter_tui() -> io::Result<Terminal<CrosstermBackend<io::Stderr>>> {
    enable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
    Terminal::new(CrosstermBackend::new(io::stderr()))
}

/// Restores the terminal to its original state.
fn leave_tui(terminal: &mut Terminal<CrosstermBackend<io::Stderr>>) {
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = terminal.show_cursor();
}

/// Runs the TUI render loop. Receives `ProgressEvent`s from the migration loop
/// and redraws the dashboard on every tick or event.
///
/// Exits when:
/// - The sender is dropped (migration finished) and the user presses `q` or `Ctrl+C`
/// - `Ctrl+C` is pressed at any time
pub async fn run_tui(mut rx: mpsc::Receiver<ProgressEvent>) {
    setup_panic_hook();

    let mut terminal = match enter_tui() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("TUI 초기화 실패: {e}");
            return;
        }
    };

    let mut app = MigrationApp::new();
    let mut event_stream = EventStream::new();
    let tick = Duration::from_millis(TICK_MS);

    loop {
        // Draw frame
        if let Err(e) = terminal.draw(|f| ui::draw(f, &app)) {
            eprintln!("TUI 렌더 오류: {e}");
            break;
        }

        tokio::select! {
            // Migration progress event
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(ev) => app.handle_event(ev),
                    None => {
                        // Channel closed — migration finished. Wait for 'q'.
                        if !app.is_finished() {
                            app.handle_event(ProgressEvent::Error {
                                message: "채널이 예기치 않게 닫혔습니다.".into(),
                            });
                        }
                        // Redraw final state then wait for quit key.
                        let _ = terminal.draw(|f| ui::draw(f, &app));
                        wait_for_quit(&mut terminal, &mut event_stream).await;
                        break;
                    }
                }
            }

            // Keyboard event
            maybe_crossterm = event_stream.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_crossterm {
                    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        break;
                    }
                    if app.is_finished() && matches!(key.code, KeyCode::Char('q') | KeyCode::Enter) {
                        break;
                    }
                }
            }

            // Periodic tick to refresh elapsed time display
            _ = tokio::time::sleep(tick) => {}
        }
    }

    leave_tui(&mut terminal);
}

/// Blocks until the user presses 'q', Enter, or Ctrl+C.
async fn wait_for_quit(
    _terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    event_stream: &mut EventStream,
) {
    loop {
        // Don't redraw — the final frame is already on screen and persists.
        if let Some(Ok(Event::Key(key))) = event_stream.next().await {
            match key.code {
                KeyCode::Char('q') | KeyCode::Enter => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                _ => {}
            }
        }
    }
}
