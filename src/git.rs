use chrono::{DateTime, Utc};
use regex::RegexSet;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::io;
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

#[derive(Deserialize)]
pub struct Repo {
    url: String,
}

pub fn update_all<P: AsRef<Path>>(root: P, repos: &BTreeMap<String, Repo>) -> io::Result<()> {
    let mut path = root.as_ref().to_path_buf();
    for (name, repo) in repos {
        path.push(name);
        if path.exists() {
            update(&path)?;
        } else {
            clone(&repo.url, &path)?;
        }
        path.pop();
    }
    Ok(())
}

pub fn blame_stats<P: AsRef<Path>>(
    path: P,
    since: &DateTime<Utc>,
) -> io::Result<HashMap<String, usize>> {
    let excludes = vec![r#"^.git/"#, r#"(^|/)Cargo.lock$"#];
    let excludes = match RegexSet::new(excludes) {
        Ok(set) => set,
        Err(e) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid exclude pattern: {}", e),
            ))
        }
    };

    let mut total_loc = HashMap::new();
    let orig_dir = env::current_dir()?;
    env::set_current_dir(&path)?;
    for entry in WalkDir::new(".") {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("cannot traverse repo: {}", e),
                ))
            }
        };
        if entry.file_type().is_dir() {
            continue;
        }
        let pathstr = match entry.path().to_str() {
            Some(pathstr) => {
                if pathstr.len() < 2 {
                    continue;
                }
                &pathstr[2..]
            }
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid file name: {}", entry.path().display()),
                ))
            }
        };
        if excludes.is_match(pathstr) {
            continue;
        }
        println!("  {}", pathstr);
        let blameout = blame(pathstr)?;
        for (email, loc) in parse_blame(&blameout, since) {
            let entry = total_loc.entry(email).or_insert(0);
            *entry += loc;
        }
    }
    env::set_current_dir(orig_dir)?;
    Ok(total_loc)
}

fn blame(filename: &str) -> io::Result<String> {
    let output = Command::new("git")
        .args(&["blame", "-e", "--date=iso", filename])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "git operation failed"));
    }
    let outstr = String::from_utf8_lossy(&output.stdout);
    Ok(outstr.to_string())
}

fn parse_blame(blame: &str, since: &DateTime<Utc>) -> HashMap<String, usize> {
    let mut loc = HashMap::new();
    for line in blame.split('\n') {
        let email_start = match line.find("(<") {
            Some(cur) => cur + 2,
            None => continue, // invalid line
        };
        let email_end = match line[email_start + 1..].find('>') {
            Some(cur) => cur + email_start + 1,
            None => continue, // invalid line
        };
        let email = &line[email_start..email_end];
        let timestamp_end = match line[email_end + 1..].find(')') {
            Some(cur) => match line[email_end + 1..=cur + email_end].rfind(' ') {
                Some(cur) => cur + email_end + 1,
                None => continue, // invalid line
            },
            None => continue, // invalid line
        };
        if timestamp_end < 25 {
            println!("error: {}..{}, {}", email_end + 1, timestamp_end, line);
        }
        let timestamp =
            match DateTime::parse_from_str(&line[timestamp_end - 25..timestamp_end], "%F %T %z") {
                Ok(timestamp) => timestamp,
                Err(_) => continue, // invalid timestamp
            };
        if timestamp < since.with_timezone(&timestamp.timezone()) {
            continue;
        }
        let entry = loc.entry(email.to_string()).or_insert(0);
        *entry += 1;
    }
    loc
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

fn update<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let orig_dir = env::current_dir()?;
    env::set_current_dir(path)?;
    let status = Command::new("git").args(&["fetch", "origin"]).status()?;
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "git operation failed"));
    }
    let status = Command::new("git")
        .args(&["reset", "--hard", "origin/master"])
        .status()?;
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "git operation failed"));
    }
    env::set_current_dir(orig_dir)?;
    Ok(())
}
