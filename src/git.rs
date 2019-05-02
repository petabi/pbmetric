use serde::Deserialize;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Deserialize)]
pub struct Repo {
    url: String,
}

pub fn update_all<P: AsRef<Path>>(root: P, repos: &BTreeMap<String, Repo>) -> io::Result<()> {
    let mut path = root.as_ref().to_path_buf();
    for (name, repo) in repos {
        path.push(name);
        if path.exists() {
            // update
        } else {
            clone(&repo.url, &path)?;
        }
        path.pop();
    }
    Ok(())
}

fn clone<P: AsRef<Path>>(url: &str, path: P) -> io::Result<()> {
    let path = match path.as_ref().to_str() {
        Some(path) => path,
        None => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid repository path",
            ))
        }
    };
    let status = Command::new("git").args(&["clone", url, &path]).status()?;
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "git operation failed"));
    }
    Ok(())
}
