use tokio::process::Command;

use crate::types::{AgentEvent, VerificationResult};

pub struct Verifier {
    workspace: std::path::PathBuf,
}

impl Verifier {
    pub fn new(workspace: std::path::PathBuf) -> Self {
        Self { workspace }
    }

    pub async fn detect_project_type(&self) -> Vec<String> {
        let mut checks = Vec::new();

        if self.workspace.join("package.json").exists() {
            checks.push("npm".into());
        }
        if self.workspace.join("Cargo.toml").exists() {
            checks.push("cargo".into());
        }
        if self.workspace.join("pyproject.toml").exists() || self.workspace.join("setup.py").exists() {
            checks.push("python".into());
        }
        if self.workspace.join("go.mod").exists() {
            checks.push("go".into());
        }

        checks
    }

    pub async fn verify_build(&self, event_tx: &Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>) -> VerificationResult {
        let project_types = self.detect_project_type().await;

        for pt in &project_types {
            let (cmd, args) = match pt.as_str() {
                "npm" => ("npm", vec!["run", "build"]),
                "cargo" => ("cargo", vec!["build"]),
                "python" => ("python", vec!["-m", "py_compile", "."]),
                "go" => ("go", vec!["build", "./..."]),
                _ => continue,
            };

            if let Some(tx) = event_tx {
                let _ = tx.send(AgentEvent::VerificationStarted {
                    step: format!("build ({})", pt),
                });
            }

            let output = Command::new(cmd)
                .args(&args)
                .current_dir(&self.workspace)
                .output()
                .await;

            match output {
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    let combined = format!("{}\n{}", stdout, stderr);

                    if out.status.success() {
                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::VerificationPassed {
                                step: format!("build ({})", pt),
                            });
                        }
                        return VerificationResult {
                            passed: true,
                            step: format!("build ({})", pt),
                            errors: Vec::new(),
                            output: combined,
                        };
                    } else {
                        let errors: Vec<String> = stderr.lines()
                            .filter(|l| l.contains("error") || l.contains("Error"))
                            .map(|l| l.to_string())
                            .collect();

                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::VerificationFailed {
                                step: format!("build ({})", pt),
                                errors: errors.clone(),
                            });
                        }

                        return VerificationResult {
                            passed: false,
                            step: format!("build ({})", pt),
                            errors,
                            output: combined,
                        };
                    }
                }
                Err(e) => {
                    return VerificationResult {
                        passed: false,
                        step: format!("build ({})", pt),
                        errors: vec![format!("Failed to run {}: {}", cmd, e)],
                        output: String::new(),
                    };
                }
            }
        }

        VerificationResult {
            passed: true,
            step: "build (no project type detected)".into(),
            errors: Vec::new(),
            output: "No build system detected, skipping build verification".into(),
        }
    }

    pub async fn verify_test(&self, event_tx: &Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>) -> VerificationResult {
        let project_types = self.detect_project_type().await;

        for pt in &project_types {
            let (cmd, args) = match pt.as_str() {
                "npm" => ("npm", vec!["test"]),
                "cargo" => ("cargo", vec!["test"]),
                "python" => ("python", vec!["-m", "pytest"]),
                "go" => ("go", vec!["test", "./..."]),
                _ => continue,
            };

            if let Some(tx) = event_tx {
                let _ = tx.send(AgentEvent::VerificationStarted {
                    step: format!("test ({})", pt),
                });
            }

            let output = Command::new(cmd)
                .args(&args)
                .current_dir(&self.workspace)
                .output()
                .await;

            match output {
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    let combined = format!("{}\n{}", stdout, stderr);

                    if out.status.success() {
                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::VerificationPassed {
                                step: format!("test ({})", pt),
                            });
                        }
                        return VerificationResult {
                            passed: true,
                            step: format!("test ({})", pt),
                            errors: Vec::new(),
                            output: combined,
                        };
                    } else {
                        let errors: Vec<String> = stderr.lines()
                            .filter(|l| l.contains("FAIL") || l.contains("error") || l.contains("Error"))
                            .take(10)
                            .map(|l| l.to_string())
                            .collect();

                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::VerificationFailed {
                                step: format!("test ({})", pt),
                                errors: errors.clone(),
                            });
                        }

                        return VerificationResult {
                            passed: false,
                            step: format!("test ({})", pt),
                            errors,
                            output: combined,
                        };
                    }
                }
                Err(e) => {
                    return VerificationResult {
                        passed: false,
                        step: format!("test ({})", pt),
                        errors: vec![format!("Failed to run {}: {}", cmd, e)],
                        output: String::new(),
                    };
                }
            }
        }

        VerificationResult {
            passed: true,
            step: "test (no test framework detected)".into(),
            errors: Vec::new(),
            output: "No test framework detected, skipping test verification".into(),
        }
    }

    pub async fn verify_lint(&self, event_tx: &Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>) -> VerificationResult {
        let project_types = self.detect_project_type().await;

        for pt in &project_types {
            let (cmd, args) = match pt.as_str() {
                "npm" => {
                    if self.workspace.join("node_modules/.bin/eslint").exists() {
                        ("npx", vec!["eslint", "."])
                    } else {
                        continue;
                    }
                }
                "cargo" => ("cargo", vec!["clippy", "--", "-D", "warnings"]),
                _ => continue,
            };

            if let Some(tx) = event_tx {
                let _ = tx.send(AgentEvent::VerificationStarted {
                    step: format!("lint ({})", pt),
                });
            }

            let output = Command::new(cmd)
                .args(&args)
                .current_dir(&self.workspace)
                .output()
                .await;

            match output {
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    let combined = format!("{}\n{}", stdout, stderr);

                    if out.status.success() {
                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::VerificationPassed {
                                step: format!("lint ({})", pt),
                            });
                        }
                        return VerificationResult {
                            passed: true,
                            step: format!("lint ({})", pt),
                            errors: Vec::new(),
                            output: combined,
                        };
                    } else {
                        let errors: Vec<String> = stderr.lines()
                            .filter(|l| l.contains("error") || l.contains("warning"))
                            .take(10)
                            .map(|l| l.to_string())
                            .collect();

                        if let Some(tx) = event_tx {
                            let _ = tx.send(AgentEvent::VerificationFailed {
                                step: format!("lint ({})", pt),
                                errors: errors.clone(),
                            });
                        }

                        return VerificationResult {
                            passed: false,
                            step: format!("lint ({})", pt),
                            errors,
                            output: combined,
                        };
                    }
                }
                Err(e) => {
                    return VerificationResult {
                        passed: false,
                        step: format!("lint ({})", pt),
                        errors: vec![format!("Failed to run {}: {}", cmd, e)],
                        output: String::new(),
                    };
                }
            }
        }

        VerificationResult {
            passed: true,
            step: "lint (no linter detected)".into(),
            errors: Vec::new(),
            output: "No linter detected, skipping lint verification".into(),
        }
    }

    pub async fn verify_all(&self, event_tx: &Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>) -> VerificationResult {
        let build = self.verify_build(event_tx).await;
        if !build.passed {
            return build;
        }

        let test = self.verify_test(event_tx).await;
        if !test.passed {
            return test;
        }

        let lint = self.verify_lint(event_tx).await;
        if !lint.passed {
            return lint;
        }

        VerificationResult {
            passed: true,
            step: "all".into(),
            errors: Vec::new(),
            output: "All verifications passed".into(),
        }
    }
}
