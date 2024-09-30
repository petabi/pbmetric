mod git;
mod github;
mod issue;
mod report;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::process::exit;

use chrono::{DateTime, FixedOffset};
use clap::{crate_version, Arg, Command};
use directories::ProjectDirs;
use lettre::message::SinglePart;
use lettre::Message;
use lettre::{transport::smtp::authentication::Credentials, SmtpTransport, Transport};
use serde::Deserialize;

use crate::report::{agenda, GithubConfig};

const QUALIFIER: &str = "com";
const ORGANIZATION: &str = "petabi";
const APPLICATION: &str = env!("CARGO_PKG_NAME");

#[derive(Default, Deserialize)]
struct MailConfig {
    server: String,
    username: String,
    password: String,
    recipient: String,
}

#[derive(Default, Deserialize)]
struct Config {
    mail: MailConfig,
    github: GithubConfig,
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
    let matches = Command::new(APPLICATION)
        .version(crate_version!())
        .arg(Arg::new("asof").long("asof").num_args(1))
        .arg(Arg::new("epoch").long("epoch").num_args(1))
        .arg(
            Arg::new("offline")
                .long("offline")
                .help("Skips updating repositories"),
        )
        .get_matches();

    let Some(dirs) = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION) else {
        eprintln!("no valid home directory path");
        exit(1);
    };
    let config = load_config(dirs.config_dir());
    let asof = matches
        .get_one::<String>("asof")
        .map_or_else(
            chrono::Utc::now,
            |v| match DateTime::<FixedOffset>::parse_from_rfc3339(v) {
                Ok(asof) => asof.with_timezone(&chrono::Utc),
                Err(e) => {
                    eprintln!("{e}: {v}");
                    exit(1);
                }
            },
        );
    let epoch =
        matches.get_one::<String>("epoch").map(
            |v| match DateTime::<FixedOffset>::parse_from_rfc3339(v) {
                Ok(epoch) => epoch.with_timezone(&chrono::Utc),
                Err(e) => {
                    eprintln!("{e}: {v}");
                    exit(1);
                }
            },
        );

    let repo_dir = match repo_dir(dirs.cache_dir()) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("cannot create the repository directory: {e}");
            exit(1);
        }
    };

    let orig_dir = match env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("cannot read the current directory: {e}");
            exit(1);
        }
    };
    if let Err(e) = git::update_all(
        &repo_dir,
        &config.repos,
        &asof,
        matches.contains_id("offline"),
    ) {
        eprintln!("cannot update git repositories: {e}");
        if let Err(e) = env::set_current_dir(orig_dir) {
            eprintln!("cannot restore the working directory: {e}");
        }
        exit(1);
    }
    let mut body = Vec::<u8>::new();
    if let Err(e) = agenda(
        &mut body,
        &config.github,
        &repo_dir,
        &config.repos,
        &config.email_map,
        &asof,
        &epoch,
    ) {
        eprintln!("cannot create an agenda: {e}");
        exit(1);
    }
    if let Err(e) = env::set_current_dir(orig_dir) {
        eprintln!("cannot restore the working directory: {e}");
        exit(1);
    }

    let part = SinglePart::html(body);
    let (Ok(to), Ok(from)) = (config.mail.recipient.parse(), config.mail.username.parse()) else {
        eprintln!("cannot parse email addresses");
        exit(1);
    };
    let msg = Message::builder()
        .to(to)
        .from(from)
        .subject(format!(
            "Project Snapshot {}",
            chrono::offset::Utc::now().date_naive()
        ))
        .singlepart(part)
        .unwrap();
    let credentials = Credentials::new(config.mail.username, config.mail.password);
    let sender = SmtpTransport::starttls_relay(&config.mail.server)
        .unwrap()
        .credentials(credentials)
        .build();
    let _result = sender.send(&msg);
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
                eprintln!("cannot load {}: {e}", path.display());
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
