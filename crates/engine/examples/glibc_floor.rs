//! Release check: fails when a gnu build needs a newer glibc than the floor
//! the installers promise (`min-glibc-version` in `dist-workspace.toml`).
//! `.github/workflows/glibc-floor.yml` runs it on every release.
//!
//! cargo run -p shipdeck-engine --example glibc_floor -- 2.28 path/to/shipdeck...

use std::path::Path;
use std::process::ExitCode;

use shipdeck_engine::build::{GlibcVersion, inspect};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let floor = args.next().as_deref().and_then(GlibcVersion::parse);
    let binaries: Vec<String> = args.collect();
    let Some(floor) = floor.filter(|_| !binaries.is_empty()) else {
        eprintln!("usage: glibc_floor <floor, such as 2.28> <binary>...");
        return ExitCode::from(2);
    };

    let mut ok = true;
    for path in &binaries {
        match inspect(Path::new(path)) {
            Ok(info) => match info.glibc {
                Some(needs) if needs > floor => {
                    eprintln!("{path}: needs glibc {needs}, newer than the {floor} floor");
                    ok = false;
                }
                Some(needs) => println!("{path}: needs glibc {needs}, within the {floor} floor"),
                None => {
                    eprintln!("{path}: links no glibc, so it is not a gnu build");
                    ok = false;
                }
            },
            Err(err) => {
                eprintln!("{path}: {err:#}");
                ok = false;
            }
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
