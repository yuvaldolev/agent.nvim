use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use lsp_types::Url;

#[derive(Clone)]
struct PendingJob {
    job_id: String,
    line: u32,
}

#[derive(Clone)]
struct ActiveJob {
    job_id: String,
    line: u32,
}

#[derive(Default)]
struct FileQueue {
    active_job: Option<ActiveJob>,
    pending: Vec<PendingJob>,
}

#[derive(Clone)]
pub struct JobQueue {
    queues: Arc<Mutex<HashMap<Url, FileQueue>>>,
    cond: Arc<Condvar>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            queues: Arc::new(Mutex::new(HashMap::new())),
            cond: Arc::new(Condvar::new()),
        }
    }

    /// Try to acquire a slot for the given URI.
    /// Returns the (possibly adjusted) line number for this job.
    /// Blocks until the slot is available if another job is active.
    pub fn acquire(&self, uri: &Url, job_id: &str, line: u32) -> u32 {
        let mut queues = self.queues.lock().unwrap();

        let queue = queues.entry(uri.clone()).or_default();

        if queue.active_job.is_none() {
            queue.active_job = Some(ActiveJob {
                job_id: job_id.to_string(),
                line,
            });
            return line;
        }

        queue.pending.push(PendingJob {
            job_id: job_id.to_string(),
            line,
        });

        let uri_clone = uri.clone();
        let job_id_owned = job_id.to_string();

        loop {
            queues = self.cond.wait(queues).unwrap();

            if let Some(queue) = queues.get_mut(&uri_clone) {
                if let Some(active) = &queue.active_job {
                    if active.job_id == job_id_owned {
                        return active.line;
                    }
                }
            }
        }
    }

    /// Release the slot for the given URI, allowing the next pending job to proceed.
    pub fn release(&self, uri: &Url, job_id: &str) {
        let mut queues = self.queues.lock().unwrap();

        if let Some(queue) = queues.get_mut(uri) {
            let is_active = queue
                .active_job
                .as_ref()
                .map(|a| a.job_id == job_id)
                .unwrap_or(false);

            if is_active {
                if let Some(next) = queue.pending.first().cloned() {
                    queue.pending.remove(0);
                    queue.active_job = Some(ActiveJob {
                        job_id: next.job_id,
                        line: next.line,
                    });
                } else {
                    queue.active_job = None;
                }
                self.cond.notify_all();
            }
        }
    }

    /// Adjust pending job line numbers after an edit was applied.
    /// All pending jobs with line > edit_line will have their line shifted by lines_delta.
    pub fn adjust_pending_lines(&self, uri: &Url, edit_line: u32, lines_delta: i32) {
        let mut queues = self.queues.lock().unwrap();

        if let Some(queue) = queues.get_mut(uri) {
            for pending in &mut queue.pending {
                if pending.line > edit_line {
                    pending.line = (pending.line as i32 + lines_delta) as u32;
                }
            }
        }
    }
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}
