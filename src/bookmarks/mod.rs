use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::PathBuf,
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};

use crate::logging::{DATA_FOLDER, project_directory};

pub static BOOKMARKS_DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Bookmarks(HashMap<String, HashSet<u64>>);

impl Bookmarks {
    pub fn add(&mut self, owner: &str, repo: &str, issue_number: u64) {
        let key = format!("{}/{}", owner, repo);
        self.0.entry(key).or_default().insert(issue_number);
    }

    pub fn remove(&mut self, owner: &str, repo: &str, issue_number: u64) {
        let key = format!("{}/{}", owner, repo);
        if let Some(issues) = self.0.get_mut(&key) {
            issues.remove(&issue_number);
            if issues.is_empty() {
                self.0.remove(&key);
            }
        }
    }

    pub fn is_bookmarked(&self, owner: &str, repo: &str, issue_number: u64) -> bool {
        let key = format!("{}/{}", owner, repo);
        self.0
            .get(&key)
            .is_some_and(|issues| issues.contains(&issue_number))
    }

    pub fn get_bookmarked_issues(&self, owner: &str, repo: &str) -> Vec<u64> {
        let key = format!("{}/{}", owner, repo);
        self.0
            .get(&key)
            .map_or(vec![], |issues| issues.iter().cloned().collect())
    }

    pub fn write(&self, buf: &mut impl Write) -> std::io::Result<()> {
        let path = get_bookmarks_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_vec(self)?;
        buf.write_all(&contents)
    }

    pub fn write_to_file(&self) -> std::io::Result<()> {
        let path = get_bookmarks_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_vec(self)?;
        std::fs::write(path, contents)
    }
}

fn get_bookmarks_file() -> &'static PathBuf {
    BOOKMARKS_DIR.get_or_init(|| {
        let bdir = if let Some(s) = DATA_FOLDER.clone() {
            s
        } else if let Some(proj_dirs) = project_directory() {
            proj_dirs.data_local_dir().to_path_buf()
        } else {
            PathBuf::from(".").join(".data")
        };
        bdir.join("bookmarks/bookmarks.json")
    })
}

pub fn read_bookmarks() -> Bookmarks {
    let path = get_bookmarks_file();
    if let Ok(contents) = std::fs::read_to_string(path) {
        serde_json::from_str(&contents).unwrap_or_default()
    } else {
        Bookmarks::default()
    }
}
