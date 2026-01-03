use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use lsp_types::Url;
use tracing::info;

pub const MAX_CONCURRENT_JOBS_PER_FILE: usize = 10;

#[derive(Clone, Debug)]
pub struct ActiveJob {
    pub job_id: String,
    pub original_line: u32,
    pub current_line: u32,
    pub function_signature: String,
}

#[derive(Clone)]
pub struct JobTracker {
    jobs: Arc<Mutex<HashMap<Url, HashMap<String, ActiveJob>>>>,
}

impl JobTracker {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new job. Returns Err if max concurrent jobs reached.
    pub fn register_job(
        &self,
        uri: &Url,
        job_id: &str,
        line: u32,
        function_signature: String,
    ) -> Result<(), String> {
        let mut jobs = self.jobs.lock().unwrap();

        let file_jobs = jobs.entry(uri.clone()).or_insert_with(HashMap::new);

        if file_jobs.len() >= MAX_CONCURRENT_JOBS_PER_FILE {
            return Err(format!(
                "Maximum concurrent implementations ({}) reached for this file. Please wait.",
                MAX_CONCURRENT_JOBS_PER_FILE
            ));
        }

        file_jobs.insert(
            job_id.to_string(),
            ActiveJob {
                job_id: job_id.to_string(),
                original_line: line,
                current_line: line,
                function_signature,
            },
        );

        info!(
            "Registered job {} for {} at line {} ({} active jobs)",
            job_id,
            uri,
            line,
            file_jobs.len()
        );

        Ok(())
    }

    /// Get current line for a job (may have been adjusted)
    pub fn get_current_line(&self, job_id: &str) -> Option<u32> {
        let jobs = self.jobs.lock().unwrap();
        for file_jobs in jobs.values() {
            if let Some(job) = file_jobs.get(job_id) {
                return Some(job.current_line);
            }
        }
        None
    }

    /// Get function signature for fallback matching
    pub fn get_function_signature(&self, job_id: &str) -> Option<String> {
        let jobs = self.jobs.lock().unwrap();
        for file_jobs in jobs.values() {
            if let Some(job) = file_jobs.get(job_id) {
                return Some(job.function_signature.clone());
            }
        }
        None
    }

    /// Adjust lines for all jobs in a file after an edit
    pub fn adjust_lines_for_edit(
        &self,
        uri: &Url,
        edit_start_line: u32,
        edit_end_line: u32,
        lines_delta: i32,
        excluding_job_id: &str,
    ) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(file_jobs) = jobs.get_mut(uri) {
            for (job_id, job) in file_jobs.iter_mut() {
                if job_id == excluding_job_id {
                    continue;
                }

                // If job's function is AFTER the edited region, shift it
                if job.current_line > edit_end_line {
                    let new_line = (job.current_line as i32 + lines_delta).max(0) as u32;
                    info!(
                        "Adjusted job {} line: {} -> {} (delta: {})",
                        job_id, job.current_line, new_line, lines_delta
                    );
                    job.current_line = new_line;
                }
                // If job's function OVERLAPS with edited region, keep current line
                // The merge logic will handle finding the function via signature matching
            }
        }
    }

    /// Remove job from tracking
    pub fn complete_job(&self, uri: &Url, job_id: &str) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(file_jobs) = jobs.get_mut(uri) {
            file_jobs.remove(job_id);
            info!(
                "Completed job {} for {} ({} remaining)",
                job_id,
                uri,
                file_jobs.len()
            );

            // Clean up empty file entries
            if file_jobs.is_empty() {
                jobs.remove(uri);
            }
        }
    }

    /// Get count of active jobs for a file
    pub fn active_job_count(&self, uri: &Url) -> usize {
        let jobs = self.jobs.lock().unwrap();
        jobs.get(uri).map(|fj| fj.len()).unwrap_or(0)
    }

    /// Get all active jobs for a file (for sending line updates)
    pub fn get_active_jobs(&self, uri: &Url) -> Vec<(String, u32)> {
        let jobs = self.jobs.lock().unwrap();
        jobs.get(uri)
            .map(|file_jobs| {
                file_jobs
                    .values()
                    .map(|job| (job.job_id.clone(), job.current_line))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for JobTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_job() {
        let tracker = JobTracker::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        let result = tracker.register_job(&uri, "job1", 10, "fn foo()".to_string());
        assert!(result.is_ok());
        assert_eq!(tracker.active_job_count(&uri), 1);
    }

    #[test]
    fn test_max_concurrent_jobs() {
        let tracker = JobTracker::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        // Register 10 jobs (max)
        for i in 0..10 {
            let result =
                tracker.register_job(&uri, &format!("job{}", i), i * 10, "fn foo()".to_string());
            assert!(result.is_ok());
        }

        assert_eq!(tracker.active_job_count(&uri), 10);

        // 11th job should fail
        let result = tracker.register_job(&uri, "job11", 100, "fn bar()".to_string());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Maximum concurrent implementations"));
    }

    #[test]
    fn test_get_current_line() {
        let tracker = JobTracker::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        tracker
            .register_job(&uri, "job1", 10, "fn foo()".to_string())
            .unwrap();

        assert_eq!(tracker.get_current_line("job1"), Some(10));
        assert_eq!(tracker.get_current_line("nonexistent"), None);
    }

    #[test]
    fn test_adjust_lines_for_edit() {
        let tracker = JobTracker::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        // Register jobs at lines 10, 20, 30
        tracker
            .register_job(&uri, "job1", 10, "fn foo()".to_string())
            .unwrap();
        tracker
            .register_job(&uri, "job2", 20, "fn bar()".to_string())
            .unwrap();
        tracker
            .register_job(&uri, "job3", 30, "fn baz()".to_string())
            .unwrap();

        // Edit at lines 10-15, adding 5 lines (job1 completes)
        tracker.adjust_lines_for_edit(&uri, 10, 15, 5, "job1");

        // job1 should be unchanged (it's the one completing)
        assert_eq!(tracker.get_current_line("job1"), Some(10));

        // job2 at line 20 (after edit) should shift to 25
        assert_eq!(tracker.get_current_line("job2"), Some(25));

        // job3 at line 30 should shift to 35
        assert_eq!(tracker.get_current_line("job3"), Some(35));
    }

    #[test]
    fn test_adjust_lines_negative_delta() {
        let tracker = JobTracker::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        tracker
            .register_job(&uri, "job1", 10, "fn foo()".to_string())
            .unwrap();
        tracker
            .register_job(&uri, "job2", 30, "fn bar()".to_string())
            .unwrap();

        // Edit removes 5 lines
        tracker.adjust_lines_for_edit(&uri, 10, 20, -5, "job1");

        // job2 should shift from 30 to 25
        assert_eq!(tracker.get_current_line("job2"), Some(25));
    }

    #[test]
    fn test_complete_job() {
        let tracker = JobTracker::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        tracker
            .register_job(&uri, "job1", 10, "fn foo()".to_string())
            .unwrap();
        tracker
            .register_job(&uri, "job2", 20, "fn bar()".to_string())
            .unwrap();

        assert_eq!(tracker.active_job_count(&uri), 2);

        tracker.complete_job(&uri, "job1");
        assert_eq!(tracker.active_job_count(&uri), 1);
        assert_eq!(tracker.get_current_line("job1"), None);
        assert_eq!(tracker.get_current_line("job2"), Some(20));
    }

    #[test]
    fn test_get_active_jobs() {
        let tracker = JobTracker::new();
        let uri = Url::parse("file:///test.rs").unwrap();

        tracker
            .register_job(&uri, "job1", 10, "fn foo()".to_string())
            .unwrap();
        tracker
            .register_job(&uri, "job2", 20, "fn bar()".to_string())
            .unwrap();

        let jobs = tracker.get_active_jobs(&uri);
        assert_eq!(jobs.len(), 2);

        // Check both jobs are present (order doesn't matter)
        let job_ids: Vec<String> = jobs.iter().map(|(id, _)| id.clone()).collect();
        assert!(job_ids.contains(&"job1".to_string()));
        assert!(job_ids.contains(&"job2".to_string()));
    }

    #[test]
    fn test_multiple_files() {
        let tracker = JobTracker::new();
        let uri1 = Url::parse("file:///test1.rs").unwrap();
        let uri2 = Url::parse("file:///test2.rs").unwrap();

        tracker
            .register_job(&uri1, "job1", 10, "fn foo()".to_string())
            .unwrap();
        tracker
            .register_job(&uri2, "job2", 20, "fn bar()".to_string())
            .unwrap();

        assert_eq!(tracker.active_job_count(&uri1), 1);
        assert_eq!(tracker.active_job_count(&uri2), 1);

        // Complete job1 shouldn't affect uri2
        tracker.complete_job(&uri1, "job1");
        assert_eq!(tracker.active_job_count(&uri1), 0);
        assert_eq!(tracker.active_job_count(&uri2), 1);
    }
}
