//! ```cargo
//! [dependencies]
//! semver = "1"
//! ```

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: bump-version <version> <major|minor|patch>");
        exit(1);
    }

    let version_str = &args[1];
    let level = &args[2];

    let mut version = match version_str.parse::<semver::Version>() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Invalid version '{version_str}': {e}");
            exit(1);
        }
    };

    match level.as_str() {
        "major" => {
            version.major += 1;
            version.minor = 0;
            version.patch = 0;
        }
        "minor" => {
            version.minor += 1;
            version.patch = 0;
        }
        "patch" => {
            version.patch += 1;
        }
        _ => {
            eprintln!("Invalid bump level '{level}'. Expected major, minor, or patch.");
            exit(1);
        }
    }

    print!("{version}");
}
