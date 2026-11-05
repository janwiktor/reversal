//! Background work on a list of frames, so the page stays usable and can show progress.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// A frame that failed, or finished with something the user should know.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Issue {
    pub file: String,
    pub message: String,
}

/// What a frame's work returns: where the result went, and anything to report about it.
pub struct Done {
    pub path: String,
    pub warning: Option<String>,
}

#[derive(Default, Serialize)]
pub struct Progress {
    /// What's being done, for the page: "save" or "roll".
    pub task: &'static str,
    pub total: usize,
    pub done: usize,
    /// Frames that failed. A frame with only a warning counts as done.
    pub failed: usize,
    pub active: Vec<String>,
    pub issues: Vec<Issue>,
    pub canceled: bool,
    /// The latest file written.
    pub last_path: Option<String>,
    pub running: bool,
    #[serde(skip)]
    queue: VecDeque<String>,
}

#[derive(Clone, Default)]
pub struct Batch(Arc<Mutex<Progress>>);

impl Batch {
    /// Start `work` on `files` with `workers` threads. Refuses while a batch is running.
    pub fn start<F>(&self, files: Vec<String>, workers: usize, work: F) -> Result<(), String>
    where
        F: Fn(&str) -> Result<Done, String> + Send + Sync + 'static,
    {
        self.start_then("save", files, workers, work, || None)
    }

    /// Like [`Batch::start`], with a last step once all frames are through: unless canceled,
    /// `then` runs on the last worker, and an error it returns is reported as an issue.
    pub fn start_then<F, T>(
        &self,
        task: &'static str,
        files: Vec<String>,
        workers: usize,
        work: F,
        then: T,
    ) -> Result<(), String>
    where
        F: Fn(&str) -> Result<Done, String> + Send + Sync + 'static,
        T: FnOnce() -> Option<String> + Send + 'static,
    {
        {
            let mut progress = self.0.lock().expect("no panics while locked");
            if progress.running {
                return Err("still saving the previous DNGs".to_string());
            }
            *progress = Progress {
                task,
                total: files.len(),
                running: !files.is_empty(),
                queue: files.into(),
                ..Progress::default()
            };
        }
        let work = Arc::new(work);
        let then = Arc::new(Mutex::new(Some(then)));
        for _ in 0..workers {
            let (batch, work, then) = (self.clone(), work.clone(), then.clone());
            std::thread::spawn(move || {
                batch.work_through(work.as_ref());
                batch.finish_last(&then);
            });
        }
        Ok(())
    }

    fn work_through<F: Fn(&str) -> Result<Done, String>>(&self, work: &F) {
        while let Some(file) = self.next_file() {
            // A bad file mustn't stop the rest, or leave the batch running for ever.
            let result = catch_unwind(AssertUnwindSafe(|| work(&file)))
                .unwrap_or_else(|_| Err("the conversion failed unexpectedly".to_string()));
            self.finish(&file, result);
        }
    }

    /// Every worker ends here; the one that sees nothing left runs the last step, and only
    /// then does the batch count as finished.
    fn finish_last<T: FnOnce() -> Option<String>>(&self, then: &Mutex<Option<T>>) {
        let progress = self.0.lock().expect("no panics while locked");
        if !progress.active.is_empty() || !progress.queue.is_empty() {
            return;
        }
        let Some(then) = then.lock().expect("no panics while locked").take() else {
            return;
        };
        let canceled = progress.canceled;
        drop(progress);
        let issue = if canceled {
            None
        } else {
            catch_unwind(AssertUnwindSafe(then))
                .unwrap_or_else(|_| Some("the last step failed unexpectedly".to_string()))
        };
        let mut progress = self.0.lock().expect("no panics while locked");
        if let Some(message) = issue {
            progress.issues.push(Issue {
                file: String::new(),
                message,
            });
        }
        progress.running = false;
    }

    fn next_file(&self) -> Option<String> {
        let mut progress = self.0.lock().expect("no panics while locked");
        let file = progress.queue.pop_front()?;
        progress.active.push(file.clone());
        Some(file)
    }

    fn finish(&self, file: &str, result: Result<Done, String>) {
        let mut progress = self.0.lock().expect("no panics while locked");
        progress.active.retain(|f| f != file);
        let mut report = |message| {
            progress.issues.push(Issue {
                file: file.to_string(),
                message,
            })
        };
        match result {
            Ok(done) => {
                if let Some(warning) = done.warning {
                    report(warning);
                }
                progress.done += 1;
                progress.last_path = Some(done.path);
            }
            Err(message) => {
                report(message);
                progress.failed += 1;
            }
        }
    }

    /// Frames being worked on finish; the rest don't start.
    pub fn cancel(&self) {
        let mut progress = self.0.lock().expect("no panics while locked");
        progress.canceled |= !progress.queue.is_empty();
        progress.queue.clear();
    }

    pub fn is_running(&self) -> bool {
        self.0.lock().expect("no panics while locked").running
    }

    pub fn status(&self) -> serde_json::Value {
        let progress = self.0.lock().expect("no panics while locked");
        serde_json::to_value(&*progress).expect("progress serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("f{i}")).collect()
    }

    fn wait(batch: &Batch) -> serde_json::Value {
        for _ in 0..1000 {
            let status = batch.status();
            if status["running"] == false {
                return status;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("batch didn't finish");
    }

    #[test]
    fn a_warning_counts_as_done_and_only_a_failure_as_failed() {
        let batch = Batch::default();
        batch
            .start(names(3), 1, |file| match file {
                "f0" => Err("no good".to_string()),
                "f1" => Ok(Done {
                    path: file.to_string(),
                    warning: Some("look again".to_string()),
                }),
                _ => Ok(Done {
                    path: file.to_string(),
                    warning: None,
                }),
            })
            .unwrap();
        let status = wait(&batch);
        assert_eq!(status["done"], 2);
        assert_eq!(status["failed"], 1);
        assert_eq!(status["issues"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_bad_frame_is_reported_and_the_rest_still_finish() {
        let batch = Batch::default();
        batch
            .start(names(6), 2, |file| match file {
                "f1" => Err("no good".to_string()),
                "f2" => panic!("boom"),
                _ => Ok(Done {
                    path: format!("{file}.dng"),
                    warning: (file == "f3").then(|| "look again".to_string()),
                }),
            })
            .unwrap();
        let status = wait(&batch);
        assert_eq!(status["total"], 6);
        assert_eq!(status["done"], 4);
        assert_eq!(status["issues"].as_array().unwrap().len(), 3);
        assert_eq!(status["active"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn the_last_step_runs_once_after_all_frames_and_reports_its_error() {
        let batch = Batch::default();
        let done = Arc::new(Mutex::new(0));
        let seen = done.clone();
        let count = Arc::new(Mutex::new(0));
        let runs = count.clone();
        batch
            .start_then(
                "test",
                names(8),
                3,
                move |file| {
                    *seen.lock().unwrap() += 1;
                    Ok(Done {
                        path: file.to_string(),
                        warning: None,
                    })
                },
                move || {
                    *runs.lock().unwrap() += 1;
                    Some("couldn't finish".to_string())
                },
            )
            .unwrap();
        let status = wait(&batch);
        assert_eq!(*done.lock().unwrap(), 8);
        assert_eq!(*count.lock().unwrap(), 1);
        assert_eq!(status["issues"][0]["message"], "couldn't finish");
    }

    #[test]
    fn cancel_stops_frames_that_havent_started_and_allows_a_new_batch() {
        let batch = Batch::default();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        batch
            .start(names(5), 1, move |file| {
                gate.lock().unwrap().recv().unwrap();
                Ok(Done {
                    path: file.to_string(),
                    warning: None,
                })
            })
            .unwrap();
        while batch.status()["active"].as_array().unwrap().is_empty() {
            std::thread::yield_now();
        }
        assert!(batch
            .start(names(1), 1, |_| Err("refused".to_string()))
            .is_err());
        batch.cancel();
        release.send(()).unwrap();
        let status = wait(&batch);
        assert_eq!(status["done"], 1);
        assert_eq!(status["canceled"], true);
        batch
            .start(names(2), 1, |file| {
                Ok(Done {
                    path: file.to_string(),
                    warning: None,
                })
            })
            .unwrap();
        assert_eq!(wait(&batch)["done"], 2);
    }
}
