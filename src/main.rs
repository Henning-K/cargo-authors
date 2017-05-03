extern crate rustc_serialize;
extern crate docopt;
extern crate cargo;

#[macro_use] extern crate error_chain;

#[macro_use] extern crate serde_derive;
extern crate serde_json;

use docopt::Docopt;

use cargo::CliResult;
use cargo::util::{Config, CargoResult, human};
use cargo::core::Workspace;
use cargo::core::shell::{Verbosity, ColorConfig};
use cargo::ops::{self, Packages};

use std::env;
use std::path::Path;
use std::collections::{BTreeMap, HashSet};

mod errors {
    error_chain! { }
}
use errors::*;

const USAGE: &'static str = r"
List all authors of all dependencies of the current crate.

Usage:
  cargo authors [options]
  cargo authors (-h | --help)

Options:
  -o --output  Write machine-readable (Json) output to stdout.
  -h --help    Print this message.
";

#[derive(Serialize, Deserialize)]
struct AuthorsResult {
    entries: BTreeMap<String, HashSet<String>>,
}

impl AuthorsResult {
    fn new(v: BTreeMap<String, HashSet<String>>) -> Self {
        AuthorsResult { entries: v }
    }
}

struct DependencyAccumulator<'a> {
    config: &'a Config,
}

type Aggregate = Result<BTreeMap<String, HashSet<String>>>;

impl<'a> DependencyAccumulator<'a> {
    fn new(c: &'a Config) -> Self {
        DependencyAccumulator { config: c }
    }

    fn accumulate(&self) -> Aggregate {
        let local_root = Path::new(".").canonicalize()
            .chain_err(|| "Failed to canonicalize local root path.")?;
        let local_root = local_root.as_path();
        let ws_path = local_root.join("Cargo.toml");
        let ws = Workspace::new(&ws_path, self.config)
            .chain_err(|| 
            "Failed creating new Workspace instance. Maybe you're not in a \
            directory with a valid Cargo.toml file?")?;

        // here starts the code ripped from cargo::ops::cargo_output_metadata.rs
        // because the visibility of the result returned from metadata_full()
        // hindered evaluation
        let specs = Packages::All.into_package_id_specs(&ws)
            .chain_err(|| "Failed getting list of packages.")?;
        let deps = ops::resolve_ws_precisely(&ws,
                                             None,
                                             &[],
                                             false,
                                             false,
                                             &specs)
            .chain_err(|| "Failed resolving Workspace.")?;
        let (packages, _resolve) = deps;

        let packages = packages.package_ids()
            .map(|i| packages.get(i).map(|p| p.clone()))
            .collect::<CargoResult<Vec<_>>>().chain_err(|| 
            "Failed collecting packages from package IDs.")?;
        // here ends the ripped code

        let mut result: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        for package in packages {
             let name = package.name();
             let authors = package.authors().clone();
             for auth in authors {
                 let crates = result.entry(auth).or_insert_with(HashSet::new);
                 crates.insert(name.to_string());
             }
        }

        Ok(result)
    }
}

#[derive(RustcDecodable)]
struct Flags {
    flag_output: bool,
}

fn real_main(flags: Flags, config: &Config) -> CliResult {
    let aggregate = match DependencyAccumulator::new(config).accumulate() {
        Err(ref e) => {
            println!("error: {}", e);

            for e in e.iter().skip(1) {
                println!("caused by: {}", e);
            }

            if let Some(backtrace) = e.backtrace() {
                println!("backtrace: {:?}", backtrace);
            }

            ::std::process::exit(1);
        },
        Ok(agg) => agg,
    };

    let max_author_len = aggregate.keys().map(|e| e.len()).max()
        .expect("No authors found.");

    if flags.flag_output {
        let ar = AuthorsResult::new(aggregate);
        let ar_json = serde_json::to_string(&ar).unwrap();
        println!("{}", ar_json);
    } else {
        println!("Authors and their respective crates for this crate:\n");

        for (author, crates) in aggregate {
            println!("{:<N$}:{:?}", author, crates, N = max_author_len);
        }
    }

    Ok(())
}

fn main() {
    let config = match Config::default() {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut shell = cargo::shell(Verbosity::Verbose, ColorConfig::Auto);
            cargo::exit_with_error(e.into(), &mut shell)
        }
    };

    let result = (|| {
        let args: Vec<_> = try!(env::args_os()
            .map(|s| {
                s.into_string().map_err(|s| 
                human(format!("invalid unicode in argument: {:?}", s)))
            })
            .collect());
        let rest = &args;
        
        let flags = Docopt::new(USAGE)
            .and_then(|d| 
                d.argv(rest.into_iter()).help(true).decode::<Flags>()
            ).unwrap_or_else(|e| {
                e.exit()
            }
        );

        real_main(flags, &config)
    })();

    match result {
        Err(e) => cargo::exit_with_error(e, &mut *config.shell()),
        Ok(()) => {}
    }
}
