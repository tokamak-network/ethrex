use std::{collections::VecDeque, time::Duration};

use super::event::ProgressEvent;

const LOG_CAPACITY: usize = 100;
const EMA_ALPHA: f64 = 0.3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationStatus {
    Waiting,
    Running,
    Completed,
    Failed,
}

pub struct MigrationApp {
    // Static info (set on Init)
    pub source_path: String,
    pub target_path: String,
    pub db_type: String,
    pub start_block: u64,
    pub end_block: u64,

    // Dynamic state
    pub status: MigrationStatus,
    pub current_block: u64,
    pub imported_blocks: u64,
    pub skipped_blocks: u64,
    pub retries_performed: u32,
    pub batch_number: u64,
    pub total_batches: u64,

    // Timing
    pub elapsed: Duration,

    // Speed (blocks/sec, EMA-smoothed)
    pub blocks_per_sec: f64,
    last_batch_blocks: u64,
    last_batch_elapsed: Duration,

    // Derived
    pub eta: Option<Duration>,

    // Log lines (capped)
    pub log_lines: VecDeque<String>,

    // Set once to signal the TUI to wait for 'q'
    pub final_message: Option<String>,
}

impl MigrationApp {
    pub fn new() -> Self {
        Self {
            source_path: String::new(),
            target_path: String::new(),
            db_type: String::new(),
            start_block: 0,
            end_block: 0,
            status: MigrationStatus::Waiting,
            current_block: 0,
            imported_blocks: 0,
            skipped_blocks: 0,
            retries_performed: 0,
            batch_number: 0,
            total_batches: 0,
            elapsed: Duration::ZERO,
            blocks_per_sec: 0.0,
            last_batch_blocks: 0,
            last_batch_elapsed: Duration::ZERO,
            eta: None,
            log_lines: VecDeque::with_capacity(LOG_CAPACITY),
            final_message: None,
        }
    }

    pub fn handle_event(&mut self, event: ProgressEvent) {
        match event {
            ProgressEvent::Init {
                source_path,
                target_path,
                db_type,
                start_block,
                end_block,
            } => {
                self.source_path = source_path;
                self.target_path = target_path;
                self.db_type = db_type;
                self.start_block = start_block;
                self.end_block = end_block;
                let total = end_block.saturating_sub(start_block) + 1;
                let batch_size = 1_000u64;
                self.total_batches = total.div_ceil(batch_size);
                self.current_block = start_block.saturating_sub(1);
                self.status = MigrationStatus::Running;
                self.push_log(format!(
                    "마이그레이션 시작: #{start_block}..=#{end_block} ({total} 블록)"
                ));
            }

            ProgressEvent::BatchCompleted {
                batch_number,
                total_batches,
                current_block,
                blocks_in_batch,
                elapsed,
            } => {
                // Update EMA speed
                let delta_elapsed = elapsed.saturating_sub(self.last_batch_elapsed);
                let delta_secs = delta_elapsed.as_secs_f64();
                if delta_secs > 0.0 {
                    let instant_speed = blocks_in_batch as f64 / delta_secs;
                    if self.blocks_per_sec == 0.0 {
                        self.blocks_per_sec = instant_speed;
                    } else {
                        self.blocks_per_sec =
                            EMA_ALPHA * instant_speed + (1.0 - EMA_ALPHA) * self.blocks_per_sec;
                    }
                }

                self.last_batch_blocks = blocks_in_batch;
                self.last_batch_elapsed = elapsed;
                self.batch_number = batch_number;
                self.total_batches = total_batches;
                self.current_block = current_block;
                self.imported_blocks += blocks_in_batch;
                self.elapsed = elapsed;

                // Compute ETA
                let remaining = self.end_block.saturating_sub(current_block);
                self.eta = if self.blocks_per_sec > 0.0 {
                    let secs = remaining as f64 / self.blocks_per_sec;
                    Some(Duration::from_secs_f64(secs))
                } else {
                    None
                };

                self.push_log(format!(
                    "배치 #{batch_number}/{total_batches} 완료 ({blocks_in_batch} 블록, 현재 #{current_block})"
                ));
            }

            ProgressEvent::BlockSkipped {
                block_number,
                reason,
            } => {
                self.skipped_blocks += 1;
                self.push_log(format!("경고: 블록 #{block_number} 스킵 — {reason}"));
            }

            ProgressEvent::Completed {
                imported_blocks,
                skipped_blocks,
                elapsed,
                retries_performed,
            } => {
                self.status = MigrationStatus::Completed;
                self.imported_blocks = imported_blocks;
                self.skipped_blocks = skipped_blocks;
                self.elapsed = elapsed;
                self.retries_performed = retries_performed;
                self.eta = Some(Duration::ZERO);
                let msg = format!(
                    "완료! {imported_blocks} 블록 마이그레이션 (스킵: {skipped_blocks}, 소요: {})",
                    format_duration(elapsed)
                );
                self.push_log(msg.clone());
                self.final_message = Some(msg);
            }

            ProgressEvent::Error { message } => {
                self.status = MigrationStatus::Failed;
                let msg = format!("오류: {message}");
                self.push_log(msg.clone());
                self.final_message = Some(msg);
            }
        }
    }

    pub fn progress_ratio(&self) -> f64 {
        let total = self.end_block.saturating_sub(self.start_block) + 1;
        if total == 0 {
            return 0.0;
        }
        let done = self
            .current_block
            .saturating_sub(self.start_block.saturating_sub(1));
        (done as f64 / total as f64).clamp(0.0, 1.0)
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            MigrationStatus::Completed | MigrationStatus::Failed
        )
    }

    fn push_log(&mut self, line: String) {
        if self.log_lines.len() >= LOG_CAPACITY {
            self.log_lines.pop_front();
        }
        self.log_lines.push_back(line);
    }
}

pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}초")
    } else if secs < 3600 {
        format!("{}분 {}초", secs / 60, secs % 60)
    } else {
        format!("{}시간 {}분", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_init_event() -> ProgressEvent {
        ProgressEvent::Init {
            source_path: "/data/geth/chaindata".into(),
            target_path: "/data/ethrex".into(),
            db_type: "Pebble".into(),
            start_block: 1,
            end_block: 5000,
        }
    }

    #[test]
    fn new_app_starts_in_waiting_status() {
        let app = MigrationApp::new();
        assert_eq!(app.status, MigrationStatus::Waiting);
        assert!(!app.is_finished());
        assert_eq!(app.progress_ratio(), 0.0);
        assert!(app.log_lines.is_empty());
    }

    #[test]
    fn init_event_transitions_to_running() {
        let mut app = MigrationApp::new();
        app.handle_event(make_init_event());

        assert_eq!(app.status, MigrationStatus::Running);
        assert_eq!(app.source_path, "/data/geth/chaindata");
        assert_eq!(app.target_path, "/data/ethrex");
        assert_eq!(app.db_type, "Pebble");
        assert_eq!(app.start_block, 1);
        assert_eq!(app.end_block, 5000);
        assert_eq!(app.total_batches, 5); // 5000 / 1000
        assert_eq!(app.current_block, 0); // start_block - 1
        assert!(!app.is_finished());
        assert_eq!(app.log_lines.len(), 1);
    }

    #[test]
    fn batch_completed_updates_progress() {
        let mut app = MigrationApp::new();
        app.handle_event(make_init_event());

        app.handle_event(ProgressEvent::BatchCompleted {
            batch_number: 1,
            total_batches: 5,
            current_block: 1000,
            blocks_in_batch: 1000,
            elapsed: Duration::from_secs(10),
        });

        assert_eq!(app.batch_number, 1);
        assert_eq!(app.current_block, 1000);
        assert_eq!(app.imported_blocks, 1000);
        assert!(app.blocks_per_sec > 0.0);
        assert!(app.eta.is_some());
        assert!(!app.is_finished());

        // Progress should be ~20% (1000/5000)
        let ratio = app.progress_ratio();
        assert!(ratio > 0.19 && ratio < 0.21, "ratio={ratio}");
    }

    #[test]
    fn multiple_batches_accumulate_correctly() {
        let mut app = MigrationApp::new();
        app.handle_event(make_init_event());

        for i in 1..=5 {
            app.handle_event(ProgressEvent::BatchCompleted {
                batch_number: i,
                total_batches: 5,
                current_block: i * 1000,
                blocks_in_batch: 1000,
                elapsed: Duration::from_secs(i * 2),
            });
        }

        assert_eq!(app.imported_blocks, 5000);
        assert_eq!(app.batch_number, 5);
        assert_eq!(app.current_block, 5000);

        // ETA should be near zero at 100%
        let ratio = app.progress_ratio();
        assert!((ratio - 1.0).abs() < 0.01, "ratio={ratio}");
    }

    #[test]
    fn block_skipped_increments_counter() {
        let mut app = MigrationApp::new();
        app.handle_event(make_init_event());

        app.handle_event(ProgressEvent::BlockSkipped {
            block_number: 42,
            reason: "missing canonical hash".into(),
        });
        app.handle_event(ProgressEvent::BlockSkipped {
            block_number: 99,
            reason: "corrupted body".into(),
        });

        assert_eq!(app.skipped_blocks, 2);
        assert_eq!(app.status, MigrationStatus::Running);
    }

    #[test]
    fn completed_event_finalizes() {
        let mut app = MigrationApp::new();
        app.handle_event(make_init_event());

        app.handle_event(ProgressEvent::Completed {
            imported_blocks: 4998,
            skipped_blocks: 2,
            elapsed: Duration::from_secs(120),
            retries_performed: 1,
        });

        assert_eq!(app.status, MigrationStatus::Completed);
        assert!(app.is_finished());
        assert_eq!(app.imported_blocks, 4998);
        assert_eq!(app.skipped_blocks, 2);
        assert_eq!(app.retries_performed, 1);
        assert_eq!(app.eta, Some(Duration::ZERO));
        assert!(app.final_message.is_some());
    }

    #[test]
    fn error_event_sets_failed() {
        let mut app = MigrationApp::new();
        app.handle_event(make_init_event());

        app.handle_event(ProgressEvent::Error {
            message: "disk full".into(),
        });

        assert_eq!(app.status, MigrationStatus::Failed);
        assert!(app.is_finished());
        assert!(app.final_message.as_ref().unwrap().contains("disk full"));
    }

    #[test]
    fn log_capacity_is_bounded() {
        let mut app = MigrationApp::new();

        for i in 0..150 {
            app.push_log(format!("line {i}"));
        }

        assert_eq!(app.log_lines.len(), LOG_CAPACITY);
        // Oldest lines should have been evicted
        assert!(app.log_lines.front().unwrap().contains("line 50"));
        assert!(app.log_lines.back().unwrap().contains("line 149"));
    }

    #[test]
    fn progress_ratio_handles_edge_cases() {
        let mut app = MigrationApp::new();
        // Before init: 0.0
        assert_eq!(app.progress_ratio(), 0.0);

        // Single block range
        app.handle_event(ProgressEvent::Init {
            source_path: "s".into(),
            target_path: "t".into(),
            db_type: "P".into(),
            start_block: 100,
            end_block: 100,
        });
        // current_block = 99, total = 1
        assert_eq!(app.progress_ratio(), 0.0);

        app.current_block = 100;
        assert!((app.progress_ratio() - 1.0).abs() < 0.01);
    }

    #[test]
    fn format_duration_displays_correctly() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0초");
        assert_eq!(format_duration(Duration::from_secs(45)), "45초");
        assert_eq!(format_duration(Duration::from_secs(90)), "1분 30초");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1시간 1분");
    }

    #[test]
    fn ema_speed_smoothing_works() {
        let mut app = MigrationApp::new();
        app.handle_event(make_init_event());

        // First batch: speed initialized directly
        app.handle_event(ProgressEvent::BatchCompleted {
            batch_number: 1,
            total_batches: 5,
            current_block: 1000,
            blocks_in_batch: 1000,
            elapsed: Duration::from_secs(1),
        });
        let speed1 = app.blocks_per_sec;
        assert!(speed1 > 900.0 && speed1 < 1100.0, "speed1={speed1}");

        // Second batch: much faster — EMA should smooth
        app.handle_event(ProgressEvent::BatchCompleted {
            batch_number: 2,
            total_batches: 5,
            current_block: 2000,
            blocks_in_batch: 1000,
            elapsed: Duration::from_millis(1100), // only 100ms for this batch
        });
        let speed2 = app.blocks_per_sec;
        // EMA should be between speed1 and the instant speed (10000)
        assert!(speed2 > speed1, "EMA should increase: speed2={speed2}");
        assert!(speed2 < 10000.0, "EMA should be smoothed: speed2={speed2}");
    }
}
