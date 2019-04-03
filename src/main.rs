use clap::{crate_version, App};

fn main() {
    let _matches = App::new("pbmetric")
        .version(&crate_version!()[..])
        .get_matches();
}
