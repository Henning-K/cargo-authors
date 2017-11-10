extern crate cargo;

#[macro_use]
extern crate error_chain;

#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate docopt;

use docopt::Docopt;

use cargo::CliResult;
use cargo::util::{Config, CargoResult, CargoError};
use cargo::core::Workspace;
use cargo::core::shell::Shell;
use cargo::ops::{self, Packages};

use std::env;
use std::path::Path;
use std::collections::{BTreeMap, HashSet};

error_chain! {
    foreign_links {
        Cargo(CargoError);
        Io(::std::io::Error);
    }
}

const USAGE: &'static str = r"
List all authors of all dependencies of the current crate.

Usage:
  cargo authors [options]
  cargo authors (-h | --help)

Options:
  -h --help         Print this message.
  -j --json         Write machine-readable (JSON) output to stdout.
  -i --ignore-self  Don't output author and package name of the current crate.
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
    ignore: bool,
}

type Aggregate = Result<BTreeMap<String, HashSet<String>>>;

impl<'a> DependencyAccumulator<'a> {
    fn new(c: &'a Config, ignore: bool) -> Self {
        DependencyAccumulator { config: c, ignore: ignore }
    }

    fn accumulate(&self) -> Aggregate {
        let local_root = Path::new(".").canonicalize()?;
        let local_root = local_root.as_path();
        let ws_path = local_root.join("Cargo.toml");
        let ws = Workspace::new(&ws_path, self.config)?;

        let self_pkg = ws.current()?.name();

        // here starts the code ripped from cargo::ops::cargo_output_metadata.rs
        // because the visibility of the result returned from metadata_full()
        // hindered evaluation
        let specs = Packages::All.into_package_id_specs(&ws)?;
        let deps = ops::resolve_ws_precisely(&ws,
                                             None,
                                             &[],
                                             false,
                                             false,
                                             &specs)?;
        let (packages, _resolve) = deps;

        let packages = packages.package_ids()
            .map(|i| packages.get(i).map(|p| p.clone()))
            .collect::<CargoResult<Vec<_>>>()?;
        // here ends the ripped code

        let mut result: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        for package in packages {
             let name = package.name();
             if name == self_pkg && self.ignore {
                 continue;
             }
             let authors = package.authors().clone();
             for auth in authors {
                 let crates = result.entry(auth).or_insert_with(HashSet::new);
                 crates.insert(name.to_string());
             }
        }

        Ok(result)
    }
}

#[derive(Debug, Deserialize)]
struct Flags {
    flag_json: bool,
    flag_ignore_self: bool,
}

fn real_main(flags: Flags, config: &Config) -> CliResult {
    let aggregate = match DependencyAccumulator::new(config, flags.flag_ignore_self).accumulate() {
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

    if flags.flag_json {
        let ar = AuthorsResult::new(aggregate);
        let ar_json = serde_json::to_string(&ar).unwrap();
        println!("{}", ar_json);
    } else {
        println!("Authors and their respective crates for this crate:\n");

        for (author, crates) in aggregate {
            let crates = crates.into_iter().collect::<Vec<String>>();
            let crates = crates[..].join(", ");
            println!("{:<N$}: {}", author, crates, N = max_author_len);
        }
    }

    Ok(())
}

fn main() {
    let config = match Config::default() {
        Ok(cfg) => cfg,
        Err(e) => {
            cargo::exit_with_error(e.into(), &mut Shell::new());
        }
    };

    let result = (|| {
        let args: Vec<_> = try!(env::args_os()
            .map(|s| {
                s.into_string().map_err(|s| {
                    CargoError::from(format!("invalid unicode in argument: {:?}", s))
                })
            })
            .collect());
        let rest = &args;
        
        let flags: Flags = Docopt::new(USAGE)
            .and_then(|d| d.argv(rest.into_iter()).deserialize())
            .unwrap_or_else(|e| e.exit());

        real_main(flags, &config)
    })();

    match result {
        Err(e) => cargo::exit_with_error(e, &mut *config.shell()),
        Ok(()) => {}
    }
}
