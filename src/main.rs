extern crate rustc_serialize;
extern crate cargo;

use cargo::CliResult;
use cargo::util::{Config, CargoResult};
use cargo::core::Workspace;
use cargo::ops::{self, Packages};

use std::path::Path;
use std::collections::{BTreeMap, HashSet};


struct DependencyAccumulator<'a> {
    config: &'a Config,
}

type Aggregate = Option<BTreeMap<String, HashSet<String>>>;

impl<'a> DependencyAccumulator<'a> {
    fn new(c: &'a Config) -> Self {
        DependencyAccumulator { config: c }
    }

    fn accumulate(&self) -> Aggregate {
        let local_root = Path::new(".").canonicalize().unwrap();
        let local_root = local_root.as_path();
        let ws_path = local_root.clone().join("Cargo.toml");
        let ws = Workspace::new(&ws_path, &self.config).unwrap();

        // here starts the code ripped from cargo::ops::cargo_output_metadata.rs because the
        // the visibility of the result returned from metadata_full() hindered evaluation
        let specs = Packages::All.into_package_id_specs(&ws).unwrap();
        let deps = ops::resolve_ws_precisely(&ws,
                                             None,
                                             &vec![],
                                             false,
                                             false,
                                             &specs).unwrap();
        let (packages, _resolve) = deps;

        let packages = packages.package_ids()
            .map(|i| packages.get(i).map(|p| p.clone()))
            .collect::<CargoResult<Vec<_>>>().unwrap();
        // here ends the ripped code

        let mut result: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        for package in packages {
             let name = package.name();
             let authors = package.authors().clone();
             for auth in authors {
                 let crates = result.entry(auth).or_insert(HashSet::new());
                 crates.insert(name.to_string());
             }
        }

        Some(result)
    }
}

// This is just to satisfy the signature for the method provided to cargo's "execute[...]" function.
#[derive(RustcDecodable)]
struct Options {}

fn real_main(_options: Options, config: &Config) -> CliResult<Option<()>> {
    let aggregate = DependencyAccumulator::new(config).accumulate().unwrap();

    let max_author_len = aggregate.keys().map(|e| e.len()).max().expect("No authors found.");
    println!("Authors and their respective crates for this crate:\n\n\n");
    for (author, crates) in aggregate {
        println!("{:<N$}:{:?}", author, crates, N = max_author_len);
    }

    Ok(None)
}

fn main() {
    cargo::execute_main_without_stdin(real_main, true, r#"
List all authors of all dependencies of the current crate.

Usage: cargo authors [options]

Options:
    -h, --help      Print this message
"#)
}
