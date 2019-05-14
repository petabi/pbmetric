mod git;
mod issue;
mod report;

use chrono::{DateTime, FixedOffset};
use clap::{crate_version, App, Arg};
use directories::ProjectDirs;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::process::exit;

use report::agenda;

const QUALIFIER: &str = "com";
const ORGANIZATION: &str = "petabi";
const APPLICATION: &str = env!("CARGO_PKG_NAME");

#[derive(Default, Deserialize)]
struct Config {
    gitlab_token: String,
    gitlab_projects: Vec<u64>,
    gitlab_usernames: Vec<String>,
    email_map: BTreeMap<String, String>,
    repos: BTreeMap<String, git::Repo>,
}

impl Config {
    fn from_path<P: AsRef<Path>>(path: P) -> io::Result<Config> {
        let mut buffer = String::new();
        File::open(path)?.read_to_string(&mut buffer)?;
        match toml::from_str::<Config>(&buffer) {
            Ok(config) => Ok(config),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        }
    }
}

fn main() {
    let matches = App::new(APPLICATION)
        .version(&crate_version!()[..])
        .arg(Arg::with_name("epoch").long("epoch").takes_value(true))
        .get_matches();

    let dirs = match ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION) {
        Some(dirs) => dirs,
        None => {
            eprintln!("no valid home directory path");
            exit(1);
        }
    };
    let config = load_config(dirs.config_dir());
    let epoch =
        matches
            .value_of("epoch")
            .map(|v| match DateTime::<FixedOffset>::parse_from_rfc3339(v) {
                Ok(epoch) => epoch.with_timezone(&chrono::Utc),
                Err(e) => {
                    eprintln!("invalid epoch: {}", e);
                    exit(1);
                }
            });

    let repo_dir = match repo_dir(dirs.cache_dir()) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("cannot create the repository directory: {}", e);
            exit(1);
        }
    };

    let orig_dir = match env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("cannot read the current directory: {}", e);
            exit(1);
        }
    };
    if let Err(e) = git::update_all(&repo_dir, &config.repos) {
        eprintln!("cannot update git repositories: {}", e);
        if let Err(e) = env::set_current_dir(orig_dir) {
            eprintln!("cannot restore the working directory: {}", e);
        }
        exit(1);
    }
    if let Err(e) = agenda(
        config.gitlab_token,
        &config.gitlab_projects,
        &config.gitlab_usernames,
        &repo_dir,
        &config.repos,
        &config.email_map,
        &epoch,
    ) {
        eprintln!("cannot create an agenda: {}", e);
        exit(1);
    }
    if let Err(e) = env::set_current_dir(orig_dir) {
        eprintln!("cannot restore the working directory: {}", e);
        exit(1);
    }
}

fn load_config<P: AsRef<Path>>(dir: P) -> Config {
    let mut path = PathBuf::new();
    path.push(dir);
    path.push("config.toml");
    match Config::from_path(&path) {
        Ok(config) => config,
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound {
                Config::default()
            } else {
                eprintln!("cannot load {}: {}", path.display(), e);
                exit(1);
            }
        }
    }
}

fn repo_dir<P: AsRef<Path>>(cache_dir: P) -> io::Result<PathBuf> {
    let mut repo_dir = PathBuf::new();
    repo_dir.push(cache_dir);
    repo_dir.push("repos");
    fs::create_dir_all(&repo_dir)?;
    Ok(repo_dir)
}
