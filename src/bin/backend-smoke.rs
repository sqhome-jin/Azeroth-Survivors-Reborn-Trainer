//! Lightweight smoke checks for CI without launching the SwiftUI host.
fn main() {
    let mut args = std::env::args().skip(1);
    let Some(flag) = args.next() else {
        eprintln!("usage: backend-smoke --smoke-test");
        std::process::exit(2);
    };
    match flag.as_str() {
        "--smoke-test" => {
            let core = env!("CARGO_PKG_VERSION")
                .split('-')
                .next()
                .expect("package version");
            reborn_trainer::parse_version(core).expect("trainer package version should be valid");
            let backend = reborn_trainer::Backend::create();
            let _ = backend.stats_metadata(false);
            let _ = backend.discover_sync();
            println!("ok {}", env!("CARGO_PKG_VERSION"));
        }
        "--smoke-test-release" => {
            let expected = args.next().expect("expected release tag");
            let status = reborn_trainer::check_update("0.0.0").expect("release check");
            if status.tag != expected || !status.newer {
                std::process::exit(1);
            }
            if status.asset_url.is_none() {
                std::process::exit(1);
            }
            println!("ok {}", status.tag);
        }
        other => {
            eprintln!("unknown flag: {other}");
            std::process::exit(2);
        }
    }
}
