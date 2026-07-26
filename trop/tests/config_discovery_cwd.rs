//! Process-working-directory regression tests for configuration discovery.
//!
//! This is intentionally a separate integration-test target: changing the
//! process working directory is global state, while tests in separate targets
//! run in separate processes.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use trop::config::{ConfigBuilder, ConfigFileKind};

struct CurrentDirGuard {
    original: PathBuf,
}

impl CurrentDirGuard {
    fn enter(path: &Path) -> Self {
        let original = env::current_dir().expect("failed to read current directory");
        env::set_current_dir(path).expect("failed to change current directory");
        Self { original }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        env::set_current_dir(&self.original).expect("failed to restore current directory");
    }
}

/// Default discovery must turn relative `.` into the absolute process CWD
/// before walking real parent directories.
#[test]
fn default_file_discovery_walks_real_parent_directories() {
    let temp = TempDir::new().expect("failed to create temporary directory");
    let project = temp.path().join("project");
    let child = project.join("nested").join("deeply");
    let isolated_data_dir = temp.path().join("data");
    fs::create_dir_all(&child).expect("failed to create nested working directory");
    let project_file = project.join("trop.yaml");
    fs::write(&project_file, "project: parent-proj\n")
        .expect("failed to write project configuration");

    let _cwd = CurrentDirGuard::enter(&child);
    let effective = ConfigBuilder::new()
        .with_data_dir(isolated_data_dir)
        .skip_env()
        .build_effective()
        .expect("default configuration discovery failed");

    assert_eq!(effective.project(), Some("parent-proj"));
    let loaded_project = effective
        .loaded_file(ConfigFileKind::Project)
        .expect("project configuration was not recorded");
    let canonical_project = project
        .canonicalize()
        .expect("failed to canonicalize project");
    assert!(loaded_project.is_absolute());
    assert_eq!(loaded_project, canonical_project.join("trop.yaml"));
    assert_eq!(loaded_project.parent(), Some(canonical_project.as_path()));
}
